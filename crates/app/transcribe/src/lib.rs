//! Piano audio to MIDI notes and pedal: High-resolution Piano Transcription
//! with Pedals by Regressing Onset and Offset Times (Q. Kong, B. Li, X. Song,
//! Y. Wan, Y. Wang, ByteDance, 2020). Code Apache-2.0; its trained model
//! CC BY 4.0 (zenodo.org/record/4034264), exported to one ONNX graph by
//! tools/transcribe-model and run here with tract, in pure Rust.
//!
//! The pipeline is the authors' own ([`post`] ports their post-processing):
//! audio as 16 kHz mono ([`audio`]), cut into 10 s segments half a segment
//! apart, each run through the model (in parallel, one segment a thread),
//! the segments' middles joined into one 100 frames-a-second picture of
//! onsets, offsets, sounding keys, velocities and the pedal, which become
//! notes with sub-frame onsets and offsets, and pedal presses.

pub mod audio;
pub mod post;
pub mod separate;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use tract_onnx::prelude::*;

pub use post::{Note, Pedal};

/// The model's sample rate.
pub const RATE: u32 = 16_000;

/// Frames a second of the model's output.
pub const FPS: f64 = 100.;

/// Samples in a segment (10 s), and between segments' starts (5 s).
const SEGMENT: usize = 160_000;
const HOP: usize = SEGMENT / 2;

/// Frames a segment gives (one more than 1000: its spectrogram is centred).
const FRAMES: usize = 1001;

/// Piano keys, from A0 (MIDI 21).
pub const KEYS: usize = 88;

/// The longest audio taken, in seconds.
pub const LONGEST: f64 = 3600.;

/// The model's outputs, in its graph's order: four for the keys, then the
/// pedal's offset and whether it is down.
const OUTPUTS: [usize; 6] = [KEYS, KEYS, KEYS, KEYS, 1, 1];

/// What a recording holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcription {
    /// In start order.
    pub notes: Vec<Note>,
    pub pedals: Vec<Pedal>,
}

impl Transcription {
    /// As MIDI channel messages on channel 1 at their time in seconds, in
    /// order: each note's on and off, the pedal down (127) and up (0).
    #[must_use]
    pub fn events(&self) -> Vec<(f64, [u8; 3])> {
        let mut out = Vec::with_capacity(self.notes.len() * 2 + self.pedals.len() * 2);
        for n in &self.notes {
            out.push((n.start, [0x90, n.key, n.velocity.clamp(1, 127)]));
            out.push((n.end, [0x80, n.key, 0]));
        }
        for p in &self.pedals {
            out.push((p.start, [0xB0, 64, 127]));
            out.push((p.end, [0xB0, 64, 0]));
        }
        // At one moment, a key let go before it is struck again, and the
        // pedal lifted before it goes down.
        let rank = |m: &[u8; 3]| match (m[0] & 0xF0, m[2]) {
            (0x80, _) | (0xB0, 0) => 0,
            _ => 1,
        };
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(rank(&a.1).cmp(&rank(&b.1))));
        out
    }
}

/// The model, loaded once and run from any thread.
pub struct Model {
    plan: std::sync::Arc<TypedSimplePlan>,
}

impl Model {
    /// The ONNX graph at `path` (tools/transcribe-model makes it).
    ///
    /// # Errors
    /// When it cannot be read or is not that model.
    pub fn load(path: &Path) -> Result<Self, String> {
        let fail = |e: TractError| format!("{}: {e}", path.display());
        let plan = tract_onnx::onnx()
            .model_for_path(path)
            .and_then(|m| m.with_input_fact(0, f32::fact([1, SEGMENT]).into()))
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(fail)?;
        Ok(Self { plan })
    }

    /// The model's six outputs for one segment, each frames by columns.
    fn segment(&self, audio: &[f32]) -> Result<Vec<Vec<f32>>, String> {
        let mut input = vec![0f32; SEGMENT];
        for (to, from) in input.iter_mut().zip(audio) {
            *to = *from;
        }
        let x = tract_ndarray::Array2::from_shape_vec((1, SEGMENT), input)
            .map_err(|e| e.to_string())?;
        let out = self
            .plan
            .run(tvec!(Tensor::from(x).into()))
            .map_err(|e| e.to_string())?;
        let mut all = Vec::with_capacity(OUTPUTS.len());
        for (o, columns) in out.iter().zip(OUTPUTS) {
            let view = o.to_plain_array_view::<f32>().map_err(|e| e.to_string())?;
            let values: Vec<f32> = view.iter().copied().collect();
            if values.len() != FRAMES * columns {
                return Err(format!("the model gave {:?}", o.shape()));
            }
            all.push(values);
        }
        if all.len() != OUTPUTS.len() {
            return Err("the model gave too few outputs".into());
        }
        Ok(all)
    }

