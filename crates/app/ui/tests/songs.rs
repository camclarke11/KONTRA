//! The browser's Songs tab driven by the pointer, headless: the tab lists
//! its folders' songs, a click plays one, a star keeps it, the shelves and
//! the search narrow the list, the touch-ups switch and say what they did,
//! and the keyboard bar names the chord sounding.

use std::path::PathBuf;

use kontra_ui::app::{
    self, Action, App, Format, Played, SongAction, SongFile, SongFolder, SongShelf, Songbook, Touch,
};
use kontra_ui::waterfall::{Note, Notes};
use mui::prelude::{Buttons, Input, Point, PointerInput, Size, Ui};

fn frame(ui: &mut Ui, app: &mut App, input: Input) -> (Vec<Action>, bool) {
    let r = app::view(ui, app);
    let f = ui.frame(r.el, Some(Size::new(1180., 780.)), input, 1. / 60.);
    (r.changed, f.is_ok_and(|f| f.animating))
}

fn settle(ui: &mut Ui, app: &mut App) -> Vec<Action> {
    let mut out = Vec::new();
    for _ in 0..240 {
        let (actions, moving) = frame(ui, app, Input::default());
        out.extend(actions);
        if !moving {
            break;
        }
    }
    out
}

fn click(ui: &mut Ui, app: &mut App, id: &str) -> Vec<Action> {
    let target = ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame);
    assert!(target.is_some(), "no {id} on screen");
    let Some(s) = target else {
        return Vec::new();
    };
    let at = Point::new(s.x + s.size.width / 2., s.y + s.size.height / 2.);
    let mut out = Vec::new();
    for buttons in [Buttons::PRIMARY, Buttons::default()] {
        let pointer = PointerInput {
            pos: Some(at),
            buttons,
            ..Default::default()
        };
        out.extend(
            frame(
                ui,
                app,
                Input {
                    pointer,
                    ..Default::default()
                },
            )
            .0,
        );
    }
    out.extend(settle(ui, app));
    out
}

fn shown(ui: &Ui, id: &str) -> bool {
    ui.scene().is_some_and(|s| s.surface(id).is_some())
}

fn text(ui: &Ui, id: &str) -> Option<String> {
    let s = ui.scene()?;
    Some(s.surface(id)?.text_value.as_deref()?.to_owned())
}

fn song(folder: Option<usize>, dir: &str, name: &str) -> SongFile {
    SongFile {
        path: PathBuf::from(format!("/songs/{dir}/{name}.mid")),
        name: name.into(),
        folder,
        dir: dir.into(),
        length: 187.,
        tracks: 2,
        needs: Touch {
            pedal: true,
            humanise: true,
            roll: true,
            velocity: false,
        },
        ..SongFile::default()
    }
}

fn editor() -> App {
    App {
        songbook: Songbook {
            folders: vec![SongFolder {
                name: "KONTRA test pieces".into(),
                path: "/songs".into(),
            }],
            songs: vec![
                song(Some(0), "", "Nocturne"),
                song(Some(0), "", "Rivers Flow In You"),
                song(Some(0), "Etudes", "Revolutionary"),
                song(None, "", "Dropped"),
            ],
            recent: vec![3, 1],
            ..Songbook::default()
        },
        ..App::default()
    }
}

#[test]
fn the_songs_tab_lists_folders_and_a_click_plays() {
    let mut ui = kontra_ui::ui();
    let mut app = editor();
    settle(&mut ui, &mut app);
    assert!(!shown(&ui, "song-0"), "the Kontakt tab first");
    let asked = click(&mut ui, &mut app, "format-Songs");
    assert_eq!(
        asked,
        [Action::Song(SongAction::Rescan)],
        "the folders read again"
    );
    assert_eq!(app.view.format, Format::Songs);
    for id in [
        "song-search",
        "songs-add",
        "song-0",
        "song-1",
        "song-2",
        "touch-pedal",
    ] {
        assert!(shown(&ui, id), "{id}");
    }
    assert!(
        !shown(&ui, "song-3"),
        "a song outside the folders is on its shelves only"
    );
    assert!(shown(&ui, "song-folder-0-KONTRA test pieces"));
    assert!(shown(&ui, "song-folder-0-KONTRA test pieces / Etudes"));
    let play = click(&mut ui, &mut app, "song-1");
    assert_eq!(
        play,
        [Action::Song(SongAction::Play(
            "/songs//Rivers Flow In You.mid".into()
        ))]
    );
    assert_eq!(
        click(&mut ui, &mut app, "songs-add"),
        [Action::Song(SongAction::AddFolder(None))]
    );
    // Folded, a folder hides its songs.
    click(
        &mut ui,
        &mut app,
        "song-folder-0-KONTRA test pieces / Etudes",
    );
    assert!(!shown(&ui, "song-2") && shown(&ui, "song-0"));
    assert_eq!(
        click(&mut ui, &mut app, "song-folder-remove-0"),
        [Action::Song(SongAction::RemoveFolder("/songs".into()))]
    );
    // Folded, the folder hides everything in it.
    click(&mut ui, &mut app, "song-folder-0-KONTRA test pieces");
    assert!(!shown(&ui, "song-0") && !shown(&ui, "song-folder-0-KONTRA test pieces / Etudes"));
    assert!(shown(&ui, "song-folder-remove-0"));
    // Back to libraries, and back again: no second read unasked.
    click(&mut ui, &mut app, "format-Kontakt");
    assert!(!shown(&ui, "song-0"));
}

