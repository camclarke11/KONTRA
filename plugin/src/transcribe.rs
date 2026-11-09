//! A recording turned into a MIDI song ([`kontra_transcribe`]): the player
//! picks or drops a recording, a thread of its own reads it, isolates its
//! piano from any voice or band (unless the Songs tab says not to, or the
//! separation model is missing), hears its notes and pedal, and writes
//! them as a MIDI file into the transcriptions folder
//! (`~/Music/KONTRA Transcriptions`), which the Songs tab then lists; the
//! tick plays it ([`crate::Tick`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use kontra_ui::transcribe::Transcribing;

use crate::folder::{Dialog, Pick};
use crate::lock;
use crate::record::smf;

/// How far a transcription is, and whether to stop it.
#[derive(Debug, Default)]
struct Progress {
    /// Done, in ten-thousandths.
    done: AtomicU32,
    cancel: AtomicBool,
    /// What it is doing: [`READING`], [`ISOLATING`] or [`LISTENING`].
    stage: AtomicU8,
}

const READING: u8 = 0;
const ISOLATING: u8 = 1;
const LISTENING: u8 = 2;

impl Progress {
    fn set(&self, done: f64) {
        let n = (done.clamp(0.0, 1.0) * 10_000.0).round() as u32;
        self.done.store(n, Ordering::Relaxed);
    }

    fn done(&self) -> f64 {
        f64::from(self.done.load(Ordering::Relaxed)) / 10_000.0
    }
}

struct Job {
    /// The recording's file name.
    name: String,
    progress: Arc<Progress>,
    thread: JoinHandle<Result<PathBuf, String>>,
}

/// Transcribing, shared by the editor and the tick.
pub struct Transcriber {
    job: Mutex<Option<Job>>,
    /// The last one's song, or why there is none.
    last: Mutex<Option<Result<PathBuf, String>>>,
    /// What the editor showed last, so the tick redraws as it moves.
    shown: AtomicU32,
    /// The "Transcribe a recording" picker.
    pub dialog: Dialog,
}

impl Default for Transcriber {
    fn default() -> Self {
        Self {
            job: Mutex::default(),
            last: Mutex::default(),
            shown: AtomicU32::new(u32::MAX),
            dialog: Dialog::new(Pick::Audio),
        }
    }
}

/// Where transcriptions go: `KONTRA Transcriptions` in the Music folder.
#[must_use]
pub fn folder() -> PathBuf {
    let music = dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .unwrap_or_default();
    music.join("KONTRA Transcriptions")
}

