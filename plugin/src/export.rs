//! Exporting the song played last: as audio, rendered offline through the
//! rack's instruments into a 48 kHz 24-bit WAV ([`audio`], [`wav`]), or as
//! a video of its notes falling onto the keys, encoded by ffmpeg into an
//! MP4 with that WAV as its sound when it has been exported ([`video`]).
//!
//! An export runs on a thread of its own, faster than real time and apart
//! from the audio thread, while the player keeps playing; the header shows
//! its progress and can cancel it. Files go to the recordings folder
//! ([`crate::record`]), named after the song; a new export of a song
//! replaces its last only once it is complete.

pub mod audio;
pub mod video;
pub mod wav;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;

use kontra_host::state::Saved;
use kontra_ui::export::Exporting;

use crate::lock;
use crate::song::Song;

/// How far an export is, and whether it should stop.
#[derive(Debug, Default)]
pub struct Progress {
    /// Done, in ten-thousandths.
    done: AtomicU32,
    cancel: AtomicBool,
}

impl Progress {
    pub fn set(&self, done: f64) {
        let n = (done.clamp(0.0, 1.0) * 10_000.0).round() as u32;
        self.done.store(n, Ordering::Relaxed);
    }

    pub fn done(&self) -> f64 {
        f64::from(self.done.load(Ordering::Relaxed)) / 10_000.0
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// What to export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Video { height: u32, fps: u32 },
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Audio => "Exporting audio",
            Self::Video { .. } => "Exporting video",
        }
    }
}

/// What an export needs from the plugin, taken on the tick.
pub struct Order {
    pub kind: Kind,
    /// The song's file.
    pub song: PathBuf,
    /// The rack as saved: what the audio export loads.
    pub saved: Saved,
    pub mix: audio::Mix,
    /// The touch-ups the song plays with, as the Songs tab has them.
    pub touch: kontra_library::Touch,
    /// The Design a video's falling notes are drawn in, by name.
    pub design: String,
    /// The folder the file goes to.
    pub folder: PathBuf,
}

struct Job {
    kind: Kind,
    progress: std::sync::Arc<Progress>,
    thread: JoinHandle<Result<PathBuf, String>>,
}

/// The export under way and the last one's outcome, shared by the editor
/// and the tick.
#[derive(Default)]
pub struct Exports {
    job: Mutex<Option<Job>>,
    /// The last file exported, or why the last export failed.
    last: Mutex<Option<Result<PathBuf, String>>>,
    /// The progress last shown, so the tick redraws as it moves.
    shown: AtomicU32,
}

/// `song`'s file name without its extension: what its exports are called.
fn stem(song: &Path) -> String {
    song.file_stem()
        .map_or_else(|| "Song".into(), |s| s.to_string_lossy().into_owned())
}

/// Where `song`'s audio export goes in `folder`.
pub fn wav_path(folder: &Path, song: &Path) -> PathBuf {
    folder.join(format!("{}.wav", stem(song)))
}

/// Where `song`'s video export goes in `folder`.
pub fn video_path(folder: &Path, song: &Path) -> PathBuf {
    folder.join(format!("{}.mp4", stem(song)))
}

/// `path`, with ".part" before its extension: written there first, so a
/// cancelled or failed export leaves the last good one as it was.
fn part(path: &Path) -> PathBuf {
    let ext = path
        .extension()
        .map_or_else(String::new, |e| e.to_string_lossy().into_owned());
    path.with_extension(format!("part.{ext}"))
}

/// Carries out `order`, reporting to `progress`; the file written.
fn run(order: Order, progress: &Progress) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&order.folder)
        .map_err(|e| format!("cannot make {}: {e}", order.folder.display()))?;
    let (song, _) = Song::open(&order.song, &order.touch)
        .map_err(|e| format!("{}: {e}", order.song.display()))?;
    match order.kind {
        Kind::Audio => {
            let path = wav_path(&order.folder, &order.song);
            let saved = order.saved;
            let setup = move |rack: &mut kontra_host::rack::Rack| {
                let failed = rack.restore(&saved);
                match failed.first() {
                    Some((part, e)) => Err(format!("part {} did not load: {e}", part + 1)),
                    None => Ok(()),
                }
            };
            let audio = audio::render(setup, song, order.mix, progress)?;
            let temp = part(&path);
            wav::write(&temp, audio::RATE, &audio)?;
            std::fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(path)
        }
        Kind::Video { height, fps } => {
            let ffmpeg = video::ffmpeg()
                .ok_or("a video needs ffmpeg: install it (brew install ffmpeg) and try again")?;
            let path = video_path(&order.folder, &order.song);
            let wav = wav_path(&order.folder, &order.song);
            let sound = wav.exists().then_some(wav.as_path());
            // As long as its sound, or the song and a second to let it ring.
            let seconds = sound.and_then(wav::seconds).unwrap_or(song.length + 1.0);
            let video = video::Video {
                notes: &song.notes,
                name: &song.name,
                seconds,
                height,
                fps,
                audio: sound,
                design: &order.design,
            };
            let temp = part(&path);
            video::encode(&ffmpeg, &video, &temp, progress)?;
            std::fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(path)
        }
    }
}

