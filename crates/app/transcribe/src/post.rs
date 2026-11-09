//! The model's frames to notes and pedal presses, as the authors' own
//! post-processing does it (piano_transcription_inference: utilities.py's
//! `RegressionPostProcessor` and piano_vad.py, Apache-2.0), with their
//! thresholds.
//!
//! An onset or offset is a peak of its regression over a threshold, the
//! highest for a few frames either side, placed between frames by the
//! slope either side of it. Beyond the authors, two kinds of onset are no
//! note: one on a silent key that then never sounds (a ghost, from a
//! voice or what isolating the piano left of one), and a weak one on a
//! key already sounding (the note going on, not struck again). A note
//! runs from its onset until its key's frame output fades or an offset
//! comes (whichever the shape suggests), the next onset on its key, or
//! six seconds. The pedal goes down where its frame output rises past a
//! half and comes up at its offset or once it has faded for a tenth of a
//! second.

use crate::{FPS, KEYS};

/// Onsets over this are notes struck; offsets over theirs, keys let go.
const ONSET: f32 = 0.3;
const OFFSET: f32 = 0.3;
/// A key sounds while its frame output is over this.
const FRAME: f32 = 0.1;
/// The pedal's offset, and how far down it must be.
const PEDAL_OFFSET: f32 = 0.2;
const PEDAL_FRAME: f32 = 0.5;
/// The longest note, in frames: six seconds.
const LONGEST: usize = 600;
/// A silent key struck sounds: its frame output reaches this within
/// [`LOOK`] frames (a tenth of a second), or the onset was a ghost.
const GHOST: f32 = 0.3;
const LOOK: usize = 10;
/// A key sounding (its frame output this high in the three frames
/// before) is struck again only by an onset over [`RESTRIKE`].
const SOUNDING: f32 = 0.5;
const RESTRIKE: f32 = 0.4;

/// The first key the model hears: A0.
const LOWEST: u8 = 21;

/// One note heard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    /// In seconds.
    pub start: f64,
    pub end: f64,
    pub key: u8,
    /// 1..=127.
    pub velocity: u8,
}

/// The sustain pedal held down, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pedal {
    pub start: f64,
    pub end: f64,
}

/// Each frame of `x` that peaks over `threshold`, the highest for
/// `neighbour` frames either side (level with its neighbours counts), with
/// how far between frames the peak lies, -0.5..0.5; None elsewhere.
fn peaks(x: &[f32], threshold: f32, neighbour: usize) -> Vec<Option<f32>> {
    let at = |n: usize| x.get(n).copied().unwrap_or(0.);
    let mut out = vec![None; x.len()];
    for n in neighbour..x.len().saturating_sub(neighbour) {
        let v = at(n);
        if v <= threshold {
            continue;
        }
        let rises = (0..neighbour).all(|i| at(n - i) >= at(n - i - 1));
        let falls = (0..neighbour).all(|i| at(n + i) >= at(n + i + 1));
        if !(rises && falls) {
            continue;
        }
        let (before, after) = (at(n - 1), at(n + 1));
        let shift = if before > after {
            (after - before) / (v - after) / 2.
        } else {
            (after - before) / (v - before) / 2.
        };
        if let Some(o) = out.get_mut(n) {
            // A flat top has no slope to place it by: on the frame.
            *o = Some(if shift.is_finite() { shift } else { 0. });
        }
    }
    out
}

/// Of one key's onset `peaks`, those that strike it: on a silent key, it
/// then sounds (`frame` reaches [`GHOST`]); on a sounding one, the `onset`
/// is over [`RESTRIKE`]. On three recordings with a voice sung over them,
/// their piano isolated (tools/transcribe-model/README.md), this took
/// away a third of the false notes, for under a percent of the true ones;
/// on a song, nearly all its notes under 60 ms.
fn struck(peaks: Vec<Option<f32>>, onset: &[f32], frame: &[f32]) -> Vec<Option<f32>> {
    let at = |n: usize| frame.get(n).copied().unwrap_or(0.);
    let most = |from: usize, to: usize| (from..to).map(at).fold(0f32, f32::max);
    peaks
        .into_iter()
        .enumerate()
        .map(|(b, shift)| {
            let shift = shift?;
            let strikes = if most(b.saturating_sub(3), b) >= SOUNDING {
                onset.get(b).is_some_and(|&o| o >= RESTRIKE)
            } else {
                most(b, b + LOOK) >= GHOST
            };
            strikes.then_some(shift)
        })
        .collect()
}

