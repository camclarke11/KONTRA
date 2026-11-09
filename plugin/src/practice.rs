//! Practice ("wait") mode: the song waits at each note the player has to
//! play until they play it, on their MIDI keyboard (or the computer's, or
//! the editor's). They choose whose notes are theirs ([`Hands`]): the song
//! does not play those, and plays the rest along with them.
//!
//! The song's clock asks the [`Gate`] how far it may go each block
//! ([`crate::song`]); the gate stops it at the next chord the player has
//! not played. A chord is the hand's notes starting within [`CHORD`] of its
//! first, since a pianist's chord is rarely struck at one instant (nor is a
//! performance file's). Each of its keys struck since the chord before was
//! played counts, still held or not; a key struck that is neither in it
//! nor sounding in the hand is wrong: it is shown while held, and holds
//! nothing up.
//!
//! The editor and the audio thread share [`Practice`]: the hands chosen,
//! the editor keyboard's presses, and what the gate shows (the keys wanted,
//! struck and wrong) in atomics.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};

use crossbeam_queue::ArrayQueue;
use kontra_host::rack::Event;
use kontra_ui::practice::Hands;
use kontra_ui::waterfall::{Note, Notes};

/// Notes of the hand starting this close after a chord's first are in it.
pub const CHORD: f64 = 0.06;

/// With one voice the hands split here: middle C and up is the right's.
const SPLIT: u8 = 60;

/// A set of keys, a bit each.
type Keys = u128;

fn bit(key: u8) -> Keys {
    1 << (key & 127)
}

fn keys(set: Keys) -> Vec<u8> {
    (0..128u8).filter(|&k| set & bit(k) != 0).collect()
}

/// Whether a note of `voice` on `key` is the player's to play.
fn mine(hands: Hands, voices: u8, voice: u8, key: u8) -> bool {
    match hands {
        Hands::Off => false,
        Hands::Both => true,
        Hands::Right if voices < 2 => key >= SPLIT,
        Hands::Left if voices < 2 => key < SPLIT,
        Hands::Right => voice == 0,
        Hands::Left => voice == 1,
    }
}

/// What the editor and the audio thread share.
pub struct Practice {
    hands: AtomicU8,
    /// The editor keyboard's keys, pressed (true) and let go.
    presses: ArrayQueue<(u8, bool)>,
    waiting: AtomicBool,
    /// The keys wanted, struck and wrong, each in two halves.
    want: [AtomicU64; 2],
    struck: [AtomicU64; 2],
    wrong: [AtomicU64; 2],
    /// Moves whenever what is shown changes; the editor draws a frame.
    revision: AtomicU32,
    seen: AtomicU32,
}

impl Default for Practice {
    fn default() -> Self {
        Self {
            hands: AtomicU8::new(0),
            presses: ArrayQueue::new(64),
            waiting: AtomicBool::new(false),
            want: Default::default(),
            struck: Default::default(),
            wrong: Default::default(),
            revision: AtomicU32::new(0),
            seen: AtomicU32::new(0),
        }
    }
}

fn store(cell: &[AtomicU64; 2], set: Keys) {
    cell[0].store(set as u64, Ordering::Relaxed);
    cell[1].store((set >> 64) as u64, Ordering::Relaxed);
}

fn load(cell: &[AtomicU64; 2]) -> Keys {
    Keys::from(cell[0].load(Ordering::Relaxed)) | Keys::from(cell[1].load(Ordering::Relaxed)) << 64
}

impl Practice {
    pub fn hands(&self) -> Hands {
        let n = self.hands.load(Ordering::Acquire);
        Hands::ALL.get(usize::from(n)).copied().unwrap_or_default()
    }

    /// Whose notes the player plays from the next block on.
    pub fn set_hands(&self, hands: Hands) {
        let n = Hands::ALL.iter().position(|h| *h == hands).unwrap_or(0);
        self.hands
            .store(u8::try_from(n).unwrap_or(0), Ordering::Release);
        self.revision.fetch_add(1, Ordering::Release);
    }