impl Exports {
    /// Starts `order` on a thread of its own, unless an export is under way.
    pub fn start(&self, order: Order) {
        let mut job = lock(&self.job);
        if job.is_some() {
            return;
        }
        let progress = std::sync::Arc::new(Progress::default());
        let kind = order.kind;
        let theirs = std::sync::Arc::clone(&progress);
        let thread = std::thread::Builder::new()
            .name("kontra-export".into())
            .spawn(move || run(order, &theirs));
        match thread {
            Ok(thread) => {
                *job = Some(Job {
                    kind,
                    progress,
                    thread,
                })
            }
            Err(e) => *lock(&self.last) = Some(Err(format!("the export did not start: {e}"))),
        }
    }

    /// Stops the export under way; nothing of it is kept.
    pub fn cancel(&self) {
        if let Some(job) = lock(&self.job).as_ref() {
            job.progress.cancel();
        }
    }

    /// The last file exported.
    pub fn last(&self) -> Option<PathBuf> {
        lock(&self.last).clone().and_then(Result::ok)
    }

    /// What the header shows; `song` is the song there is to export.
    pub fn shown(&self, song: Option<&Path>) -> Exporting {
        let job = lock(&self.job)
            .as_ref()
            .map(|j| (j.kind.label().to_owned(), j.progress.done()));
        let last = lock(&self.last).as_ref().map(|r| match r {
            Ok(path) => format!("saved {}", path.display()),
            Err(e) => e.clone(),
        });
        let file = (self.last().as_deref())
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned());
        Exporting {
            song: song
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned()),
            job,
            last,
            file,
        }
    }

    /// On the tick: takes a finished export's outcome; that outcome, once,
    /// and whether what the header shows moved.
    pub fn tick(&self) -> (Option<Result<PathBuf, String>>, bool) {
        let mut job = lock(&self.job);
        let done = job
            .as_ref()
            .map_or(0, |j| j.progress.done.load(Ordering::Relaxed) / 50);
        let moved = self.shown.swap(done, Ordering::Relaxed) != done;
        if !job.as_ref().is_some_and(|j| j.thread.is_finished()) {
            return (None, moved);
        }
        let Some(finished) = job.take() else {
            return (None, moved);
        };
        let outcome = finished
            .thread
            .join()
            .unwrap_or_else(|_| Err("the export stopped unexpectedly".into()));
        *lock(&self.last) = Some(outcome.clone());
        (Some(outcome), true)
    }
}

