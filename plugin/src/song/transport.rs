//! A song's transport: pause and resume, seek, speed, an A–B loop,
//! transpose, muted hands or tracks, a metronome and a count-in.
//!
//! The editor sets what it wants in [`Transport`]'s atomics; [`Playing`]
//! on the audio thread reads them once a block, lock-free and without
//! allocating, and plays the song's events from where it is:
//!
//! - It keeps its place in *song frames*, seconds of the song times the
//!   output's rate, which run at the speed for each output frame: whole at
//!   speed 1, so an event lands on the frame its time falls on exactly as
//!   it did before there was a speed, and on the frame that time reaches at
//!   any other. Pitch is untouched: it is MIDI, only time scales.
//! - A loop splits a block where it wraps: the part up to B, then the rest
//!   from A, each event still on its own frame.
//! - Pausing, seeking and wrapping end every note sounding and lift the
//!   pedal; resuming, seeking and wrapping then send the controllers (the
//!   pedal above all) as the song has them at the new place: notes struck
//!   before it are not struck again, as Synthesia does.
//! - Each block it reports where its last stretch starts and ends and at
//!   what speed, under a sequence lock, so [`Head::at`] moves the playhead
//!   smoothly between blocks at the song's speed.

use std::f64::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering, fence};
use std::time::Instant;

use kontra_host::rack::Event;

use super::{Message, Song, Songs, event, lock};
use crate::practice::Practice;

/// The slowest and fastest speeds.
pub const SLOWEST: f64 = 0.25;
pub const FASTEST: f64 = 2.0;
/// Semitones a song transposes up or down at most.
pub const TRANSPOSE: i8 = 12;
/// A loop shorter than this, in seconds, plays through.
pub const SHORTEST: f64 = 0.5;
/// Bars of count-in on offer at most.
pub const COUNT_IN: u8 = 2;
/// The key below which a one-voice song's left hand plays: middle C.
pub const SPLIT: u8 = 60;

/// In a key table: no note sounds for that key.
const NONE: u8 = 0xFF;
/// In a key table: that key's note is muted.
const MUTED: u8 = 0xFE;
/// Clicks one block holds; more are not heard.
const CLICKS: usize = 16;
/// "No seek asked": a NaN's bits.
const NO_SEEK: u64 = u64::MAX;
/// "No mark set".
const NO_MARK: u64 = u64::MAX;

/// What the editor wants of the song playing; the audio thread follows.
pub struct Transport {
    paused: AtomicBool,
    /// A place to seek to, in seconds, as `f64` bits; [`NO_SEEK`] for none.
    seek: AtomicU64,
    /// `f64` bits.
    speed: AtomicU64,
    transpose: AtomicI32,
    /// Muted voices, a bit each.
    mute: AtomicU64,
    /// Bit 0: the left hand muted, bit 1 the right; by key around `split`.
    hands: AtomicU32,
    split: AtomicU32,
    /// The loop's marks, in seconds, as `f64` bits; [`NO_MARK`] unset.
    a: AtomicU64,
    b: AtomicU64,
    looping: AtomicBool,
    metronome: AtomicBool,
    /// The click's level from 0 to 1, as `f64` bits.
    volume: AtomicU64,
    count_in: AtomicU32,
    /// A song is loaded: playing, paused, or ended and waiting to play again.
    loaded: AtomicBool,
    /// It played to its end and stopped there.
    ended: AtomicBool,
    /// Which start of a song this is: a new song or a replay from its end.
    take: AtomicU32,
    pub(super) head: Head,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            paused: AtomicBool::new(false),
            seek: AtomicU64::new(NO_SEEK),
            speed: AtomicU64::new(1f64.to_bits()),
            transpose: AtomicI32::new(0),
            mute: AtomicU64::new(0),
            hands: AtomicU32::new(0),
            split: AtomicU32::new(u32::from(SPLIT)),
            a: AtomicU64::new(NO_MARK),
            b: AtomicU64::new(NO_MARK),
            looping: AtomicBool::new(false),
            metronome: AtomicBool::new(false),
            volume: AtomicU64::new(0.6f64.to_bits()),
            count_in: AtomicU32::new(0),
            loaded: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            take: AtomicU32::new(0),
            head: Head::default(),
        }
    }
}

