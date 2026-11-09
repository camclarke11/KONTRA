//! The editor headless with the browser's Songs tab open, a folder of
//! songs listed and one playing: its notes falling, the chord it sounds
//! named in the keyboard bar, the touch-ups it got beside it and in the
//! tab. The songbook and the song are made here (a layout check, not the
//! player's files): F#m, D, A and E broken over two beats each with a
//! melody above, as a pedal rings them. Saves `<out>/songs-<w>-<at>.png`
//! and prints the chord at each.
//! `cargo run --release -p kontra-ui --example songs_shot -- <out>`

use std::error::Error;
use std::path::{Path, PathBuf};

use kontra_ui::app::{
    self, App, Format, Part, Played, Song, SongFile, SongFolder, Songbook, Touch,
};
use kontra_ui::waterfall::{Note, Notes};
use mui::prelude::{Input, Size, Ui};
use mui::vello;

/// Seconds a beat lasts.
const BEAT: f64 = 0.92;

/// The song as its keys play it, and as it rings under the pedal (each
/// note to the next change, two beats on).
fn song() -> (Notes, Notes) {
    let chords: [[u8; 3]; 4] = [[54, 61, 66], [50, 57, 64], [45, 52, 61], [56, 59, 64]];
    let melody = [81u8, 80, 81, 76, 81, 80, 78, 76];
    let (mut keys, mut rung) = (Vec::new(), Vec::new());
    for half in 0..32usize {
        let at = half as f64 * 2.0 * BEAT;
        let change = at + 2.0 * BEAT;
        let figure = chords.get(half % 4).copied().unwrap_or_default();
        for (n, key) in figure.into_iter().enumerate() {
            let start = at + n as f64 * 0.5 * BEAT;
            keys.push(Note {
                start,
                end: start + 0.5 * BEAT,
                key,
                velocity: 70,
                voice: 1,
            });
            rung.push(Note {
                start,
                end: change,
                key,
                velocity: 70,
                voice: 1,
            });
        }
        for (n, key) in [half % 8, (half + 3) % 8]
            .map(|n| melody.get(n).copied().unwrap_or(81))
            .into_iter()
            .enumerate()
        {
            let start = at + n as f64 * BEAT;
            keys.push(Note {
                start,
                end: start + BEAT,
                key,
                velocity: 85,
                voice: 0,
            });
            rung.push(Note {
                start,
                end: change,
                key,
                velocity: 85,
                voice: 0,
            });
        }
    }
    (Notes::new(keys), Notes::new(rung))
}

fn file(dir: &str, name: &str, length: f64, tracks: u16, needs: Touch) -> SongFile {
    SongFile {
        path: PathBuf::from(format!("/Songs/{dir}/{name}.mid")),
        name: name.into(),
        folder: Some(0),
        dir: dir.into(),
        length,
        tracks,
        needs,
        ..SongFile::default()
    }
}

fn editor() -> App {
    let (notes, ringing) = song();
    let played = Touch {
        pedal: false,
        humanise: false,
        roll: false,
        velocity: false,
    };
    let rough = Touch {
        pedal: true,
        humanise: true,
        roll: true,
        velocity: false,
    };
    let mut songs: Vec<SongFile> = [
        ("01 Chopin - Nocturne in E-flat, Op 9 No 2", 225.0),
        ("02 Liszt - La Campanella", 301.0),
        (
            "03 Rachmaninoff - Prelude in C-sharp minor, Op 3 No 2",
            244.0,
        ),
        ("04 Chopin - Revolutionary Etude, Op 10 No 12", 158.0),
        ("05 Schubert - Impromptu in G-flat, Op 90 No 3", 351.0),
    ]
    .into_iter()
    .map(|(name, length)| file("", name, length, 1, played))
    .collect();
    songs.push(file("Pop", "Yiruma - Rivers Flow In You", 165.0, 1, rough));
    songs.push(file(
        "Pop",
        "Arpeggios",
        40.0,
        1,
        Touch {
            velocity: true,
            ..rough
        },
    ));
    if let Some(first) = songs.first_mut() {
        first.starred = true;
    }
    let name = "Yiruma - Rivers Flow In You.mid";
    App {
        parts: vec![Part {
            name: "Grand Piano".into(),
            library: "Synthetic".into(),
            output: 1,
            keys: (21, 108),
            ..Part::default()
        }],
        song: Some(Song {
            name: name.into(),
            position: 0.,
            length: 165.,
            notes,
            ..Default::default()
        }),
        songbook: Songbook {
            folders: vec![SongFolder {
                name: "KONTRA test pieces".into(),
                path: "/Songs".into(),
            }],
            recent: vec![5, 2],
            playing: Some(Played {
                path: "/Songs/Pop/Yiruma - Rivers Flow In You.mid".into(),
                name: name.into(),
                needs: rough,
                applied: Touch::default(),
                ringing,
            }),
            songs,
            ..Songbook::default()
        },
        view: app::View {
            format: Format::Songs,
            ..app::View::default()
        },
        ..App::default()
    }
}

fn frame(ui: &mut Ui, app: &mut App, size: (u16, u16)) -> Result<bool, Box<dyn Error>> {
    let r = app::view(ui, app);
    let size = Size::new(f64::from(size.0), f64::from(size.1));
    let f = ui
        .frame(r.el, Some(size), Input::default(), 1. / 60.)
        .map_err(|e| format!("{e:?}"))?;
    Ok(f.animating)
}

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: songs_shot <out dir>")?,
    );
    std::fs::create_dir_all(&out)?;
    for size in [(1180u16, 780u16), (900, 640)] {
        let mut ui = kontra_ui::ui();
        let mut app = editor();
        for _ in 0..240 {
            if !frame(&mut ui, &mut app, size)? {
                break;
            }
        }
        for at in [1.5, 3.4, 5.2, 7.0] {
            let at = at * BEAT;
            for n in (0..30).rev() {
                if let Some(s) = app.song.as_mut() {
                    s.position = (at - f64::from(n) / 60.).max(0.);
                }
                frame(&mut ui, &mut app, size)?;
            }
            let chord = ui
                .scene()
                .and_then(|s| s.surface("song-chord")?.text_value.clone())
                .unwrap_or_default();
            let (w, h) = size;
            let path = out.join(format!("songs-{w}-{at:05.2}.png"));
            save(&path, w, h, &paint(&ui, w, h)?)?;
            println!("{{\"width\":{w},\"at\":{at:.2},\"chord\":{chord:?},\"shot\":{path:?}}}");
        }
        // A recording being transcribed into a song, then the song it made.
        let (w, h) = size;
        for (state, t) in [
            (
                "transcribing",
                kontra_ui::transcribe::Transcribing {
                    job: Some(("Nocturne in E-flat (my take).m4a".into(), 0.42)),
                    ready: true,
                    ..Default::default()
                },
            ),
            (
                "transcribed",
                kontra_ui::transcribe::Transcribing {
                    last: Some("Made Nocturne in E-flat (my take).mid".into()),
                    ready: true,
                    ..Default::default()
                },
            ),
        ] {
            app.transcribe = t;
            for _ in 0..30 {
                frame(&mut ui, &mut app, size)?;
            }
            let path = out.join(format!("songs-{w}-{state}.png"));
            save(&path, w, h, &paint(&ui, w, h)?)?;
            println!("{{\"width\":{w},\"state\":{state:?},\"shot\":{path:?}}}");
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
