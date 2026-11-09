//! Transcribes a recording and prints its notes and pedal as CSV, with how
//! long each step took on stderr: a check of the pipeline outside the app.
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
    let x = kontra_transcribe::audio::read(std::path::Path::new(&audio))?;
    eprintln!(
        "read and resampled {:.1} s of audio in {:?}",
        x.len() as f64 / 16_000.,
        t.elapsed()
    );
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