/// The transport as one block plays it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub speed: f64,
    pub transpose: i8,
    pub mute: u64,
    pub left: bool,
    pub right: bool,
    pub split: u8,
    pub a: Option<f64>,
    pub b: Option<f64>,
    pub looping: bool,
    pub metronome: bool,
    pub volume: f64,
    pub count_in: u8,
}

impl Settings {
    /// Whether a note of `voice` on `key` is muted.
    #[must_use]
    pub fn mutes(&self, voice: u8, key: u8) -> bool {
        (voice < 64 && self.mute >> voice & 1 == 1)
            || (self.left && key < self.split)
            || (self.right && key >= self.split)
    }

    /// The loop a song `length` seconds long plays, from A to B: unset
    /// marks are its start and end. None when off or too short.
    #[must_use]
    pub fn region(&self, length: f64) -> Option<(f64, f64)> {
        if !self.looping {
            return None;
        }
        let a = self.a.unwrap_or(0.).clamp(0., length.max(0.));
        let b = self.b.unwrap_or(length).clamp(0., length.max(0.));
        (b - a >= SHORTEST).then_some((a, b))
    }
}

fn mark(bits: u64) -> Option<f64> {
    (bits != NO_MARK).then(|| f64::from_bits(bits))
}

fn bits(mark: Option<f64>) -> u64 {
    mark.filter(|m| m.is_finite()).map_or(NO_MARK, f64::to_bits)
}

impl Transport {
    /// What a block plays with.
    #[must_use]
    pub fn settings(&self) -> Settings {
        let hands = self.hands.load(Ordering::Acquire);
        Settings {
            speed: f64::from_bits(self.speed.load(Ordering::Acquire)).clamp(SLOWEST, FASTEST),
            transpose: i8::try_from(self.transpose.load(Ordering::Acquire)).unwrap_or(0),
            mute: self.mute.load(Ordering::Acquire),
            left: hands & 1 != 0,
            right: hands & 2 != 0,
            split: u8::try_from(self.split.load(Ordering::Acquire)).unwrap_or(SPLIT),
            a: mark(self.a.load(Ordering::Acquire)),
            b: mark(self.b.load(Ordering::Acquire)),
            looping: self.looping.load(Ordering::Acquire),
            metronome: self.metronome.load(Ordering::Acquire),
            volume: f64::from_bits(self.volume.load(Ordering::Acquire)).clamp(0., 1.),
            count_in: u8::try_from(self.count_in.load(Ordering::Acquire)).unwrap_or(0),
        }
    }

    pub fn pause(&self, on: bool) {
        self.paused.store(on, Ordering::Release);
    }

    /// Whether the song is paused (or has ended), as asked.
    #[must_use]
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    /// Whether it played to its end and waits there.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// Which start of a song plays: it moves when one starts or replays.
    #[must_use]
    pub fn take(&self) -> u32 {
        self.take.load(Ordering::Acquire)
    }

    /// Plays on from `seconds` into the song (paused, it stays paused).
    pub fn seek(&self, seconds: f64) {
        if seconds.is_finite() {
            self.seek.store(seconds.to_bits(), Ordering::Release);
        }
    }

    fn take_seek(&self) -> Option<f64> {
        let bits = self.seek.swap(NO_SEEK, Ordering::AcqRel);
        (bits != NO_SEEK).then(|| f64::from_bits(bits))
    }

    pub fn set_speed(&self, speed: f64) {
        let speed = if speed.is_finite() { speed } else { 1. };
        self.speed
            .store(speed.clamp(SLOWEST, FASTEST).to_bits(), Ordering::Release);
    }

    pub fn set_transpose(&self, semitones: i8) {
        let t = semitones.clamp(-TRANSPOSE, TRANSPOSE);
        self.transpose.store(i32::from(t), Ordering::Release);
    }

    /// Mutes the voices whose bits `mask` sets.
    pub fn set_mute(&self, mask: u64) {
        self.mute.store(mask, Ordering::Release);
    }

    /// Mutes the keys below `split` (`left`) or from it up (`right`).
    pub fn set_hands(&self, left: bool, right: bool, split: u8) {
        self.split
            .store(u32::from(split.min(127)), Ordering::Release);
        let hands = u32::from(left) | u32::from(right) << 1;
        self.hands.store(hands, Ordering::Release);
    }

