//! The whole editor headless, state by state, as a pianist meets it: a
//! first run with nothing found, libraries and an empty rack, a piano
//! loaded and played, a MIDI file playing (its menu open), the Songs tab,
//! recording, exporting and practice under way, the settings, and the
//! Stage. Every state is drawn at the default window (1180 x 780) and the
//! smallest (900 x 600), so a review compares like with like. Saves
//! `<out>/<state>-<w>x<h>.png`.
//! `cargo run --profile fast -p kontra-ui --example tour_shot -- <out>`

use std::error::Error;
use std::path::{Path, PathBuf};

use kontra_ui::app::{
    self, App, Format, Library, Part, Played as SongPlayed, Song, SongFile, SongFolder, Songbook,
    Stats, Touch,
};
use kontra_ui::export::Exporting;
use kontra_ui::practice::{Hands, Practice};
use kontra_ui::record::{Recording, Take};
use kontra_ui::stage::{self, Transport};
use kontra_ui::waterfall::{Note, Notes, Pedal, Played};
use mui::prelude::{Buttons, Input, Point, PointerInput, Size, Ui};
use mui::vello;

const SIZES: [(u16, u16); 2] = [(1180, 780), (900, 600)];

fn note(start: f64, end: f64, key: u8, velocity: u8, voice: u8) -> Note {
    Note {
        start,
        end,
        key,
        velocity,
        voice,
    }
}

/// Left-hand chords a bar long under right-hand runs, the pedal changed
/// each bar.
fn song() -> Notes {
    let mut notes = Vec::new();
    let mut pedal = Vec::new();
    let chords: [[u8; 3]; 4] = [[48, 55, 64], [45, 52, 60], [41, 48, 57], [43, 50, 59]];
    let scale = [72u8, 74, 76, 77, 79, 81, 83, 84];
    for bar in 0..24usize {
        let at = bar as f64 * 2.;
        for &key in chords.get(bar % 4).into_iter().flatten() {
            notes.push(note(at, at + 1.98, key, 60, 1));
        }
        pedal.push((at + 0.03, 0, 0));
        pedal.push((at + 0.12, 0, 127));
        for s in 0..16usize {
            let step = if s < 8 { s } else { 15 - s };
            let key = scale.get(step).copied().unwrap_or(72) + if bar % 2 == 1 { 12 } else { 0 };
            let start = at + s as f64 / 8.;
            notes.push(note(start, start + 0.11, key, 40 + (70 * s / 15) as u8, 0));
        }
    }
    Notes::new(notes).with_pedal(Pedal::new(pedal))
}

/// What the player played up to `now` seconds, the last notes held.
fn played(now: f64) -> Played {
    let held = f64::INFINITY;
    let notes = vec![
        note(3.2, 5.0, 36, 70, 0),
        note(3.5, 5.2, 43, 62, 0),
        note(3.8, held, 52, 66, 0),
        note(3.0, 3.6, 64, 92, 0),
        note(3.6, 4.2, 67, 98, 0),
        note(4.2, 5.4, 72, 110, 0),
        note(5.85, held, 72, 104, 0),
        note(5.9, held, 76, 96, 0),
    ];
    Played {
        notes: Notes::new(notes).with_pedal(Pedal::new([(3.1, 0, 127)])),
        at: now,
        begun: true,
    }
}

fn libraries() -> Vec<Library> {
    let library = |name: &str, format, presets: &[&str], starred| Library {
        name: name.into(),
        format,
        presets: presets.iter().map(|p| (*p).to_owned()).collect(),
        starred,
        art: Vec::new(),
    };
    vec![
        library(
            "Una Corda",
            Format::Kontakt,
            &["Una Corda Cotton", "Una Corda Felt", "Una Corda Pure"],
            true,
        ),
        library(
            "Noire",
            Format::Kontakt,
            &["Noire Pure", "Noire Felt", "Noire Particles"],
            true,
        ),
        library("Alicia's Keys", Format::Kontakt, &["Alicia's Keys"], false),
        library("The Gentleman", Format::Kontakt, &["The Gentleman"], false),
        library(
            "Felt Piano",
            Format::Uvi,
            &["Felt Piano Soft", "Felt Piano Hard"],
            false,
        ),
    ]
}

