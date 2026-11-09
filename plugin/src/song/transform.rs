//! Touch-ups for rough MIDI files, applied as a song is read
//! ([`Song::read_with`]): pure functions from a song's events to new ones,
//! each applied only where [`Analysis`] says the file needs it, so a real
//! performance plays exactly as it was written.
//!
//! - [`auto_pedal`]: a file that never presses the sustain pedal gets one,
//!   laid as a pianist's foot would: down just after each new bass note or
//!   harmony, lifted and pressed again at the next, and lifted in any rest
//!   longer than about a beat.
//! - [`humanise`]: a file whose notes sit on a grid gets its chords' notes
//!   spread by a few milliseconds, the top note first (big chords rolled
//!   from the bottom up, if asked), and its velocities varied a little.
//! - [`velocity_curve`]: a file whose notes share one velocity gets one
//!   shaped by the beat, the voice and the line's rise and fall.
//!
//! Every variation comes from a hash of the note it changes, so a song
//! sounds the same each time it is read.

use kontra_library::Touch;
use kontra_ui::waterfall::{Note, Notes};

use super::Message;

/// One of a song's events: its time in seconds, its track and the message.
pub(super) type Timed = (f64, u16, Message);

// MARK: the tempo map

/// A file's tempo map: what a tick lasts from each change on.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tempo {
    /// Ticks per quarter note; `None` for SMPTE timing, which has no beat.
    division: Option<usize>,
    /// From each change on, in order (never empty): its time in seconds,
    /// its tick and the seconds a tick lasts.
    changes: Vec<(f64, usize, f64)>,
}

impl Tempo {
    /// A file `division` ticks to the quarter note, a tick lasting
    /// `per_tick` seconds until a change.
    pub(super) fn new(division: Option<usize>, per_tick: f64) -> Self {
        Self {
            division,
            changes: vec![(0.0, 0, per_tick)],
        }
    }

    /// From `tick`, `time` seconds in, a tick lasts `per_tick`.
    pub(super) fn change(&mut self, time: f64, tick: usize, per_tick: f64) {
        match self.changes.last_mut() {
            Some(last) if last.1 == tick => *last = (time, tick, per_tick),
            _ => self.changes.push((time, tick, per_tick)),
        }
    }

    /// The change in force at `time`.
    fn at(&self, time: f64) -> (f64, usize, f64) {
        let n = self.changes.partition_point(|c| c.0 <= time);
        let first = self.changes.first().copied().unwrap_or((0.0, 0, 1e-3));
        self.changes
            .get(n.wrapping_sub(1))
            .copied()
            .unwrap_or(first)
    }

    /// The tick `time` falls on.
    fn tick(&self, time: f64) -> f64 {
        let (start, tick, per_tick) = self.at(time);
        tick as f64 + (time - start) / per_tick.max(1e-12)
    }

    /// Seconds a beat (a quarter note) lasts at `time`; half a second in a
    /// file timed in SMPTE frames, which has no beat.
    fn beat(&self, time: f64) -> f64 {
        match self.division {
            Some(d) => (self.at(time).2 * d as f64).clamp(0.05, 4.0),
            None => 0.5,
        }
    }

    /// Where `time` falls in its beat, from 0 to 1; `None` without a beat.
    fn in_beat(&self, time: f64) -> Option<f64> {
        let beats = self.tick(time) / self.division? as f64;
        Some(beats - beats.floor())
    }

    /// Whether `time` sits on a grid of sixteenths, thirty-seconds or
    /// triplets, as a sequencer quantises.
    fn on_grid(&self, time: f64) -> bool {
        let Some(division) = self.division else {
            return false;
        };
        let tick = self.tick(time).round() as usize;
        [4, 8, 3, 6]
            .into_iter()
            .filter(|&n| division % n == 0 && division >= n)
            .any(|n| tick.is_multiple_of(division / n))
    }
}

// MARK: what a file needs

/// The fewest notes a file needs before it is judged quantised or played
/// at one velocity.
const JUDGED: usize = 16;

/// Onsets within this many seconds are one chord, as a sequencer writes it.
const CHORD: f64 = 0.001;