    /// Sets the loop's marks, in seconds; None clears one.
    pub fn set_marks(&self, a: Option<f64>, b: Option<f64>) {
        self.a.store(bits(a), Ordering::Release);
        self.b.store(bits(b), Ordering::Release);
    }

    pub fn set_looping(&self, on: bool) {
        self.looping.store(on, Ordering::Release);
    }

    pub fn set_metronome(&self, on: bool, volume: f64) {
        let volume = if volume.is_finite() { volume } else { 0.6 };
        self.volume
            .store(volume.clamp(0., 1.).to_bits(), Ordering::Release);
        self.metronome.store(on, Ordering::Release);
    }

    pub fn set_count_in(&self, bars: u8) {
        self.count_in
            .store(u32::from(bars.min(COUNT_IN)), Ordering::Release);
    }

    /// A new song is loaded: it plays, without the last one's loop or
    /// mutes. Where it starts, in seconds: before 0 by its count-in.
    pub(super) fn begin(&self, song: &Song) -> f64 {
        self.set_marks(None, None);
        self.set_looping(false);
        self.set_mute(0);
        self.hands.store(0, Ordering::Release);
        self.seek.store(NO_SEEK, Ordering::Release);
        self.paused.store(false, Ordering::Release);
        self.ended.store(false, Ordering::Release);
        self.loaded.store(true, Ordering::Release);
        self.take.fetch_add(1, Ordering::AcqRel);
        count(song, self.settings().count_in).map_or(0., |c| c.start())
    }

    /// The song playing, read again, plays on: its loop, mutes and pause
    /// stay as they were.
    pub(super) fn resume(&self) {
        self.seek.store(NO_SEEK, Ordering::Release);
        self.ended.store(false, Ordering::Release);
        self.loaded.store(true, Ordering::Release);
    }

    /// The song played to its end, and waits there paused.
    pub(super) fn finish(&self) {
        self.ended.store(true, Ordering::Release);
        self.paused.store(true, Ordering::Release);
    }

    /// Nothing is loaded any more.
    pub(super) fn end(&self) {
        self.loaded.store(false, Ordering::Release);
        self.ended.store(false, Ordering::Release);
        self.paused.store(false, Ordering::Release);
    }

    pub(super) fn loaded(&self) -> bool {
        self.loaded.load(Ordering::Acquire)
    }
}

// MARK: the playhead

/// Where the song is as the editor sees it: the last stretch of the last
/// block rendered, as written by [`Head::stamp`] under a sequence lock.
pub struct Head {
    seq: AtomicU32,
    /// Seconds into the song where the stretch starts, `f64` bits.
    base: AtomicU64,
    /// When, in nanoseconds from `epoch`, the stretch started rendering.
    origin: AtomicI64,
    /// Seconds into the song at the block's end, `f64` bits.
    end: AtomicU64,
    /// Seconds of song a second, `f64` bits: 0 while paused.
    speed: AtomicU64,
    epoch: Instant,
}

impl Default for Head {
    fn default() -> Self {
        Self {
            seq: AtomicU32::new(0),
            base: AtomicU64::new(0),
            origin: AtomicI64::new(0),
            end: AtomicU64::new(0),
            speed: AtomicU64::new(1f64.to_bits()),
            epoch: Instant::now(),
        }
    }
}

impl Head {
    /// A stretch from `base` to `end` seconds of the song at `speed` is
    /// being rendered, starting `after` seconds of output from now.
    pub(super) fn stamp(&self, base: f64, after: f64, end: f64, speed: f64) {
        let origin = self
            .nanos(Instant::now())
            .saturating_add((after * 1e9) as i64);
        self.seq.fetch_add(1, Ordering::Relaxed);
        fence(Ordering::Release);
        self.base.store(base.to_bits(), Ordering::Relaxed);
        self.origin.store(origin, Ordering::Relaxed);
        self.end.store(end.to_bits(), Ordering::Relaxed);
        self.speed.store(speed.to_bits(), Ordering::Relaxed);
        self.seq.fetch_add(1, Ordering::Release);
    }