/// Shows `path` in the system's file browser: selected in the Finder on
/// macOS, its folder elsewhere. A thread waits on the opener so it never
/// lingers.
pub fn reveal(path: &Path) {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        if path.is_file() {
            c.arg("-R");
        }
        c.arg(path);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        });
        c
    };
    if let Ok(mut child) = command.spawn() {
        std::thread::spawn(move || child.wait());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_are_named_after_the_song_and_written_aside_first() {
        let folder = Path::new("/out");
        let song = Path::new("/songs/Clair de lune.mid");
        assert_eq!(wav_path(folder, song), Path::new("/out/Clair de lune.wav"));
        assert_eq!(
            video_path(folder, song),
            Path::new("/out/Clair de lune.mp4")
        );
        assert_eq!(
            part(&wav_path(folder, song)),
            Path::new("/out/Clair de lune.part.wav")
        );
    }

    /// The whole path, through the tick's order: a song, an empty rack,
    /// a WAV of the song's length in the folder; a missing song says why.
    #[test]
    fn an_order_runs_on_its_own_thread_to_a_file() {
        let dir = std::env::temp_dir().join(format!("kontra-exports-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let song = dir.join("scale.mid");
        std::fs::write(&song, crate::song::tests::song()).unwrap();
        let exports = Exports::default();
        let order = |song: &Path| Order {
            kind: Kind::Audio,
            song: song.to_path_buf(),
            saved: Saved::new(Vec::new()),
            mix: audio::Mix {
                gain: 1.0,
                align: false,
            },
            touch: kontra_library::Touch::NONE,
            design: String::new(),
            folder: dir.join("out"),
        };
        exports.start(order(&song));
        let outcome = loop {
            if let (Some(outcome), _) = exports.tick() {
                break outcome;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let path = outcome.unwrap();
        assert_eq!(path, dir.join("out/scale.wav"));
        // 3 s of song at 48 kHz, 6 bytes a frame.
        let len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(len, 44 + 6 * 3 * 48_000);
        assert!(!part(&path).exists());
        assert_eq!(exports.last(), Some(path));
        assert!(
            exports
                .shown(Some(&song))
                .last
                .unwrap()
                .starts_with("saved")
        );

        exports.start(order(&dir.join("missing.mid")));
        let outcome = loop {
            if let (Some(outcome), _) = exports.tick() {
                break outcome;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(outcome.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A whole export by hand, with sound: `KONTRA_DEMO_MIDI` rendered
    /// through a synthetic piano (a generated sample of decaying harmonics,
    /// keytracked, its release a third of a second) into a WAV in
    /// `KONTRA_DEMO_OUT`, then its video at 1080p30 with that WAV, each
    /// timed against the song's length.
    #[test]
    #[ignore = "a demo: set KONTRA_DEMO_MIDI and KONTRA_DEMO_OUT"]
    fn demo_export_through_a_synthetic_piano() {
        let (Some(midi), Some(out)) = (
            std::env::var_os("KONTRA_DEMO_MIDI").map(PathBuf::from),
            std::env::var_os("KONTRA_DEMO_OUT").map(PathBuf::from),
        ) else {
            eprintln!("skipped: no KONTRA_DEMO_MIDI and KONTRA_DEMO_OUT");
            return;
        };
        std::fs::create_dir_all(&out).unwrap();
        // Middle C, three seconds: six harmonics, the higher dying faster.
        let rate = 48_000.0;
        let sample: Vec<i16> = (0..144_000)
            .map(|n| {
                let t = f64::from(n) / rate;
                let x: f64 = (1..=6)
                    .map(|h| {
                        let h = f64::from(h);
                        (std::f64::consts::TAU * 261.63 * h * t).sin() / h * (-t * 1.2 * h).exp()
                    })
                    .sum();
                (x * 9000.0 * (t / 0.003).min(1.0)) as i16
            })
            .collect();
        let data = 2 * sample.len() as u32;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(48_000u32.to_le_bytes());
        wav.extend(96_000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data.to_le_bytes());
        wav.extend(sample.iter().flat_map(|s| s.to_le_bytes()));
        std::fs::write(out.join("piano-c4.wav"), wav).unwrap();
        let sfz = "<region> sample=piano-c4.wav lokey=0 hikey=127 pitch_keycenter=60 \
                   amp_veltrack=80 ampeg_release=0.35\n";
        let instrument = kontra_format_sfz::parse(sfz, &out);
        let setup = move |rack: &mut kontra_host::rack::Rack| {
            rack.load_ir(0, instrument, None).map_err(|e| e.to_string())
        };
        let song = Song::open(&midi, &kontra_library::Touch::NONE).unwrap().0;
        let length = song.length;
        let started = std::time::Instant::now();
        let mix = audio::Mix {
            gain: 1.0,
            align: false,
        };
        let audio = audio::render(setup, song, mix, &Progress::default()).unwrap();
        let rendered = started.elapsed().as_secs_f64();
        let wav = wav_path(&out, &midi);
        wav::write(&wav, audio::RATE, &audio).unwrap();
        let seconds = audio.len() as f64 / f64::from(audio::RATE);
        let peak = audio
            .iter()
            .map(|m| m[0].abs().max(m[1].abs()))
            .fold(0.0, f32::max);
        eprintln!(
            "audio: {length:.2} s song, {seconds:.2} s file (tail {:.2} s), peak {peak:.3}, \
             rendered in {rendered:.2} s ({:.0}x real time): {}",
            seconds - length,
            seconds / rendered,
            wav.display()
        );
        let Some(ffmpeg) = video::ffmpeg() else {
            eprintln!("no ffmpeg: no video");
            return;
        };
        let song = Song::open(&midi, &kontra_library::Touch::NONE).unwrap().0;
        let video = video::Video {
            notes: &song.notes,
            name: &song.name,
            seconds,
            height: 1080,
            fps: 30,
            audio: Some(&wav),
            design: "",
        };
        let mp4 = video_path(&out, &midi);
        let started = std::time::Instant::now();
        video::encode(&ffmpeg, &video, &mp4, &Progress::default()).unwrap();
        let took = started.elapsed().as_secs_f64();
        eprintln!(
            "video: {} frames in {took:.1} s ({:.1}x real time): {}",
            (seconds * 30.0).ceil(),
            seconds / took,
            mp4.display()
        );
    }
}