/// One key's notes: (onset frame, offset frame, onset shift, offset
/// shift, velocity 0..1), in onset order.
fn key_notes(
    frame: &[f32],
    onset: &[Option<f32>],
    offset: &[Option<f32>],
    velocity: &[f32],
) -> Vec<(usize, usize, f32, f32, f32)> {
    let shift = |s: &[Option<f32>], n: usize| s.get(n).copied().flatten().unwrap_or(0.);
    let vel = |n: usize| velocity.get(n).copied().unwrap_or(0.);
    let last = onset.len().saturating_sub(1);
    let mut out = Vec::new();
    let mut begun: Option<usize> = None;
    let (mut faded, mut ended): (Option<usize>, Option<usize>) = (None, None);
    for i in 0..onset.len() {
        if onset.get(i).is_some_and(Option::is_some) {
            // Struck again before it ended: the last one ends just before.
            if let Some(b) = begun {
                out.push((b, i.saturating_sub(1), shift(onset, b), 0., vel(b)));
                (faded, ended) = (None, None);
            }
            begun = Some(i);
        }
        let Some(b) = begun.filter(|&b| i > b) else {
            continue;
        };
        if faded.is_none() && frame.get(i).is_some_and(|&f| f <= FRAME) {
            faded = Some(i);
        }
        if ended.is_none() && offset.get(i).is_some_and(Option::is_some) {
            ended = Some(i);
        }
        if let Some(f) = faded {
            // An offset well into the note, nearer its fading than its
            // start, ends it; else the fading does.
            let fin = match ended {
                Some(o) if o - b > f.saturating_sub(o) => o,
                _ => f,
            };
            out.push((b, fin, shift(onset, b), shift(offset, fin), vel(b)));
            (begun, faded, ended) = (None, None, None);
        } else if i - b >= LONGEST || i == last {
            out.push((b, i, shift(onset, b), shift(offset, i), vel(b)));
            (begun, faded, ended) = (None, None, None);
        }
    }
    out
}

/// The pedal's presses: (down frame, up frame, up shift), in order. One
/// still down at the end is left out, as the authors do.
fn pedal_presses(frame: &[f32], offset: &[Option<f32>]) -> Vec<(usize, usize, f32)> {
    let at = |n: usize| frame.get(n).copied().unwrap_or(0.);
    let shift = |n: usize| offset.get(n).copied().flatten().unwrap_or(0.);
    let mut out = Vec::new();
    let mut begun: Option<usize> = None;
    let (mut faded, mut ended): (Option<usize>, Option<usize>) = (None, None);
    for i in 1..frame.len() {
        if begun.is_none() && at(i) >= PEDAL_FRAME && at(i) > at(i - 1) {
            begun = Some(i);
        }
        let Some(b) = begun.filter(|&b| i > b) else {
            continue;
        };
        if faded.is_none() && at(i) <= PEDAL_FRAME {
            faded = Some(i);
        }
        if ended.is_none() && offset.get(i).is_some_and(Option::is_some) {
            ended = Some(i);
        }
        if let Some(o) = ended {
            out.push((b, o, shift(o)));
            (begun, faded, ended) = (None, None, None);
        } else if let Some(f) = faded.filter(|&f| i - f >= 10) {
            out.push((b, f, shift(f)));
            (begun, faded, ended) = (None, None, None);
        }
    }
    out
}