    /// The stamp, read whole: (base, origin, end, speed).
    fn read(&self) -> (f64, i64, f64, f64) {
        let mut read = (0., 0, 0., 0.);
        // The writer never waits; a reader tries again, a few times.
        for _ in 0..8 {
            let before = self.seq.load(Ordering::Acquire);
            read = (
                f64::from_bits(self.base.load(Ordering::Relaxed)),
                self.origin.load(Ordering::Relaxed),
                f64::from_bits(self.end.load(Ordering::Relaxed)),
                f64::from_bits(self.speed.load(Ordering::Relaxed)),
            );
            fence(Ordering::Acquire);
            if before.is_multiple_of(2) && self.seq.load(Ordering::Relaxed) == before {
                break;
            }
        }
        read
    }

    /// Where the song is at `at`, in seconds: the last stretch rendered,
    /// moved on at its speed by the time since, never past its end.
    #[must_use]
    pub fn at(&self, at: Instant) -> f64 {
        let (base, origin, end, speed) = self.read();
        let since = self.nanos(at).saturating_sub(origin) as f64 / 1e9;
        (base + since * speed).clamp(base.min(end), end.max(base))
    }

    /// Seconds into the song at the end of the last block rendered.
    #[must_use]
    pub fn end(&self) -> f64 {
        self.read().2
    }

    fn nanos(&self, at: Instant) -> i64 {
        let since = at.saturating_duration_since(self.epoch).as_nanos();
        i64::try_from(since).unwrap_or(i64::MAX)
    }
}

// MARK: playing

/// A value by channel and key: which key a song's key sounds as, the
/// voice that struck it, a controller's value.
#[derive(Clone, Debug)]
struct Table([[u8; 128]; 16]);

impl Table {
    const fn new() -> Self {
        Self([[NONE; 128]; 16])
    }

    fn get(&self, channel: u8, key: u8) -> u8 {
        (self.0.get(usize::from(channel)))
            .and_then(|c| c.get(usize::from(key)))
            .copied()
            .unwrap_or(NONE)
    }

    fn set(&mut self, channel: u8, key: u8, value: u8) {
        if let Some(v) =
            (self.0.get_mut(usize::from(channel))).and_then(|c| c.get_mut(usize::from(key)))
        {
            *v = value;
        }
    }
}

/// The count-in: `beats` clicks `interval` apart, the last a beat before
/// the first note at `first` seconds, accented each `per_bar`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Count {
    first: f64,
    interval: f64,
    beats: u8,
    per_bar: u8,
}

impl Count {
    fn start(&self) -> f64 {
        self.first - f64::from(self.beats) * self.interval
    }
}

/// `bars` of count-in before `song`'s first note, by its beat there.
fn count(song: &Song, bars: u8) -> Option<Count> {
    if bars == 0 {
        return None;
    }
    let first = (song.events.iter())
        .find(|(_, [status, _, velocity])| status & 0xF0 == 0x90 && *velocity > 0)?
        .0;
    let beats = &song.beats;
    let i = (beats.partition_point(|b| b.time <= first + 1e-6)).saturating_sub(1);
    let interval = match (beats.get(i), beats.get(i + 1)) {
        (Some(a), Some(b)) => b.time - a.time,
        _ => 0.5,
    }
    .clamp(0.1, 2.);
    // Beats a bar: from the downbeat at or before the first note's beat
    // to the next downbeat.
    let down = (beats.get(..=i).unwrap_or_default().iter()).rposition(|b| b.downbeat);
    let next = (beats.get(i + 1..).unwrap_or_default().iter()).position(|b| b.downbeat);
    let per_bar = match (down, next) {
        (Some(d), Some(n)) => (i + 1 + n).saturating_sub(d),
        _ => 4,
    }
    .clamp(1, 12);
    let per_bar = u8::try_from(per_bar).unwrap_or(4);
    Some(Count {
        first,
        interval,
        beats: per_bar.saturating_mul(bars.min(COUNT_IN)),
        per_bar,
    })
}

/// The metronome's clicks: this block's, and the one sounding.
#[derive(Clone, Debug)]
struct Clicks {
    /// Their frames in this block and whether each is a bar's first.
    list: [(u32, bool); CLICKS],
    len: usize,
    /// The click sounding: frames into it, its pitch, its level.
    sounding: Option<(u32, f64, f64)>,
    volume: f64,
}

impl Clicks {
    fn push(&mut self, at: u32, accent: bool) {
        if let Some(slot) = self.list.get_mut(self.len) {
            *slot = (at, accent);
            self.len += 1;
        }
    }
}

