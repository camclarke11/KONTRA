//! The editor headless while a song plays: its notes falling onto the
//! keyboard, drawn at several moments of the song. Saves
//! `<out>/song-<width>-<seconds>.png` and prints one JSON line per shot
//! with what a frame costs while the song moves (`frame_us`: building,
//! laying out and resolving the view; `paint_ms`: vello's CPU renderer
//! drawing it), how many shapes it drew, and whether the editor still asks
//! for frames. The song is made here, like the shared test file
//! pedal-runs.mid: left-hand chords a bar long, right-hand sixteenth runs,
//! velocities from 40 to 110, 120 BPM then 80, the pedal changed each bar.
//!
//! `cargo run --release -p kontra-ui --example song_shot -- <out>
//!   [--design <name> | --designs] [--midi <file.mid>] [--at <s>,<s>,...]
//!   [--stage <w>x<h>] [--scale <device scale>]`
//!
//! `--designs` draws every Design that ships; `--midi` a real file's notes
//! and pedal instead of the made one; `--stage` the falling notes over a
//! keybed filling a whole window of that size, as a full-screen view would
//! (`stage-<w>x<h>-<design>-<seconds>.png`), with no editor around them.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use kontra_ui::app::{self, App};
use kontra_ui::design::{self, Design};
use kontra_ui::waterfall::Notes;
use mui::prelude::*;
use mui::vello;

#[path = "shared/song.rs"]
mod song;
use song::{editor, midi};

/// One frame of `app` on `ui`; its cost in µs and whether motion runs on.
fn frame(ui: &mut Ui, app: &mut App, size: (u16, u16)) -> Result<(u128, bool), Box<dyn Error>> {
    let t = Instant::now();
    let r = app::view(ui, app);
    let size = Size::new(f64::from(size.0), f64::from(size.1));
    let f = ui
        .frame(r.el, Some(size), Input::default(), 1. / 60.)
        .map_err(|e| format!("{e:?}"))?;
    Ok((t.elapsed().as_micros(), f.animating))
}

struct Args {
    out: PathBuf,
    designs: Vec<Arc<Design>>,
    midi: Option<PathBuf>,
    at: Vec<f64>,
    stage: Option<(u16, u16)>,
    scale: f64,
    /// Frames to time the editor over, then stop: for a profiler.
    bench: Option<u32>,
}

fn args() -> Result<Args, Box<dyn Error>> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        out: PathBuf::new(),
        designs: vec![design::named("")],
        midi: None,
        at: vec![0.0, 1.37, 4.9, 13.2, 21.05],
        stage: None,
        scale: 1.,
        bench: None,
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--design" => a.designs = vec![design::named(&it.next().ok_or("--design <name>")?)],
            "--designs" => a.designs = design::presets().to_vec(),
            "--midi" => a.midi = Some(it.next().ok_or("--midi <file>")?.into()),
            "--at" => {
                let list = it.next().ok_or("--at <s>,<s>")?;
                a.at = list.split(',').map(str::parse).collect::<Result<_, _>>()?;
            }
            "--stage" => {
                let size = it.next().ok_or("--stage <w>x<h>")?;
                let (w, h) = size.split_once('x').ok_or("--stage <w>x<h>")?;
                a.stage = Some((w.parse()?, h.parse()?));
            }
            "--scale" => a.scale = it.next().ok_or("--scale <s>")?.parse()?,
            "--bench" => a.bench = Some(it.next().ok_or("--bench <frames>")?.parse()?),
            _ => a.out = arg.into(),
        }
    }
    if a.out.as_os_str().is_empty() {
        return Err("usage: song_shot <out dir> [options]".into());
    }
    Ok(a)
}

