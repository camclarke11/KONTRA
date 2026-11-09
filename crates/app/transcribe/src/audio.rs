//! A recording as the model hears it: decoded (WAV, AIFF, CAF, FLAC, MP3,
//! AAC or ALAC in M4A, Ogg Vorbis), its channels mixed to one, resampled
//! to 16 kHz with a windowed-sinc filter.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// The file extensions [`read`] takes.
pub const EXTENSIONS: [&str; 11] = [
    "wav", "wave", "aif", "aiff", "caf", "flac", "mp3", "m4a", "aac", "ogg", "oga",
];

/// Whether `path` names audio [`read`] takes, by its extension.
#[must_use]
pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The recording at `path`, its channels mixed to one, at [`crate::RATE`];
/// at most [`crate::LONGEST`] seconds of it.
///
/// # Errors
/// When it cannot be opened or decoded.
pub fn read(path: &Path) -> Result<Vec<f32>, String> {
    let (mono, rate) = decode(path)?;
    Ok(resample(&mono, rate, crate::RATE))
}

/// The recording at `path` as one channel at its own rate.
fn decode(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let fail = |e: Error| format!("{}: {e}", path.display());
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(fail)?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| format!("{}: no audio in it", path.display()))?;
    let id = track.id;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| format!("{}: no sample rate", path.display()))?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(fail)?;
    let most = (crate::LONGEST * f64::from(rate)) as usize;
    let mut mono: Vec<f32> = Vec::new();
    let mut buffer: Option<SampleBuffer<f32>> = None;
    while mono.len() < most {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // The end of the stream.
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(fail(e)),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged packet: skip it, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(fail(e)),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let b = match &mut buffer {
            Some(b) if b.capacity() >= decoded.capacity() * channels => b,
            _ => buffer.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
        };
        b.copy_interleaved_ref(decoded);
        let scale = 1. / channels as f32;
        mono.extend(
            b.samples()
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() * scale),
        );
    }
    mono.truncate(most);
    if mono.is_empty() {
        return Err(format!("{}: no audio in it", path.display()));
    }
    Ok((mono, rate))
}

/// The zeroth modified Bessel function of the first kind, for the Kaiser
/// window.
fn bessel0(x: f64) -> f64 {
    let (mut sum, mut term) = (1., 1.);
    for k in 1..50 {
        let f = x / (2. * f64::from(k));
        term *= f * f;
        sum += term;
        if term < sum * 1e-12 {
            break;
        }
    }
    sum
}

/// `x` from `from` Hz to `to` Hz: each output sample a Kaiser-windowed
/// sinc (32 zero crossings either side, β 9) over the input, cut off just
/// under the lower rate's Nyquist; the kernel tabled finely and read
/// between its entries.
#[must_use]
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    /// Table entries an input sample.
    const STEPS: f64 = 512.;
    if from == to || from == 0 || to == 0 {
        return x.to_vec();
    }
    let ratio = f64::from(to) / f64::from(from);
    // Cycles a sample of the input, a little under the lower Nyquist.
    let cutoff = 0.5 * ratio.min(1.) * 0.95;
    let half = 32. / (2. * cutoff);
    let beta = 9.;
    let norm = bessel0(beta);
    let kernel = |u: f64| {
        let r = u / half;
        let w = bessel0(beta * (1. - r * r).max(0.).sqrt()) / norm;
        let a = 2. * cutoff * u;
        let sinc = if a.abs() < 1e-12 {
            1.
        } else {
            (std::f64::consts::PI * a).sin() / (std::f64::consts::PI * a)
        };
        2. * cutoff * sinc * w
    };
    let table: Vec<f64> = (0..(half * STEPS).ceil() as usize + 2)
        .map(|i| kernel(i as f64 / STEPS))
        .collect();
    let tap = |u: f64| {
        let p = u.abs() * STEPS;
        let i = p as usize;
        let f = p - i as f64;
        match (table.get(i), table.get(i + 1)) {
            (Some(a), Some(b)) => a + (b - a) * f,
            _ => 0.,
        }
    };
    let at = |k: i64| -> f64 {
        usize::try_from(k)
            .ok()
            .and_then(|k| x.get(k))
            .map_or(0., |&v| f64::from(v))
    };
    let len = (x.len() as f64 * ratio).ceil() as usize;
    (0..len)
        .map(|n| {
            let t = n as f64 / ratio;
            let (first, last) = ((t - half).ceil() as i64, (t + half).floor() as i64);
            (first..=last)
                .map(|k| at(k) * tap(t - k as f64))
                .sum::<f64>() as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        let n = (seconds * f64::from(rate)) as usize;
        (0..n)
            .map(|i| (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin() as f32)
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn a_tone_keeps_its_pitch_and_level_and_one_past_nyquist_goes() {
        let low = resample(&tone(440., 44_100, 1.), 44_100, 16_000);
        assert_eq!(low.len(), 16_000);
        // Away from the ends, the same sine at the new rate.
        let want = tone(440., 16_000, 1.);
        let err = low[1000..15_000]
            .iter()
            .zip(&want[1000..15_000])
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(err < 2e-3, "{err}");
        // 12 kHz cannot be held at 16 kHz: it is filtered out, not folded.
        let high = resample(&tone(12_000., 44_100, 1.), 44_100, 16_000);
        assert!(
            rms(&high[1000..15_000]) < 1e-3,
            "{}",
            rms(&high[1000..15_000])
        );
    }

    #[test]
    fn audio_is_known_by_its_extension() {
        assert!(is_audio(Path::new("take.M4A")));
        assert!(is_audio(Path::new("a/b/song.wav")));
        assert!(!is_audio(Path::new("song.mid")));
        assert!(!is_audio(Path::new("noext")));
    }
}