/// How one block went, for the playhead.
#[derive(Clone, Copy, Debug)]
pub(super) struct Report {
    /// The output frame its last stretch starts at (a loop's wrap), and
    /// where in the song that stretch starts and the block ends, in seconds.
    pub from: u32,
    pub base: f64,
    pub end: f64,
    /// 0 while paused.
    pub speed: f64,
    /// It played its last event this block, and stopped.
    pub ended: bool,
    /// It started again from its end.
    pub replayed: bool,
}

/// A song on the audio thread: where it is, the keys it holds, the
/// controllers it has sent.
#[derive(Debug)]
pub struct Playing {
    pub(super) song: Box<Song>,
    /// Its next event.
    next: usize,
    /// Its next beat.
    beat: usize,
    /// Where it is: song frames, seconds of the song times the rate.
    at: f64,
    /// The key each key of the song sounds as (transposed), by channel.
    keys: Table,
    /// The voice that struck each.
    voices: Table,
    /// Each controller's value as last sent, by channel.
    sent: Table,
    /// The channels it has played on, a bit each.
    channels: u16,
    paused: bool,
    ended: bool,
    /// It has not played a block yet: what lies before where it starts
    /// (a count-in past a silence) is sent first.
    fresh: bool,
    /// The mutes its notes last played under.
    muting: (u64, bool, bool, u8),
    count: Option<Count>,
    clicks: Clicks,
}

/// `seconds` of the song at `rate`, in song frames.
fn frame(seconds: f64, rate: f64) -> f64 {
    (seconds * rate).round()
}

/// `key` moved `by` semitones, if it is still a key.
fn transposed(key: u8, by: i8) -> Option<u8> {
    u8::try_from(i16::from(key) + i16::from(by))
        .ok()
        .filter(|k| *k < 128)
}

/// Controllers whose value is state worth restoring: not the channel mode
/// messages, nor the parameter numbers whose data only means anything in
/// the order they came.
fn chased(controller: u8) -> bool {
    controller < 120 && !matches!(controller, 6 | 38 | 96..=101)
}

/// Pushes `e` at frame `at` while `out` has room.
fn push(out: &mut Vec<(u32, Event)>, at: u32, e: Event) {
    if out.len() < out.capacity() {
        out.push((at, e));
    }
}

impl Playing {
    /// `song` from its start, or from `settings`' count-in before its
    /// first note.
    pub(super) fn new(song: Box<Song>, settings: &Settings, rate: f64) -> Self {
        let volume = settings.volume;
        let mut p = Self {
            song,
            next: 0,
            beat: 0,
            at: 0.,
            keys: Table::new(),
            voices: Table::new(),
            sent: Table::new(),
            channels: 0,
            paused: false,
            ended: false,
            fresh: true,
            muting: (0, false, false, SPLIT),
            count: None,
            clicks: Clicks {
                list: [(0, false); CLICKS],
                len: 0,
                sounding: None,
                volume,
            },
        };
        p.restart(settings, rate);
        if let Some(at) = p.song.from {
            p.locate(at, rate);
        }
        p
    }

    /// Whether it played to its end and waits there.
    #[cfg(test)]
    #[must_use]
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// To its start, or its count-in's.
    fn restart(&mut self, settings: &Settings, rate: f64) {
        let count = count(&self.song, settings.count_in);
        self.locate(count.map_or(0., |c| c.start()), rate);
        self.count = count;
    }

    /// To `seconds` into the song, playing nothing between.
    fn locate(&mut self, seconds: f64, rate: f64) {
        self.at = frame(seconds, rate);
        let at = self.at;
        self.next = (self.song.events).partition_point(|e| frame(e.0, rate) < at);
        self.beat = (self.song.beats).partition_point(|b| frame(b.time, rate) < at);
        self.count = None;
        // Practice waits next for the player's first chord from here.
        self.song.gate.find(at.max(0.) as u64, rate);
    }