/// What a song's events say about how it was made.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Analysis {
    /// Its notes, and the tracks they are in.
    pub notes: usize,
    pub tracks: u16,
    /// It presses the sustain pedal somewhere.
    pub pedal: bool,
    /// The share of its notes that start on a grid.
    pub on_grid: f64,
    /// Its chords (onsets two or more notes share), and the share of them
    /// whose notes share one velocity.
    pub chords: usize,
    pub level_chords: f64,
    /// The share of its notes at its most common velocity.
    pub one_velocity: f64,
}

impl Analysis {
    /// It has notes and never presses the pedal.
    #[must_use]
    pub fn unpedalled(&self) -> bool {
        self.notes > 0 && !self.pedal
    }

    /// Its notes sit on a grid, or its chords are struck at one velocity:
    /// a sequencer's timing, not a hand's.
    #[must_use]
    pub fn quantised(&self) -> bool {
        self.notes >= JUDGED
            && (self.on_grid >= 0.8 || (self.chords >= 8 && self.level_chords >= 0.8))
    }

    /// Most of its notes share one velocity.
    #[must_use]
    pub fn flat(&self) -> bool {
        self.notes >= JUDGED && self.one_velocity >= 0.6
    }

    /// The touch-ups of `touch` this song needs: each only where it lacks
    /// what it adds, rolled chords only with a human hand.
    #[must_use]
    pub fn applies(&self, touch: &Touch) -> Touch {
        let humanise = touch.humanise && self.quantised();
        Touch {
            pedal: touch.pedal && self.unpedalled(),
            humanise,
            roll: touch.roll && humanise,
            velocity: touch.velocity && self.flat(),
        }
    }
}

/// What reading a song with touch-ups found and did.
#[derive(Clone, Debug)]
pub struct Touched {
    pub analysis: Analysis,
    /// The touch-ups it got.
    pub applied: Touch,
    /// Its notes as they ring, pedal and all ([`ringing`]).
    pub ringing: Notes,
}

/// What a MIDI file holds, read cheaply for the Songs tab.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct About {
    /// Seconds to its last event, as written.
    pub length: f64,
    pub analysis: Analysis,
}

/// What the Standard MIDI File in `bytes` holds, as written; Err says why
/// it cannot play.
pub fn about(bytes: &[u8]) -> Result<About, String> {
    let (timeline, tempo) = super::timeline(bytes)?;
    let timeline: Vec<Timed> = (timeline.into_iter())
        .filter(|&(_, _, [status, ..])| status & 15 != super::DRUMS)
        .collect();
    let analysis = analyse(&timeline, &tempo);
    if analysis.notes == 0 {
        return Err("it has no notes outside the drum channel (10) to play".into());
    }
    Ok(About {
        length: timeline.last().map_or(0.0, |e| e.0),
        analysis,
    })
}

fn is_on([status, _, velocity]: Message) -> bool {
    status & 0xF0 == 0x90 && velocity > 0
}

fn is_off([status, _, velocity]: Message) -> bool {
    status & 0xF0 == 0x80 || (status & 0xF0 == 0x90 && velocity == 0)
}

fn is_pedal([status, controller, _]: Message) -> bool {
    status & 0xF0 == 0xB0 && controller == 64
}

/// What `events` say about how the song was made.
pub(super) fn analyse(events: &[Timed], tempo: &Tempo) -> Analysis {
    let ons: Vec<(f64, u8)> = (events.iter())
        .filter(|e| is_on(e.2))
        .map(|&(time, _, [_, _, velocity])| (time, velocity))
        .collect();
    let mut tracks: Vec<u16> = (events.iter())
        .filter(|e| is_on(e.2))
        .map(|e| e.1)
        .collect();
    tracks.sort_unstable();
    tracks.dedup();
    let share = |n: usize| n as f64 / ons.len().max(1) as f64;
    let mut counts = [0usize; 128];
    for &(_, v) in &ons {
        if let Some(c) = counts.get_mut(usize::from(v & 127)) {
            *c += 1;
        }
    }
    let (mut chords, mut level) = (0, 0);
    for chord in groups(&ons, |&(time, _)| time, CHORD) {
        if chord.len() > 1 {
            chords += 1;
            level += usize::from(
                chord
                    .iter()
                    .all(|n| Some(&n.1) == chord.first().map(|f| &f.1)),
            );
        }
    }
    Analysis {
        notes: ons.len(),
        tracks: u16::try_from(tracks.len()).unwrap_or(u16::MAX),
        // A file that only ever lifts the pedal (a reset) never presses it.
        pedal: events.iter().any(|e| is_pedal(e.2) && e.2[2] > 0),
        on_grid: share(ons.iter().filter(|n| tempo.on_grid(n.0)).count()),
        chords,
        level_chords: level as f64 / chords.max(1) as f64,
        one_velocity: share(counts.iter().copied().max().unwrap_or(0)),
    }
}

