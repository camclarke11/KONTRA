//! The Stage and the song transport headless, drawn to PNG: the header's
//! compact transport and its menu, then the Stage with its transport
//! showing, its title card, a loop marked, and the transport hidden after
//! the pointer rests. Saves `<out>/stage-*.png` and prints one JSON line
//! per shot. The song is song_shot's: left-hand chords, right-hand runs.
//! `cargo run --release -p kontra-ui --example stage_shot -- <out>`

use std::error::Error;
use std::path::{Path, PathBuf};

use kontra_ui::app::{self, App, Part, Song};
use kontra_ui::stage::{self, Transport};
use kontra_ui::waterfall::{Note, Notes};
use mui::prelude::{Input, Point, PointerInput, Size, Ui};
use mui::vello;

fn song() -> Notes {
    let mut notes = Vec::new();
    let chords: [[u8; 3]; 4] = [[48, 55, 64], [45, 52, 60], [41, 48, 57], [43, 50, 59]];
    let scale = [72u8, 74, 76, 77, 79, 81, 83, 84];
    let mut at = 0.;
    for bar in 0..16 {
        let b = if bar < 8 { 0.5 } else { 0.75 };
        for &key in chords.get(bar % 4).into_iter().flatten() {
            notes.push(Note {
                start: at,
                end: at + 4. * b - 0.02,
                key,
                velocity: 60,
                voice: 1,
            });
        }
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

fn editor() -> App {
    App {
        parts: vec![Part {
            name: "Una Corda Cotton".into(),
            library: "Una Corda".into(),
            output: 1,
            keys: (21, 108),
            ..Part::default()
        }],
        song: Some(Song {
            name: "01 Satie - Gymnopédie No 1.mid".into(),
            position: 0.,
            length: 40.,
            notes: song(),
            transport: Transport {
                title: "Gymnopédie No 1".into(),
                composer: Some("Satie".into()),
                voices: vec!["Right".into(), "Left".into()],
                take: 1,
                full_screen: Some(false),
                ..Transport::default()
            },
        }),
        ..App::default()
    }
}

/// One frame of `app` at `size`, the pointer at `at`; whether motion runs on.
fn frame(
    ui: &mut Ui,
    app: &mut App,
    size: (u16, u16),
    at: Option<Point>,
) -> Result<bool, Box<dyn Error>> {
    let r = app::view(ui, app);
    let s = Size::new(f64::from(size.0), f64::from(size.1));
    let input = Input {
        pointer: PointerInput {
            pos: at,
            ..Default::default()
        },
        ..Default::default()
    };
    let f = ui
        .frame(r.el, Some(s), input, 1. / 60.)
        .map_err(|e| format!("{e:?}"))?;
    Ok(f.animating || f.repaint_after.is_some_and(|d| d.as_secs_f64() < 0.1))
}

/// Frames until nothing moves, at most `n`.
fn settle(
    ui: &mut Ui,
    app: &mut App,
    size: (u16, u16),
    at: Option<Point>,
    n: usize,
) -> Result<usize, Box<dyn Error>> {
    for k in 0..n {
        if !frame(ui, app, size, at)? {
            return Ok(k);
        }
    }
    Ok(n)
}

fn shot(ui: &Ui, out: &Path, name: &str, (w, h): (u16, u16)) -> Result<(), Box<dyn Error>> {
    let file = out.join(format!("stage-{name}.png"));
    save(&file, w, h, &paint(ui, w, h)?)?;
    println!("{{\"shot\":{file:?},\"width\":{w},\"height\":{h}}}");
    Ok(())
}

fn click(ui: &mut Ui, app: &mut App, size: (u16, u16), id: &str) -> Result<(), Box<dyn Error>> {
    use mui::prelude::Buttons;
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
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: stage_shot <out dir>")?,
    );
    std::fs::create_dir_all(&out)?;
    // The header's compact transport, and its menu.
    for size in [(1180u16, 780u16), (900, 640)] {
        let mut ui = kontra_ui::ui();
        let mut app = editor();
        if let Some(s) = app.song.as_mut() {
            s.position = 12.4;
        }
        settle(&mut ui, &mut app, size, None, 240)?;
        shot(&ui, &out, &format!("header-{}", size.0), size)?;
        click(&mut ui, &mut app, size, "song-more")?;
        settle(&mut ui, &mut app, size, None, 240)?;
        shot(&ui, &out, &format!("header-menu-{}", size.0), size)?;
    }
    for size in [(1180u16, 780u16), (1512, 945)] {
        let (w, h) = size;
        let mut ui = kontra_ui::ui();
        let mut app = editor();
        let mut actions = Vec::new();
        stage::shortcut(&mut app, stage::Shortcut::Stage, &mut actions);
        // As the song starts: the title card, the transport showing.
        let pointer = Some(Point::new(f64::from(w) / 2., f64::from(h) / 2.));
        for n in 0..70 {
            if let Some(s) = app.song.as_mut() {
                s.position = f64::from(n) / 60.;
            }
            frame(&mut ui, &mut app, size, pointer)?;
        }
        shot(&ui, &out, &format!("{w}-start"), size)?;
        // Playing on, a loop marked, the metronome on, slowed to 75%.
        if let Some(s) = app.song.as_mut() {
            s.transport.a = Some(13.);
            s.transport.b = Some(19.);
            s.transport.looping = true;
            s.transport.metronome = true;
            s.transport.speed = 0.75;
            s.transport.count_in = 1;
            s.transport.left = true;
        }
        for n in 0..240 {
            if let Some(s) = app.song.as_mut() {
                s.position = 14. + f64::from(n) / 80.;
            }
            // The pointer moves now and then, so the transport stays.
            let p = pointer.map(|p| Point::new(p.x + f64::from(n / 60), p.y));
            frame(&mut ui, &mut app, size, p)?;
        }
        shot(&ui, &out, &format!("{w}-transport"), size)?;
        // The pointer rests: two seconds on, the transport has gone.
        let rest = pointer.map(|p| Point::new(p.x + 9., p.y));
        let mut drawn = 0;
        for n in 0..200 {
            if let Some(s) = app.song.as_mut() {
                s.position = 17. + f64::from(n) / 80.;
            }
            frame(&mut ui, &mut app, size, rest)?;
            drawn += 1;
        }
        let gone = ui.scene().and_then(|s| s.surface("stage-bar")).is_none();
        shot(&ui, &out, &format!("{w}-resting"), size)?;
        // Paused: it comes back and stays.
        if let Some(s) = app.song.as_mut() {
            s.transport.paused = true;
        }
        let settled = settle(&mut ui, &mut app, size, rest, 400)?;
        shot(&ui, &out, &format!("{w}-paused"), size)?;
        println!(
            "{{\"width\":{w},\"frames_resting\":{drawn},\"bar_hidden\":{gone},\"paused_settle_frames\":{settled}}}"
        );
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