    /// The song's events to frame `until` of the song: this block's,
    /// stretch by stretch, at their frames in it. The first `heard` of
    /// `out` are the player's events this block, which `practice` waits for.
    pub(super) fn play(
        &mut self,
        s: &Settings,
        t: &Transport,
        (practice, heard): (&Practice, usize),
        rate: f64,
        frames: usize,
        out: &mut Vec<(u32, Event)>,
    ) -> Report {
        self.clicks.len = 0;
        self.clicks.volume = s.volume;
        let mut report = Report {
            from: 0,
            base: 0.,
            end: 0.,
            speed: 0.,
            ended: false,
            replayed: false,
        };
        let paused = t.paused();
        if std::mem::take(&mut self.fresh) {
            self.chase(0, out);
        }
        if let Some(to) = t.take_seek() {
            self.release(0, out);
            self.locate(to.clamp(0., self.song.length), rate);
            // Paused, the pedal waits up until it plays on.
            if !paused {
                self.chase(0, out);
            }
            self.ended = false;
            t.ended.store(false, Ordering::Release);
        }
        if paused != self.paused {
            if paused {
                self.release(0, out);
            } else {
                if self.ended {
                    self.restart(s, rate);
                    self.ended = false;
                    t.ended.store(false, Ordering::Release);
                    t.take.fetch_add(1, Ordering::AcqRel);
                    report.replayed = true;
                }
                self.chase(0, out);
            }
            self.paused = paused;
        }
        self.remute(s, out);
        report.base = self.at / rate;
        report.end = report.base;
        if self.paused || frames == 0 {
            return report;
        }
        let speed = s.speed.clamp(SLOWEST, FASTEST);
        let region = (s.region(self.song.length)).map(|(a, b)| (frame(a, rate), frame(b, rate)));
        let total = frames as f64;
        // Practice: the song stands still while it waits for the player
        // (`idle` frames), and stops at the next notes of theirs not yet
        // played (`stop`). Counting in, it runs up to their first chord.
        let (idle, stop) = if self.at < 0. {
            (self.song.gate).clock(practice, &[], (0, 0), rate, speed);
            (0., self.waits_at())
        } else {
            let input = out.get(..heard).unwrap_or_default();
            let at = (self.at as u64, frames as u64);
            let (idle, _) = (self.song.gate).clock(practice, input, at, rate, speed);
            (idle.min(frames as u64) as f64, self.waits_at())
        };
        let mut stop = stop;
        // Output frames done; the frame and song frame the stretch starts at.
        let mut done = idle;
        let mut from = ((done as u32).min(frames as u32 - 1), self.at);
        let mut last = None;
        for _ in 0..4 {
            if let Some((a, b)) = region
                && self.at >= b
            {
                let at = (done as u32).min(frames as u32 - 1);
                self.release(at, out);
                self.locate(a / rate, rate);
                self.chase(at, out);
                from = (at, self.at);
                stop = self.waits_at();
            }
            let end = (self.at + (total - done) * speed).min(stop).max(self.at);
            let until = region.map_or(end, |(_, b)| end.min(b));
            last = self
                .events(s, until, done, speed, frames, rate, out)
                .or(last);
            self.beats(s, until, done, speed, frames, rate);
            if until < end {
                // B is reached inside the block: the rest plays from A.
                done += ((until - self.at) / speed).ceil();
                self.at = until;
                if done >= total {
                    break;
                }
                continue;
            }
            self.at = end;
            break;
        }
        report.from = from.0;
        report.base = from.1 / rate;
        report.end = self.at / rate;
        report.speed = speed;
        // After its last event, at that event's frame, whatever the block.
        if region.is_none() && self.next >= self.song.events.len() {
            self.release(last.unwrap_or(from.0), out);
            self.ended = true;
            self.paused = true;
            report.ended = true;
        }
        report
    }

    /// The events before song frame `until`, from output frame `done`.
    /// The frame of the last; while `out` is full, the rest wait.
    #[expect(clippy::too_many_arguments, reason = "one stretch of one block")]
    fn events(
        &mut self,
        s: &Settings,
        until: f64,
        done: f64,
        speed: f64,
        frames: usize,
        rate: f64,
        out: &mut Vec<(u32, Event)>,
    ) -> Option<u32> {
        let mut last = None;
        let latest = frames.saturating_sub(1) as f64;
        while let Some(&(time, message)) = self.song.events.get(self.next) {
            let e = frame(time, rate);
            if e >= until || out.len() == out.capacity() {
                break;
            }
            // Late (the last block was full): at the stretch's start.
            let at = (done + ((e - self.at) / speed).floor())
                .max(done)
                .min(latest) as u32;
            let voice = self.song.voice_of.get(self.next).copied().unwrap_or(NONE);
            // Practising, the player's own notes are theirs to play.
            if self.song.gate.plays(voice, message) {
                self.send(message, voice, at, s, out);
            }
            last = Some(at);
            self.next += 1;
        }
        last
    }