/// `items` (in time order) cut into runs that start within `within`
/// seconds of each run's first.
fn groups<T>(items: &[T], time: impl Fn(&T) -> f64, within: f64) -> Vec<&[T]> {
    let mut out = Vec::new();
    let mut start = 0;
    for n in 1..=items.len() {
        let first = items.get(start).map(&time);
        let next = items.get(n).map(&time);
        if let (Some(first), Some(next)) = (first, next)
            && next - first <= within
        {
            continue;
        }
        if let Some(run) = items.get(start..n) {
            out.push(run);
        }
        start = n;
    }
    out
}

// MARK: notes

/// A note in a song's events: the indices of its note-on, of the note-off
/// that ends it (none when its key is struck again first, or never let
/// go), and of the note-on that strikes its key next; and when it ends.
#[derive(Clone, Copy, Debug)]
struct Span {
    on: usize,
    off: Option<usize>,
    next: Option<usize>,
    start: f64,
    end: f64,
    channel: u8,
    key: u8,
}

/// The notes of `events`, in note-on order. A note ends at its key's next
/// note-off or note-on on its channel, as the editor pairs them, or at
/// `length`.
fn spans(events: &[Timed], length: f64) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut open: Vec<Option<usize>> = vec![None; 16 * 128];
    let mut last: Vec<Option<usize>> = vec![None; 16 * 128];
    for (i, &(time, _, message @ [status, key, _])) in events.iter().enumerate() {
        if !matches!(status & 0xF0, 0x80 | 0x90) {
            continue;
        }
        let slot = usize::from(status & 15) * 128 + usize::from(key & 127);
        let (Some(open), Some(last)) = (open.get_mut(slot), last.get_mut(slot)) else {
            continue;
        };
        if let Some(s) = open.take().and_then(|s| out.get_mut(s)) {
            s.end = time;
            if is_off(message) {
                s.off = Some(i);
            }
        }
        if is_on(message) {
            if let Some(p) = last.and_then(|p| out.get_mut(p)) {
                p.next = Some(i);
            }
            *open = Some(out.len());
            *last = Some(out.len());
            out.push(Span {
                on: i,
                off: None,
                next: None,
                start: time,
                end: length,
                channel: status & 15,
                key: key & 127,
            });
        }
    }
    out
}

