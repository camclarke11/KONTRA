//! The editor headless while the player plays: with no song, their notes
//! rising from the keys (a left-hand arpeggio and a right-hand melody, the
//! pedal changed between), in every Design that ships; and under a falling
//! song, the keys they strike lit in a colour of their own. Saves
//! `<out>/played-<width>-<design>.png` and `<out>/played-<width>-over-song.png`.
//! `cargo run --release -p kontra-ui --example played_shot -- <out>`

use std::error::Error;
use std::path::{Path, PathBuf};

use kontra_ui::app::{self, App, Part, Song};
use kontra_ui::design;
use kontra_ui::waterfall::{Note, Notes, Pedal, Played};
use mui::prelude::{Input, Size, Ui};
use mui::vello;

/// What was played up to `now` seconds: a held note ends at infinity.
fn played(now: f64) -> Played {
    let note = |start: f64, end: f64, key: u8, velocity: u8| Note {
        start,
        end,
        key,
        velocity,
        voice: 0,
    };
    let held = f64::INFINITY;
    let notes = vec![
        // Left hand: C, G, E, the E still held.
        note(3.2, 5.0, 36, 70),
        note(3.5, 5.2, 43, 62),
        note(3.8, held, 52, 66),
        // Right hand: a phrase, its last two notes held.
        note(3.0, 3.6, 64, 92),
        note(3.6, 4.2, 67, 98),
        note(4.2, 5.4, 72, 110),
        note(5.4, 5.8, 71, 84),
        note(5.85, held, 72, 104),
        note(5.9, held, 76, 96),
    ];
    let pedal = Pedal::new([(3.1, 0, 127), (4.9, 0, 0), (5.0, 0, 127)]);
    Played {
        notes: Notes::new(notes).with_pedal(pedal),
        at: now,
        begun: true,
    }
}

fn editor(design: &str) -> App {
    let mut app = App {
        parts: vec![Part {
            name: "Una Corda Cotton".into(),
            library: "Una Corda".into(),
            output: 1,
            keys: (21, 108),
            ..Part::default()
        }],
        played: played(6.0),
        ..App::default()
    };
    app.view.design = design.into();
    app
}

/// A song of chords a beat apart, to fall over what the player plays.
fn song() -> Notes {
    let mut notes = Vec::new();
    for (bar, root) in [48u8, 53, 55, 48].into_iter().enumerate() {
        let start = bar as f64;
        for (k, step) in [0u8, 4, 7].into_iter().enumerate() {
            notes.push(Note {
                start: start + k as f64 * 0.05,
                end: start + 0.9,
                key: root + step + 12,
                velocity: 90,
                voice: 0,
            });
        }
    }
    Notes::new(notes)
}

fn frame(ui: &mut Ui, app: &mut App, size: (u16, u16)) -> Result<bool, Box<dyn Error>> {
    let r = app::view(ui, app);
    let s = Size::new(f64::from(size.0), f64::from(size.1));
    let f = ui
        .frame(r.el, Some(s), Input::default(), 1. / 60.)
        .map_err(|e| format!("{e:?}"))?;
    Ok(f.animating)
}

fn settle(ui: &mut Ui, app: &mut App, size: (u16, u16)) -> Result<(), Box<dyn Error>> {
    // The strip takes its height from the window's last frame: a few
    // frames before it eases to it.
    for n in 0..240 {
        if !frame(ui, app, size)? && n > 4 {
            break;
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: played_shot <out dir>")?,
    );
    std::fs::create_dir_all(&out)?;
    let size = (1180u16, 780u16);
    let w = size.0;
    for d in design::all() {
        let mut ui = kontra_ui::ui();
        let mut app = editor(&d.meta.name);
        settle(&mut ui, &mut app, size)?;
        let name = d.meta.name.to_lowercase();
        shoot(&ui, &out.join(format!("played-{w}-{name}.png")), size)?;
    }
    // Under a song: the player holds two of its chord's keys and a wrong one.
    let mut ui = kontra_ui::ui();
    let mut app = editor("");
    let struck = |start: f64, key: u8| Note {
        start,
        end: f64::INFINITY,
        key,
        velocity: 100,
        voice: 0,
    };
    app.played = Played {
        notes: Notes::new(vec![struck(9.95, 60), struck(9.97, 64), struck(9.99, 66)]),
        at: 10.0,
        begun: true,
    };
    app.song = Some(Song {
        name: "chords.mid".into(),
        position: 0.05,
        length: 4.0,
        notes: song(),
        ..Default::default()
    });
    settle(&mut ui, &mut app, size)?;
    shoot(&ui, &out.join(format!("played-{w}-over-song.png")), size)?;
    Ok(())
}

fn shoot(ui: &Ui, file: &Path, (w, h): (u16, u16)) -> Result<(), Box<dyn Error>> {
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
    let rgba: Vec<u8> = pix
        .take_unpremultiplied()
        .iter()
        .flat_map(|p| [p.r, p.g, p.b, p.a])
        .collect();
    let mut e = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(file)?),
        w.into(),
        h.into(),
    );
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&rgba)?;
    println!("{}", file.display());
    Ok(())
}