    /// The clicks before song frame `until`, from output frame `done`: the
    /// count-in's before the first note, and the metronome's on its beats.
    fn beats(&mut self, s: &Settings, until: f64, done: f64, speed: f64, frames: usize, rate: f64) {
        let latest = frames.saturating_sub(1) as f64;
        let at =
            |e: f64, from: f64| (done + ((e - from) / speed).floor()).max(done).min(latest) as u32;
        let from = self.at;
        let count = self.count;
        if let Some(c) = count {
            for j in (1..=c.beats).rev() {
                let e = frame(c.first - f64::from(j) * c.interval, rate);
                if (from..until).contains(&e) {
                    let accent = (c.beats - j) % c.per_bar.max(1) == 0;
                    self.clicks.push(at(e, from), accent);
                }
            }
            if until > frame(c.first, rate) {
                self.count = None;
            }
        }
        while let Some(b) = self.song.beats.get(self.beat) {
            let e = frame(b.time, rate);
            if e >= until {
                break;
            }
            // Counting in, its own clicks lead up to the first note.
            let counted = count.is_some_and(|c| b.time < c.first);
            if s.metronome && e >= from && !counted {
                self.clicks.push(at(e, from), b.downbeat);
            }
            self.beat += 1;
        }
    }

    /// `message` of `voice` at frame `at`, transposed or muted as `s` says.
    fn send(
        &mut self,
        [status, key, value]: Message,
        voice: u8,
        at: u32,
        s: &Settings,
        out: &mut Vec<(u32, Event)>,
    ) {
        let channel = status & 15;
        self.channels |= 1 << channel;
        match status & 0xF0 {
            0x90 if value > 0 => {
                let was = self.keys.get(channel, key);
                let to = if s.mutes(voice, key) {
                    None
                } else {
                    transposed(key, s.transpose)
                };
                // Struck again, transposed since: the old note ends first.
                if was < MUTED && Some(was) != to {
                    push(out, at, event([0x80 | channel, was, 0]));
                }
                self.voices.set(channel, key, voice);
                self.keys.set(channel, key, to.unwrap_or(MUTED));
                if let Some(to) = to {
                    push(out, at, event([status, to, value]));
                }
            }
            0x80 | 0x90 => {
                let was = self.keys.get(channel, key);
                self.keys.set(channel, key, NONE);
                // A note it never struck (begun before a seek, or muted)
                // has nothing to end.
                if was < MUTED {
                    push(out, at, event([0x80 | channel, was, 0]));
                }
            }
            kind => {
                if kind == 0xB0 {
                    self.sent.set(channel, key, value);
                }
                push(out, at, event([status, key, value]));
            }
        }
    }

    /// The song frame practice waits at next, or none.
    fn waits_at(&self) -> f64 {
        (self.song.gate.waits_at()).map_or(f64::INFINITY, |f| f as f64)
    }

    /// Notes sounding that the mutes now silence end at the block's start.
    fn remute(&mut self, s: &Settings, out: &mut Vec<(u32, Event)>) {
        let now = (s.mute, s.left, s.right, s.split);
        if now == self.muting {
            return;
        }
        self.muting = now;
        for channel in 0..16u8 {
            for key in 0..128u8 {
                let was = self.keys.get(channel, key);
                if was < MUTED && s.mutes(self.voices.get(channel, key), key) {
                    push(out, 0, event([0x80 | channel, was, 0]));
                    self.keys.set(channel, key, MUTED);
                }
            }
        }
    }

    /// Ends every key it holds and lifts the sustain pedal on its channels,
    /// at frame `at` of the block: stopping it leaves nothing sounding.
    pub(super) fn release(&mut self, at: u32, out: &mut Vec<(u32, Event)>) {
        for channel in 0..16u8 {
            for key in 0..128u8 {
                let was = self.keys.get(channel, key);
                if was < MUTED {
                    push(out, at, event([0x80 | channel, was, 0]));
                }
                self.keys.set(channel, key, NONE);
            }
            if self.channels & 1 << channel != 0 {
                push(out, at, Event::midi1(0xB0 | channel, 64, 0));
                self.sent.set(channel, 64, 0);
            }
        }
    }