    /// `audio` (mono at [`RATE`]) transcribed, run on up to `threads`
    /// threads, telling `progress` the share done (0..=1) as segments
    /// finish; stopped, with an error, once `cancel` is set.
    ///
    /// # Errors
    /// When cancelled, or the model fails.
    pub fn transcribe(
        &self,
        audio: &[f32],
        threads: usize,
        progress: &(dyn Fn(f64) + Sync),
        cancel: &AtomicBool,
    ) -> Result<Transcription, String> {
        // Whole segments: the last padded with silence.
        let segments = audio.len().div_ceil(SEGMENT).max(1) * 2 - 1;
        let results: Mutex<Vec<Option<Vec<Vec<f32>>>>> = Mutex::new(vec![None; segments]);
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let failed: Mutex<Option<String>> = Mutex::new(None);
        std::thread::scope(|s| {
            for _ in 0..threads.clamp(1, segments) {
                s.spawn(|| {
                    loop {
                        let n = next.fetch_add(1, Ordering::Relaxed);
                        if n >= segments || cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let from = (n * HOP).min(audio.len());
                        let to = (from + SEGMENT).min(audio.len());
                        let piece = audio.get(from..to).unwrap_or_default();
                        match self.segment(piece) {
                            Ok(out) => {
                                let mut r = results.lock().unwrap_or_else(PoisonError::into_inner);
                                if let Some(slot) = r.get_mut(n) {
                                    *slot = Some(out);
                                }
                            }
                            Err(e) => {
                                *failed.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
                                cancel.store(true, Ordering::Relaxed);
                            }
                        }
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        progress(d as f64 / segments as f64);
                    }
                });
            }
        });
        if let Some(e) = failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
            return Err(e);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let results = results.into_inner().unwrap_or_else(PoisonError::into_inner);
        let results: Option<Vec<Vec<Vec<f32>>>> = results.into_iter().collect();
        let results = results.ok_or("a segment went missing")?;
        // The frames the audio itself covers.
        let frames = audio.len() / (RATE as usize / FPS as usize) + 1;
        let joined: Vec<Vec<f32>> = OUTPUTS
            .iter()
            .enumerate()
            .map(|(k, &columns)| {
                let parts: Vec<&[f32]> = results
                    .iter()
                    .map(|r| r.get(k).map_or(&[][..], Vec::as_slice))
                    .collect();
                let mut all = deframe(&parts, columns);
                all.truncate(frames * columns);
                all
            })
            .collect();
        Ok(post::transcription(&joined))
    }
}

/// Segments' outputs (each [`FRAMES`] by `columns`) joined into one: the
/// first segment's first three quarters, each next one's middle half, the
/// last one's last three quarters, as the authors join them. A lone
/// segment is kept whole.
fn deframe(segments: &[&[f32]], columns: usize) -> Vec<f32> {
    if let [only] = segments {
        return only.to_vec();
    }
    let last = segments.len().saturating_sub(1);
    let mut out = Vec::with_capacity(segments.len() * 500 * columns);
    for (n, s) in segments.iter().enumerate() {
        // The last frame of each is dropped: 1000 frames, in quarters.
        let (from, to) = match n {
            0 => (0, 750),
            n if n == last => (250, 1000),
            _ => (250, 750),
        };
        out.extend_from_slice(s.get(from * columns..to * columns).unwrap_or_default());
    }
    out
}

/// Where the transcription model is looked for: `KONTRA_TRANSCRIBE_MODEL`,
/// beside the app (in a macOS bundle's Resources), then the app's data
/// folder.
#[must_use]
pub fn model_path() -> Option<std::path::PathBuf> {
    find("piano-transcription.onnx", "KONTRA_TRANSCRIBE_MODEL")
}

/// Where the separation model ([`separate`]) is looked for, as
/// [`model_path`] looks: `KONTRA_SEPARATE_MODEL` first.
#[must_use]
pub fn separator_path() -> Option<std::path::PathBuf> {
    find("piano-separation.safetensors", "KONTRA_SEPARATE_MODEL")
}

/// The file `name`: at the path in `var`, beside the app, or in the app's
/// data folder's `models`.
fn find(name: &str, var: &str) -> Option<std::path::PathBuf> {
    let env = std::env::var_os(var).map(std::path::PathBuf::from);
    let exe = std::env::current_exe().ok();
    let beside = exe
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| [dir.join("../Resources").join(name), dir.join(name)]);
    let data = dirs::data_dir().map(|d| d.join("kontra-v3").join("models").join(name));
    env.into_iter()
        .chain(beside.into_iter().flatten())
        .chain(data)
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_join_by_their_middles() {
        // Three segments of one column, each frame its segment's number.
        let seg = |n: f32| vec![n; FRAMES];
        let (a, b, c) = (seg(0.), seg(1.), seg(2.));
        let joined = deframe(&[&a, &b, &c], 1);
        assert_eq!(joined.len(), 750 + 500 + 750);
        assert!(joined[..750].iter().all(|&v| v == 0.));
        assert!(joined[750..1250].iter().all(|&v| v == 1.));
        assert!(joined[1250..].iter().all(|&v| v == 2.));
        assert_eq!(deframe(&[&a], 1).len(), FRAMES, "a lone one whole");
    }

    #[test]
    fn events_let_go_before_striking_again() {
        let t = Transcription {
            notes: vec![
                Note {
                    start: 0.,
                    end: 1.,
                    key: 60,
                    velocity: 200,
                },
                Note {
                    start: 1.,
                    end: 2.,
                    key: 60,
                    velocity: 0,
                },
            ],
            pedals: vec![Pedal {
                start: 0.5,
                end: 1.,
            }],
        };
        let e = t.events();
        assert_eq!(e[0], (0., [0x90, 60, 127]), "velocity in range");
        assert_eq!(e[1], (0.5, [0xB0, 64, 127]));
        assert_eq!(e[2].1, [0x80, 60, 0]);
        assert_eq!(e[3].1, [0xB0, 64, 0]);
        assert_eq!(e[4], (1., [0x90, 60, 1]));
    }
}
