//! Play MIDI: a Standard MIDI File (`.mid`) played into the rack as if a
//! keyboard played it. [`Song::open`] reads the file on the tick into
//! channel messages at their time in seconds, by the file's tempo map;
//! [`Songs::process`] on the audio thread stamps each with its frame in the
//! block, alongside the host's events, so a song plays sample-accurately.
//!
//! The editor and the audio thread share [`Songs`]: lock-free queues in and
//! out (a song to play or stop; songs done with, freed on the tick) and
//! atomics for where it is ([`Songs::position_at`]).
//!
//! A song plays on its own port ([`PORT`]), so neither the host's nor the
//! editor's note-offs end its notes, and on its own channels, so each part
//! hears what its channel (or omni) takes. General MIDI's drum channel is
//! skipped: a melodic instrument would play its hits as pitches.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Instant;

use crossbeam_queue::ArrayQueue;
use kontra_host::rack::{Event, HostNote, HostPattern};
use kontra_ui::waterfall::{Note, Notes};

use crate::folder::{Dialog, Pick};
use crate::lock;

mod meta;
mod transport;

pub use transport::{Playing, SLOWEST, SPLIT, Transport};

/// The port a song's notes come from: not the host's, not the editor's.
const PORT: u8 = 254;

/// General MIDI's drum channel, 10 (9 from 0), which is skipped.
const DRUMS: u8 = 9;

/// A channel voice message: status, then data; the second data byte is 0
/// for the messages that carry one.
type Message = [u8; 3];

/// A MIDI file read for playing.
#[derive(Debug)]
pub struct Song {
    /// Its file name, as the header shows it.
    pub name: String,
    /// Channel messages at their time in seconds, in order.
    events: Vec<(f64, Message)>,
    /// Seconds to its last event.
    pub length: f64,
    /// Its notes, for the editor to show falling onto the keyboard.
    pub notes: Notes,
    /// The voice (a track and channel, as [`Note::voice`] numbers them)
    /// of each of `events`; [`transport`] mutes by it.
    voice_of: Vec<u8>,
    /// Its metre's beats, for the metronome and the count-in.
    beats: Vec<meta::Beat>,
    /// What the Stage's title card and track list show.
    pub about: About,
}

/// What a song is called, by whom, and what its voices are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct About {
    pub title: String,
    pub composer: Option<String>,
    /// Each voice's name: its track's, or "Track n".
    pub voices: Vec<String>,
}

impl Song {
    /// Reads the MIDI file at `path`; Err says why it cannot play.
    pub fn open(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self::read(name, &bytes)
    }

    /// The song in `bytes`, a Standard MIDI File, as `name`.
    pub fn read(name: String, bytes: &[u8]) -> Result<Self, String> {
        let timeline: Vec<(f64, u16, Message)> = timeline(bytes)?
            .into_iter()
            .filter(|&(_, _, [status, ..])| status & 15 != DRUMS)
            .collect();
        let length = timeline.last().map_or(0.0, |&(time, ..)| time);
        let notes = notes(&timeline, length);
        if notes.is_empty() {
            return Err("it has no notes outside the drum channel (10) to play".into());
        }
        let meta = meta::read(bytes);
        let voices = voices(&timeline);
        let voice_of = (timeline.iter())
            .map(|&(_, track, [status, ..])| {
                let voice = voices.binary_search(&(track, status & 15)).ok();
                voice.and_then(|v| u8::try_from(v).ok()).unwrap_or(u8::MAX)
            })
            .collect();
        let (title, composer) = meta::from_name(&name);
        let about = About {
            title: meta.title.unwrap_or(title),
            composer: meta.composer.or(composer),
            voices: (voices.iter())
                .map(|&(track, _)| {
                    let named = meta.tracks.get(usize::from(track)).cloned().flatten();
                    named.unwrap_or_else(|| format!("Track {}", track + 1))
                })
                .collect(),
        };
        Ok(Self {
            name,
            events: timeline.into_iter().map(|(time, _, m)| (time, m)).collect(),
            length,
            notes,
            voice_of,
            beats: meta.beats,
            about,
        })
    }
}