/// A deterministic number from 0 to 1 for the note on `channel` and `key`
/// at `time`, one per `salt`: SplitMix64 over them.
fn noise(time: f64, channel: u8, key: u8, salt: u64) -> f64 {
    let mut x = 0x4B4F_4E54_5241_u64 // "KONTRA"
        ^ time.to_bits().rotate_left(21)
        ^ (u64::from(channel) << 8 | u64::from(key)).wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// A number from -1 to 1, most often near 0: two [`noise`]s.
fn wobble(span: &Span, salt: u64) -> f64 {
    noise(span.start, span.channel, span.key, salt)
        + noise(span.start, span.channel, span.key, salt + 1)
        - 1.0
}

/// `events` with the note-ons' velocities moved by `by`, kept 1..=127.
fn with_velocities(events: &mut [Timed], spans: &[Span], by: &[f64]) {
    for (span, by) in spans.iter().zip(by) {
        if let Some((_, _, [_, _, velocity])) = events.get_mut(span.on) {
            let v = (f64::from(*velocity) + by).round().clamp(1.0, 127.0);
            *velocity = v as u8;
        }
    }
}

/// `events` in time order, those at one time in the order they came.
fn in_time_order(events: Vec<Timed>) -> Vec<Timed> {
    let mut events = events;
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    events
}

// MARK: the touch-ups

/// `events` with the touch-ups `applies` names: the velocity curve, then
/// a human hand, then a pedal laid where the notes now fall.
pub(super) fn apply(events: Vec<Timed>, tempo: &Tempo, applies: Touch) -> Vec<Timed> {
    let mut events = events;
    if applies.velocity {
        events = velocity_curve(&events, tempo);
    }
    if applies.humanise {
        events = humanise(&events, applies.roll);
    }
    if applies.pedal {
        events = auto_pedal(&events, tempo);
    }
    events
}

/// The latest a chord's lower notes follow its top note, in seconds.
const SPREAD: f64 = 0.010;
/// The least they follow it by.
const SPREAD_LEAST: f64 = 0.002;
/// The fewest notes of a chord that is rolled, when chords roll.
const ROLL_NOTES: usize = 4;
/// A rolled chord's notes are at most this far apart, and all of them
/// within [`ROLL_SPAN`] of the first.
const ROLL_STEP: f64 = 0.012;
const ROLL_SPAN: f64 = 0.048;
/// The most a note's velocity moves by chance.
const VELOCITY: f64 = 5.0;

/// `events` played by a hand: each chord's notes spread over a few
/// milliseconds after its top note (a melody leads), or, with `roll`, a
/// chord of [`ROLL_NOTES`] or more rolled from its lowest note up within
/// [`ROLL_SPAN`]; each note's velocity moved by up to [`VELOCITY`], and a
/// chord struck at one velocity voiced (its top note louder, its inner
/// notes softer). A note keeps its length and ends before its key is
/// struck again; notes alone keep their time.
pub(super) fn humanise(events: &[Timed], roll: bool) -> Vec<Timed> {
    let length = events.last().map_or(0.0, |e| e.0);
    let spans = spans(events, length);
    let mut delay = vec![0.0; spans.len()];
    let mut velocity: Vec<f64> = spans.iter().map(|s| VELOCITY * wobble(s, 1)).collect();
    let order: Vec<usize> = (0..spans.len()).collect();
    for chord in groups(&order, |&n| spans.get(n).map_or(0.0, |s| s.start), CHORD) {
        if chord.len() < 2 {
            continue;
        }
        // Lowest to highest.
        let mut keys: Vec<(u8, usize)> = (chord.iter())
            .filter_map(|&n| Some((spans.get(n)?.key, n)))
            .collect();
        keys.sort_unstable();
        let level = {
            let v = |n: usize| spans.get(n).and_then(|s| events.get(s.on)).map(|e| e.2[2]);
            chord
                .iter()
                .all(|&n| v(n) == chord.first().and_then(|&f| v(f)))
        };
        let top = keys.len().saturating_sub(1);
        let step = ROLL_STEP.min(ROLL_SPAN / top as f64);
        for (rank, &(_, n)) in keys.iter().enumerate() {
            let Some(span) = spans.get(n) else { continue };
            let d = if roll && keys.len() >= ROLL_NOTES {
                rank as f64 * step + 0.3 * step * noise(span.start, span.channel, span.key, 3)
            } else if rank == top {
                0.0
            } else {
                SPREAD_LEAST
                    + (SPREAD - SPREAD_LEAST) * noise(span.start, span.channel, span.key, 4)
            };
            if let Some(slot) = delay.get_mut(n) {
                *slot = d;
            }
            if level && let Some(v) = velocity.get_mut(n) {
                *v += match rank {
                    r if r == top => 4.0,
                    0 if keys.len() > 2 => 2.0,
                    _ => -3.0,
                };
            }
        }
    }
    let mut out = events.to_vec();
    with_velocities(&mut out, &spans, &velocity);
    for (span, &d) in spans.iter().zip(&delay) {
        for at in std::iter::once(span.on).chain(span.off) {
            if let Some(e) = out.get_mut(at) {
                e.0 += d;
            }
        }
    }
    // A note ends before its key is struck again, and never before it starts.
    for span in &spans {
        let start = out.get(span.on).map_or(span.start, |e| e.0);
        let next = span
            .next
            .and_then(|n| out.get(n))
            .map_or(f64::INFINITY, |e| e.0);
        if let Some(e) = span.off.and_then(|off| out.get_mut(off)) {
            e.0 = e.0.min(next).max(start);
        }
    }
    in_time_order(out)
}

/// The velocity a flat file's curve centres on, at most and at least.
const CURVE_CENTRE: (f64, f64) = (40.0, 100.0);
/// Seconds of the line before a note its contour is measured against.
const CONTOUR: f64 = 2.0;

/// `events` given a velocity curve, for a file whose notes share one
/// velocity: centred on that velocity, louder on the beat and softer off
/// it, a chord's top note over its others, a note above the line around
/// it louder than one below (so a melody sings over its accompaniment and
/// a rising line swells), and a little chance. Times are not touched.
pub(super) fn velocity_curve(events: &[Timed], tempo: &Tempo) -> Vec<Timed> {
    let length = events.last().map_or(0.0, |e| e.0);
    let spans = spans(events, length);
    let mut counts = [0usize; 128];
    for e in events.iter().filter(|e| is_on(e.2)) {
        if let Some(c) = counts.get_mut(usize::from(e.2[2] & 127)) {
            *c += 1;
        }
    }
    let common = (0..128).max_by_key(|&v| counts.get(v).copied().unwrap_or(0));
    let centre = (common.unwrap_or(80) as f64).clamp(CURVE_CENTRE.0, CURVE_CENTRE.1);
    let contour = contour(&spans);
    let mut by: Vec<f64> = vec![0.0; spans.len()];
    let order: Vec<usize> = (0..spans.len()).collect();
    for chord in groups(&order, |&n| spans.get(n).map_or(0.0, |s| s.start), CHORD) {
        let top = (chord.iter())
            .filter_map(|&n| spans.get(n).map(|s| s.key))
            .max()
            .unwrap_or(0);
        for &n in chord {
            let Some(span) = spans.get(n) else { continue };
            let beat = match tempo.in_beat(span.start) {
                Some(f) if !(0.03..=0.97).contains(&f) => 6.0,
                Some(f) if (f - 0.5).abs() < 0.03 => 2.0,
                Some(_) => -2.0,
                None => 0.0,
            };
            let voice = match chord.len() {
                1 => 0.0,
                _ if span.key == top => 4.0,
                _ => -4.0,
            };
            let line = contour.get(n).copied().unwrap_or(0.0);
            if let Some(b) = by.get_mut(n) {
                *b = beat + voice + line + 2.0 * wobble(span, 5);
            }
        }
    }
    let mut out = events.to_vec();
    // The curve replaces the one velocity: each note starts from the centre.
    for span in &spans {
        if let Some(e) = out.get_mut(span.on) {
            e.2[2] = centre as u8;
        }
    }
    with_velocities(&mut out, &spans, &by);
    out
}

/// How far each note of `spans` is above the line around it (the notes
/// struck in the [`CONTOUR`] seconds up to it), as velocity: a third of a
/// step a semitone, at most 8.
fn contour(spans: &[Span]) -> Vec<f64> {
    let (mut from, mut to, mut sum) = (0, 0, 0.0);
    let key = |n: usize| spans.get(n).map_or(0.0, |s| f64::from(s.key));
    let start = |n: usize| spans.get(n).map_or(f64::INFINITY, |s| s.start);
    (0..spans.len())
        .map(|n| {
            let now = start(n);
            while start(to) <= now {
                sum += key(to);
                to += 1;
            }
            while start(from) < now - CONTOUR {
                sum -= key(from);
                from += 1;
            }
            let mean = sum / (to - from).max(1) as f64;
            ((key(n) - mean) * 0.3).clamp(-8.0, 8.0)
        })
        .collect()
}

/// Onsets within this many seconds are struck together, for the pedal: a
/// spread or rolled chord is one.
const TOGETHER: f64 = 0.06;
/// A bass note is the lowest struck within this many beats either side.
const BASS_WINDOW: f64 = 0.75;
/// A bass note on the same pitch class changes the pedal again only after
/// this many beats; any onset does after [`LONGEST`].
const REFRESH: f64 = 4.0;
const LONGEST: f64 = 8.0;
/// Changes closer than this many beats are one.
const CLOSEST: f64 = 0.25;
/// A rest this many beats long, or longer, lifts the pedal.
const REST: f64 = 1.0;

/// Notes struck together: when, the lowest key, the pitch classes (a bit
/// each) and how many.
#[derive(Clone, Copy, Debug)]
struct Onset {
    time: f64,
    low: u8,
    classes: u16,
    notes: usize,
}

/// `events` with a sustain pedal, for a file with none: pressed just after
/// each change of harmony (a new bass note, or a new chord of three or
/// more notes) and lifted at the next, as a pianist changes it; lifted in
/// any rest of about a beat or longer, and after the last note. Its
/// presses and lifts go to every channel that plays notes. Nothing else
/// changes.
pub(super) fn auto_pedal(events: &[Timed], tempo: &Tempo) -> Vec<Timed> {
    let length = events.last().map_or(0.0, |e| e.0);
    let spans = spans(events, length);
    if spans.is_empty() {
        return events.to_vec();
    }
    // The channels that play, and the track each first plays in.
    let mut channels: Vec<(u8, u16)> = Vec::new();
    for span in &spans {
        if !channels.iter().any(|c| c.0 == span.channel)
            && let Some(e) = events.get(span.on)
        {
            channels.push((span.channel, e.1));
        }
    }
    let onsets = onsets(&spans);
    let rests = rests(&spans, tempo);
    let changes = changes(&spans, &onsets, &rests, tempo);

    // Down `press` after each change while its notes still sound; up at
    // the next change, or in a rest.
    let press = |at: f64| (0.12 * tempo.beat(at)).clamp(0.05, 0.15);
    let mut pedal: Vec<(f64, bool)> = Vec::new();
    let mut down = false;
    let mut rest = rests.iter().peekable();
    for (n, &at) in changes.iter().enumerate() {
        while let Some(&&(from, to)) = rest.peek() {
            if from >= at {
                break;
            }
            if down {
                pedal.push((lift_in(from, to, tempo), false));
                down = false;
            }
            rest.next();
        }
        if down {
            pedal.push((at, false));
            down = false;
        }
        let next = changes.get(n + 1).copied().unwrap_or(f64::INFINITY);
        let quiet = rest.peek().map_or(f64::INFINITY, |r| r.0);
        if at + press(at) < next.min(quiet) {
            pedal.push((at + press(at), true));
            down = true;
        }
    }
    for &(from, to) in rest {
        if down {
            pedal.push((lift_in(from, to, tempo), false));
            down = false;
        }
    }

    // Pedal first among events at one time: lifted, then the new chord.
    let mut out: Vec<Timed> = Vec::with_capacity(events.len() + pedal.len() * channels.len());
    for &(at, down) in &pedal {
        for &(channel, track) in &channels {
            out.push((at, track, [0xB0 | channel, 64, if down { 127 } else { 0 }]));
        }
    }
    out.extend_from_slice(events);
    in_time_order(out)
}

/// When the pedal lifts in a rest from `from` to `to`: half a beat in, or
/// halfway through a shorter one.
fn lift_in(from: f64, to: f64, tempo: &Tempo) -> f64 {
    from + (0.5 * tempo.beat(from)).min(0.5 * (to - from))
}

/// The notes of `spans` struck together, in order.
fn onsets(spans: &[Span]) -> Vec<Onset> {
    groups(spans, |s| s.start, TOGETHER)
        .into_iter()
        .filter_map(|chord| {
            Some(Onset {
                time: chord.first()?.start,
                low: chord.iter().map(|s| s.key).min()?,
                classes: chord.iter().fold(0, |c, s| c | 1 << (s.key % 12)),
                notes: chord.len(),
            })
        })
        .collect()
}

/// Where no key is held for [`REST`] beats or longer: (from, to), the
/// last open-ended after the last note.
fn rests(spans: &[Span], tempo: &Tempo) -> Vec<(f64, f64)> {
    let mut held: Vec<(f64, f64)> = spans.iter().map(|s| (s.start, s.end)).collect();
    held.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let mut reach = f64::NEG_INFINITY;
    for (start, end) in held {
        if reach > f64::NEG_INFINITY && start - reach >= REST * tempo.beat(reach) {
            out.push((reach, start));
        }
        reach = reach.max(end);
    }
    if reach > f64::NEG_INFINITY {
        out.push((reach, f64::INFINITY));
    }
    out
}

/// Pitch-class profiles less alike than this are a new harmony; less
/// alike than [`ALIKE`], one when their chords' roots differ too.
const NEW_HARMONY: f64 = 0.7;
const ALIKE: f64 = 0.85;

/// When the harmony changes, for the pedal. A change may come with a new
/// bass note (the lowest struck within [`BASS_WINDOW`] beats either side,
/// or, on a beat, a bass the next onset repeats and none below it in that
/// time after it) or a chord
/// of three or more notes on new pitch classes. It is one when what is
/// heard either side of it ([`profile`], over the same span: one to two
/// beats, as far as the candidates either side) differs: what follows
/// brings a pitch class not heard before, and the two are unalike
/// ([`NEW_HARMONY`]), or somewhat unalike ([`ALIKE`]) with chords on
/// different roots. So a figure climbing from its bass, a new inversion
/// or a passing tone is not a change. The first onset and the first after
/// a rest are changes; so is a candidate after [`REFRESH`] beats, and any
/// onset after [`LONGEST`]. Changes within [`CLOSEST`] beats of the last
/// are one.
fn changes(spans: &[Span], onsets: &[Onset], rests: &[(f64, f64)], tempo: &Tempo) -> Vec<f64> {
    let mut last_chord: Option<u16> = None;
    let candidates: Vec<bool> = (onsets.iter().enumerate())
        .map(|(k, onset)| {
            let window = BASS_WINDOW * tempo.beat(onset.time);
            let near = |j: usize| {
                (onsets.get(j)).filter(|o| j != k && (o.time - onset.time).abs() <= window)
            };
            // A tie goes to a later bass on a stronger beat: a pickup to
            // the downbeat is not the bass.
            let below = |o: &Onset| {
                o.low < onset.low
                    || (o.low == onset.low
                        && o.time > onset.time
                        && strength(tempo, o.time) > strength(tempo, onset.time))
            };
            let lowest = !(0..onsets.len()).filter_map(near).any(below);
            // A bass the next onset repeats, struck on a beat, starts a run.
            let on_beat = strength(tempo, onset.time) == 2;
            let repeated = onsets
                .get(k + 1)
                .is_some_and(|o| o.low % 12 == onset.low % 12);
            let ahead = repeated
                && (k + 1..onsets.len())
                    .filter_map(near)
                    .all(|o| onset.low <= o.low);
            let chord = onset.notes >= 3 && last_chord != Some(onset.classes);
            if onset.notes >= 3 {
                last_chord = Some(onset.classes);
            }
            lowest || (on_beat && ahead) || chord
        })
        .collect();
    let times: Vec<f64> = (onsets.iter().zip(&candidates))
        .filter(|(_, c)| **c)
        .map(|(o, _)| o.time)
        .collect();
    let mut out: Vec<f64> = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for (k, (onset, &candidate)) in onsets.iter().zip(&candidates).enumerate() {
        let at = onset.time;
        let beat = tempo.beat(at);
        let after_rest = k == 0
            || onsets.get(k - 1).is_some_and(|before| {
                rests
                    .iter()
                    .any(|&(from, to)| before.time < from && at >= to)
            });
        let since = (at - last) / beat;
        let mut change = after_rest || since >= LONGEST;
        if candidate && !change {
            let n = times.partition_point(|&t| t < at);
            let gap_before = n
                .checked_sub(1)
                .and_then(|p| times.get(p))
                .map_or(f64::INFINITY, |t| at - t);
            let gap_after = times.get(n + 1).map_or(f64::INFINITY, |t| t - at);
            let span = gap_before.min(gap_after).clamp(beat, 2.0 * beat);
            change = since >= REFRESH
                || match (
                    profile(spans, at - span, at, tempo),
                    profile(spans, at, at + span, tempo),
                ) {
                    (Some(before), Some(after)) => {
                        let alike = similarity(&before.0, &after.0);
                        let chord =
                            |(w, bass): ([f64; 12], u8)| kontra_ui::chord::identify(&w, Some(bass));
                        let roots = (chord(before).zip(chord(after)))
                            .is_some_and(|(b, a)| b.root != a.root);
                        brings_a_new_note(&before.0, &after.0)
                            && (alike < NEW_HARMONY || (roots && alike < ALIKE))
                    }
                    _ => true,
                };
        }
        if change && (after_rest || since >= CLOSEST) {
            out.push(at);
            last = at;
        }
    }
    out
}

/// How strong a beat `time` falls on: 2 on a beat, 1 halfway, else 0 (and
/// 0 in a file with no beat).
fn strength(tempo: &Tempo, time: f64) -> u8 {
    match tempo.in_beat(time) {
        Some(f) if !(0.03..=0.97).contains(&f) => 2,
        Some(f) if (f - 0.5).abs() < 0.03 => 1,
        _ => 0,
    }
}

/// What is heard of the notes struck from `from` to `to` seconds: each
/// pitch class weighed by how long its keys are held (two beats at most)
/// and by its register: twice below middle C, which carries the harmony,
/// and 0.6 from the C above, a melody's, whose passing tones are not it.
/// And the lowest key. `None` when none is struck.
fn profile(spans: &[Span], from: f64, to: f64, tempo: &Tempo) -> Option<([f64; 12], u8)> {
    let first = spans.partition_point(|s| s.start < from);
    let last = spans.partition_point(|s| s.start < to);
    let struck = spans.get(first..last.max(first))?;
    let mut weights = [0.0; 12];
    for s in struck {
        let held = (s.end - s.start).clamp(0.0, 2.0 * tempo.beat(s.start));
        let register = match s.key {
            0..60 => 2.0,
            60..72 => 1.0,
            _ => 0.6,
        };
        if let Some(w) = weights.get_mut(usize::from(s.key % 12)) {
            *w += held * register;
        }
    }
    Some((weights, struck.iter().map(|s| s.key).min()?))
}

/// Whether `after` sounds a pitch class `before` does not (each counted
/// from [`kontra_ui::chord`]'s share of its heaviest): a new harmony
/// brings a new note, and a last note or a new bass from the chord does
/// not.
fn brings_a_new_note(before: &[f64; 12], after: &[f64; 12]) -> bool {
    const HEARD: f64 = 0.15;
    let most = |v: &[f64; 12]| v.iter().copied().fold(0.0, f64::max);
    let (b, a) = (HEARD * most(before), HEARD * most(after));
    before
        .iter()
        .zip(after)
        .any(|(&x, &y)| y > a && y > 0.0 && x <= b)
}

/// How alike two pitch-class profiles are, from 0 to 1: their cosine.
fn similarity(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f64; 12]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
    let n = norm(a) * norm(b);
    if n > 0.0 { dot / n } else { 0.0 }
}

