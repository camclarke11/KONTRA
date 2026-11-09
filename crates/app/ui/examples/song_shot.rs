//! The editor headless while a song plays: its notes falling onto the
//! keyboard, drawn at several moments of the song. Saves
//! `<out>/song-<width>-<seconds>.png` and prints one JSON line per shot
//! with what a frame costs while the song moves, and whether the editor
//! still asks for frames. The song is made here, like the shared test file
//! pedal-runs.mid: left-hand chords a bar long, right-hand sixteenth runs,
//! velocities from 40 to 110, 120 BPM then 80.
//! `cargo run --release -p kontra-ui --example song_shot -- <out>`

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

use kontra_ui::app::{self, App, Part, Song};
use kontra_ui::waterfall::{Note, Notes};
use mui::prelude::{Input, Size, Ui};
use mui::vello;

/// Seconds a beat lasts at `bar`.
fn beat(bar: usize) -> f64 {
    if bar < 8 { 0.5 } else { 0.75 }
}

fn song() -> Notes {
    let mut notes = Vec::new();
    let chords: [[u8; 3]; 4] = [[48, 55, 64], [45, 52, 60], [41, 48, 57], [43, 50, 59]];
    let scale = [72u8, 74, 76, 77, 79, 81, 83, 84];
    let mut at = 0.;
    for bar in 0..16 {
        let b = beat(bar);
        // The left hand: a chord held the bar, the pedal's job done by hand.
        for &key in chords.get(bar % 4).into_iter().flatten() {
            notes.push(Note {
                start: at,
                end: at + 4. * b - 0.02,
                key,
                velocity: 60,
                voice: 1,
            });
        }
        // The right hand: sixteenths up and back down, swelling.
        for s in 0..16usize {
            let step = if s < 8 { s } else { 15 - s };
            let key = scale.get(step).copied().unwrap_or(72) + if bar % 2 == 1 { 12 } else { 0 };
            notes.push(Note {
                start: at + s as f64 * b / 4.,
                end: at + (s + 1) as f64 * b / 4. - 0.015,
                key: if s % 4 == 3 { key + 1 } else { key },
                velocity: 40 + (70 * s / 15) as u8,
                voice: 0,
            });
        }
        at += 4. * b;
    }
    Notes::new(notes)
}

fn editor(notes: Notes) -> App {
    App {
        parts: vec![Part {
            name: "Una Corda Cotton".into(),
            library: "Una Corda".into(),
            output: 1,
            keys: (21, 108),
            ..Part::default()
        }],
        song: Some(Song {
            name: "pedal-runs.mid".into(),
            position: 0.,
            length: 40.,
            notes,
        }),
        ..App::default()
    }
}

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

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: song_shot <out dir>")?,
    );
    std::fs::create_dir_all(&out)?;
    let notes = song();
    for size in [(1180u16, 780u16), (900, 600)] {
        let mut ui = kontra_ui::ui();
        let mut app = editor(notes.clone());
        // The keyboard eases open: let it settle.
        for _ in 0..240 {
            if !frame(&mut ui, &mut app, size)?.1 {
                break;
            }
        }
        for at in [0.0, 1.37, 4.9, 13.2, 21.05] {
            // A second of frames up to it, as a playing song moves.
            let mut costs = Vec::new();
            for n in (0..60).rev() {
                if let Some(s) = app.song.as_mut() {
                    s.position = (at - f64::from(n) / 60.).max(0.);
                }
                costs.push(frame(&mut ui, &mut app, size)?.0);
            }
            costs.sort_unstable();
            let (w, h) = size;
            let file = out.join(format!("song-{w}-{at:05.2}.png"));
            save(&file, w, h, &paint(&ui, w, h)?)?;
            println!(
                "{{\"width\":{w},\"at\":{at},\"frame_us\":{},\"follows\":{},\"shot\":{file:?}}}",
                costs.get(costs.len() / 2).copied().unwrap_or(0),
                app.follows_song(),
            );
        }
        // Hidden, then stopped: the keyboard eases back and the frames end.
        ui_click(&mut ui, &mut app, size, "song-notes")?;
        let (w, h) = size;
        save(
            &out.join(format!("song-{w}-notes-hidden.png")),
            w,
            h,
            &paint(&ui, w, h)?,
        )?;
        app.song = None;
        let mut settle = 0;
        while frame(&mut ui, &mut app, size)?.1 && settle < 240 {
            settle += 1;
        }
        save(
            &out.join(format!("song-{w}-stopped.png")),
            w,
            h,
            &paint(&ui, w, h)?,
        )?;
        println!(
            "{{\"width\":{w},\"stopped\":true,\"settle_frames\":{settle},\"follows\":{}}}",
            app.follows_song()
        );
    }
    Ok(())
}

fn ui_click(ui: &mut Ui, app: &mut App, size: (u16, u16), id: &str) -> Result<(), Box<dyn Error>> {
    use mui::prelude::{Buttons, Point, PointerInput};
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

fn paint(ui: &Ui, w: u16, h: u16) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut ctx = vello::vello_cpu::RenderContext::new(w, h);
    let mut resources = vello::vello_cpu::Resources::default();
    let scene = ui.scene().ok_or("no scene")?;
    let cpu = &mut vello::Cpu {
        ctx: &mut ctx,
        resources: &mut resources,
        cache: &mut vello::Cache::default(),
    };
    vello::paint(cpu, scene, vello::kurbo::Affine::IDENTITY).map_err(|e| format!("{e:?}"))?;
    ctx.flush();
    let mut pix = vello::vello_cpu::Pixmap::new(w, h);
    ctx.render(&mut pix, &mut resources);
    Ok(pix
        .take_unpremultiplied()
        .iter()
        .flat_map(|p| [p.r, p.g, p.b, p.a])
        .collect())
}

fn save(file: &Path, w: u16, h: u16, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
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