    /// The editor keyboard pressed (`on`) or let go of `key`; for the gate
    /// at the next block, when the rack plays it too.
    pub fn press(&self, key: u8, on: bool) {
        let _ = self.presses.force_push((key, on));
    }

    /// What the editor shows.
    pub fn shown(&self) -> kontra_ui::practice::Practice {
        kontra_ui::practice::Practice {
            hands: self.hands(),
            waiting: self.waiting.load(Ordering::Relaxed),
            want: keys(load(&self.want)),
            struck: keys(load(&self.struck)),
            wrong: keys(load(&self.wrong)),
        }
    }

    /// Whether what is shown moved since the editor last asked.
    pub fn changed(&self) -> bool {
        let now = self.revision.load(Ordering::Acquire);
        self.seen.swap(now, Ordering::AcqRel) != now
    }

    /// From the audio thread: what the gate shows now.
    fn show(&self, waiting: bool, want: Keys, struck: Keys, wrong: Keys) {
        let same = self.waiting.load(Ordering::Relaxed) == waiting
            && load(&self.want) == want
            && load(&self.struck) == struck
            && load(&self.wrong) == wrong;
        if same {
            return;
        }
        self.waiting.store(waiting, Ordering::Relaxed);
        store(&self.want, want);
        store(&self.struck, struck);
        store(&self.wrong, wrong);
        self.revision.fetch_add(1, Ordering::Release);
    }

    /// No song: nothing to wait for or mark.
    pub fn clear(&self) {
        while self.presses.pop().is_some() {}
        self.show(false, 0, 0, 0);
    }
}

/// The chord the song waits for next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chord {
    keys: Keys,
    /// The song's frame its first note starts on: where the song waits.
    frame: u64,
    /// Into the notes, past its last.
    end: usize,
}

/// A song's practice on the audio thread: the player's notes, the chord the
/// song waits for and the keys they hold. Built off the audio thread with
/// the song ([`Gate::new`]); it allocates nothing as it runs.
#[derive(Debug, Default)]
pub struct Gate {
    /// The song's notes in start order.
    notes: Box<[Note]>,
    /// How many voices the song has (tracks and channels with notes).
    voices: u8,
    /// The longest note: how far back a note sounding now can start.
    longest: f64,
    hands: Hands,
    chord: Option<Chord>,
    struck: Keys,
    /// The keys of the chord played last, which the player may still hold.
    before: Keys,
    /// The keys the player holds.
    held: Keys,
    /// The song moved on from a wait last block: the editor is told again,
    /// so it sees the song moving and follows it.
    resumed: bool,
}

impl Gate {
    /// The gate for a song of `notes`.
    pub fn new(notes: &Notes) -> Self {
        let notes: Box<[Note]> = notes
            .between(f64::NEG_INFINITY, f64::INFINITY)
            .copied()
            .collect();
        let voices = notes
            .iter()
            .map(|n| n.voice)
            .max()
            .map_or(0, |v| v.saturating_add(1));
        let longest = notes.iter().map(|n| n.end - n.start).fold(0., f64::max);
        Self {
            notes,
            voices,
            longest,
            ..Self::default()
        }
    }

    fn mine(&self, note: &Note) -> bool {
        mine(self.hands, self.voices, note.voice, note.key)
    }

    /// Whether the song plays `message` of `voice` itself: not a note of
    /// the player's.
    pub fn plays(&self, voice: u8, [status, key, _]: [u8; 3]) -> bool {
        !matches!(status & 0xF0, 0x80 | 0x90) || !mine(self.hands, self.voices, voice, key)
    }

    /// The chord whose first note is the hand's first from note `from` on.
    fn chord_from(&self, from: usize, rate: f64) -> Option<Chord> {
        let rest = self.notes.get(from..)?;
        let first = rest.iter().position(|n| self.mine(n))? + from;
        let start = self.notes.get(first)?.start;
        let end = (self.notes.get(first..)?.iter())
            .position(|n| n.start >= start + CHORD)
            .map_or(self.notes.len(), |n| n + first);
        let keys = (self.notes.get(first..end)?.iter())
            .filter(|n| self.mine(n))
            .fold(0, |k, n| k | bit(n.key));
        Some(Chord {
            keys,
            frame: (start * rate).round() as u64,
            end,
        })
    }

