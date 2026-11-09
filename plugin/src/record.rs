//! Recording what the player plays into a MIDI file: the host's MIDI (a
//! MIDI keyboard, and the standalone's computer keyboard, which it sends
//! on the same port) and the editor's keyboard, with their timing,
//! velocities, pedal and controllers. Not a song playing: that is not
//! played by the player.
//!
//! The audio thread stamps each message with its frame from the take's
//! start ([`Recorder::process`]) and hands it to the tick, which gathers
//! the take and, once it stops, writes it as a Standard MIDI File
//! ([`smf`]): format 1 at [`BPM`], a track for each source, into the
//! recordings folder (`~/Music/KONTRA Recordings` until one is picked).
//!
//! A take may start with a count-in: a bar of [`BEATS`] clicks at [`BPM`]
//! on the first output pair, the take's time starting after it, so a take
//! begun on the next beat lies on the file's bar lines. Notes played in
//! the count-in are kept, the bar becoming the file's first. Without one,
//! the take starts at the first thing played.
// ponytail: a fixed 120 BPM count-in and click. Take the tempo and the
// click from the transport's metronome once it has one.

pub mod smf;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};

use crossbeam_queue::ArrayQueue;
use kontra_host::rack::Event;
use kontra_ui::record::{Recording, Take};

use crate::folder::{Dialog, Pick};
use crate::lock;

/// The count-in's and the file's tempo.
pub const BPM: f64 = 120.0;

/// Beats in the count-in: a bar of 4/4.
pub const BEATS: u64 = 4;

/// The source the editor's keyboard is.
const KEYBOARD: u8 = u8::MAX;

/// Messages the audio thread hands the tick between two ticks, at most.
const CAPTURED: usize = 16_384;

/// A channel message: status, then data.
type Message = [u8; 3];

/// What the audio thread hands the tick.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Captured {
    /// A take starts: its count-in in frames (0 for none), at `rate`.
    Begin { count_in: u64, rate: f64 },
    /// `message` from `source` (a host port, or [`KEYBOARD`]) at `frame`
    /// from the take's start.
    Message {
        frame: u64,
        source: u8,
        message: Message,
    },
    /// The take stops at `frame`.
    End { frame: u64 },
}

/// What the editor asks of the audio thread.
#[derive(Clone, Copy, Debug)]
enum Command {
    Start { count_in: bool },
    Stop,
}

/// `e` as a MIDI 1.0 channel message and the port it came on; None for
/// what a MIDI file does not hold (note expressions, chokes, system).
fn message(e: &Event) -> Option<(u8, Message)> {
    let seven = |x: f64| (x.clamp(0.0, 1.0) * 127.0).round() as u8;
    let port = |p: i32| u8::try_from(p).unwrap_or(0);
    let pattern = |p: &kontra_host::rack::HostPattern| {
        let channel = u8::try_from(p.channel).ok()? & 15;
        Some((u8::try_from(p.key).ok()? & 127, channel))
    };
    Some(match e {
        Event::NoteOn { note, velocity, .. } => (
            note.port,
            [
                0x90 | note.channel & 15,
                note.key & 127,
                seven(*velocity).max(1),
            ],
        ),
        Event::NoteOff(p) => {
            let (key, channel) = pattern(p)?;
            (port(p.port), [0x80 | channel, key, 0])
        }
        Event::NoteOffVelocity(p, v) => {
            let (key, channel) = pattern(p)?;
            (port(p.port), [0x80 | channel, key, seven(*v)])
        }
        &Event::Ump([word, data]) => {
            let status = (word >> 16) as u8;
            let (a, b) = ((word >> 8) as u8 & 0x7F, word as u8 & 0x7F);
            let m = match word >> 28 {
                // MIDI 1.0 channel voice.
                2 if (0x80..0xF0).contains(&status) => [status, a, b],
                // MIDI 2.0 channel voice, its wider values cut to MIDI 1.0's.
                4 => match status & 0xF0 {
                    0xA0 | 0xB0 | 0xD0 => {
                        let first = if status & 0xF0 == 0xD0 {
                            (data >> 25) as u8
                        } else {
                            a
                        };
                        [status, first, (data >> 25) as u8]
                    }
                    0xC0 => [status, (data >> 24) as u8 & 0x7F, 0],
                    0xE0 => [status, (data >> 18) as u8 & 0x7F, (data >> 25) as u8],
                    _ => return None,
                },
                _ => return None,
            };
            (0, m)
        }
        _ => return None,
    })
}

