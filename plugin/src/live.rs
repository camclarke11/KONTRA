//! What the player plays, shown as they play it. The notes and sustain
//! pedal of the host's MIDI (a MIDI keyboard, and the standalone's
//! computer keyboard, which it sends on the same port) and of the editor's
//! keyboard, stamped as they arrive ([`Live`]) and gathered by the editor
//! ([`Tracker`]) into notes on a clock of its own, which the keyboard strip
//! draws rising from its keys ([`kontra_ui::waterfall::Played`]). Not a
//! song playing: its notes fall onto the keys already.

use std::time::{Duration, Instant};

use crossbeam_queue::ArrayQueue;
use kontra_host::rack::Event;
use kontra_ui::waterfall::{AHEAD, Note, Notes, Pedal, Played};

/// Hits the queue holds between two frames; past it, the oldest go. An
/// editor that is closed reads none, so it keeps only the latest.
const QUEUE: usize = 1024;

/// How long a note stays after it ends: it rises out of view, and its
/// sparks and its key's light fade, in this.
const KEEP: f64 = AHEAD + 1.5;

/// One thing the player did.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    /// A key down, at 1..=127.
    On(u8, u8),
    Off(u8),
    /// The sustain pedal (CC64) moved to 0..=127.
    Pedal(u8),
    /// All notes off or all sound off: every key up.
    Silence,
}

/// `e` as what the player did, if it is one of those.
fn hit(e: &Event) -> Option<Hit> {
    let (_, [status, a, b]) = crate::record::message(e)?;
    Some(match status & 0xF0 {
        0x90 if b > 0 => Hit::On(a, b),
        0x80 | 0x90 => Hit::Off(a),
        0xB0 if a == 64 => Hit::Pedal(b),
        0xB0 if a == 120 || a == 123 => Hit::Silence,
        _ => return None,
    })
}

/// What the audio thread and the editor's keyboard hand the editor.
pub struct Live {
    queue: ArrayQueue<(Instant, Hit)>,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            queue: ArrayQueue::new(QUEUE),
        }
    }
}

impl Live {
    /// On the audio thread, as a block starts, before a song's events join
    /// `input`: what the player played this block, each at its frame from
    /// now. The clock is read only for a block that holds something.
    pub fn process(&self, rate: f64, input: &[(u32, Event)]) {
        let mut now = None;
        for (at, e) in input {
            let Some(hit) = hit(e) else { continue };
            let start = *now.get_or_insert_with(Instant::now);
            let into = Duration::from_secs_f64(f64::from(*at) / rate.max(1.0));
            self.queue.force_push((start + into, hit));
        }
    }

    /// The editor's keyboard played `e`.
    pub fn keyboard(&self, e: &Event) {
        if let Some(hit) = hit(e) {
            self.queue.force_push((Instant::now(), hit));
        }
    }

    /// Whether something waits for the editor: it draws a frame for it.
    pub fn waiting(&self) -> bool {
        !self.queue.is_empty()
    }
}

/// The editor's side: the notes played lately, on a clock from when it
/// opened.
pub struct Tracker {
    origin: Instant,
    /// Notes that ended, in the order they did.
    ended: Vec<Note>,
    /// Each key held: when it went down, how hard.
    held: [Option<(f64, u8)>; 128],
    /// The pedal's moves: (seconds, 0, value), as [`Pedal::new`] reads them.
    pedal: Vec<(f64, u8, u8)>,
    /// The last thing played, in seconds.
    last: Option<f64>,
    /// Whether a note has been played yet.
    begun: bool,
    /// The notes as last built; built again only when they change.
    shown: Notes,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl Tracker {
    #[must_use]
    pub fn new(origin: Instant) -> Self {
        Self {
            origin,
            ended: Vec::new(),
            held: [None; 128],
            pedal: Vec::new(),
            last: None,
            begun: false,
            shown: Notes::default(),
        }
    }

    fn seconds(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.origin).as_secs_f64()
    }