fn main() -> Result<(), Box<dyn Error>> {
    let a = args()?;
    std::fs::create_dir_all(&a.out)?;
    let (notes, name) = match &a.midi {
        Some(file) => (
            midi(&std::fs::read(file)?)?,
            file.file_name()
                .map_or("song.mid".into(), |n| n.to_string_lossy().into_owned()),
        ),
        None => (song::song(), "pedal-runs.mid".to_owned()),
    };
    if let Some(frames) = a.bench {
        let d = a.designs.first().cloned().unwrap_or_default();
        let mut ui = kontra_ui::ui();
        let mut app = editor(notes.clone(), &name, &d.meta.name);
        let mut costs = Vec::new();
        for n in 0..frames {
            if let Some(s) = app.song.as_mut() {
                s.position = 2. + f64::from(n % 1200) / 60.;
            }
            costs.push(frame(&mut ui, &mut app, (1180, 780))?.0);
        }
        costs.sort_unstable();
        let at = |q: usize| costs.get(costs.len() * q / 100).copied().unwrap_or(0);
        println!(
            "{{\"bench\":{frames},\"p50_us\":{},\"p95_us\":{}}}",
            at(50),
            at(95)
        );
        return Ok(());
    }
    if let Some(size) = a.stage {
        for d in &a.designs {
            stage(&a, &notes, d, size)?;
        }
        return Ok(());
    }
    let one = a.designs.len() == 1 && a.designs.first().is_some_and(|d| **d == Design::default());
    for size in [(1180u16, 780u16), (900, 600)] {
        for d in &a.designs {
            shots(&a, &notes, &name, d, size, one)?;
        }
    }
    Ok(())
}

/// The editor at `size` in Design `d`, at each of the moments asked for;
/// with the default alone, also with the notes hidden, then stopped.
fn shots(
    a: &Args,
    notes: &Notes,
    name: &str,
    d: &Design,
    size: (u16, u16),
    plain: bool,
) -> Result<(), Box<dyn Error>> {
    let mut ui = kontra_ui::ui();
    let mut app = editor(notes.clone(), name, &d.meta.name);
    // The keyboard eases open: let it settle.
    for _ in 0..240 {
        if !frame(&mut ui, &mut app, size)?.1 {
            break;
        }
    }
    let (w, h) = size;
    let tag = if plain {
        String::new()
    } else {
        format!("{}-", d.meta.name.to_lowercase())
    };
    for &at in &a.at {
        // A second of frames up to it, as a playing song moves.
        let mut costs = Vec::new();
        for n in (0..60).rev() {
            if let Some(s) = app.song.as_mut() {
                s.position = (at - f64::from(n) / 60.).max(0.);
            }
            costs.push(frame(&mut ui, &mut app, size)?.0);
        }
        costs.sort_unstable();
        let file = a.out.join(format!("song-{w}-{tag}{at:05.2}.png"));
        let (rgba, paint_ms) = paint(&ui, w, h, a.scale)?;
        save(&file, w, h, a.scale, &rgba)?;
        println!(
            "{{\"width\":{w},\"design\":{:?},\"at\":{at},\"frame_us\":{},\"paint_ms\":{paint_ms:.1},\"shapes\":{},\"follows\":{},\"shot\":{file:?}}}",
            d.meta.name,
            costs.get(costs.len() / 2).copied().unwrap_or(0),
            ui.scene().map_or(0, |s| s.paint.len()),
            app.follows_song(),
        );
    }
    if !plain {
        return Ok(());
    }
    // Hidden (the notes dropdown's Off, after the Designs), then stopped:
    // the keyboard eases back and the frames end.
    ui_click(&mut ui, &mut app, size, "song-notes")?;
    let off = format!("song-notes-{}", design::all().len());
    ui_click(&mut ui, &mut app, size, &off)?;
    let (rgba, _) = paint(&ui, w, h, a.scale)?;
    save(
        &a.out.join(format!("song-{w}-notes-hidden.png")),
        w,
        h,
        a.scale,
        &rgba,
    )?;
    app.song = None;
    let mut settle = 0;
    while frame(&mut ui, &mut app, size)?.1 && settle < 240 {
        settle += 1;
    }
    let (rgba, _) = paint(&ui, w, h, a.scale)?;
    save(
        &a.out.join(format!("song-{w}-stopped.png")),
        w,
        h,
        a.scale,
        &rgba,
    )?;
    println!(
        "{{\"width\":{w},\"stopped\":true,\"settle_frames\":{settle},\"follows\":{}}}",
        app.follows_song()
    );
    Ok(())
}