#[test]
fn stars_shelves_and_the_search_narrow_the_list() {
    let mut ui = kontra_ui::ui();
    let mut app = editor();
    app.view.format = Format::Songs;
    settle(&mut ui, &mut app);
    let star = click(&mut ui, &mut app, "song-star-0");
    assert_eq!(
        star,
        [Action::Song(SongAction::Star(
            "/songs//Nocturne.mid".into(),
            true
        ))]
    );
    assert!(app.songbook.songs[0].starred, "shown at once");
    click(&mut ui, &mut app, "song-shelf-Favorites");
    assert_eq!(app.view.songs.shelf, SongShelf::Favorites);
    assert!(shown(&ui, "song-0") && !shown(&ui, "song-1"));
    click(&mut ui, &mut app, "song-shelf-Recent");
    assert!(shown(&ui, "song-3") && shown(&ui, "song-1") && !shown(&ui, "song-0"));
    click(&mut ui, &mut app, "song-shelf-Recent");
    assert_eq!(app.view.songs.shelf, SongShelf::All);
    app.view.songs.search = "rev etudes".into();
    settle(&mut ui, &mut app);
    assert!(shown(&ui, "song-2") && !shown(&ui, "song-0") && !shown(&ui, "song-1"));
    app.view.songs.search = "nothing like it".into();
    settle(&mut ui, &mut app);
    assert!(shown(&ui, "songs-empty"));
    app.view.songs.search.clear();
    app.songbook = Songbook::default();
    settle(&mut ui, &mut app);
    assert!(shown(&ui, "songs-empty"), "no folders yet");
}

/// C E G held from 1 s: the chord they make.
fn triad() -> Notes {
    Notes::new(
        [48, 64, 67]
            .map(|key| Note {
                start: 1.0,
                end: 3.0,
                key,
                velocity: 80,
                voice: 0,
            })
            .to_vec(),
    )
}

#[test]
fn the_touch_ups_switch_and_say_what_they_did_to_the_song_playing() {
    let mut ui = kontra_ui::ui();
    let mut app = editor();
    app.view.format = Format::Songs;
    settle(&mut ui, &mut app);
    let off = click(&mut ui, &mut app, "touch-pedal");
    let mut t = Touch {
        pedal: false,
        ..Touch::default()
    };
    assert_eq!(off, [Action::Song(SongAction::Touch(t))]);
    assert!(!app.songbook.touch.pedal);
    let roll = click(&mut ui, &mut app, "touch-roll");
    t.roll = true;
    assert_eq!(roll, [Action::Song(SongAction::Touch(t))]);

    // A song plays that got a pedal and a hand.
    app.songbook.playing = Some(Played {
        path: "/songs//Rivers Flow In You.mid".into(),
        name: "Rivers Flow In You.mid".into(),
        needs: Touch {
            velocity: false,
            ..Touch::default()
        },
        applied: Touch::default(),
        ringing: triad(),
    });
    // Its keys let go at once: only the pedal holds the chord.
    let struck: Vec<Note> = (triad().between(0.0, 9.0))
        .map(|n| Note { end: 1.2, ..*n })
        .collect();
    app.song = Some(app::Song {
        name: "Rivers Flow In You.mid".into(),
        position: 2.0,
        length: 180.0,
        notes: Notes::new(struck),
    });
    settle(&mut ui, &mut app);
    assert_eq!(
        text(&ui, "song-chord").as_deref(),
        Some("C"),
        "heard as it rings"
    );
    assert_eq!(
        text(&ui, "song-touched").as_deref(),
        Some("auto-pedal · humanised")
    );
    assert_eq!(
        text(&ui, "touch-status").as_deref(),
        Some("This song got auto-pedal, humanise")
    );
    // Another song, not read by the tab: its keys alone, which let go.
    app.song.as_mut().unwrap().name = "other.mid".into();
    settle(&mut ui, &mut app);
    assert_eq!(text(&ui, "song-chord").as_deref(), Some(""));
    assert!(!shown(&ui, "song-touched"));
}

#[test]
fn the_keyboard_bar_names_the_chord_from_the_keys_alone_without_the_tab() {
    let mut ui = kontra_ui::ui();
    let mut app = App {
        song: Some(app::Song {
            name: "dropped.mid".into(),
            position: 2.0,
            length: 4.0,
            notes: triad(),
        }),
        ..App::default()
    };
    settle(&mut ui, &mut app);
    assert_eq!(text(&ui, "song-chord").as_deref(), Some("C"));
    assert!(!shown(&ui, "song-touched"), "nothing touched");
    app.song.as_mut().unwrap().position = 3.5;
    settle(&mut ui, &mut app);
    assert_eq!(
        text(&ui, "song-chord").as_deref(),
        Some(""),
        "silent: no chord"
    );
    app.song = None;
    settle(&mut ui, &mut app);
    assert!(!shown(&ui, "song-chord"));
}