/// The model's joined outputs (onset, offset, frame, velocity by frames ×
/// 88 keys, then the pedal's offset and frame by frames) as notes and
/// pedal presses.
#[must_use]
pub fn transcription(outputs: &[Vec<f32>]) -> crate::Transcription {
    let get = |k: usize| outputs.get(k).map_or(&[][..], Vec::as_slice);
    let (onset, offset, frame, velocity) = (get(0), get(1), get(2), get(3));
    let frames = frame.len() / KEYS;
    let column = |x: &[f32], key: usize| -> Vec<f32> {
        x.iter()
            .skip(key)
            .step_by(KEYS)
            .take(frames)
            .copied()
            .collect()
    };
    let time = |frame: usize, shift: f32| (frame as f64 + f64::from(shift)) / FPS;
    let mut notes = Vec::new();
    for key in 0..KEYS {
        let (onsets, sounds) = (column(onset, key), column(frame, key));
        let on = struck(peaks(&onsets, ONSET, 2), &onsets, &sounds);
        let off = peaks(&column(offset, key), OFFSET, 4);
        let found = key_notes(&sounds, &on, &off, &column(velocity, key));
        for (b, f, s, e, v) in found {
            let start = time(b, s).max(0.);
            notes.push(crate::Note {
                start,
                // Never ending before it starts, nor in no time at all.
                end: time(f, e).max(start + 1. / FPS),
                key: LOWEST + key as u8,
                // As the authors scale it, kept in MIDI's range.
                velocity: ((v * 128.) as i32).clamp(1, 127) as u8,
            });
        }
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.key.cmp(&b.key)));
    let pedal_off = peaks(get(4), PEDAL_OFFSET, 4);
    let pedals = pedal_presses(get(5), &pedal_off)
        .into_iter()
        .map(|(b, f, e)| {
            let start = time(b, 0.);
            Pedal {
                start,
                end: time(f, e).max(start),
            }
        })
        .collect();
    crate::Transcription { notes, pedals }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peak_is_placed_between_frames_by_its_slopes() {
        let x = [0., 0., 0.15, 0.3, 0.4, 0.35, 0.2, 0.05, 0., 0.];
        let p = peaks(&x, 0.3, 2);
        let found: Vec<usize> = (0..x.len()).filter(|&n| p[n].is_some()).collect();
        assert_eq!(found, [4]);
        // Falling slower after than it rose: a little after the frame.
        let s = p[4].unwrap();
        assert!((s - (0.35 - 0.3) / (0.4 - 0.3) / 2.).abs() < 1e-6, "{s}");
        // Under the threshold, or not the highest around: none.
        assert!(peaks(&x, 0.45, 2).iter().all(Option::is_none));
        let flat = [0., 0., 0.5, 0.5, 0.5, 0., 0.];
        assert!(peaks(&flat, 0.3, 2).iter().flatten().all(|s| s.is_finite()));
    }

    #[test]
    fn a_note_runs_from_its_onset_until_its_frames_fade() {
        let n = 40;
        let mut frame = vec![0f32; n];
        for f in frame.iter_mut().take(20).skip(5) {
            *f = 0.9;
        }
        let mut onset = vec![None; n];
        onset[5] = Some(0.25);
        let offset = vec![None; n];
        let velocity = vec![0.5f32; n];
        let notes = key_notes(&frame, &onset, &offset, &velocity);
        assert_eq!(notes, [(5, 20, 0.25, 0., 0.5)]);
        // Struck again at 12: the first ends at 11.
        onset[12] = Some(0.);
        let notes = key_notes(&frame, &onset, &offset, &velocity);
        assert_eq!(notes.len(), 2);
        assert_eq!((notes[0].0, notes[0].1), (5, 11));
        assert_eq!((notes[1].0, notes[1].1), (12, 20));
    }

    #[test]
    fn a_ghost_is_no_note_nor_a_weak_strike_on_a_sounding_key() {
        let n = 40;
        let mut frame = vec![0f32; n];
        let mut onset = vec![0f32; n];
        let mut at = vec![None; n];
        at[5] = Some(0.);
        onset[5] = 0.9;
        // Struck, but the key never sounds: a ghost.
        assert_eq!(struck(at.clone(), &onset, &frame)[5], None);
        for f in frame.iter_mut().take(30).skip(7) {
            *f = 0.8;
        }
        assert_eq!(struck(at.clone(), &onset, &frame)[5], Some(0.));
        // Struck again while it sounds: weakly, the note goes on.
        at[15] = Some(0.1);
        onset[15] = 0.35;
        let on = struck(at.clone(), &onset, &frame);
        assert_eq!(on[15], None);
        let velocity = vec![0.5f32; n];
        let notes = key_notes(&frame, &on, &vec![None; n], &velocity);
        assert_eq!(notes.len(), 1, "one note, not cut in two");
        // Firmly: a new note.
        onset[15] = 0.7;
        assert_eq!(struck(at, &onset, &frame)[15], Some(0.1));
    }

    #[test]
    fn the_pedal_goes_down_and_comes_up() {
        let n = 60;
        let mut frame = vec![0f32; n];
        for f in frame.iter_mut().take(30).skip(10) {
            *f = 0.9;
        }
        let none = vec![None; n];
        assert_eq!(pedal_presses(&frame, &none), [(10, 30, 0.)]);
        let mut offset = vec![None; n];
        offset[25] = Some(0.1);
        assert_eq!(pedal_presses(&frame, &offset), [(10, 25, 0.1)]);
    }
}