    /// What the player played, as the editor draws it at `now`: what
    /// `live` handed over since the last frame joins it, and what has
    /// risen out of view leaves.
    pub fn played(&mut self, live: &Live, now: Instant) -> Played {
        let at = self.seconds(now);
        let mut changed = false;
        while let Some((when, hit)) = live.queue.pop() {
            let t = self.seconds(when);
            self.hit(t, hit);
            changed = true;
        }
        changed |= self.forget(at - KEEP);
        if changed {
            self.shown = self.notes();
        }
        Played {
            notes: self.shown.clone(),
            at,
            begun: self.begun,
        }
    }

    fn hit(&mut self, t: f64, hit: Hit) {
        self.last = Some(t);
        match hit {
            Hit::On(key, velocity) => {
                self.begun = true;
                // A key struck again without letting go: the first ends.
                self.release(key, t);
                if let Some(h) = self.held.get_mut(usize::from(key)) {
                    *h = Some((t, velocity.clamp(1, 127)));
                }
            }
            Hit::Off(key) => self.release(key, t),
            Hit::Pedal(value) => self.pedal.push((t, 0, value)),
            Hit::Silence => self.silence(t),
        }
    }

    fn release(&mut self, key: u8, t: f64) {
        let Some((start, velocity)) = self.held.get_mut(usize::from(key)).and_then(Option::take)
        else {
            return;
        };
        self.ended.push(Note {
            start,
            end: t.max(start),
            key,
            velocity,
            voice: 0,
        });
    }

    fn silence(&mut self, t: f64) {
        for key in 0..128 {
            self.release(key, t);
        }
        self.pedal.push((t, 0, 0));
    }

    /// Lets go of what ended before `before`, keeping the pedal's state
    /// there; whether anything went.
    fn forget(&mut self, before: f64) -> bool {
        let notes = self.ended.len();
        self.ended.retain(|n| n.end >= before);
        // The last move before the cut says how the pedal stood at it.
        let gone = self.pedal.partition_point(|p| p.0 < before);
        let drop = gone.saturating_sub(1);
        self.pedal.drain(..drop);
        notes != self.ended.len() || drop > 0
    }

    /// The notes to draw: those that ended, and those held, which end
    /// nowhere yet.
    fn notes(&self) -> Notes {
        let held = (0u8..128).zip(self.held.iter()).filter_map(|(key, h)| {
            let (start, velocity) = (*h)?;
            Some(Note {
                start,
                end: f64::INFINITY,
                key,
                velocity,
                voice: 0,
            })
        });
        let all = self.ended.iter().copied().chain(held).collect();
        Notes::new(all).with_pedal(Pedal::new(self.pedal.iter().copied()))
    }

    /// Whether what it shows at `now` still moves: a key held (it grows
    /// and sparks), or something played lately still rising.
    #[must_use]
    pub fn moving(&self, now: Instant) -> bool {
        let at = self.seconds(now);
        self.held.iter().any(Option::is_some) || self.last.is_some_and(|t| at - t < KEEP)
    }