    /// Sends, at frame `at`, each controller as the song has it where it
    /// now is (the pedals up where it has not pressed them yet), where it
    /// differs from what was last sent.
    fn chase(&mut self, at: u32, out: &mut Vec<(u32, Event)>) {
        let mut want = Table::new();
        let mut used = 0u16;
        let before = self.song.events.get(..self.next).unwrap_or_default();
        for &(_, [status, controller, value]) in before {
            used |= 1 << (status & 15);
            if status & 0xF0 == 0xB0 && chased(controller) {
                want.set(status & 15, controller, value);
            }
        }
        for channel in 0..16u8 {
            if (used | self.channels) & 1 << channel == 0 {
                continue;
            }
            // A pedal pressed later than here, already sent: up again.
            for pedal in [64, 66, 67] {
                if want.get(channel, pedal) == NONE && self.sent.get(channel, pedal) != NONE {
                    want.set(channel, pedal, 0);
                }
            }
            for controller in 0..120u8 {
                let value = want.get(channel, controller);
                if value != NONE && value != self.sent.get(channel, controller) {
                    push(out, at, Event::midi1(0xB0 | channel, controller, value));
                    self.sent.set(channel, controller, value);
                }
            }
        }
        self.channels |= used;
    }

    /// This block's clicks, rendered, `frames` long at `rate`: `add` takes
    /// each stretch that sounds and where in the block it starts, to mix
    /// into the output. Nothing when no click sounds.
    pub fn click(&mut self, rate: f64, frames: usize, mut add: impl FnMut(usize, &[f32])) {
        const STRETCH: usize = 256;
        if self.clicks.len == 0 && self.clicks.sounding.is_none() {
            return;
        }
        let list = self.clicks.list;
        let mut next = 0;
        let mut buffer = [0f32; STRETCH];
        for start in (0..frames).step_by(STRETCH) {
            let len = STRETCH.min(frames - start);
            let mut any = false;
            for (n, x) in buffer.iter_mut().take(len).enumerate() {
                let frame = (start + n) as u32;
                while let Some(&(at, accent)) = list.get(next).filter(|_| next < self.clicks.len) {
                    if at > frame {
                        break;
                    }
                    let (hz, level) = if accent { (1568., 0.5) } else { (1046.5, 0.32) };
                    self.clicks.sounding = Some((0, hz, level * self.clicks.volume));
                    next += 1;
                }
                *x = 0.;
                let Some((t, hz, level)) = self.clicks.sounding else {
                    continue;
                };
                // A sine that dies away in a few milliseconds: a tick.
                let secs = f64::from(t) / rate;
                *x = ((TAU * hz * secs).sin() * (-secs / 0.008).exp() * level) as f32;
                any = true;
                self.clicks.sounding = (secs < 0.05).then_some((t + 1, hz, level));
            }
            if any {
                add(start, buffer.get(..len).unwrap_or_default());
            }
        }
    }
}

// MARK: for the editor

/// A song loaded, as the editor shows it.
#[derive(Clone, Debug)]
pub struct Now {
    pub name: String,
    /// Where it is and how long it is, in seconds; before 0 counting in.
    pub position: f64,
    pub length: f64,
    pub paused: bool,
    pub ended: bool,
    /// Which start of a song this is.
    pub take: u32,
    pub settings: Settings,
}

impl Songs {
    /// The song's transport: what the editor sets.
    #[must_use]
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// A song loaded (playing, paused or ended), as the editor shows it.
    #[must_use]
    pub fn now(&self) -> Option<Now> {
        let t = &self.transport;
        if !t.loaded() {
            return None;
        }
        let (name, length) = lock(&self.title).clone()?;
        let ended = t.ended();
        let position = if ended {
            length
        } else {
            self.position_at(Instant::now())
                .unwrap_or_else(|| t.head.at(Instant::now()))
                .min(length)
        };
        Some(Now {
            name,
            position,
            length,
            paused: t.paused() || ended,
            ended,
            take: t.take(),
            settings: t.settings(),
        })
    }
}

#[cfg(test)]
mod render;
#[cfg(test)]
mod tests;