fn piano() -> Part {
    Part {
        name: "Una Corda Cotton".into(),
        library: "Una Corda".into(),
        channel: 0,
        output: 1,
        keys: (21, 108),
        level: vec![[0.35; 2]],
        ..Part::default()
    }
}

fn songbook(name: &str) -> Songbook {
    let none = Touch {
        pedal: false,
        humanise: false,
        roll: false,
        velocity: false,
    };
    let rough = Touch {
        pedal: true,
        humanise: true,
        roll: false,
        velocity: false,
    };
    let file = |dir: &str, name: &str, length: f64, needs: Touch| SongFile {
        path: PathBuf::from(format!("/Songs/{dir}/{name}.mid")),
        name: name.into(),
        folder: Some(0),
        dir: dir.into(),
        length,
        tracks: 2,
        needs,
        ..SongFile::default()
    };
    let mut songs = vec![
        file("", "Chopin - Nocturne Op 9 No 2", 265., none),
        file("", "Debussy - Clair de Lune", 312., none),
        file("", "Satie - Gymnopedie No 1", 196., none),
        file("Practice", "Hanon No 1", 120., rough),
        file("Practice", "Scales in C", 48., rough),
    ];
    if let Some(s) = songs.first_mut() {
        s.starred = true;
    }
    Songbook {
        folders: vec![SongFolder {
            name: "Piano".into(),
            path: "/Songs".into(),
        }],
        recent: vec![1, 0],
        playing: Some(SongPlayed {
            path: "/Songs/Debussy - Clair de Lune.mid".into(),
            name: name.into(),
            needs: rough,
            applied: rough,
            ringing: Notes::default(),
        }),
        songs,
        ..Songbook::default()
    }
}

/// A fresh editor: nothing scanned, nothing loaded.
fn first_run() -> App {
    App {
        build: "KONTRA 3.0.0-dev (c8035dcff)".into(),
        ..App::default()
    }
}

/// Libraries found, nothing loaded.
fn found() -> App {
    App {
        libraries: libraries(),
        folders: vec!["/Users/me/Libraries".into()],
        ..first_run()
    }
}

/// A piano loaded and played.
fn playing() -> App {
    let mut app = App {
        parts: vec![piano()],
        stats: Stats {
            cpu: 4.,
            voices: 18,
            ram_mb: 612,
            samples_mb: 280,
            disk_mbs: 2.1,
            cache_mb: 128,
        },
        level: 0.45,
        recent: vec![(0, 0)],
        played: played(6.),
        ..found()
    };
    app.view.library = Some(0);
    app
}

/// A MIDI file playing into the piano.
fn with_song() -> App {
    let name = "Debussy - Clair de Lune.mid";
    App {
        song: Some(Song {
            name: name.into(),
            position: 12.6,
            length: 312.,
            notes: song(),
            transport: Transport {
                title: "Clair de Lune".into(),
                composer: Some("Debussy".into()),
                voices: vec!["Right".into(), "Left".into()],
                take: 1,
                full_screen: Some(false),
                ..Transport::default()
            },
        }),
        songbook: songbook(name),
        export: Exporting {
            song: Some(name.into()),
            ..Exporting::default()
        },
        record: Recording {
            folder: "~/Music/KONTRA Recordings".into(),
            ..Recording::default()
        },
        played: Played::default(),
        ..playing()
    }
}

fn frame(
    ui: &mut Ui,
    app: &mut App,
    size: (u16, u16),
    input: Input,
) -> Result<bool, Box<dyn Error>> {
    let r = app::view(ui, app);
    let s = Size::new(f64::from(size.0), f64::from(size.1));
    let f = ui
        .frame(r.el, Some(s), input, 1. / 60.)
        .map_err(|e| format!("{e:?}"))?;
    Ok(f.animating)
}

