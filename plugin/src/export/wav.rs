//! Writing a stereo WAV file: 24-bit PCM, little-endian, as a DAW reads it.

use std::io::Write;
use std::path::Path;

/// Bytes a stereo 24-bit frame takes.
const FRAME: u32 = 6;

/// The 44 bytes before the samples of `frames` stereo 24-bit frames at
/// `rate`.
fn header(rate: u32, frames: usize) -> Result<[u8; 44], String> {
    let data = u32::try_from(frames)
        .ok()
        .and_then(|f| f.checked_mul(FRAME))
        .filter(|d| *d <= u32::MAX - 36)
        .ok_or("the audio is too long for a WAV file (4 GB)")?;
    let mut h = [0u8; 44];
    let mut at = 0;
    let mut put = |bytes: &[u8]| {
        if let Some(slot) = h.get_mut(at..at + bytes.len()) {
            slot.copy_from_slice(bytes);
        }
        at += bytes.len();
    };
    put(b"RIFF");
    put(&(36 + data).to_le_bytes());
    put(b"WAVEfmt ");
    put(&16u32.to_le_bytes());
    put(&1u16.to_le_bytes()); // PCM
    put(&2u16.to_le_bytes()); // stereo
    put(&rate.to_le_bytes());
    put(&(rate * FRAME).to_le_bytes());
    put(&(FRAME as u16).to_le_bytes());
    put(&24u16.to_le_bytes());
    put(b"data");
    put(&data.to_le_bytes());
    Ok(h)
}

/// `x` as a 24-bit sample: full scale is ±1, beyond it clips.
fn sample(x: f32) -> [u8; 3] {
    let x = if x.is_finite() {
        x.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let n = (f64::from(x) * 8_388_607.0).round() as i32;
    let [a, b, c, _] = n.to_le_bytes();
    [a, b, c]
}

/// Writes `audio` (left, right) at `rate` to `path` as 24-bit PCM.
///
/// # Errors
/// When the file cannot be written, or holds more than a WAV can.
pub fn write(path: &Path, rate: u32, audio: &[[f32; 2]]) -> Result<(), String> {
    let header = header(rate, audio.len())?;
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let file = std::fs::File::create(path).map_err(fail)?;
    let mut out = std::io::BufWriter::new(file);
    out.write_all(&header).map_err(fail)?;
    for [l, r] in audio {
        out.write_all(&sample(*l)).map_err(fail)?;
        out.write_all(&sample(*r)).map_err(fail)?;
    }
    out.flush().map_err(fail)
}

/// How long the WAV file at `path` plays, in seconds, by its header.
pub fn seconds(path: &Path) -> Option<f64> {
    let mut head = [0u8; 44];
    let mut file = std::fs::File::open(path).ok()?;
    std::io::Read::read_exact(&mut file, &mut head).ok()?;
    let word = |at: usize| Some(u32::from_le_bytes(head.get(at..at + 4)?.try_into().ok()?));
    let rate = word(24)?;
    let per_second = word(28)?;
    let data = word(40)?;
    (rate > 0 && per_second > 0).then(|| f64::from(data) / f64::from(per_second))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_24_bit_and_clip_at_full_scale() {
        assert_eq!(sample(0.0), [0, 0, 0]);
        assert_eq!(sample(1.0), [0xFF, 0xFF, 0x7F]);
        assert_eq!(sample(-1.0), [0x01, 0x00, 0x80]);
        assert_eq!(sample(2.0), sample(1.0));
        assert_eq!(sample(f32::NAN), [0, 0, 0]);
        // Half scale: 0x3FFFFF.8 rounds up.
        assert_eq!(sample(0.5), [0x00, 0x00, 0x40]);
    }

    #[test]
    fn a_file_holds_every_frame_and_says_how_long_it_is() {
        let dir = std::env::temp_dir().join(format!("kontra-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        let audio: Vec<[f32; 2]> = (0..48_000).map(|n| [n as f32 / 48_000.0, -0.25]).collect();
        write(&path, 48_000, &audio).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 6 * 48_000);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 24);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            48_000
        );
        assert!((seconds(&path).unwrap() - 1.0).abs() < 1e-12);
        // The last frame: left just under full scale, right a quarter down.
        let last = &bytes[bytes.len() - 6..];
        assert_eq!(&last[3..], &sample(-0.25));
        std::fs::remove_dir_all(&dir).ok();
    }
}