/// `folder/<stem>.mid`, or with a number when that is taken.
fn free(folder: &Path, stem: &str) -> PathBuf {
    let first = folder.join(format!("{stem}.mid"));
    if !first.exists() {
        return first;
    }
    (2u32..)
        .map(|n| folder.join(format!("{stem} {n}.mid")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// The piano alone in the recording at `audio`, as the transcription
/// hears it (mono at its rate), from the separator at `weights`;
/// `progress` told how far it is, from 0 to `share`.
fn isolate(
    audio: &Path,
    weights: &Path,
    progress: &Progress,
    share: f64,
) -> Result<Vec<f32>, String> {
    use kontra_transcribe::{RATE, audio, separate::Separator};
    let separator = Separator::load(weights)?;
    let rate = separator.settings.sample_rate;
    let [l, r] = audio::read_stereo(audio, rate)?;
    progress.stage.store(ISOLATING, Ordering::Relaxed);
    progress.set(0.01);
    let [pl, pr] = separator.piano(
        &l,
        &r,
        &|d| progress.set(0.01 + (share - 0.01) * d),
        &progress.cancel,
    )?;
    drop(separator);
    let mono: Vec<f32> = pl.iter().zip(&pr).map(|(a, b)| (a + b) * 0.5).collect();
    Ok(audio::resample(&mono, rate, RATE))
}

/// The recording at `audio` heard and written into `folder` as a MIDI
/// file, its piano `isolated` first when the separator is there;
/// `progress` told how far it is.
fn run(
    audio: &Path,
    folder: &Path,
    isolated: bool,
    progress: &Progress,
) -> Result<PathBuf, String> {
    let model = kontra_transcribe::model_path().ok_or(
        "the transcription model is not installed (piano-transcription.onnx; see tools/transcribe-model)",
    )?;
    // Isolating takes most of the time; transcribing what is left, the rest.
    let separator = kontra_transcribe::separator_path().filter(|_| isolated);
    let (samples, from) = match &separator {
        Some(weights) => (isolate(audio, weights, progress, 0.8)?, 0.8),
        None => (kontra_transcribe::audio::read(audio)?, 0.03),
    };
    progress.stage.store(LISTENING, Ordering::Relaxed);
    progress.set(from);
    if progress.cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let model = kontra_transcribe::Model::load(&model)?;
    let from = from + 0.02;
    progress.set(from);
    // Half the cores, at most four: each holds a few hundred megabytes
    // while it works, and the rack keeps playing beside it.
    let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
    let heard = model.transcribe(
        &samples,
        threads,
        &|d| progress.set(from + (0.98 - from) * d),
        &progress.cancel,
    )?;
    if heard.notes.is_empty() {
        return Err("no piano notes were heard in it".into());
    }
    let stem = audio.file_stem().map_or_else(
        || "Transcription".into(),
        |s| s.to_string_lossy().into_owned(),
    );
    let track = smf::Track {
        name: stem.clone(),
        events: heard.events(),
    };
    let bytes = smf::write(&stem, 120.0, &[track]);
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let path = free(folder, &stem);
    // Written whole, then named: the Songs tab never reads half a file.
    let temp = path.with_extension("mid.part");
    std::fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
    std::fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    progress.set(1.0);
    Ok(path)
}

impl Transcriber {
    /// Starts transcribing the recording at `audio` on a thread of its
    /// own, into `folder`, its piano `isolated` first, unless one is under
    /// way.
    pub fn start(&self, audio: PathBuf, folder: PathBuf, isolated: bool) {
        let mut job = lock(&self.job);
        if job.is_some() {
            return;
        }
        let name = audio.file_name().map_or_else(
            || audio.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let progress = Arc::new(Progress::default());
        let theirs = Arc::clone(&progress);
        let thread = std::thread::Builder::new()
            .name("kontra-transcribe".into())
            .spawn(move || run(&audio, &folder, isolated, &theirs));
        match thread {
            Ok(thread) => {
                *job = Some(Job {
                    name,
                    progress,
                    thread,
                });
            }
            Err(e) => *lock(&self.last) = Some(Err(format!("it did not start: {e}"))),
        }
    }

    /// Stops the one under way; nothing of it is kept.
    pub fn cancel(&self) {
        if let Some(job) = lock(&self.job).as_ref() {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// What the Songs tab shows.
    pub fn shown(&self) -> Transcribing {
        let job = lock(&self.job).as_ref().map(|j| {
            let what = match j.progress.stage.load(Ordering::Relaxed) {
                READING => format!("Reading {}", j.name),
                ISOLATING => format!("Isolating the piano in {}", j.name),
                _ => format!("Transcribing {}", j.name),
            };
            (what, j.progress.done())
        });
        let last = lock(&self.last).as_ref().map(|r| match r {
            Ok(path) => format!(
                "Made {}",
                path.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            ),
            Err(e) => e.clone(),
        });
        Transcribing {
            job,
            last,
            ready: kontra_transcribe::model_path().is_some(),
            separator: kontra_transcribe::separator_path().is_some(),
        }
    }

    /// On the tick: a finished transcription's outcome, once, and whether
    /// what the editor shows moved.
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
            .unwrap_or_else(|_| Err("it stopped unexpectedly".into()));
        // A cancel leaves nothing to say.
        *lock(&self.last) = match &outcome {
            Err(e) if e == "cancelled" => None,
            r => Some(r.clone()),
        };
        (Some(outcome), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_name_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("kontra-transcribe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free(&dir, "Take"), dir.join("Take.mid"));
        std::fs::write(dir.join("Take.mid"), b"").unwrap();
        assert_eq!(free(&dir, "Take"), dir.join("Take 2.mid"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Without the model there is nothing to hear with: it says so.
    #[test]
    fn without_the_model_it_says_what_is_missing() {
        if kontra_transcribe::model_path().is_some() {
            return;
        }
        let t = Transcriber::default();
        t.start(
            PathBuf::from("/nowhere/take.wav"),
            std::env::temp_dir(),
            false,
        );
        let outcome = loop {
            if let (Some(o), _) = t.tick() {
                break o;
            }
            std::thread::yield_now();
        };
        assert!(outcome.unwrap_err().contains("not installed"));
        assert!(t.shown().last.is_some_and(|l| l.contains("not installed")));
    }
}