    /// Panic: every key up, the pedal too, at `now`.
    pub fn clear(&mut self, now: Instant) {
        let t = self.seconds(now);
        self.silence(t);
        self.shown = self.notes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kontra_host::rack::{HostNote, HostPattern};

    fn on(key: u8, velocity: f64) -> Event {
        Event::NoteOn {
            note: HostNote {
                port: 0,
                channel: 0,
                key,
                id: -1,
                clap: false,
            },
            velocity,
            tune: 0.0,
        }
    }

    fn off(key: u8) -> Event {
        Event::NoteOff(HostPattern {
            port: 0,
            channel: 0,
            key: i32::from(key),
            id: -1,
            clap: false,
        })
    }

    fn cc(controller: u8, value: u8) -> Event {
        // A MIDI 1.0 channel voice packet on group 0, channel 0.
        let word = 0x2000_0000 | 0xB0 << 16 | u32::from(controller) << 8 | u32::from(value);
        Event::Ump([word, 0])
    }

    fn secs(origin: Instant, s: f64) -> Instant {
        origin + Duration::from_secs_f64(s)
    }

    #[test]
    fn what_the_player_plays_becomes_notes_a_held_one_ending_nowhere() {
        let origin = Instant::now();
        let live = Live::default();
        let mut t = Tracker::new(origin);
        assert!(!t.played(&live, origin).begun, "nothing played yet");
        // A block at 48 kHz holding a C struck and an E struck a quarter
        // second in; the C let go in the next.
        live.process(
            48_000.0,
            &[(0, on(60, 100.0 / 127.0)), (12_000, on(64, 0.5))],
        );
        assert!(live.waiting());
        let shown = t.played(&live, secs(origin, 1.0));
        assert!(!live.waiting());
        assert!(shown.begun);
        let notes: Vec<Note> = shown.notes.between(0.0, f64::INFINITY).copied().collect();
        assert_eq!(notes.len(), 2);
        let (c, e) = (notes[0], notes[1]);
        assert_eq!((c.key, c.velocity, e.key, e.velocity), (60, 100, 64, 64));
        assert!((e.start - c.start - 0.25).abs() < 0.01, "{c:?} {e:?}");
        assert!(c.end.is_infinite() && e.end.is_infinite(), "both held");
        assert!(t.moving(secs(origin, 30.0)), "a held key keeps moving");
        live.keyboard(&off(60));
        let shown = t.played(&live, secs(origin, 1.0));
        let c = *shown
            .notes
            .between(0.0, f64::INFINITY)
            .find(|n| n.key == 60)
            .unwrap();
        assert!(c.end.is_finite() && c.end >= c.start, "{c:?}");
        let lit: Vec<u8> = shown.notes.sounding(shown.at).map(|n| n.key).collect();
        assert_eq!(lit, [64], "only the E still sounds");
    }

    #[test]
    fn the_pedal_and_silence_and_what_rose_out_of_view() {
        let origin = Instant::now();
        let live = Live::default();
        let mut t = Tracker::new(origin);
        live.process(
            48_000.0,
            &[(0, on(48, 0.8)), (0, cc(64, 127)), (0, cc(1, 90))],
        );
        let shown = t.played(&live, secs(origin, 0.5));
        assert!(
            shown.notes.pedal().depth(shown.at) > 0.99,
            "the pedal is down"
        );
        // Panic: every key up and the pedal with them.
        t.clear(secs(origin, 1.0));
        let shown = t.played(&live, secs(origin, 1.0));
        assert_eq!(shown.notes.sounding(shown.at).count(), 0);
        assert!(shown.notes.pedal().depth(shown.at) <= 0.0);
        assert!(t.moving(secs(origin, 2.0)), "still rising");
        // All notes off does as Panic does.
        live.process(48_000.0, &[(0, on(50, 0.8)), (0, cc(123, 0))]);
        let shown = t.played(&live, secs(origin, 2.0));
        assert_eq!(shown.notes.sounding(shown.at).count(), 0);
        // Long after, everything has risen away and nothing moves.
        let later = secs(origin, 2.0 + KEEP + 1.0);
        assert!(!t.moving(later));
        let shown = t.played(&live, later);
        assert!(shown.notes.is_empty());
        assert!(shown.notes.pedal().is_empty());
    }

    #[test]
    fn a_full_queue_keeps_the_latest() {
        let origin = Instant::now();
        let live = Live::default();
        let mut t = Tracker::new(origin);
        let mut block: Vec<(u32, Event)> = Vec::new();
        for n in 0..(QUEUE as u32 + 12) {
            let key = (n % 2) as u8 + 60;
            block.push((n, if n % 4 < 2 { on(key, 0.5) } else { off(key) }));
        }
        live.process(48_000.0, &block);
        let shown = t.played(&live, secs(origin, 1.0));
        assert_eq!(shown.notes.sounding(shown.at).count(), 0, "no key stuck");
    }
}