/// The audio thread's side of a take.
#[derive(Debug, Default)]
pub struct Capture {
    /// Frames since the take (its count-in first) started; None when idle.
    frame: Option<u64>,
    /// Where this block started, for its clicks.
    block: u64,
    /// The count-in's length, and a beat's, in frames.
    count_in: u64,
    beat: u64,
    rate: f64,
    /// A stop not handed to the tick yet (its queue was full).
    ending: Option<u64>,
}

impl Capture {
    /// Adds this block's count-in clicks to `out`, one output channel.
    pub fn click(&self, out: &mut [f32]) {
        if self.frame.is_none() || self.block >= self.count_in || self.beat == 0 {
            return;
        }
        let len = (0.06 * self.rate) as u64;
        for (n, x) in (self.block..).zip(out.iter_mut()) {
            if n >= self.count_in {
                break;
            }
            let (beat, into) = (n / self.beat, n % self.beat);
            if into >= len {
                continue;
            }
            // A high click on the bar's first beat, lower on the rest.
            let hz = if beat == 0 { 1760.0 } else { 1320.0 };
            let t = into as f64 / self.rate;
            let y = 0.25 * (std::f64::consts::TAU * hz * t).sin() * (-t / 0.012).exp();
            *x += y as f32;
        }
    }
}

/// Recording, shared by the editor, the tick and the audio thread.
pub struct Recorder {
    commands: ArrayQueue<Command>,
    /// The editor keyboard's messages, for the audio thread to stamp as the
    /// rack plays them, at the start of the next block.
    keyboard: ArrayQueue<Message>,
    captured: ArrayQueue<Captured>,
    /// What the header shows: 0 idle, 1 counting in, 2 recording; the
    /// count-in's beat; seconds into the take, as `f64` bits.
    state: AtomicU8,
    beat: AtomicU8,
    seconds: AtomicU64,
    count_in: AtomicBool,
    /// Messages lost to a full queue in this take.
    lost: AtomicU32,
    /// The take the tick is gathering.
    take: Mutex<Option<Gathered>>,
    folder: Mutex<PathBuf>,
    /// The last take saved.
    saved: Mutex<Option<PathBuf>>,
    /// The "Recordings folder" picker.
    pub dialog: Dialog,
    /// The header's state as last shown, so the tick redraws as it moves.
    shown: AtomicU64,
}

/// A take as the tick gathers it.
#[derive(Debug, Default)]
struct Gathered {
    count_in: u64,
    rate: f64,
    messages: Vec<(u64, u8, Message)>,
}

/// Where takes go until the player picks a folder.
fn default_folder() -> PathBuf {
    let music = dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .unwrap_or_default();
    music.join("KONTRA Recordings")
}

/// Where the folder picked is remembered; none in tests.
fn remembered() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(
        dirs::config_dir()?
            .join("kontra")
            .join("recordings-folder.txt"),
    )
}

impl Default for Recorder {
    fn default() -> Self {
        let folder = (remembered().and_then(|f| std::fs::read_to_string(f).ok()))
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| p.is_absolute())
            .unwrap_or_else(default_folder);
        Self {
            commands: ArrayQueue::new(8),
            keyboard: ArrayQueue::new(256),
            captured: ArrayQueue::new(CAPTURED),
            state: AtomicU8::new(0),
            beat: AtomicU8::new(0),
            seconds: AtomicU64::new(0),
            count_in: AtomicBool::new(false),
            lost: AtomicU32::new(0),
            take: Mutex::new(None),
            folder: Mutex::new(folder),
            saved: Mutex::new(None),
            dialog: Dialog::new(Pick::Recordings),
            shown: AtomicU64::new(u64::MAX),
        }
    }
}