// MARK: ringing

/// The notes of `timeline` as they ring: each to its key's note-off, or,
/// while the sustain pedal is down then, to the pedal's lift; and in any
/// case to its key's next strike or `length`. What a chord is heard from
/// ([`kontra_ui::chord::chord_at`]).
pub(super) fn ringing(timeline: &[Timed], length: f64) -> Notes {
    let mut voices: Vec<(u16, u8)> = (timeline.iter())
        .filter(|e| is_on(e.2))
        .map(|&(_, track, [status, ..])| (track, status & 15))
        .collect();
    voices.sort_unstable();
    voices.dedup();
    let mut out: Vec<Note> = Vec::new();
    // By channel and key: the note open, and whether its key is let go.
    let mut open: Vec<Option<(usize, bool)>> = vec![None; 16 * 128];
    let mut pedal = [false; 16];
    for &(time, track, message @ [status, key, value]) in timeline {
        let channel = status & 15;
        let base = usize::from(channel) * 128;
        if is_pedal(message) {
            let down = value >= 64;
            let was = pedal
                .get_mut(usize::from(channel))
                .map(|p| std::mem::replace(p, down));
            if was == Some(true) && !down {
                for slot in open.iter_mut().skip(base).take(128) {
                    if let Some((n, true)) = *slot {
                        if let Some(note) = out.get_mut(n) {
                            note.end = time;
                        }
                        *slot = None;
                    }
                }
            }
            continue;
        }
        let Some(slot) = open.get_mut(base + usize::from(key & 127)) else {
            continue;
        };
        if is_off(message) {
            let held = pedal.get(usize::from(channel)).copied().unwrap_or(false);
            match *slot {
                Some((n, _)) if held => *slot = Some((n, true)),
                Some((n, _)) => {
                    if let Some(note) = out.get_mut(n) {
                        note.end = time;
                    }
                    *slot = None;
                }
                None => {}
            }
        } else if is_on(message) {
            if let Some(note) = slot.take().and_then(|(n, _)| out.get_mut(n)) {
                note.end = time;
            }
            *slot = Some((out.len(), false));
            let voice = voices.binary_search(&(track, channel)).unwrap_or(0);
            out.push(Note {
                start: time,
                end: length,
                key: key & 127,
                velocity: value,
                voice: u8::try_from(voice).unwrap_or(u8::MAX),
            });
        }
    }
    Notes::new(out)
}

#[cfg(test)]
mod heard;
#[cfg(test)]
mod real;
#[cfg(test)]
mod tests;
