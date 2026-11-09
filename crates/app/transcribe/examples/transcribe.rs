//! Transcribes a recording and prints its notes and pedal as CSV, with how
//! long each step took on stderr: a check of the pipeline outside the app.
//! With `KONTRA_SEPARATE_MODEL` set, its piano is isolated first.
//! `cargo run --release -p kontra-transcribe --example transcribe -- <audio> [model.onnx] [threads]`
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let audio = args
        .next()
        .ok_or("usage: transcribe <audio> [model.onnx] [threads]")?;
    let model = args
        .next()
        .map(std::path::PathBuf::from)
        .or_else(kontra_transcribe::model_path);
    let model = model.ok_or("no model: pass one or set KONTRA_TRANSCRIBE_MODEL")?;
    let threads = args.next().and_then(|t| t.parse().ok()).unwrap_or(4);
    let t = Instant::now();
    let path = std::path::Path::new(&audio);
    // With KONTRA_SEPARATE_MODEL set, the piano is isolated first.
    let x = match std::env::var_os("KONTRA_SEPARATE_MODEL") {
        Some(weights) => {
            let s = kontra_transcribe::separate::Separator::load(std::path::Path::new(&weights))?;
            let rate = s.settings.sample_rate;
            let [l, r] = kontra_transcribe::audio::read_stereo(path, rate)?;
            eprintln!(
                "read {:.1} s of stereo in {:?}",
                l.len() as f64 / f64::from(rate),
                t.elapsed()
            );
            let t = Instant::now();
            let [pl, pr] = s.piano(&l, &r, &|_| {}, &AtomicBool::new(false))?;
            eprintln!(
                "isolated the piano in {:?} (GPU: {})",
                t.elapsed(),
                s.on_gpu()
            );
            let mono: Vec<f32> = pl.iter().zip(&pr).map(|(a, b)| (a + b) * 0.5).collect();
            kontra_transcribe::audio::resample(&mono, rate, kontra_transcribe::RATE)
        }
        None => kontra_transcribe::audio::read(path)?,
    };
    eprintln!("{:.1} s of audio to transcribe", x.len() as f64 / 16_000.);
    let t = Instant::now();
    let m = kontra_transcribe::Model::load(&model)?;
    eprintln!("loaded the model in {:?}", t.elapsed());
    let t = Instant::now();
    let out = m.transcribe(&x, threads, &|_| {}, &AtomicBool::new(false))?;
    eprintln!(
        "transcribed in {:?} on {threads} threads: {} notes, {} pedal presses",
        t.elapsed(),
        out.notes.len(),
        out.pedals.len()
    );
    println!("kind,start,end,key,velocity");
    for n in &out.notes {
        println!("note,{:.4},{:.4},{},{}", n.start, n.end, n.key, n.velocity);
    }
    for p in &out.pedals {
        println!("pedal,{:.4},{:.4},,", p.start, p.end);
    }
    Ok(())
}