impl Recorder {
    /// Starts a take at the next block (its count-in first, if on).
    pub fn start(&self) {
        let count_in = self.count_in.load(Ordering::Relaxed);
        let _ = self.commands.push(Command::Start { count_in });
    }

    /// Stops the take at the next block; the tick saves it.
    pub fn stop(&self) {
        let _ = self.commands.push(Command::Stop);
    }

    pub fn set_count_in(&self, on: bool) {
        self.count_in.store(on, Ordering::Relaxed);
    }

    /// The folder takes go to.
    pub fn folder(&self) -> PathBuf {
        lock(&self.folder).clone()
    }

    /// Takes go to `folder` from now on, this run and the next.
    pub fn set_folder(&self, folder: PathBuf) {
        if let Some(file) = remembered() {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(file, folder.to_string_lossy().as_bytes());
        }
        *lock(&self.folder) = folder;
    }

    /// The last take saved.
    pub fn saved(&self) -> Option<PathBuf> {
        lock(&self.saved).clone()
    }

    /// The editor keyboard played `e`: recorded while a take runs.
    pub fn keyboard(&self, e: &Event) {
        if self.state.load(Ordering::Relaxed) == 0 {
            return;
        }
        if let Some((_, m)) = message(e)
            && self.keyboard.push(m).is_err()
        {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// What the header shows.
    pub fn shown(&self) -> Recording {
        let take = match self.state.load(Ordering::Relaxed) {
            1 => Take::CountIn(self.beat.load(Ordering::Relaxed)),
            2 => Take::Recording,
            _ => Take::Idle,
        };
        Recording {
            take,
            seconds: f64::from_bits(self.seconds.load(Ordering::Relaxed)),
            count_in: self.count_in.load(Ordering::Relaxed),
            folder: self.folder().display().to_string(),
            saved: (self.saved().as_deref())
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned()),
        }
    }

    fn hand(&self, c: Captured) {
        if self.captured.push(c).is_err() {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// On the audio thread, as a block starts, before a song's events join
    /// `input`: starts and stops takes as the editor asked and stamps what
    /// the player played this block.
    pub fn process(&self, c: &mut Capture, rate: f64, frames: usize, input: &[(u32, Event)]) {
        if let Some(frame) = c.ending
            && self.captured.push(Captured::End { frame }).is_ok()
        {
            c.ending = None;
        }
        while let Some(command) = self.commands.pop() {
            match (command, c.frame) {
                (Command::Start { count_in }, None) if c.ending.is_none() => {
                    c.rate = rate.max(1.0);
                    c.beat = (c.rate * 60.0 / BPM).round() as u64;
                    c.count_in = if count_in { c.beat * BEATS } else { 0 };
                    c.frame = Some(0);
                    let count_in = c.count_in;
                    self.lost.store(0, Ordering::Relaxed);
                    self.hand(Captured::Begin { count_in, rate });
                }
                (Command::Stop, Some(frame)) => {
                    c.frame = None;
                    if self.captured.push(Captured::End { frame }).is_err() {
                        c.ending = Some(frame);
                    }
                }
                _ => {}
            }
        }
        let Some(start) = c.frame else {
            while self.keyboard.pop().is_some() {}
            self.state.store(0, Ordering::Relaxed);
            return;
        };
        c.block = start;
        while let Some(message) = self.keyboard.pop() {
            let (frame, source) = (start, KEYBOARD);
            self.hand(Captured::Message {
                frame,
                source,
                message,
            });
        }
        for (at, e) in input {
            if let Some((source, message)) = message(e) {
                let frame = start + u64::from(*at);
                self.hand(Captured::Message {
                    frame,
                    source,
                    message,
                });
            }
        }
        let end = start + frames as u64;
        c.frame = Some(end);
        if end < c.count_in {
            self.state.store(1, Ordering::Relaxed);
            let beat = (end / c.beat.max(1) + 1).min(BEATS);
            self.beat
                .store(u8::try_from(beat).unwrap_or(0), Ordering::Relaxed);
        } else {
            self.state.store(2, Ordering::Relaxed);
            let seconds = (end - c.count_in) as f64 / c.rate;
            self.seconds.store(seconds.to_bits(), Ordering::Relaxed);
        }
    }

    /// On the tick: gathers what the audio thread captured and saves a take
    /// that stopped. Its file once saved, or why it was not; and whether
    /// the header's state moved.
    pub fn tick(&self) -> (Option<Result<PathBuf, String>>, bool) {
        let mut done = None;
        while let Some(c) = self.captured.pop() {
            let mut take = lock(&self.take);
            match c {
                Captured::Begin { count_in, rate } => {
                    *take = Some(Gathered {
                        count_in,
                        rate,
                        messages: Vec::new(),
                    });
                }
                Captured::Message {
                    frame,
                    source,
                    message,
                } => {
                    if let Some(t) = take.as_mut() {
                        t.messages.push((frame, source, message));
                    }
                }
                Captured::End { frame } => {
                    if let Some(t) = take.take() {
                        let saved = self.save(&t, frame);
                        if let Ok(path) = &saved {
                            *lock(&self.saved) = Some(path.clone());
                        }
                        done = Some(saved);
                    }
                }
            }
        }
        let state = u64::from(self.state.load(Ordering::Relaxed));
        let seconds = f64::from_bits(self.seconds.load(Ordering::Relaxed)) as u64;
        let beat = u64::from(self.beat.load(Ordering::Relaxed));
        let now = state | beat << 8 | seconds << 16;
        let moved = self.shown.swap(now, Ordering::Relaxed) != now || done.is_some();
        (done, moved)
    }

    /// Writes `take`, stopped at `end`, into the folder as the next
    /// "Recording N.mid".
    fn save(&self, take: &Gathered, end: u64) -> Result<PathBuf, String> {
        let lost = self.lost.load(Ordering::Relaxed);
        let tracks = tracks(take, end);
        if tracks.is_empty() {
            return Err("nothing was played, so no take was saved".into());
        }
        let folder = self.folder();
        std::fs::create_dir_all(&folder)
            .map_err(|e| format!("cannot make {}: {e}", folder.display()))?;
        let path = free(&folder, "Recording", "mid");
        let name = path
            .file_stem()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let bytes = smf::write(&name, BPM, &tracks);
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        if lost > 0 {
            return Err(format!(
                "{}: {lost} messages were played too fast to record",
                path.display()
            ));
        }
        Ok(path)
    }
}

/// The first `<stem> N.<ext>` in `folder` that does not exist, from 1.
pub fn free(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    (1u32..)
        .map(|n| folder.join(format!("{stem} {n}.{ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| folder.join(format!("{stem}.{ext}")))
}

/// A source's track name.
fn track_name(source: u8) -> String {
    match source {
        KEYBOARD => "On-screen keyboard".into(),
        0 => "MIDI input".into(),
        n => format!("MIDI input {}", u16::from(n) + 1),
    }
}

/// `take`, stopped at frame `end`, as a file's tracks: one per source, the
/// host's ports first, each in time from where the take starts. A key
/// still down or a pedal still held at the end is let go there.
fn tracks(take: &Gathered, end: u64) -> Vec<smf::Track> {
    if take.messages.is_empty() {
        return Vec::new();
    }
    let first = take.messages.iter().map(|m| m.0).min().unwrap_or(0);
    // After the count-in; a note in it makes it the file's first bar.
    let origin = match take.count_in {
        0 => first,
        count_in if first < count_in => 0,
        count_in => count_in,
    };
    let rate = take.rate.max(1.0);
    let time = |frame: u64| frame.saturating_sub(origin) as f64 / rate;
    let mut sources: Vec<u8> = take.messages.iter().map(|m| m.1).collect();
    sources.sort_unstable();
    sources.dedup();
    sources
        .into_iter()
        .map(|source| {
            let mut events: Vec<(f64, Message)> = Vec::new();
            // Keys down by channel, and the pedal by channel.
            let mut down = [0u128; 16];
            let mut pedal = [false; 16];
            for &(frame, _, m @ [status, key, value]) in
                take.messages.iter().filter(|m| m.1 == source)
            {
                let channel = usize::from(status & 15);
                match status & 0xF0 {
                    0x90 if value > 0 => {
                        if let Some(d) = down.get_mut(channel) {
                            *d |= 1 << (key & 127);
                        }
                    }
                    0x80 | 0x90 => {
                        if let Some(d) = down.get_mut(channel) {
                            *d &= !(1 << (key & 127));
                        }
                    }
                    0xB0 if key == 64 => {
                        if let Some(p) = pedal.get_mut(channel) {
                            *p = value >= 64;
                        }
                    }
                    _ => {}
                }
                events.push((time(frame), m));
            }
            let at = time(end.max(first));
            for (channel, keys) in (0u8..).zip(down) {
                for key in (0..128u8).filter(|k| keys & 1 << k != 0) {
                    events.push((at, [0x80 | channel, key, 0]));
                }
            }
            for (channel, _) in (0u8..).zip(pedal).filter(|p| p.1) {
                events.push((at, [0xB0 | channel, 64, 0]));
            }
            smf::Track {
                name: track_name(source),
                events,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kontra_host::rack::{HostNote, HostPattern};

    const RATE: f64 = 48_000.0;
    const BLOCK: usize = 480;

    fn on(at: u32, port: u8, key: u8, velocity: u8) -> (u32, Event) {
        let note = HostNote {
            port,
            channel: 0,
            key,
            id: -1,
            clap: false,
        };
        let velocity = f64::from(velocity) / 127.0;
        (
            at,
            Event::NoteOn {
                note,
                velocity,
                tune: 0.0,
            },
        )
    }

    fn off(at: u32, port: u8, key: u8) -> (u32, Event) {
        let p = HostPattern {
            port: i32::from(port),
            channel: 0,
            key: i32::from(key),
            id: -1,
            clap: false,
        };
        (at, Event::NoteOff(p))
    }

    fn recorder(dir: &Path) -> Recorder {
        let r = Recorder::default();
        r.set_folder(dir.to_path_buf());
        r
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kontra-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    /// `blocks` blocks of 10 ms; `input(n)` gives block `n`'s events.
    fn run(
        r: &Recorder,
        c: &mut Capture,
        blocks: std::ops::Range<usize>,
        input: impl Fn(usize) -> Vec<(u32, Event)>,
    ) {
        for n in blocks {
            r.process(c, RATE, BLOCK, &input(n));
        }
    }

    #[test]
    fn messages_become_midi_1() {
        let m = |e| message(&e).map(|x| x.1);
        assert_eq!(m(on(0, 0, 60, 100).1), Some([0x90, 60, 100]));
        assert_eq!(m(off(0, 0, 60).1), Some([0x80, 60, 0]));
        assert_eq!(m(Event::midi1(0xB3, 64, 127)), Some([0xB3, 64, 127]));
        assert_eq!(m(Event::midi1(0xE0, 0, 64)), Some([0xE0, 0, 64]));
        // MIDI 2.0: a 32-bit controller and bend cut to 7 and 14 bits.
        let cc2 = Event::Ump([0x40B4_4000, 0xFFFF_FFFF]);
        assert_eq!(m(cc2), Some([0xB4, 64, 127]));
        let bend2 = Event::Ump([0x40E0_0000, 0x8000_0000]);
        assert_eq!(m(bend2), Some([0xE0, 0, 64]));
        // Not a channel message.
        assert_eq!(m(Event::Ump([0x1000_0000, 0])), None);
    }

    /// Played on a MIDI keyboard and on the editor's: a file with a track
    /// each, notes at their time from the first, velocities kept, and the
    /// pedal; it plays back through Play MIDI's parser.
    #[test]
    fn a_take_saves_a_track_per_source_from_the_first_note() {
        let dir = scratch("record");
        let r = recorder(&dir);
        let mut c = Capture::default();
        r.start();
        // A second of nothing: the take starts at the first note.
        run(&r, &mut c, 0..100, |_| Vec::new());
        assert_eq!(r.shown().take, Take::Recording);
        run(&r, &mut c, 100..200, |n| match n {
            100 => vec![on(240, 0, 60, 90), (240, Event::midi1(0xB0, 64, 127))],
            150 => vec![off(0, 0, 60)],
            160 => vec![on(10, 0, 64, 30)],
            190 => vec![(0, Event::midi1(0xB0, 64, 0))],
            _ => Vec::new(),
        });
        // The editor's keyboard, stamped at the next block's start.
        r.keyboard(&on(0, 255, 48, 100).1);
        run(&r, &mut c, 200..201, |_| Vec::new());
        r.keyboard(&off(0, 255, 48).1);
        run(&r, &mut c, 201..230, |_| Vec::new());
        r.stop();
        run(&r, &mut c, 230..231, |_| Vec::new());
        assert_eq!(r.shown().take, Take::Idle);
        let (saved, moved) = r.tick();
        assert!(moved);
        let path = saved.unwrap().unwrap();
        assert_eq!(path, dir.join("Recording 1.mid"));
        assert_eq!(r.shown().saved.as_deref(), Some("Recording 1.mid"));

        let song = crate::song::Song::open(&path, &kontra_library::Touch::NONE)
            .unwrap()
            .0;
        let notes: Vec<_> = (song.notes.between(0.0, 100.0))
            .map(|n| {
                (
                    (n.start * 1000.0).round(),
                    (n.end * 1000.0).round(),
                    n.key,
                    n.velocity,
                    n.voice,
                )
            })
            .collect();
        // From the first note: 60 at 0 s; 64 at 0.6 s + 10 frames; the
        // editor's 48 a second in. 64, still down, ends with the take.
        assert_eq!(
            notes,
            [
                (0.0, 495.0, 60, 90, 0),
                (595.0, 1295.0, 64, 30, 0),
                (995.0, 1005.0, 48, 100, 1)
            ]
        );
        // A second take is the next file.
        r.start();
        run(&r, &mut c, 0..2, |n| {
            if n == 1 {
                vec![on(0, 0, 72, 64)]
            } else {
                Vec::new()
            }
        });
        r.stop();
        run(&r, &mut c, 2..3, |_| Vec::new());
        assert_eq!(r.tick().0.unwrap().unwrap(), dir.join("Recording 2.mid"));
        // Nothing played: nothing saved, and it says so.
        r.start();
        run(&r, &mut c, 0..5, |_| Vec::new());
        r.stop();
        run(&r, &mut c, 5..6, |_| Vec::new());
        assert!(r.tick().0.unwrap().is_err());
        assert!(!dir.join("Recording 3.mid").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The pedal held as the take stops is let go there, and so is its
    /// last key; the file plays back with the pedal on its frames.
    #[test]
    fn the_pedal_round_trips_through_play_midi() {
        let dir = scratch("record-pedal");
        let r = recorder(&dir);
        let mut c = Capture::default();
        r.start();
        run(&r, &mut c, 0..100, |n| match n {
            0 => vec![(0, Event::midi1(0xB0, 64, 127)), on(0, 0, 60, 100)],
            50 => vec![off(0, 0, 60), (0, Event::midi1(0xB0, 64, 0))],
            60 => vec![(0, Event::midi1(0xB0, 64, 100)), on(0, 0, 62, 100)],
            _ => Vec::new(),
        });
        r.stop();
        run(&r, &mut c, 100..101, |_| Vec::new());
        let path = r.tick().0.unwrap().unwrap();
        let songs = crate::song::Songs::default();
        songs
            .open_at(&path, &kontra_library::Touch::NONE, None)
            .unwrap();
        let mut now = None;
        let mut pedal = Vec::new();
        for b in 0..200u64 {
            let mut out = Vec::with_capacity(64);
            songs.process(&mut now, RATE, BLOCK, &mut out);
            for (at, e) in out {
                if let Some((_, [0xB0, 64, v])) = message(&e) {
                    pedal.push((b * BLOCK as u64 + u64::from(at), v));
                }
            }
        }
        // Down at 0, up at 0.5 s, down at 0.6 s, up as the take stops (1 s),
        // where Play MIDI lifts it again as the song ends.
        assert_eq!(
            pedal,
            [
                (0, 127),
                (24_000, 0),
                (28_800, 100),
                (48_000, 0),
                (48_000, 0)
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_count_in_clicks_a_bar_and_the_take_starts_after_it() {
        let dir = scratch("record-count");
        let r = recorder(&dir);
        r.set_count_in(true);
        let mut c = Capture::default();
        r.start();
        let mut clicks = Vec::new();
        // A bar at 120 BPM is 2 s: 200 blocks.
        for n in 0..300usize {
            let input = if n == 220 {
                vec![on(0, 0, 60, 100)]
            } else {
                Vec::new()
            };
            r.process(&mut c, RATE, BLOCK, &input);
            if n == 50 {
                assert_eq!(r.shown().take, Take::CountIn(2));
            }
            let mut out = vec![0.0f32; BLOCK];
            c.click(&mut out);
            clicks.extend(out);
        }
        assert_eq!(r.shown().take, Take::Recording);
        // A click on each beat, none after the bar.
        let onsets: Vec<usize> = (1..clicks.len())
            .filter(|&n| clicks[n - 1] == 0.0 && clicks[n].abs() > 0.0)
            .collect();
        assert_eq!(onsets.len(), 4, "{onsets:?}");
        assert!(
            onsets
                .iter()
                .zip([0, 24_000, 48_000, 72_000])
                .all(|(a, b)| a.abs_diff(b) < 5)
        );
        assert!(clicks[96_000..].iter().all(|x| *x == 0.0));
        r.stop();
        r.process(&mut c, RATE, BLOCK, &[]);
        let path = r.tick().0.unwrap().unwrap();
        let song = crate::song::Song::open(&path, &kontra_library::Touch::NONE)
            .unwrap()
            .0;
        // Played 0.2 s after the count-in: at 0.2 s in the file.
        let n = song.notes.between(0.0, 10.0).next().copied().unwrap();
        assert!((n.start - 0.2).abs() < 1e-3, "{}", n.start);

        // A pickup in the count-in keeps the bar.
        r.start();
        let mut c = Capture::default();
        for n in 0..250usize {
            let input = match n {
                150 => vec![on(0, 0, 55, 100)],
                210 => vec![on(0, 0, 60, 100)],
                _ => Vec::new(),
            };
            r.process(&mut c, RATE, BLOCK, &input);
        }
        r.stop();
        r.process(&mut c, RATE, BLOCK, &[]);
        let song =
            crate::song::Song::open(&r.tick().0.unwrap().unwrap(), &kontra_library::Touch::NONE)
                .unwrap()
                .0;
        let starts: Vec<f64> = song.notes.between(0.0, 10.0).map(|n| n.start).collect();
        assert!(
            (starts[0] - 1.5).abs() < 1e-3 && (starts[1] - 2.1).abs() < 1e-3,
            "{starts:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn idle_records_nothing() {
        let r = Recorder::default();
        let mut c = Capture::default();
        r.keyboard(&on(0, 255, 60, 100).1);
        r.process(&mut c, RATE, BLOCK, &[on(0, 0, 60, 100)]);
        assert!(r.captured.is_empty());
        assert_eq!(r.tick().0, None);
        let mut out = vec![0.0f32; BLOCK];
        c.click(&mut out);
        assert!(out.iter().all(|x| *x == 0.0));
    }
}