    /// The song is at `frame`: the chord it meets next, from there.
    fn find(&mut self, frame: u64, rate: f64) {
        let from = self
            .notes
            .partition_point(|n| ((n.start * rate).round() as u64) < frame);
        self.chord = self.chord_from(from, rate);
        self.struck = 0;
        self.before = 0;
    }

    /// The player struck `key`: true when it completes the chord.
    fn strike(&mut self, key: u8, rate: f64) -> bool {
        self.held |= bit(key);
        let Some(chord) = self.chord else {
            return false;
        };
        if chord.keys & bit(key) == 0 {
            return false;
        }
        self.struck |= bit(key);
        if chord.keys & !self.struck != 0 {
            return false;
        }
        self.before = chord.keys;
        self.struck = 0;
        self.chord = self.chord_from(chord.end, rate);
        true
    }

    /// The keys of the hand's notes sounding at `at` seconds.
    fn sounding(&self, at: f64) -> Keys {
        let last = self.notes.partition_point(|n| n.start <= at);
        let first = self.notes.partition_point(|n| n.start < at - self.longest);
        (self.notes.get(first..last).unwrap_or_default().iter())
            .filter(|n| n.end > at && self.mine(n))
            .fold(0, |k, n| k | bit(n.key))
    }

    /// How the song's clock moves this block: the frames of it the song
    /// stands before it moves (all of them while it waits for the player)
    /// and the song's frame at the block's end. The song is at `frame`;
    /// `input` is the block's events from the player.
    pub fn clock(
        &mut self,
        practice: &Practice,
        input: &[(u32, Event)],
        frame: u64,
        frames: u64,
        rate: f64,
    ) -> (u64, u64) {
        let hands = practice.hands();
        if hands != self.hands {
            self.hands = hands;
            self.find(frame, rate);
        }
        if hands == Hands::Off {
            self.chord = None;
            practice.show(false, 0, 0, 0);
            return (0, frame + frames);
        }
        // Past the chord: the song was moved on (a seek), or began late.
        if self.chord.is_some_and(|c| c.frame < frame) {
            self.find(frame, rate);
        }
        let waited = self.chord.is_some_and(|c| c.frame == frame);
        // The frame of the block the chord was completed on, if it was.
        let mut played = None;
        while let Some((key, on)) = practice.presses.pop() {
            if on {
                if self.strike(key, rate) {
                    played = played.or(Some(0));
                }
            } else {
                self.held &= !bit(key);
            }
        }
        for &(at, ref e) in input {
            match *e {
                Event::NoteOn { note, .. } => {
                    if self.strike(note.key, rate) {
                        played = played.or(Some(u64::from(at)));
                    }
                }
                Event::NoteOff(p) | Event::NoteOffVelocity(p, _) => {
                    if let Ok(key) = u8::try_from(p.key) {
                        self.held &= !bit(key);
                    }
                }
                _ => {}
            }
        }
        if std::mem::take(&mut self.resumed) {
            practice.revision.fetch_add(1, Ordering::Release);
        }
        self.resumed = waited && played.is_some();
        let idle = match (waited, played) {
            (true, Some(at)) => at.min(frames),
            (true, None) => frames,
            (false, _) => 0,
        };
        let mut end = frame + (frames - idle);
        if let Some(c) = self.chord {
            end = end.min(c.frame.max(frame));
        }
        let waiting = self.chord.is_some_and(|c| c.frame == end);
        let want = self.chord.map_or(0, |c| c.keys);
        let ok = want | self.before | self.sounding(end as f64 / rate.max(1.0));
        practice.show(waiting, want, self.struck, self.held & !ok);
        (idle, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kontra_host::rack::{HostNote, HostPattern};

    const RATE: f64 = 1000.0;

    fn note(start: f64, key: u8, voice: u8) -> Note {
        Note {
            start,
            end: start + 0.4,
            key,
            velocity: 100,
            voice,
        }
    }

    /// Right hand (voice 0): C at 1 s, E+G at 2 s (G 30 ms late, one
    /// chord), C again at 3 s. Left hand (voice 1): C2 at 1 s, held long.
    fn gate(hands: Hands) -> (Gate, Practice) {
        let mut long = note(1.0, 36, 1);
        long.end = 3.5;
        let notes = Notes::new(vec![
            note(1.0, 60, 0),
            long,
            note(2.0, 64, 0),
            note(2.03, 67, 0),
            note(3.0, 60, 0),
        ]);
        let practice = Practice::default();
        practice.set_hands(hands);
        (Gate::new(&notes), practice)
    }

    fn on(at: u32, key: u8) -> (u32, Event) {
        let note = HostNote {
            port: 0,
            channel: 0,
            key,
            id: -1,
            clap: false,
        };
        (
            at,
            Event::NoteOn {
                note,
                velocity: 0.8,
                tune: 0.0,
            },
        )
    }

    fn off(at: u32, key: u8) -> (u32, Event) {
        let p = HostPattern {
            port: 0,
            channel: 0,
            key: i32::from(key),
            id: -1,
            clap: false,
        };
        (at, Event::NoteOff(p))
    }

    /// Blocks of 100 frames (0.1 s) from `frame`, each with `input`; the
    /// song's frame after them.
    fn run(g: &mut Gate, p: &Practice, mut frame: u64, blocks: usize) -> u64 {
        for _ in 0..blocks {
            frame = g.clock(p, &[], frame, 100, RATE).1;
        }
        frame
    }

    #[test]
    fn the_song_waits_at_the_players_note_until_it_is_played() {
        let (mut g, p) = gate(Hands::Right);
        // It runs up to the first note at 1 s and stands there.
        let frame = run(&mut g, &p, 0, 30);
        assert_eq!(frame, 1000);
        let shown = p.shown();
        assert!(shown.waiting);
        assert_eq!(shown.want, [60]);
        // A wrong key holds it, and shows.
        let (idle, end) = g.clock(&p, &[on(10, 62)], frame, 100, RATE);
        assert_eq!((idle, end), (100, 1000));
        assert_eq!(p.shown().wrong, [62]);
        // Letting go of it clears it.
        g.clock(&p, &[off(0, 62)], frame, 100, RATE);
        assert!(p.shown().wrong.is_empty());
        // The right key, 40 frames into a block: the song moves on from
        // there, up to the next chord.
        let (idle, end) = g.clock(&p, &[on(40, 60)], frame, 100, RATE);
        assert_eq!((idle, end), (40, 1060));
        assert!(!p.shown().waiting);
        assert_eq!(p.shown().want, [64, 67], "the next chord");
        // Still holding C as it moves on is not wrong.
        assert!(p.shown().wrong.is_empty());
    }

    #[test]
    fn a_chord_needs_each_of_its_keys_in_any_order() {
        let (mut g, p) = gate(Hands::Right);
        let mut frame = run(&mut g, &p, 0, 30);
        frame = g.clock(&p, &[on(0, 60)], frame, 100, RATE).1;
        frame = run(&mut g, &p, frame, 20);
        assert_eq!(frame, 2000, "waits at the chord's first note");
        // G first, then E a block later: only then does it move.
        assert_eq!(g.clock(&p, &[on(0, 67)], frame, 100, RATE), (100, 2000));
        assert_eq!(p.shown().struck, [67]);
        assert_eq!(g.clock(&p, &[on(5, 64)], frame, 100, RATE), (5, 2095));
        // The same key again later is struck again: C at 3 s waits.
        frame = run(&mut g, &p, 2095, 20);
        assert_eq!(frame, 3000);
        assert_eq!(p.shown().want, [60]);
    }

    #[test]
    fn a_chord_played_early_lets_the_song_through_without_waiting() {
        let (mut g, p) = gate(Hands::Right);
        let frame = run(&mut g, &p, 0, 5);
        assert_eq!(frame, 500);
        // C struck half a second early: the song does not stop at 1 s.
        let (idle, end) = g.clock(&p, &[on(0, 60)], frame, 100, RATE);
        assert_eq!((idle, end), (0, 600));
        assert_eq!(run(&mut g, &p, end, 10), 1600);
    }

    #[test]
    fn the_song_plays_the_other_hand_and_not_the_players() {
        let (mut g, p) = gate(Hands::Left);
        g.clock(&p, &[], 0, 100, RATE);
        assert!(!g.plays(1, [0x90, 36, 100]));
        assert!(!g.plays(1, [0x80, 36, 0]));
        assert!(g.plays(0, [0x90, 60, 100]));
        // The pedal and other messages always play.
        assert!(g.plays(1, [0xB0, 64, 127]));
        // The left hand's one note at 1 s, held to 3.5 s: holding it past
        // the end of the song's notes is not wrong.
        let frame = run(&mut g, &p, 0, 30);
        assert_eq!(frame, 1000);
        g.clock(&p, &[on(0, 36)], frame, 100, RATE);
        assert_eq!(run(&mut g, &p, 1000, 30), 4000, "nothing left to wait for");
        assert!(p.shown().wrong.is_empty() && p.shown().want.is_empty());
    }

    #[test]
    fn a_held_note_of_the_hand_is_not_wrong() {
        let (mut g, p) = gate(Hands::Both);
        let frame = run(&mut g, &p, 0, 30);
        assert_eq!(p.shown().want, [36, 60]);
        let frame = g.clock(&p, &[on(0, 36), on(0, 60)], frame, 100, RATE).1;
        let frame = run(&mut g, &p, frame, 20);
        assert_eq!(frame, 2000);
        // C2 still held (its note sounds to 3.5 s), C3 let go: nothing wrong.
        g.clock(&p, &[off(0, 60)], frame, 100, RATE);
        assert!(p.shown().wrong.is_empty());
    }

    #[test]
    fn off_plays_through_and_choosing_a_hand_mid_song_finds_its_next_note() {
        let (mut g, p) = gate(Hands::Off);
        assert_eq!(g.clock(&p, &[], 0, 100, RATE), (0, 100));
        assert!(g.plays(0, [0x90, 60, 100]));
        let frame = run(&mut g, &p, 0, 15);
        assert_eq!(frame, 1500);
        p.set_hands(Hands::Right);
        // Past the first C: the next is the chord at 2 s.
        assert_eq!(run(&mut g, &p, frame, 10), 2000);
        assert_eq!(p.shown().want, [64, 67]);
        // A seek past it finds the one after.
        assert_eq!(run(&mut g, &p, 2500, 10), 3000);
        assert_eq!(p.shown().want, [60]);
    }

    #[test]
    fn one_voice_splits_its_hands_at_middle_c() {
        let notes = Notes::new(vec![note(0.5, 48, 0), note(1.0, 72, 0)]);
        let mut g = Gate::new(&notes);
        let p = Practice::default();
        p.set_hands(Hands::Right);
        assert_eq!(run(&mut g, &p, 0, 20), 1000);
        assert!(g.plays(0, [0x90, 48, 100]));
        assert!(!g.plays(0, [0x90, 72, 100]));
        // The editor keyboard plays too.
        p.press(72, true);
        assert_eq!(run(&mut g, &p, 1000, 1), 1100);
    }

    #[test]
    fn the_editor_redraws_when_the_marks_change() {
        let (mut g, p) = gate(Hands::Right);
        assert!(p.changed());
        assert!(!p.changed());
        run(&mut g, &p, 0, 30);
        assert!(p.changed(), "now waiting");
        run(&mut g, &p, 1000, 3);
        assert!(!p.changed(), "still waiting, nothing new");
        p.clear();
        assert!(p.changed());
        assert_eq!(p.shown().want, Vec::<u8>::new());
    }
}
