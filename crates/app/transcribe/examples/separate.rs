//! Checks the separator against PyTorch: the chunk `tools/separate-model`
//! saved beside the weights, then (given them) a whole recording against
//! PyTorch's demixed piano, sample by sample; and times both.
//! `cargo run --release -p kontra-transcribe --example separate -- <weights> [<in.wav> <piano.wav>]`
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use kontra_transcribe::separate::{Separator, Stft};

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// How far `got` is from `want`, channel `ch`, on stderr.
fn compare(ch: usize, got: &[f32], want: &[f32]) {
    let diff: Vec<f32> = got.iter().zip(want).map(|(a, b)| a - b).collect();
    let worst = diff.iter().fold(0f32, |m, v| m.max(v.abs()));
    let off = diff.iter().filter(|v| v.abs() > 1e-3).count();
    eprintln!(
        "channel {ch}: {} samples (got {}, want {}), max |diff| {worst:.2e}, rms want {:.4}, rms diff {:.2e}, off by over 0.001: {off}",
        diff.len(),
        got.len(),
        want.len(),
        rms(want),
        rms(&diff)
    );
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: separate <weights> [<in.wav> <piano.wav>]")?;
    let t = Instant::now();
    let s = Separator::load(Path::new(&path))?;
    eprintln!("loaded in {:?}, on the GPU: {}", t.elapsed(), s.on_gpu());
    let check = candle_core::safetensors::load(format!("{path}.check"), &candle_core::Device::Cpu)
        .map_err(|e| e.to_string())?;
    let get = |k: &str| -> Result<Vec<Vec<f32>>, String> {
        check
            .get(k)
            .ok_or(format!("no {k}"))?
            .to_vec2::<f32>()
            .map_err(|e| e.to_string())
    };
    let (input, want) = (get("input")?, get("piano")?);
    let (Some(l), Some(r)) = (input.first(), input.get(1)) else {
        return Err("the check has no stereo input".into());
    };
    let mut stft = Stft::new(s.settings.n_fft, s.settings.hop);
    let mut got = [Vec::new(), Vec::new()];
    for run in 0..3 {
        let t = Instant::now();
        got = s.chunk(&mut stft, l, r)?;
        eprintln!(
            "chunk {run} ({} samples) in {:?}",
            s.settings.chunk,
            t.elapsed()
        );
    }
    for (ch, (g, w)) in got.iter().zip(&want).enumerate() {
        compare(ch, g, w);
    }
    if let (Some(input), Some(want)) = (args.next(), args.next()) {
        let rate = s.settings.sample_rate;
        let [l, r] = kontra_transcribe::audio::read_stereo(Path::new(&input), rate)?;
        let t = Instant::now();
        let got = s.piano(&l, &r, &|_| {}, &AtomicBool::new(false))?;
        eprintln!(
            "{:.0} s separated in {:?}",
            l.len() as f64 / f64::from(rate),
            t.elapsed()
        );
        let want = kontra_transcribe::audio::read_stereo(Path::new(&want), rate)?;
        for (ch, (g, w)) in got.iter().zip(&want).enumerate() {
            compare(ch, g, w);
        }
    }
    Ok(())
}