/// The voices that play notes in `timeline`, a track and channel each, in
/// order: what [`Note::voice`] numbers.
fn voices(timeline: &[(f64, u16, Message)]) -> Vec<(u16, u8)> {
    let mut voices: Vec<(u16, u8)> = (timeline.iter())
        .filter(|&&(_, _, [status, _, velocity])| status & 0xF0 == 0x90 && velocity > 0)
        .map(|&(_, track, [status, ..])| (track, status & 15))
        .collect();
    voices.sort_unstable();
    voices.dedup();
    voices
}

/// The notes in `timeline` as the editor shows them: each from its
/// note-on to its key's next note-off or note-on on that channel (the
/// song's end if neither comes), coloured by voice, its track and channel,
/// numbered in that order (a piano file's right hand first, then its left).
fn notes(timeline: &[(f64, u16, Message)], length: f64) -> Notes {
    let on = |[status, _, velocity]: Message| status & 0xF0 == 0x90 && velocity > 0;
    let voices = voices(timeline);
    // The note each key of each channel has open, by index into `out`.
    let mut open = vec![None; 16 * 128];
    let mut out: Vec<Note> = Vec::new();
    for &(time, track, message @ [status, key, velocity]) in timeline {
        let channel = status & 15;
        if !matches!(status & 0xF0, 0x80 | 0x90) {
            continue;
        }
        let Some(slot) = open.get_mut(usize::from(channel) * 128 + usize::from(key)) else {
            continue;
        };
        if let Some(n) = slot.take().and_then(|i: usize| out.get_mut(i)) {
            n.end = time;
        }
        if on(message) {
            *slot = Some(out.len());
            let voice = voices.binary_search(&(track, channel)).unwrap_or(0);
            out.push(Note {
                start: time,
                end: length,
                key,
                velocity,
                voice: u8::try_from(voice).unwrap_or(u8::MAX),
            });
        }
    }
    Notes::new(out)
}

/// `message` as the engine takes it: notes on the song's port, the rest as
/// their MIDI 1.0 packet.
fn event([status, key, value]: Message) -> Event {
    let channel = status & 15;
    match status & 0xF0 {
        0x90 if value > 0 => Event::NoteOn {
            note: HostNote {
                port: PORT,
                channel,
                key,
                id: -1,
                clap: false,
            },
            velocity: f64::from(value) / 127.0,
            tune: 0.0,
        },
        0x80 | 0x90 => Event::NoteOff(HostPattern {
            port: i32::from(PORT),
            channel: i32::from(channel),
            key: i32::from(key),
            id: -1,
            clap: false,
        }),
        _ => Event::midi1(status, key, value),
    }
}

// MARK: reading the file

fn truncated() -> String {
    "the file ends in the middle of a track".into()
}

