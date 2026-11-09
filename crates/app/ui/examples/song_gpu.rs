//! The falling notes timed on the GPU: the editor, and the falling notes
//! filling a whole screen, drawn by vello's GPU renderer into an offscreen
//! texture at the sizes a Mac shows them, in every Design and in a plain
//! one with every effect off (the look before them). Prints one JSON line
//! each: the median and 95th percentile of building the view (`ui_us`) and
//! of drawing it (`gpu_ms`: encoding, the GPU's work and the wait for it),
//! over a few seconds of the song played at 60 frames a second.
//!
//! `cargo run --release -p kontra-ui --features gpu-bench --example song_gpu
//!   -- [--midi <file.mid>] [--at <seconds>] [--frames <n>]`

use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use kontra_ui::app;
use kontra_ui::design::{self, Design, Shading};
use mui::prelude::*;
use mui::vello::host::Host;
use mui::vello::kurbo::Affine;

#[path = "shared/song.rs"]
mod song;

/// Every effect off: flat bars on a flat field, as before them.
fn plain() -> Arc<Design> {
    let mut d = Design::default();
    d.meta.name = "Plain".into();
    d.background.aurora.clear();
    d.background.vignette = 0.;
    d.background.sharp_lanes = 0.;
    d.notes.shading = Shading::None;
    d.notes.gloss = 0.;
    d.notes.rim_strength = 0.;
    d.glow.strength = 0.;
    d.glow.bloom = 0.;
    d.beams.strength = 0.;
    d.strike.strength = 0.;
    d.particles.strength = 0.;
    d.pedal.strength = 0.;
    Arc::new(d)
}

fn percentile(v: &mut [f64], q: usize) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() * q / 100).copied().unwrap_or(0.)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let (mut file, mut at, mut frames) = (None, 20., 180u32);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--midi" => file = args.next(),
            "--at" => at = args.next().ok_or("--at <s>")?.parse()?,
            "--frames" => frames = args.next().ok_or("--frames <n>")?.parse()?,
            _ => return Err(format!("unknown argument {a}").into()),
        }
    }
    let notes = match &file {
        Some(f) => song::midi(&std::fs::read(f)?)?,
        None => song::song(),
    };
    let mut designs = vec![plain()];
    designs.extend(design::presets().iter().cloned());
    // (what, logical size, device pixels a point)
    let scenes = [
        ("editor", (1180., 780.), 2.),
        ("stage", (2560., 1440.), 1.),
        ("stage", (1728., 1117.), 2.),
        ("stage", (2560., 1440.), 2.),
    ];
    for (what, (w, h), scale) in scenes {
        let device = ((w * scale) as u32, (h * scale) as u32);
        let mut host = Host::headless(device).map_err(|e| format!("{e:?}"))?;
        for d in &designs {
            let mut ui = kontra_ui::ui();
            let mut editor = song::editor(notes.clone(), "song.mid", &d.meta.name);
            let (mut build, mut draw) = (Vec::new(), Vec::new());
            for n in 0..frames + 30 {
                let now = at + f64::from(n) / 60.;
                let t = Instant::now();
                let el = if what == "editor" {
                    if let Some(s) = editor.song.as_mut() {
                        s.position = now;
                    }
                    app::view(&mut ui, &mut editor).el
                } else {
                    song::stage(&mut ui, &notes, d, now, (w, h))
                };
                ui.frame(el, Some(Size::new(w, h)), Input::default(), 1. / 60.)
                    .map_err(|e| format!("{e:?}"))?;
                let built = t.elapsed().as_secs_f64();
                let scene = ui.scene().ok_or("no scene")?;
                let t = Instant::now();
                host.render_offscreen(scene, Affine::scale(scale))
                    .map_err(|e| format!("{e:?}"))?;
                // The first frames warm the renderer's caches.
                if n >= 30 {
                    build.push(built * 1e6);
                    draw.push(t.elapsed().as_secs_f64() * 1e3);
                }
            }
            println!(
                "{{\"view\":\"{what}\",\"size\":\"{w}x{h}@{scale}\",\"pixels\":\"{}x{}\",\"design\":{:?},\"ui_us\":[{:.0},{:.0}],\"gpu_ms\":[{:.2},{:.2}],\"shapes\":{}}}",
                device.0,
                device.1,
                d.meta.name,
                percentile(&mut build, 50),
                percentile(&mut build, 95),
                percentile(&mut draw, 50),
                percentile(&mut draw, 95),
                ui.scene().map_or(0, |s| s.paint.len()),
            );
        }
    }
    Ok(())
}