/// The falling notes over a keybed, filling a window of `size` alone: what
/// a full-screen view draws.
fn stage(a: &Args, notes: &Notes, d: &Arc<Design>, size: (u16, u16)) -> Result<(), Box<dyn Error>> {
    let mut ui = kontra_ui::ui();
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    for &at in &a.at {
        let mut costs = Vec::new();
        for n in (0..30).rev() {
            let t = Instant::now();
            let now = (at - f64::from(n) / 60.).max(0.);
            let el = song::stage(&mut ui, notes, d, now, (w, h));
            ui.frame(el, Some(Size::new(w, h)), Input::default(), 1. / 60.)
                .map_err(|e| format!("{e:?}"))?;
            costs.push(t.elapsed().as_micros());
        }
        costs.sort_unstable();
        let (sw, sh) = size;
        let file = a.out.join(format!(
            "stage-{sw}x{sh}-{}-{at:05.2}.png",
            d.meta.name.to_lowercase()
        ));
        let (rgba, paint_ms) = paint(&ui, sw, sh, a.scale)?;
        save(&file, sw, sh, a.scale, &rgba)?;
        println!(
            "{{\"stage\":\"{sw}x{sh}\",\"design\":{:?},\"at\":{at},\"frame_us\":{},\"paint_ms\":{paint_ms:.1},\"shapes\":{},\"shot\":{file:?}}}",
            d.meta.name,
            costs.get(costs.len() / 2).copied().unwrap_or(0),
            ui.scene().map_or(0, |s| s.paint.len()),
        );
    }
    Ok(())
}

fn ui_click(ui: &mut Ui, app: &mut App, size: (u16, u16), id: &str) -> Result<(), Box<dyn Error>> {
    let at = ui
        .scene()
        .and_then(|s| s.surface(id))
        .map(|s| {
            Point::new(
                s.frame.x + s.frame.size.width / 2.,
                s.frame.y + s.frame.size.height / 2.,
            )
        })
        .ok_or_else(|| format!("no {id} on screen"))?;
    for buttons in [Buttons::PRIMARY, Buttons::default()] {
        let r = app::view(ui, app);
        let input = Input {
            pointer: PointerInput {
                pos: Some(at),
                buttons,
                ..Default::default()
            },
            ..Default::default()
        };
        let s = Size::new(f64::from(size.0), f64::from(size.1));
        ui.frame(r.el, Some(s), input, 1. / 60.)
            .map_err(|e| format!("{e:?}"))?;
    }
    for _ in 0..240 {
        if !frame(ui, app, size)?.1 {
            break;
        }
    }
    Ok(())
}

/// The scene at `scale` device pixels a point, and how long vello's CPU
/// renderer took.
fn paint(ui: &Ui, w: u16, h: u16, scale: f64) -> Result<(Vec<u8>, f64), Box<dyn Error>> {
    let (pw, ph) = device(w, h, scale);
    let t = Instant::now();
    let mut ctx = vello::vello_cpu::RenderContext::new(pw, ph);
    let mut resources = vello::vello_cpu::Resources::default();
    let scene = ui.scene().ok_or("no scene")?;
    let cpu = &mut vello::Cpu {
        ctx: &mut ctx,
        resources: &mut resources,
        cache: &mut vello::Cache::default(),
    };
    vello::paint(cpu, scene, vello::kurbo::Affine::scale(scale)).map_err(|e| format!("{e:?}"))?;
    ctx.flush();
    let mut pix = vello::vello_cpu::Pixmap::new(pw, ph);
    ctx.render(&mut pix, &mut resources);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    Ok((
        pix.take_unpremultiplied()
            .iter()
            .flat_map(|p| [p.r, p.g, p.b, p.a])
            .collect(),
        ms,
    ))
}

fn device(w: u16, h: u16, scale: f64) -> (u16, u16) {
    let at = |v: u16| (f64::from(v) * scale).round().clamp(1., 65_535.) as u16;
    (at(w), at(h))
}

fn save(file: &Path, w: u16, h: u16, scale: f64, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
    let (w, h) = device(w, h, scale);
    let mut e = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(file)?),
        w.into(),
        h.into(),
    );
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(rgba)?;
    Ok(())
}
