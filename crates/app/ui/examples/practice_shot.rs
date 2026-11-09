//! The editor headless with recording, practice and exporting showing:
//! idle, a take under way, an export's progress, and a song waiting for the
//! player with the keys to press, one struck and a wrong one marked. Saves
//! `<out>/studio-<width>-<state>.png`, and `<out>/video-frame.png`, one
//! frame of a song's video.
//! `cargo run --release -p kontra-ui --example practice_shot -- <out>`

use std::error::Error;
use std::path::{Path, PathBuf};

use kontra_ui::app::{self, App, Part, Song};
use kontra_ui::export::{Exporting, Frames};
use kontra_ui::practice::{Hands, Practice};
use kontra_ui::record::{Recording, Take};
use kontra_ui::waterfall::{Note, Notes};
use mui::prelude::{Input, Size, Ui};
use mui::vello;

/// Chords a beat apart, a melody over them: the right hand voice 0.
fn song() -> Notes {
    let mut notes = Vec::new();
    let chords: [[u8; 3]; 4] = [[48, 55, 64], [45, 52, 60], [41, 48, 57], [43, 50, 59]];
    for (bar, chord) in chords.iter().enumerate() {
        let at = bar as f64 * 2.0;
        for &key in chord {
            notes.push(Note {
                start: at,
                end: at + 1.9,
                key,
                velocity: 60,
                voice: 1,
            });
        }
        for (n, key) in [72u8, 76, 79, 76].into_iter().enumerate() {
            notes.push(Note {
                start: at + n as f64 * 0.5,
                end: at + n as f64 * 0.5 + 0.45,
                key: key - bar as u8,
                velocity: 90,
                voice: 0,
            });
        }
    }
    Notes::new(notes)
}

fn editor(notes: Notes) -> App {
    App {
        parts: vec![Part {
            name: "Synthetic Organ".into(),
            output: 1,
            keys: (21, 108),
            ..Part::default()
        }],
        song: Some(Song {
            name: "chords.mid".into(),
            position: 2.0,
            length: 8.0,
            notes,
        }),
        export: Exporting {
            song: Some("chords.mid".into()),
            ..Exporting::default()
        },
        record: Recording {
            folder: "~/Music/KONTRA Recordings".into(),
            ..Recording::default()
        },
        ..App::default()
    }
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
    for _ in 0..240 {
        if !frame(ui, app, size)? {
            break;
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: practice_shot <out dir>")?,
    );
    std::fs::create_dir_all(&out)?;
    for size in [(1180u16, 780u16), (900, 640)] {
        let (w, h) = size;
        let mut ui = kontra_ui::ui();
        let mut app = editor(song());
        settle(&mut ui, &mut app, size)?;
        shoot(&ui, &out.join(format!("studio-{w}-idle.png")), size)?;

        app.record.take = Take::Recording;
        app.record.seconds = 74.0;
        app.export.job = Some(("Exporting video".into(), 0.42));
        app.practice = Practice {
            hands: Hands::Right,
            waiting: true,
            // The melody's note at 2 s, and a chord tone to show a struck one.
            want: vec![64, 71],
            struck: vec![64],
            wrong: vec![73],
        };
        settle(&mut ui, &mut app, size)?;
        shoot(&ui, &out.join(format!("studio-{w}-busy.png")), size)?;

        app.record.take = Take::CountIn(3);
        app.export.job = None;
        app.export.last = Some("saved ~/Music/KONTRA Recordings/chords.wav".into());
        settle(&mut ui, &mut app, size)?;
        shoot(&ui, &out.join(format!("studio-{w}-count-in.png")), size)?;
        println!("{{\"width\":{w},\"height\":{h}}}");
    }
    // One frame of the video, at 1080p, two seconds in.
    let notes = song();
    let mut frames = Frames::new(1920, 1080);
    let rgba = frames.draw(&notes, 2.2, kontra_ui::export::range(&notes), "chords.mid")?;
    save(&out.join("video-frame.png"), 1920, 1080, &rgba)?;
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
    save(file, w.into(), h.into(), &rgba)
}

fn save(file: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut e = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(file)?), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(rgba)?;
    Ok(())
}