/// `len` big-endian bytes of `bytes` from `at`.
fn big_endian(bytes: &[u8], at: usize, len: usize) -> Option<usize> {
    let field = bytes.get(at..at.checked_add(len)?)?;
    Some(field.iter().fold(0, |v, &b| v << 8 | usize::from(b)))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, String> {
        let b = *self.bytes.get(self.at).ok_or_else(truncated)?;
        self.at += 1;
        Ok(b)
    }

    /// A variable-length quantity: seven bits a byte, at most four bytes.
    fn number(&mut self) -> Result<usize, String> {
        let mut value = 0;
        for _ in 0..4 {
            let b = self.byte()?;
            value = value << 7 | usize::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err("a length in the file is malformed".into())
    }

    fn skip(&mut self, len: usize) -> Result<(), String> {
        let end = (self.at.checked_add(len))
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(truncated)?;
        self.at = end;
        Ok(())
    }
}

/// What a track says at a tick.
enum Said {
    /// Microseconds per quarter note from here on.
    Tempo(u32),
    /// A channel message, and the track it is in.
    Message(u16, Message),
}

/// The file's channel messages in order, with their time in seconds by its
/// tempo map and the track they are in: format 0 and 1 (format 2's
/// patterns play together), ticks per quarter note or SMPTE frames.
fn timeline(bytes: &[u8]) -> Result<Vec<(f64, u16, Message)>, String> {
    if bytes.get(..4) != Some(b"MThd".as_slice()) {
        return Err("it is not a Standard MIDI File".into());
    }
    let header = big_endian(bytes, 4, 4).ok_or_else(truncated)?;
    let division = big_endian(bytes, 12, 2).ok_or_else(truncated)?;
    if header < 6 {
        return Err("its header is too short".into());
    }
    // The top bit set: SMPTE, frames a second (negated) and ticks a frame.
    let smpte = (division & 0x8000 != 0).then(|| (256 - (division >> 8), division & 0xFF));
    let per_tick = match smpte {
        Some((fps, ticks)) if fps > 0 && ticks > 0 => Some(1.0 / (fps * ticks) as f64),
        None if division > 0 => None,
        _ => return Err("its timing division is zero".into()),
    };

    let mut said = Vec::new();
    let mut tracks = 0u16;
    let mut at = header.checked_add(8).ok_or_else(truncated)?;
    while let (Some(id), Some(len)) = (bytes.get(at..at + 4), big_endian(bytes, at + 4, 4)) {
        let start = at + 8;
        // Some files overstate their last chunk; read what is there.
        let end = start.saturating_add(len).min(bytes.len());
        at = end.max(start);
        if id == b"MTrk" {
            track(bytes.get(start..end).unwrap_or_default(), tracks, &mut said)?;
            tracks = tracks.saturating_add(1);
        }
    }
    // Stable: what one tick holds keeps the file's order (tempo first).
    said.sort_by_key(|&(tick, _)| tick);

    let mut tempo = 500_000.0; // 120 BPM until the file says otherwise
    let (mut time, mut last) = (0.0, 0);
    let mut out = Vec::with_capacity(said.len());
    for (tick, what) in said {
        let ticks = tick.saturating_sub(last) as f64;
        time += ticks * per_tick.unwrap_or(tempo / 1e6 / division as f64);
        last = tick;
        match what {
            Said::Tempo(t) => tempo = f64::from(t),
            Said::Message(track, m) => out.push((time, track, m)),
        }
    }
    Ok(out)
}

/// Track `n`'s tempo changes and channel messages, at their tick, into `out`.
fn track(bytes: &[u8], n: u16, out: &mut Vec<(usize, Said)>) -> Result<(), String> {
    let mut r = Reader { bytes, at: 0 };
    let mut tick = 0usize;
    let mut running = 0u8;
    while r.at < bytes.len() {
        tick = tick.saturating_add(r.number()?);
        let mut status = r.byte()?;
        // Running status: the byte is the first data byte of the last status.
        let mut first = None;
        if status < 0x80 {
            if running == 0 {
                return Err("a track has data with no status byte".into());
            }
            first = Some(status);
            status = running;
        }
        match status {
            0xFF => {
                running = 0;
                let kind = r.byte()?;
                let len = r.number()?;
                if kind == 0x51 && len == 3 {
                    let mut tempo = 0;
                    for _ in 0..3 {
                        tempo = tempo << 8 | u32::from(r.byte()?);
                    }
                    out.push((tick, Said::Tempo(tempo)));
                } else {
                    r.skip(len)?;
                    if kind == 0x2F {
                        break; // end of track
                    }
                }
            }
            // System exclusive is not passed on.
            0xF0 | 0xF7 => {
                running = 0;
                let len = r.number()?;
                r.skip(len)?;
            }
            0x80..=0xEF => {
                running = status;
                let data = match first {
                    Some(b) => b,
                    None => r.byte()?,
                };
                let second = if matches!(status & 0xF0, 0xC0 | 0xD0) {
                    0
                } else {
                    r.byte()?
                };
                out.push((tick, Said::Message(n, [status, data & 0x7F, second & 0x7F])));
            }
            _ => {
                return Err(format!(
                    "it has an unexpected byte {status:#04X} in a track"
                ));
            }
        }
    }
    Ok(())
}

// MARK: playing it

/// What the editor asks of the audio thread.
enum Command {
    Play(Box<Song>),
    Stop,
}

/// Playing songs, shared by the editor, the tick and the audio thread.
pub struct Songs {
    commands: ArrayQueue<Command>,
    /// Songs the audio thread is done with, freed on the tick.
    retired: ArrayQueue<Box<Song>>,
    /// The "Play MIDI" picker.
    pub dialog: Dialog,
    playing: AtomicBool,
    /// Pause, seek, speed, loop and the rest, and where the song is: the
    /// audio clock as the editor sees it ([`Songs::position_at`]).
    transport: Transport,
    /// What plays: its name and length in seconds, set as it starts.
    title: Mutex<Option<(String, f64)>>,
    /// What plays: its title, composer and voices.
    about: Mutex<About>,
    /// What plays' notes, for the editor.
    notes: Mutex<Notes>,
    /// The folder the last song came from, where the picker opens.
    folder: Mutex<Option<PathBuf>>,
    /// The second the header last showed: the tick redraws as it moves.
    shown: AtomicU32,
    /// Whether `KONTRA_PLAY` has been read ([`Songs::autoplay`]).
    autoplayed: AtomicBool,
}

impl Default for Songs {
    fn default() -> Self {
        Self {
            commands: ArrayQueue::new(8),
            retired: ArrayQueue::new(8),
            dialog: Dialog::new(Pick::Song),
            playing: AtomicBool::new(false),
            transport: Transport::default(),
            title: Mutex::new(None),
            about: Mutex::new(About::default()),
            notes: Mutex::new(Notes::default()),
            folder: Mutex::new(None),
            shown: AtomicU32::new(u32::MAX),
            autoplayed: AtomicBool::new(false),
        }
    }
}

impl Songs {
    /// Reads the file at `path` and plays it, replacing what plays.
    pub fn open(&self, path: &Path) -> Result<(), String> {
        let song = Song::open(path)?;
        *lock(&self.folder) = path.parent().map(Path::to_path_buf);
        self.play(song);
        Ok(())
    }

    /// Plays `song`, replacing what plays.
    pub fn play(&self, mut song: Song) {
        *lock(&self.title) = Some((song.name.clone(), song.length));
        // The audio thread has no use for them.
        *lock(&self.notes) = std::mem::take(&mut song.notes);
        *lock(&self.about) = song.about.clone();
        // Before 0 by its count-in, if it has one.
        let start = self.transport.begin(&song);
        self.transport.head.stamp(start, 0., start, 1.0);
        self.playing.store(true, Ordering::Release);
        // Full only if the audio thread has stopped: nothing would play it.
        let _ = self.commands.push(Command::Play(Box::new(song)));
    }

    /// Stops what plays: shown at once, its notes end on the next block.
    pub fn stop(&self) {
        self.playing.store(false, Ordering::Release);
        self.transport.end();
        let _ = self.commands.push(Command::Stop);
    }

    /// What plays: its title, composer and voices.
    pub fn about(&self) -> About {
        lock(&self.about).clone()
    }

    /// The MIDI file `KONTRA_PLAY` names, once per plugin: a way to start a
    /// song without the pointer, for tests and screenshots of the standalone.
    pub fn autoplay(&self) -> Option<PathBuf> {
        if self.autoplayed.swap(true, Ordering::AcqRel) {
            return None;
        }
        std::env::var_os("KONTRA_PLAY").map(PathBuf::from)
    }

    /// Where the picker opens: the last song's folder.
    pub fn folder(&self) -> Option<PathBuf> {
        lock(&self.folder).clone()
    }

    /// While a song plays: its name, where it is and its length, in seconds.
    /// The editor reads [`Songs::now`], which also has a song that waits.
    #[allow(dead_code, reason = "the song's plain status, for any caller")]
    pub fn status(&self) -> Option<(String, f64, f64)> {
        if !self.playing.load(Ordering::Acquire) {
            return None;
        }
        let (name, length) = lock(&self.title).clone()?;
        let position = self.position_at(Instant::now())?;
        Some((name, position.min(length), length))
    }

    /// While a song plays: where it is, in seconds, at `at` (the instant a
    /// frame draws for): the last block rendered, moved on by the time
    /// since at the song's speed, never past what has been rendered. It
    /// moves smoothly between blocks, so a picture of the song can be drawn
    /// from it every frame; paused, it holds still. The audio is heard
    /// later than this by the output's latency.
    pub fn position_at(&self, at: Instant) -> Option<f64> {
        if !self.playing.load(Ordering::Acquire) {
            return None;
        }
        Some(self.transport.head.at(at))
    }

    /// Seconds into the song at the end of the last block rendered.
    fn rendered(&self) -> f64 {
        self.transport.head.end()
    }

    /// The notes of the song playing, or the last one played.
    pub fn notes(&self) -> Notes {
        lock(&self.notes).clone()
    }

    /// On the tick: frees songs the audio thread is done with; true when
    /// what the header shows has changed.
    pub fn tick(&self) -> bool {
        while self.retired.pop().is_some() {}
        let now = if self.playing.load(Ordering::Acquire) {
            self.rendered() as u32
        } else {
            u32::MAX
        };
        self.shown.swap(now, Ordering::Relaxed) != now
    }

    /// On the audio thread, as a block starts: carries out what the editor
    /// asked, then puts the song's events in this block into `out`. True
    /// when it added any, so `out` needs putting back in time order.
    pub fn process(
        &self,
        now: &mut Option<Playing>,
        rate: f64,
        frames: usize,
        out: &mut Vec<(u32, Event)>,
    ) -> bool {
        let mut added = false;
        while let Some(command) = self.commands.pop() {
            if let Some(mut old) = now.take() {
                old.release(0, out);
                added = true;
                self.retire(old.song);
            }
            match command {
                Command::Play(song) => {
                    let settings = self.transport.settings();
                    *now = Some(Playing::new(song, &settings, rate));
                    self.playing.store(true, Ordering::Release);
                }
                Command::Stop => self.playing.store(false, Ordering::Release),
            }
        }
        let Some(playing) = now.as_mut() else {
            return added;
        };
        let before = out.len();
        let settings = self.transport.settings();
        let r = playing.play(&settings, &self.transport, rate, frames, out);
        let after = f64::from(r.from) / rate.max(1.0);
        self.transport.head.stamp(r.base, after, r.end, r.speed);
        // Played to its end: it stays, paused there, to play again.
        if r.ended {
            self.transport.finish();
            self.playing.store(false, Ordering::Release);
        }
        if r.replayed {
            self.playing.store(true, Ordering::Release);
        }
        added || out.len() > before
    }

    /// Hands `song` to the tick to free; freed here only if the tick is far
    /// behind.
    fn retire(&self, song: Box<Song>) {
        let _ = self.retired.push(song);
    }
}

/// Puts `events` in frame order, keeping the order of events at one frame,
/// without allocating: an insertion sort over a block's few events.
pub fn sort_by_frame(events: &mut [(u32, Event)]) {
    for i in 1..events.len() {
        let mut j = i;
        while j > 0 && events.get(j - 1).map(|e| e.0) > events.get(j).map(|e| e.0) {
            events.swap(j - 1, j);
            j -= 1;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A variable-length quantity.
    fn number(mut n: usize) -> Vec<u8> {
        let mut out = vec![(n & 0x7F) as u8];
        n >>= 7;
        while n > 0 {
            out.insert(0, (n & 0x7F) as u8 | 0x80);
            n >>= 7;
        }
        out
    }

    fn chunk(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(body);
        out
    }

    /// A track from (tick, bytes) in order, closed with end of track.
    pub(crate) fn track(events: &[(usize, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        let mut last = 0;
        for &(tick, bytes) in events {
            body.extend(number(tick - last));
            body.extend(bytes);
            last = tick;
        }
        body.extend([0, 0xFF, 0x2F, 0]);
        chunk(b"MTrk", &body)
    }

    pub(crate) fn file(format: u16, division: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend(format.to_be_bytes());
        header.extend((tracks.len() as u16).to_be_bytes());
        header.extend(division.to_be_bytes());
        let mut out = chunk(b"MThd", &header);
        for t in tracks {
            out.extend(t);
        }
        out
    }

    /// 120 BPM, then 60 BPM from the second bar; C D E F as quarters (the
    /// last three in running status), G after the change, a drum hit.
    pub(crate) fn song() -> Vec<u8> {
        let tempo = track(&[
            (0, &[0xFF, 0x51, 3, 0x07, 0xA1, 0x20]),
            (1920, &[0xFF, 0x51, 3, 0x0F, 0x42, 0x40]),
        ]);
        let melody = track(&[
            (0, &[0xB0, 64, 127]),
            (0, &[0x90, 60, 100]),
            (480, &[60, 0]),
            (480, &[62, 100]),
            (960, &[62, 0]),
            (960, &[64, 100]),
            (1440, &[64, 0]),
            (1440, &[65, 100]),
            (1920, &[65, 0]),
            (1920, &[67, 90]),
            (2400, &[0x80, 67, 0]),
        ]);
        let drums = track(&[(0, &[0x99, 36, 127]), (240, &[0x89, 36, 0])]);
        file(1, 480, &[tempo, melody, drums])
    }

    fn times(song: &Song, status: u8) -> Vec<(f64, u8)> {
        song.events
            .iter()
            .filter(|&&(_, [s, _, v])| s == status && v > 0)
            .map(|&(t, [_, key, _])| ((t * 1000.0).round() / 1000.0, key))
            .collect()
    }

    #[test]
    fn notes_follow_the_tempo_map_and_running_status() {
        let song = Song::read("a.mid".into(), &song()).unwrap();
        assert_eq!(
            times(&song, 0x90),
            [(0.0, 60), (0.5, 62), (1.0, 64), (1.5, 65), (2.0, 67)]
        );
        // G lasts a quarter at 60 BPM: a second.
        assert!((song.length - 3.0).abs() < 1e-9, "{}", song.length);
        assert_eq!(song.events.first().map(|e| e.1), Some([0xB0, 64, 127]));
    }

    #[test]
    fn notes_pair_by_key_and_take_their_voice_from_track_and_channel() {
        let song = Song::read("a.mid".into(), &song()).unwrap();
        let seen: Vec<(f64, f64, u8, u8)> = song
            .notes
            .between(0.0, 10.0)
            .map(|n| (n.start, n.end, n.key, n.voice))
            .collect();
        assert_eq!(
            seen,
            [
                (0.0, 0.5, 60, 0),
                (0.5, 1.0, 62, 0),
                (1.0, 1.5, 64, 0),
                (1.5, 2.0, 65, 0),
                (2.0, 3.0, 67, 0)
            ],
            "the drum hit is not among them"
        );
        // Two hands on one channel, in two tracks: a key struck again ends
        // its last note; a note never released lasts to the end.
        let right = track(&[
            (0, &[0x90, 60, 100]),
            (240, &[0x90, 60, 80]),
            (480, &[0x80, 60, 0]),
            (960, &[0x80, 60, 0]),
        ]);
        let left = track(&[(0, &[0x90, 48, 50]), (720, &[0xB0, 64, 0])]);
        let two = Song::read("b.mid".into(), &file(1, 480, &[right, left])).unwrap();
        let seen: Vec<(f64, f64, u8, u8, u8)> = two
            .notes
            .between(0.0, 10.0)
            .map(|n| (n.start, n.end, n.key, n.velocity, n.voice))
            .collect();
        assert_eq!(
            seen,
            [
                (0.0, 0.25, 60, 100, 0),
                (0.0, 1.0, 48, 50, 1),
                (0.25, 0.5, 60, 80, 0)
            ]
        );
    }

    #[test]
    fn the_editor_keeps_a_songs_notes_and_the_audio_thread_does_not() {
        let songs = Songs::default();
        assert!(songs.notes().is_empty());
        songs.play(Song::read("a.mid".into(), &song()).unwrap());
        assert_eq!(songs.notes().len(), 5);
        assert_eq!(songs.notes().keys(), Some((60, 67)));
        let mut now = None;
        play(&songs, &mut now, 512, 1);
        assert!(now.as_ref().is_some_and(|p| p.song.notes.is_empty()));
    }

    #[test]
    fn the_drum_channel_is_skipped() {
        let song = Song::read("a.mid".into(), &song()).unwrap();
        assert!(song.events.iter().all(|&(_, [s, ..])| s & 15 != DRUMS));
        let drums_only = file(
            0,
            480,
            &[track(&[(0, &[0x99, 36, 127]), (240, &[0x89, 36, 0])])],
        );
        assert!(Song::read("d.mid".into(), &drums_only).is_err());
    }

    #[test]
    fn smpte_division_counts_frames() {
        // 25 frames a second, 40 ticks a frame: a millisecond a tick.
        let division = u16::from_be_bytes([(256 - 25) as u8, 40]);
        let t = track(&[(0, &[0x90, 60, 100]), (250, &[0x90, 62, 100])]);
        let song = Song::read("s.mid".into(), &file(0, division, &[t])).unwrap();
        assert_eq!(times(&song, 0x90), [(0.0, 60), (0.25, 62)]);
    }

    #[test]
    fn broken_files_say_why() {
        assert!(Song::read("x".into(), b"RIFF....").is_err());
        // Cut inside a track's last event (its end-of-track length).
        let mut cut = file(0, 480, &[track(&[(0, &[0x90, 60, 100])])]);
        cut.truncate(cut.len() - 1);
        assert!(Song::read("x".into(), &cut).is_err());
        let no_status = file(0, 480, &[chunk(b"MTrk", &[0, 60, 100])]);
        assert!(Song::read("x".into(), &no_status).is_err());
    }

    /// Blocks of `frames` at 48 kHz until the song ends or `blocks` pass:
    /// each event at its frame from the start.
    fn play(
        songs: &Songs,
        now: &mut Option<Playing>,
        frames: usize,
        blocks: usize,
    ) -> Vec<(u64, Event)> {
        let mut out = Vec::with_capacity(64);
        let mut all = Vec::new();
        for b in 0..blocks {
            out.clear();
            songs.process(now, 48_000.0, frames, &mut out);
            all.extend(
                out.iter()
                    .map(|&(at, e)| ((b * frames) as u64 + u64::from(at), e)),
            );
        }
        all
    }

    fn key_on(e: &Event) -> Option<u8> {
        match e {
            Event::NoteOn { note, .. } => Some(note.key),
            _ => None,
        }
    }

    #[test]
    fn events_land_on_their_frame_across_blocks() {
        let songs = Songs::default();
        songs.play(Song::read("a.mid".into(), &song()).unwrap());
        let mut now = None;
        let events = play(&songs, &mut now, 512, 400);
        let ons: Vec<(u64, u8)> = events
            .iter()
            .filter_map(|(at, e)| Some((*at, key_on(e)?)))
            .collect();
        assert_eq!(
            ons,
            [
                (0, 60),
                (24_000, 62),
                (48_000, 64),
                (72_000, 65),
                (96_000, 67)
            ]
        );
        // Ended by itself: the pedal lifted, nothing playing; it waits at
        // its end to play again.
        assert!(now.as_ref().is_some_and(Playing::ended));
        assert!(songs.status().is_none());
        assert!(songs.now().is_some_and(|n| n.ended && n.paused));
        assert!(matches!(events.last(), Some((_, e)) if *e == Event::midi1(0xB0, 64, 0)));
        assert!(events.iter().all(|(_, e)| e.channel() != Some(DRUMS)));
    }

    #[test]
    fn the_header_redraws_as_the_clock_moves() {
        let songs = Songs::default();
        let mut now = None;
        assert!(!songs.tick(), "nothing playing, nothing to show");
        songs.play(Song::read("a.mid".into(), &song()).unwrap());
        play(&songs, &mut now, 512, 1);
        assert!(songs.tick(), "a song started");
        assert!(!songs.tick(), "the same second");
        // 512-frame blocks at 48 kHz: 94 make a second.
        play(&songs, &mut now, 512, 94);
        assert!(songs.tick(), "the next second");
        play(&songs, &mut now, 512, 400);
        assert!(songs.tick(), "it ended");
        assert!(!songs.tick());
    }

    #[test]
    fn stopping_ends_held_keys_and_the_pedal() {
        let songs = Songs::default();
        songs.play(Song::read("a.mid".into(), &song()).unwrap());
        let mut now = None;
        // Into the first note, with the pedal down.
        play(&songs, &mut now, 512, 10);
        assert!(songs.status().is_some());
        songs.stop();
        assert!(songs.status().is_none());
        let after = play(&songs, &mut now, 512, 1);
        let off = Event::NoteOff(HostPattern {
            port: i32::from(PORT),
            channel: 0,
            key: 60,
            id: -1,
            clap: false,
        });
        assert_eq!(after, [(0, off), (0, Event::midi1(0xB0, 64, 0))]);
        assert!(now.is_none());
        // Nothing more once stopped.
        assert!(play(&songs, &mut now, 512, 5).is_empty());
    }

    #[test]
    fn a_new_song_replaces_the_one_playing() {
        let songs = Songs::default();
        let mut now = None;
        songs.play(Song::read("a.mid".into(), &song()).unwrap());
        play(&songs, &mut now, 512, 10);
        songs.play(Song::read("b.mid".into(), &song()).unwrap());
        let next = play(&songs, &mut now, 512, 1);
        // The first song's key ends before the second's first key starts.
        let firsts: Vec<_> = next.iter().take(3).map(|(_, e)| *e).collect();
        assert!(matches!(firsts.first(), Some(Event::NoteOff(p)) if p.key == 60));
        assert!(next.iter().any(|(_, e)| key_on(e) == Some(60)));
        assert_eq!(songs.status().map(|s| s.0), Some("b.mid".into()));
    }

    /// The song holds the pedal from its start; ending, it lifts it on the
    /// frame of its last event (G's note-off at 3 s), not at the start of
    /// the block that event falls in.
    #[test]
    fn an_ending_song_lets_go_after_its_last_event_at_any_block_size() {
        for frames in [64, 512, 1000, 4096] {
            let songs = Songs::default();
            songs.play(Song::read("a.mid".into(), &song()).unwrap());
            let mut now = None;
            let events = play(&songs, &mut now, frames, 200_000 / frames);
            let up = events
                .iter()
                .rev()
                .find(|(_, e)| *e == Event::midi1(0xB0, 64, 0));
            assert_eq!(up.map(|e| e.0), Some(144_000), "blocks of {frames}");
            assert!(now.as_ref().is_some_and(Playing::ended));
        }
    }

    /// Between blocks the playhead moves on with the clock, from where the
    /// last block started and never past where it ended.
    #[test]
    fn the_playhead_moves_between_blocks_up_to_what_is_rendered() {
        let songs = Songs::default();
        assert_eq!(songs.position_at(Instant::now()), None);
        songs.play(Song::read("a.mid".into(), &song()).unwrap());
        assert_eq!(songs.position_at(Instant::now()), Some(0.0), "not begun");
        let mut now = None;
        // A tenth of a second, 0.0..0.1.
        play(&songs, &mut now, 4800, 1);
        let rendered = Instant::now();
        let at = |ms| songs.position_at(rendered + std::time::Duration::from_millis(ms));
        let half = at(50).unwrap();
        assert!((0.05..0.1).contains(&half), "{half}");
        assert_eq!(at(1000), Some(0.1), "held where the block ended");
        assert!(at(20).unwrap() < half);
        // The next block starts where the last ended.
        play(&songs, &mut now, 4800, 1);
        let next = songs.position_at(Instant::now()).unwrap();
        assert!((0.1..0.15).contains(&next), "{next}");
        songs.stop();
        assert_eq!(songs.position_at(Instant::now()), None);
    }

    #[test]
    fn a_full_block_holds_the_rest_for_the_next() {
        let songs = Songs::default();
        songs.play(Song::read("a.mid".into(), &song()).unwrap());
        let mut now = None;
        let mut out = Vec::with_capacity(1);
        songs.process(&mut now, 48_000.0, 512, &mut out);
        assert_eq!(out.len(), 1);
        out.clear();
        songs.process(&mut now, 48_000.0, 512, &mut out);
        // Late, so at the block's start.
        assert_eq!(
            out.first().map(|&(at, e)| (at, key_on(&e))),
            Some((0, Some(60)))
        );
    }

    #[test]
    fn sorting_keeps_one_frames_order() {
        let a = Event::midi1(0xB0, 1, 1);
        let b = Event::midi1(0xB0, 2, 2);
        let c = Event::midi1(0xB0, 3, 3);
        let mut events = [(5, a), (0, b), (5, c), (0, a)];
        sort_by_frame(&mut events);
        assert_eq!(events, [(0, b), (0, a), (5, a), (5, c)]);
    }
}

#[cfg(test)]
mod render;