fn settle(ui: &mut Ui, app: &mut App, size: (u16, u16)) -> Result<(), Box<dyn Error>> {
    for _ in 0..240 {
        if !frame(ui, app, size, Input::default())? {
            break;
        }
    }
    Ok(())
}

/// A click on `id`, wherever it is drawn; nothing when it is not.
fn click(ui: &mut Ui, app: &mut App, size: (u16, u16), id: &str) -> Result<bool, Box<dyn Error>> {
    let Some(at) = ui.scene().and_then(|s| s.surface(id)).map(|s| {
        Point::new(
            s.frame.x + s.frame.size.width / 2.,
            s.frame.y + s.frame.size.height / 2.,
        )
    }) else {
        eprintln!("no {id} on screen");
        return Ok(false);
    };
    for buttons in [Buttons::PRIMARY, Buttons::default()] {
        let pointer = PointerInput {
            pos: Some(at),
            buttons,
            ..Default::default()
        };
        frame(
            ui,
            app,
            size,
            Input {
                pointer,
                ..Default::default()
            },
        )?;
    }
    settle(ui, app, size)?;
    Ok(true)
}

fn shoot(ui: &Ui, out: &Path, state: &str, (w, h): (u16, u16)) -> Result<(), Box<dyn Error>> {
    let file = out.join(format!("{state}-{w}x{h}.png"));
    save(&file, w, h, &paint(ui, w, h)?)?;
    println!("{}", file.display());
    Ok(())
}

/// Draws `app` fresh at `size` after `prepare` (clicks and the like).
fn state(
    out: &Path,
    name: &str,
    size: (u16, u16),
    mut app: App,
    prepare: impl FnOnce(&mut Ui, &mut App) -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let mut ui = kontra_ui::ui();
    settle(&mut ui, &mut app, size)?;
    // Twice: the first frame lays out with no width known.
    settle(&mut ui, &mut app, size)?;
    prepare(&mut ui, &mut app)?;
    settle(&mut ui, &mut app, size)?;
    shoot(&ui, out, name, size)
}

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: tour_shot <out dir>")?,
    );
    std::fs::create_dir_all(&out)?;
    for size in SIZES {
        let none = |_: &mut Ui, _: &mut App| Ok(());
        state(&out, "1-first-run", size, first_run(), none)?;
        state(&out, "2-libraries", size, found(), none)?;
        state(&out, "3-piano", size, playing(), none)?;
        state(&out, "4-song", size, with_song(), none)?;
        state(&out, "5-song-menu", size, with_song(), |ui, app| {
            // The song's settings: the header's menu, or where it moved.
            for id in ["song-more", "song-settings"] {
                if click(ui, app, size, id)? {
                    break;
                }
            }
            Ok(())
        })?;
        let mut songs = with_song();
        songs.view.format = Format::Songs;
        state(&out, "6-songs-tab", size, songs, none)?;
        let mut busy = with_song();
        busy.record.take = Take::Recording;
        busy.record.seconds = 74.;
        busy.export.job = Some(("Exporting video".into(), 0.42));
        busy.practice = Practice {
            hands: Hands::Right,
            waiting: true,
            want: vec![76],
            struck: vec![],
            wrong: vec![73],
        };
        state(&out, "7-busy", size, busy, none)?;
        state(&out, "8-settings", size, playing(), |ui, app| {
            click(ui, app, size, "app-menu")?;
            click(ui, app, size, "app-menu-0")?;
            Ok(())
        })?;
        let mut on_stage = with_song();
        let mut actions = Vec::new();
        stage::shortcut(&mut on_stage, stage::Shortcut::Stage, &mut actions);
        state(&out, "9-stage", size, on_stage, |ui, app| {
            // Past the title card, the pointer moving: the transport shows.
            let (w, h) = (f64::from(size.0), f64::from(size.1));
            for n in 0..260 {
                let pointer = PointerInput {
                    pos: Some(Point::new(w / 2. + f64::from(n % 3), h / 2.)),
                    ..Default::default()
                };
                frame(
                    ui,
                    app,
                    size,
                    Input {
                        pointer,
                        ..Default::default()
                    },
                )?;
            }
            Ok(())
        })?;
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
