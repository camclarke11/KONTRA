//! The editor driven by the pointer, headless: a click drills into a
//! library, a click on a preset asks to load it, a menu pick lands, and
//! once motion settles the editor asks for no more frames.

use kontra_ui::app::{
    self, Action, App, Envelope, Face, Format, Library, Param, Part, PartTab, Sound,
};
use kontra_ui::waterfall::{Note, Notes, Played};
use mui::prelude::{Buttons, Input, Point, PointerInput, Size, Ui};

fn frame(ui: &mut Ui, app: &mut App, input: Input) -> (Vec<Action>, bool) {
    let r = app::view(ui, app);
    let f = ui.frame(r.el, Some(Size::new(900., 600.)), input, 1. / 60.);
    (r.changed, f.is_ok_and(|f| f.animating))
}

/// Frames until nothing moves; the actions they carried (a click lands the
/// frame after its release).
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
    clicks(ui, app, id, 1)
}

/// `n` clicks on `id` in quick succession (2: a double-click).
fn clicks(ui: &mut Ui, app: &mut App, id: &str, n: usize) -> Vec<Action> {
    // A missing target clicks nothing, and the asserts after it say so.
    let Some(s) = ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame) else {
        return Vec::new();
    };
    let at = Point::new(s.x + s.size.width / 2., s.y + s.size.height / 2.);
    let mut out = Vec::new();
    let presses = [Buttons::PRIMARY, Buttons::default()].repeat(n);
    for buttons in presses {
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

#[test]
fn clicks_drive_the_editor_and_idle_costs_no_frames() {
    let mut ui = kontra_ui::ui();
    let mut app = App {
        libraries: vec![Library {
            name: "Areia".into(),
            format: Format::Kontakt,
            presets: vec!["Violins".into()],
            starred: false,
            art: Vec::new(),
        }],
        ..App::default()
    };
    settle(&mut ui, &mut app);
    assert!(click(&mut ui, &mut app, "library-0").is_empty());
    assert_eq!(app.view.library, Some(0));
    // A click selects; a double-click loads.
    assert!(click(&mut ui, &mut app, "preset-0").is_empty());
    assert_eq!(app.view.picked, Some((0, 0)));
    assert_eq!(
        clicks(&mut ui, &mut app, "preset-0", 2),
        vec![Action::Load {
            library: 0,
            preset: 0
        }]
    );
    app.parts.push(Part {
        name: "Violins".into(),
        channel: 1,
        ..Part::default()
    });
    settle(&mut ui, &mut app);
    click(&mut ui, &mut app, "part-0-channel");
    assert_eq!(app.view.menu.as_deref(), Some("part-0-channel"));
    assert_eq!(
        click(&mut ui, &mut app, "part-0-channel-0"),
        vec![Action::Channel(0, 0)],
        "the first cell is every channel"
    );
    assert_eq!(app.view.menu, None, "a pick closes the menu");
    click(&mut ui, &mut app, "part-0-output");
    assert_eq!(
        click(&mut ui, &mut app, "part-0-output-2"),
        vec![Action::Output(0, 2)]
    );
    click(&mut ui, &mut app, "tab-Mixer");
    assert_eq!(
        click(&mut ui, &mut app, "mix-0-sm-mute"),
        vec![Action::Mute(0, true)],
        "the mixer's strip drives the same part"
    );
    assert!(
        !frame(&mut ui, &mut app, Input::default()).1,
        "settled: the host draws nothing until something changes"
    );
}

/// An empty editor says what to do first, with the button that does it:
/// add the folder the libraries are in; with libraries, pick one (and
/// show the browser to pick from); with no songs, add a folder of them.
#[test]
fn an_empty_editor_says_what_to_do_first() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    settle(&mut ui, &mut app);
    assert_eq!(
        click(&mut ui, &mut app, "rack-add-folder"),
        [Action::AddFolder]
    );
    assert_eq!(
        click(&mut ui, &mut app, "browser-empty-add"),
        [Action::AddFolder]
    );
    app.libraries = vec![Library {
        name: "Una Corda".into(),
        format: Format::Kontakt,
        presets: vec!["Cotton".into()],
        starred: false,
        art: Vec::new(),
    }];
    app.view.browser_hidden = true;
    settle(&mut ui, &mut app);
    let shown = |ui: &Ui, id: &str| ui.scene().and_then(|s| s.surface(id)).is_some();
    assert!(!shown(&ui, "rack-add-folder"));
    assert!(click(&mut ui, &mut app, "rack-show-browser").is_empty());
    assert!(!app.view.browser_hidden);
    assert!(!shown(&ui, "rack-show-browser"), "the browser is there");
    // The Falcon / UVI tab has none of them.
    click(&mut ui, &mut app, "format-Falcon / UVI");
    assert!(shown(&ui, "browser-empty-add"));
    click(&mut ui, &mut app, "format-Songs");
    assert_eq!(
        click(&mut ui, &mut app, "songs-empty-add"),
        [Action::Songs(app::SongAction::AddFolder(None))]
    );
}

/// The settings, opened from the menu in the smallest window while notes
/// rise over the keys, scroll inside the room there is: the keyboard stays
/// in the window.
#[test]
fn the_settings_fit_a_small_window_and_leave_the_keyboard_in_it() {
    let mut ui = kontra_ui::ui();
    let mut app = App {
        folders: vec!["/Libraries".into()],
        played: Played {
            notes: Notes::new(vec![Note {
                start: 0.5,
                end: 1.0,
                key: 60,
                velocity: 100,
                voice: 0,
            }]),
            at: 1.0,
            begun: true,
        },
        ..App::default()
    };
    settle(&mut ui, &mut app);
    settle(&mut ui, &mut app);
    click(&mut ui, &mut app, "app-menu");
    click(&mut ui, &mut app, "app-menu-0");
    assert_eq!(app.view.sheet, Some(app::Sheet::Settings));
    let frame_of = |ui: &Ui, id: &str| ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame);
    let bed = frame_of(&ui, "keybed").unwrap();
    assert!(bed.y + bed.size.height <= 600.5, "{bed:?}");
    let sheet = frame_of(&ui, "sheet").unwrap();
    assert!(
        sheet.y >= 0. && sheet.y + sheet.size.height <= bed.y,
        "{sheet:?}"
    );
}

/// "Play MIDI…" in the header asks for a file, and stays to pick another
/// while one plays; the song's transport is the keyboard bar's, and its
/// Stop stops it. However much is going on, a 900 pt window keeps every
/// header control in view, giving up the load readouts first.
#[test]
fn the_header_plays_and_stops_a_midi_file() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    settle(&mut ui, &mut app);
    // The first frame does not know the window's width yet; the next does.
    settle(&mut ui, &mut app);
    let in_view = |ui: &Ui, id: &str| {
        let f = ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame);
        f.is_some_and(|f| f.x >= 0. && f.x + f.size.width <= 900.)
    };
    for id in ["song-play", "midi-thru", "panic", "app-menu", "stat-CPU"] {
        assert!(in_view(&ui, id), "{id}");
    }
    assert!(!in_view(&ui, "stat-RAM"), "memory and disk need more room");
    assert_eq!(click(&mut ui, &mut app, "song-play"), [Action::PlaySong]);
    app.song = Some(app::Song {
        name: "Clair de Lune.mid".into(),
        position: 65.0,
        length: 312.0,
        notes: Notes::default(),
        ..Default::default()
    });
    app.export.song = Some("Clair de Lune.mid".into());
    app.export.job = Some(("Exporting video".into(), 0.4));
    app.report = vec!["a warning".into()];
    settle(&mut ui, &mut app);
    for id in [
        "song-play",
        "exporting",
        "midi-thru",
        "panic",
        "warnings",
        "app-menu",
    ] {
        assert!(in_view(&ui, id), "{id}");
    }
    let frame_of = |ui: &Ui, id: &str| ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame);
    let bar = frame_of(&ui, "keys-bar").unwrap();
    let stop = frame_of(&ui, "song-stop").unwrap();
    assert!(
        stop.y >= bar.y && stop.y + stop.size.height <= bar.y + bar.size.height,
        "the transport is the keyboard bar's: {stop:?} {bar:?}"
    );
    assert_eq!(click(&mut ui, &mut app, "song-stop"), [Action::StopSong]);
}

/// Picks `design` (by its index among the Designs) in the keyboard bar's
/// notes dropdown, or Off with `None`.
fn pick_notes(ui: &mut Ui, app: &mut App, design: Option<usize>) -> Vec<Action> {
    let off = kontra_ui::design::all().len();
    let mut out = click(ui, app, "song-notes");
    out.extend(click(
        ui,
        app,
        &format!("song-notes-{}", design.unwrap_or(off)),
    ));
    out
}

/// While a song plays its notes fall onto the keyboard in a lane exactly
/// as wide as the keybed, which shows every key the song plays; "Notes"
/// hides them and gives the keybed back its octaves; once the song ends
/// the keyboard is as it was and the editor asks for no more frames.
#[test]
fn a_playing_songs_notes_fall_onto_the_keys() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    settle(&mut ui, &mut app);
    let frame_of = |ui: &Ui, id: &str| ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame);
    let resting = frame_of(&ui, "keybed").unwrap();
    assert!(!app.follows_song());
    // A song above the keybed's C-2..B4: it moves to C1..B7 to show it.
    let note = |start: f64, key: u8| Note {
        start,
        end: start + 0.5,
        key,
        velocity: 100,
        voice: 0,
    };
    app.song = Some(app::Song {
        name: "runs.mid".into(),
        position: 1.2,
        length: 4.0,
        notes: Notes::new(vec![note(1.0, 41), note(1.5, 97), note(3.0, 60)]),
        ..Default::default()
    });
    settle(&mut ui, &mut app);
    assert!(app.follows_song(), "the host draws every refresh");
    let lane = frame_of(&ui, "song-falling").expect("the notes fall");
    let bed = frame_of(&ui, "keybed").unwrap();
    assert!((lane.x - bed.x).abs() < 0.5 && (lane.size.width - bed.size.width).abs() < 0.5);
    assert!(
        (lane.y + lane.size.height - bed.y).abs() < 0.5,
        "{lane:?} {bed:?}"
    );
    assert!(lane.size.height >= 90., "{lane:?}");
    assert!(
        (bed.size.height - resting.size.height).abs() < 0.5,
        "the keys keep their size"
    );
    // The keybed follows the song, so its octave arrows wait.
    assert!(frame_of(&ui, "octave-up").is_none());
    assert!(
        pick_notes(&mut ui, &mut app, None).is_empty(),
        "the editor's own"
    );
    assert!(app.view.notes_hidden);
    assert!(frame_of(&ui, "song-falling").is_none());
    assert!(frame_of(&ui, "octave-up").is_some());
    assert!(app.follows_song(), "its keys still light as they sound");
    pick_notes(&mut ui, &mut app, Some(0));
    assert!(frame_of(&ui, "song-falling").is_some());
    assert!(frame_of(&ui, "octave-up").is_none());
    // The song ends: the keyboard as it was, and not a frame more.
    app.song = None;
    settle(&mut ui, &mut app);
    assert!(!app.follows_song());
    assert!(frame_of(&ui, "song-falling").is_none());
    assert!(frame_of(&ui, "octave-up").is_some());
    assert_eq!(frame_of(&ui, "keybed"), Some(resting));
    assert!(
        !frame(&mut ui, &mut app, Input::default()).1,
        "stopped: the host draws nothing until something changes"
    );
}

/// With no song, the keyboard strip is as it was until the player plays;
/// from then on what they play rises from the keys in a lane as wide as
/// the keybed, which shows a piano's keys. "Notes" hides it, and the
/// keybed has its octaves back; a song's notes take its place while one
/// plays.
#[test]
fn what_the_player_plays_rises_from_the_keys() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    settle(&mut ui, &mut app);
    let frame_of = |ui: &Ui, id: &str| ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame);
    let resting = frame_of(&ui, "keybed").unwrap();
    assert!(
        frame_of(&ui, "played-rising").is_none(),
        "nothing played yet"
    );
    assert!(frame_of(&ui, "song-notes").is_none());
    let held = Note {
        start: 0.5,
        end: f64::INFINITY,
        key: 60,
        velocity: 100,
        voice: 0,
    };
    app.played = Played {
        notes: Notes::new(vec![held]),
        at: 1.0,
        begun: true,
    };
    settle(&mut ui, &mut app);
    let lane = frame_of(&ui, "played-rising").expect("the notes rise");
    let bed = frame_of(&ui, "keybed").unwrap();
    assert!((lane.x - bed.x).abs() < 0.5 && (lane.size.width - bed.size.width).abs() < 0.5);
    assert!((lane.y + lane.size.height - bed.y).abs() < 0.5);
    assert!((bed.size.height - resting.size.height).abs() < 0.5);
    assert!(frame_of(&ui, "octave-up").is_none(), "a piano's keys");
    pick_notes(&mut ui, &mut app, None);
    assert!(app.view.notes_hidden);
    assert!(frame_of(&ui, "played-rising").is_none());
    assert!(frame_of(&ui, "octave-up").is_some());
    pick_notes(&mut ui, &mut app, Some(0));
    assert!(frame_of(&ui, "played-rising").is_some());
    // A song: its notes fall in the lane instead.
    app.song = Some(app::Song {
        name: "one.mid".into(),
        position: 0.6,
        length: 2.0,
        notes: Notes::new(vec![Note {
            start: 0.5,
            end: 1.0,
            ..held
        }]),
        ..Default::default()
    });
    settle(&mut ui, &mut app);
    assert!(frame_of(&ui, "song-falling").is_some());
    assert!(frame_of(&ui, "played-rising").is_none());
}

/// While a song's notes fall, the keyboard bar's notes dropdown picks
/// their Design, or Off; the pick is the editor's own, kept in its view.
#[test]
fn the_falling_notes_take_the_design_picked() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    settle(&mut ui, &mut app);
    let shown = |ui: &Ui, id: &str| ui.scene().and_then(|s| s.surface(id)).is_some();
    assert!(!shown(&ui, "song-notes"), "no notes, no picker");
    let note = Note {
        start: 0.5,
        end: 1.0,
        key: 60,
        velocity: 100,
        voice: 0,
    };
    app.song = Some(app::Song {
        name: "one.mid".into(),
        position: 0.6,
        length: 2.0,
        notes: Notes::new(vec![note]),
        ..Default::default()
    });
    settle(&mut ui, &mut app);
    assert!(shown(&ui, "song-notes"));
    assert_eq!(app.view.design, "", "the default until one is picked");
    assert!(click(&mut ui, &mut app, "song-notes").is_empty());
    assert!(shown(&ui, "song-notes-2"), "the menu lists the Designs");
    assert!(click(&mut ui, &mut app, "song-notes-2").is_empty());
    assert_eq!(app.view.design, "Ivory");
    assert!(!shown(&ui, "song-notes-2"), "the pick closes the menu");
    // Off hides them, and keeps the Design for when they are back.
    pick_notes(&mut ui, &mut app, None);
    assert!(app.view.notes_hidden && !shown(&ui, "song-falling"));
    assert_eq!(app.view.design, "Ivory");
    assert!(shown(&ui, "song-notes"), "to bring them back");
    pick_notes(&mut ui, &mut app, Some(2));
    assert!(!app.view.notes_hidden && shown(&ui, "song-falling"));
    assert_eq!(app.view.design, "Ivory");
}

#[test]
fn a_dragged_envelope_handle_sets_every_group() {
    let mut ui = kontra_ui::ui();
    let sound = Sound {
        envelope: Some(Envelope {
            attack: 0.1,
            decay: 0.5,
            sustain: 0.5,
            release: 0.4,
            ..Envelope::default()
        }),
        ..Sound::default()
    };
    let mut app = App {
        parts: vec![Part {
            name: "Violins".into(),
            groups: vec!["Long".into(), "Short".into()],
            sounds: vec![sound.clone(), sound],
            open: true,
            tab: PartTab::Sound,
            ..Part::default()
        }],
        ..App::default()
    };
    settle(&mut ui, &mut app);
    let f = ui
        .scene()
        .and_then(|s| s.surface("part-0-envelope"))
        .map(|s| s.frame);
    assert!(f.is_some(), "the envelope graph is on screen");
    let Some(f) = f else { return };
    // The decay handle: 0.6 s of a 1.25 s span, at the sustain; the graph
    // is inset by 8 points.
    let (w, h) = (f.size.width - 16., f.size.height - 16.);
    let at = |x: f64, y: f64| Point::new(f.x + 8. + x * w, f.y + 8. + (1. - y) * h);
    let press = |pos, buttons| Input {
        pointer: PointerInput {
            pos: Some(pos),
            buttons,
            ..Default::default()
        },
        ..Default::default()
    };
    let start = at(0.6 / 1.25, 0.5);
    let mut out = Vec::new();
    for (pos, b) in [
        (start, Buttons::PRIMARY),
        (at(0.8 / 1.25, 0.25), Buttons::PRIMARY),
        (at(0.8 / 1.25, 0.25), Buttons::PRIMARY),
        (at(0.8 / 1.25, 0.25), Buttons::default()),
    ] {
        out.extend(frame(&mut ui, &mut app, press(pos, b)).0);
    }
    out.extend(settle(&mut ui, &mut app));
    let sustain = |g: usize| {
        app.parts
            .first()
            .and_then(|p| p.sounds.get(g))
            .and_then(|s| s.envelope)
            .map(|e| e.sustain)
    };
    assert!(out.iter().any(|a| matches!(a, Action::Sound { group: None, param: Param::Decay, value, .. } if (value - 0.7).abs() < 0.02)), "{out:?}");
    assert!((sustain(0).unwrap_or(0.) - 0.25).abs() < 0.02);
    assert_eq!(sustain(0), sustain(1), "All groups: both move");
}

/// Two rack parts showing their libraries' faces in one frame: each control
/// takes its own surface, and the frame lays out (two faces once shared
/// their ids, and the Rack stopped drawing: "two nodes share the id").
#[test]
fn two_open_faces_lay_out_side_by_side() {
    use kontra_ui_model::{Interface, Kind, Page, PageRef, Range, Rect, Size as PageSize, Widget};
    let mut tree = Interface::default();
    tree.pages.push(Page {
        size: PageSize {
            width: 300.,
            height: 100.,
        },
        ..Page::default()
    });
    let range = Range {
        max: 1.,
        ..Range::default()
    };
    let knob = Kind::Knob {
        range,
        display: kontra_ui_model::Display::default(),
    };
    tree.widgets.push(Widget::new(
        "$k",
        PageRef(0),
        Rect::new(10, 10, 40, 40),
        knob,
    ));
    let part = || Part {
        name: "Strings".into(),
        face: Some(Face {
            tree: tree.clone(),
            pictures: kontra_ui::Pictures::new(Box::new(|_| None)),
            gesture: kontra_ui::input::Gesture::default(),
            meters: Default::default(),
        }),
        open: true,
        tab: PartTab::Interface,
        ..Part::default()
    };
    let mut ui = kontra_ui::ui();
    let mut app = App {
        parts: vec![part(), part()],
        ..App::default()
    };
    let r = app::view(&mut ui, &mut app);
    let laid = ui.frame(
        r.el,
        Some(Size::new(900., 900.)),
        Input::default(),
        1. / 60.,
    );
    assert!(laid.is_ok(), "{:?}", laid.err());
    for n in 0..2 {
        let id = kontra_ui::input::surface(n, 0);
        assert!(ui.scene().and_then(|s| s.surface(&id)).is_some(), "{id}");
    }
}

#[test]
fn a_small_window_keeps_the_keyboard_in_it() {
    use kontra_ui_model::{Interface, Page, Size as PageSize};
    let mut tree = Interface::default();
    tree.pages.push(Page {
        size: PageSize {
            width: 1000.,
            height: 700.,
        },
        ..Page::default()
    });
    let mut ui = kontra_ui::ui();
    let mut app = App {
        parts: vec![Part {
            name: "Strings".into(),
            face: Some(Face {
                tree,
                pictures: kontra_ui::Pictures::new(Box::new(|_| None)),
                gesture: kontra_ui::input::Gesture::default(),
                meters: Default::default(),
            }),
            open: true,
            tab: PartTab::Interface,
            ..Part::default()
        }],
        ..App::default()
    };
    let window = Size::new(900., 560.);
    for _ in 0..3 {
        let r = app::view(&mut ui, &mut app);
        let laid = ui.frame(r.el, Some(window), Input::default(), 1. / 60.);
        assert!(laid.is_ok(), "{:?}", laid.err());
    }
    assert!((app.width - window.width).abs() < 0.5, "{}", app.width);
    let bed = ui.scene().and_then(|s| s.layout.frame("keybed"));
    assert!(bed.is_some(), "no keybed");
    let Some(bed) = bed else { return };
    let (x1, y1) = (bed.x + bed.size.width, bed.y + bed.size.height);
    assert!(bed.x >= 0. && x1 <= window.width + 0.5, "{bed:?}");
    assert!(bed.y >= 0. && y1 <= window.height + 0.5, "{bed:?}");
}

#[test]
fn a_face_in_the_rack_takes_the_pointer() {
    use kontra_ui_model::{Interface, Kind, Page, PageRef, Rect, Size as PageSize, Widget};
    let mut tree = Interface::default();
    tree.pages.push(Page {
        size: PageSize {
            width: 600.,
            height: 300.,
        },
        ..Page::default()
    });
    tree.widgets.push(Widget::new(
        "$b",
        PageRef(0),
        Rect::new(100, 100, 40, 40),
        Kind::Button { momentary: false },
    ));
    let mut ui = kontra_ui::ui();
    let mut app = App {
        parts: vec![Part {
            name: "Piano".into(),
            face: Some(Face {
                tree,
                pictures: kontra_ui::Pictures::new(Box::new(|_| None)),
                gesture: kontra_ui::input::Gesture::default(),
                meters: Default::default(),
            }),
            open: true,
            tab: PartTab::Interface,
            ..Part::default()
        }],
        ..App::default()
    };
    settle(&mut ui, &mut app);
    let id = kontra_ui::input::surface(0, 0);
    let out = click(&mut ui, &mut app, &id);
    assert!(
        out.iter().any(|a| matches!(a, Action::Control { .. })),
        "{out:?}"
    );
}

/// Pointer frames pressing on `from`, moving to `to` and releasing there.
fn drag(ui: &mut Ui, app: &mut App, from: &str, to: &str) -> Vec<Action> {
    let centre = |ui: &Ui, id: &str| {
        let f = ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame)?;
        Some(Point::new(
            f.x + f.size.width / 2.,
            f.y + f.size.height / 2.,
        ))
    };
    let (Some(a), Some(b)) = (centre(ui, from), centre(ui, to)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let steps = 8;
    for k in 0..=steps + 1 {
        let t = f64::from(k.min(steps)) / f64::from(steps);
        let at = Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        let buttons = if k <= steps {
            Buttons::PRIMARY
        } else {
            Buttons::default()
        };
        let pointer = PointerInput {
            pos: Some(at),
            buttons,
            ..Default::default()
        };
        let input = Input {
            pointer,
            ..Default::default()
        };
        out.extend(frame(ui, app, input).0);
    }
    out.extend(settle(ui, app));
    out
}

#[test]
fn a_preset_dragged_onto_a_part_replaces_it_and_onto_free_space_adds_one() {
    let mut ui = kontra_ui::ui();
    let mut app = App {
        libraries: vec![Library {
            name: "Areia".into(),
            format: Format::Kontakt,
            presets: vec!["Violins".into(), "Violas".into()],
            starred: false,
            art: Vec::new(),
        }],
        parts: vec![Part {
            name: "Violins".into(),
            library: "Areia".into(),
            ..Part::default()
        }],
        ..App::default()
    };
    settle(&mut ui, &mut app);
    click(&mut ui, &mut app, "library-0");
    let onto_part = drag(&mut ui, &mut app, "preset-1", "part-0-band");
    assert!(
        onto_part.contains(&Action::Replace {
            part: 0,
            library: 0,
            preset: 1
        }),
        "{onto_part:?}"
    );
    let onto_rack = drag(&mut ui, &mut app, "preset-1", "rack");
    assert!(
        onto_rack.contains(&Action::Load {
            library: 0,
            preset: 1
        }),
        "{onto_rack:?}"
    );
}

/// The edge between the libraries and the presets drags: down gives the
/// libraries more of the browser's height, and the view keeps the share.
#[test]
fn the_browser_split_drags_and_keeps_its_share() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    settle(&mut ui, &mut app);
    let at = (ui.scene())
        .and_then(|s| s.surface("browser-split"))
        .map(|s| s.frame);
    assert!(at.is_some(), "the browser has a split edge");
    let Some(f) = at else { return };
    let (x, y) = (f.x + f.size.width / 2., f.y + f.size.height / 2.);
    for k in 0..=10 {
        let pointer = PointerInput {
            pos: Some(Point::new(x, y + f64::from(k.min(9)) * 10.)),
            buttons: if k < 10 {
                Buttons::PRIMARY
            } else {
                Buttons::default()
            },
            ..Default::default()
        };
        let input = Input {
            pointer,
            ..Default::default()
        };
        frame(&mut ui, &mut app, input);
    }
    assert!(app.view.split > 0.55, "dragged down: {}", app.view.split);
    assert!(
        app.view.split <= 0.7,
        "held to v2's bound: {}",
        app.view.split
    );
}

// MARK: the transport and the Stage

use kontra_ui::stage::{self, Command, Hand, Shortcut, Transport};

/// A song loaded 20 s into its 120, as the host fills it.
fn loaded() -> App {
    let note = |start: f64, key: u8| Note {
        start,
        end: start + 0.5,
        key,
        velocity: 100,
        voice: 0,
    };
    App {
        song: Some(app::Song {
            name: "01 Chopin - Nocturne.mid".into(),
            position: 20.,
            length: 120.,
            notes: Notes::new(vec![note(19.5, 48), note(20.5, 60), note(21., 72)]),
            transport: Transport {
                title: "Nocturne".into(),
                composer: Some("Chopin".into()),
                take: 1,
                full_screen: Some(false),
                ..Transport::default()
            },
        }),
        ..App::default()
    }
}

fn song_mut(app: &mut App) -> &mut app::Song {
    app.song.get_or_insert_with(app::Song::default)
}

/// What the transport asked of the host in `actions`.
fn commands(actions: Vec<Action>) -> Vec<Command> {
    (actions.into_iter())
        .filter_map(|a| match a {
            Action::Song(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn on_screen(ui: &Ui, id: &str) -> bool {
    ui.scene().and_then(|s| s.surface(id)).is_some()
}

/// A frame with the pointer at `at` (Shift held if `shift`), pressed or
/// not; its actions, and when the editor asked to be woken, if it did.
fn point(
    ui: &mut Ui,
    app: &mut App,
    at: Point,
    pressed: bool,
    shift: bool,
) -> (Vec<Action>, Option<std::time::Duration>, bool) {
    let r = app::view(ui, app);
    let pointer = PointerInput {
        pos: Some(at),
        buttons: if pressed {
            Buttons::PRIMARY
        } else {
            Buttons::default()
        },
        mods: mui::prelude::Mods {
            shift,
            ..Default::default()
        },
    };
    let input = Input {
        pointer,
        ..Default::default()
    };
    let f = ui.frame(r.el, Some(Size::new(900., 600.)), input, 1. / 60.);
    let (wake, moving) = f.map_or((None, false), |f| (f.repaint_after, f.animating));
    (r.changed, wake, moving)
}

#[test]
fn the_keyboard_bars_transport_plays_pauses_seeks_and_sets_the_song() {
    let mut ui = kontra_ui::ui();
    let mut app = loaded();
    settle(&mut ui, &mut app);
    assert!(on_screen(&ui, "keys-bar") && on_screen(&ui, "song-seek"));
    assert!(
        !on_screen(&ui, "keys-range"),
        "the song's transport instead"
    );
    assert_eq!(
        commands(click(&mut ui, &mut app, "song-pause")),
        [Command::Pause]
    );
    song_mut(&mut app).transport.paused = true;
    settle(&mut ui, &mut app);
    assert_eq!(
        commands(click(&mut ui, &mut app, "song-pause")),
        [Command::Play]
    );
    // The seek bar's middle is the song's.
    let seek = commands(click(&mut ui, &mut app, "song-seek"));
    assert!(
        matches!(seek[..], [Command::Seek(t)] if (t - 60.).abs() < 2.),
        "{seek:?}"
    );
    // The rest under its menu, which stays open as they are set.
    assert!(commands(click(&mut ui, &mut app, "song-more")).is_empty());
    assert_eq!(app.view.menu.as_deref(), Some("song-more"));
    for (id, want) in [
        ("song-transpose-up", vec![Command::Transpose(1)]),
        ("song-transpose-down", vec![Command::Transpose(-1)]),
        ("song-loop-a", vec![Command::Marks(Some(20.), None)]),
        (
            "song-loop-b",
            vec![Command::Marks(None, Some(20.)), Command::Loop(true)],
        ),
        ("song-loop", vec![Command::Loop(true)]),
        ("song-left", vec![Command::Hand(Hand::Left, true)]),
        ("song-right", vec![Command::Hand(Hand::Right, true)]),
        ("song-metronome", vec![Command::Metronome(true)]),
        ("song-count-2", vec![Command::CountIn(2)]),
    ] {
        assert_eq!(commands(click(&mut ui, &mut app, id)), want, "{id}");
        assert_eq!(app.view.menu.as_deref(), Some("song-more"), "{id}");
    }
    // What the host sends back shows.
    let t = &mut song_mut(&mut app).transport;
    (t.speed, t.transpose, t.count_in, t.metronome) = (0.5, -3, 2, true);
    t.voices = vec!["Melody".into(), "Bass".into(), "Strings".into()];
    settle(&mut ui, &mut app);
    let text = |ui: &Ui, id: &str| {
        let s = ui.scene().and_then(|s| s.surface(id));
        s.and_then(|s| s.text_value.as_deref().map(str::to_owned))
    };
    assert_eq!(text(&ui, "song-speed-shown").as_deref(), Some("50%"));
    assert_eq!(text(&ui, "song-transpose-shown").as_deref(), Some("-3"));
    assert!(on_screen(&ui, "song-volume"), "the metronome's level");
    // Three voices: a mute for each track instead of the hands.
    assert!(!on_screen(&ui, "song-left"));
    assert_eq!(
        commands(click(&mut ui, &mut app, "song-track-2")),
        [Command::Voice(2, true)]
    );
    assert_eq!(
        commands(click(&mut ui, &mut app, "song-count-2")),
        [Command::CountIn(0)],
        "the count-in chosen again turns it off"
    );
}

/// Folded, the keyboard strip keeps its bar: the song's transport stays
/// (its clock woken each second, with no frame following the song), and
/// the same place brings the keys back.
#[test]
fn a_folded_keyboard_keeps_its_bar_and_the_songs_transport() {
    let mut ui = kontra_ui::ui();
    let mut app = loaded();
    settle(&mut ui, &mut app);
    click(&mut ui, &mut app, "keys-fold");
    assert!(app.view.keyboard_hidden);
    assert!(!on_screen(&ui, "keybed") && !on_screen(&ui, "song-falling"));
    assert!(on_screen(&ui, "song-pause") && on_screen(&ui, "song-seek"));
    assert!(!app.follows_song(), "no frame every refresh");
    let (_, wake, _) = point(&mut ui, &mut app, Point::new(600., 250.), false, false);
    let wake = wake.map(|w| w.as_secs_f64()).unwrap_or_default();
    assert!(wake > 0. && wake <= 1.1, "{wake}");
    // Paused, the clock stands: once any tooltip under the pointer has
    // shown, nothing to wake for.
    song_mut(&mut app).transport.paused = true;
    let mut wake = None;
    for _ in 0..90 {
        wake = point(&mut ui, &mut app, Point::new(600., 250.), false, false).1;
    }
    assert_eq!(wake, None);
    click(&mut ui, &mut app, "keys-fold");
    assert!(!app.view.keyboard_hidden && on_screen(&ui, "keybed"));
}

/// The Stage fills the window with the falling notes and the keys; F or
/// the keyboard bar opens it, F, Escape or its close button closes it.
#[test]
fn the_stage_fills_the_window_and_opens_and_closes_by_its_keys() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    let mut out = Vec::new();
    assert!(!stage::shortcut(&mut app, Shortcut::Stage, &mut out));
    assert!(!stage::shortcut(&mut app, Shortcut::PlayPause, &mut out));
    assert!(!app.view.stage.open, "no song, no Stage");
    let mut app = loaded();
    settle(&mut ui, &mut app);
    click(&mut ui, &mut app, "song-stage");
    assert!(app.view.stage.open);
    settle(&mut ui, &mut app);
    for gone in ["keys-bar", "browser", "rack", "keybed", "tab-Rack"] {
        assert!(!on_screen(&ui, gone), "{gone} is hidden");
    }
    let frame_of = |ui: &Ui, id: &str| ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame);
    let bed = frame_of(&ui, "stage-keybed").unwrap();
    let lane = frame_of(&ui, "stage-falling").unwrap();
    assert!((bed.size.width - 900.).abs() < 0.5 && (bed.y + bed.size.height - 600.).abs() < 0.5);
    assert!(lane.y.abs() < 0.5 && (lane.y + lane.size.height - bed.y).abs() < 0.5);
    assert!(lane.size.height > 400., "{lane:?}");
    assert!(app.follows_song(), "frames follow the song in the Stage");
    // Space plays and pauses; Escape closes it.
    assert!(stage::shortcut(&mut app, Shortcut::PlayPause, &mut out));
    assert_eq!(commands(std::mem::take(&mut out)), [Command::Pause]);
    assert!(stage::shortcut(&mut app, Shortcut::Leave, &mut out));
    assert!(!app.view.stage.open && out.is_empty());
    settle(&mut ui, &mut app);
    assert!(on_screen(&ui, "song-seek") && !on_screen(&ui, "stage"));
    // F opens and closes it.
    assert!(stage::shortcut(&mut app, Shortcut::Stage, &mut out));
    settle(&mut ui, &mut app);
    assert!(on_screen(&ui, "stage"));
    assert!(stage::shortcut(&mut app, Shortcut::Stage, &mut out));
    assert!(!app.view.stage.open);
    assert!(!stage::shortcut(&mut app, Shortcut::Leave, &mut out));
    // Its close button; the full screen it asked for is given back.
    stage::shortcut(&mut app, Shortcut::Stage, &mut out);
    settle(&mut ui, &mut app);
    assert_eq!(
        commands(click(&mut ui, &mut app, "stage-full")),
        [Command::FullScreen(true)]
    );
    assert!(app.view.stage.full);
    assert_eq!(
        commands(click(&mut ui, &mut app, "stage-close")),
        [Command::FullScreen(false)]
    );
    assert!(!app.view.stage.open);
    // A song that is stopped closes it.
    stage::shortcut(&mut app, Shortcut::Stage, &mut out);
    app.song = None;
    settle(&mut ui, &mut app);
    assert!(!app.view.stage.open && !on_screen(&ui, "stage"));
}

/// The transport hides two seconds after the pointer last moved, woken by
/// a timer rather than drawing every frame, comes back when it moves, and
/// stays while the song is paused. The title card shows as a take starts.
#[test]
fn the_stages_transport_hides_while_the_pointer_rests() {
    let mut ui = kontra_ui::ui();
    let mut app = loaded();
    let mut out = Vec::new();
    stage::shortcut(&mut app, Shortcut::Stage, &mut out);
    let here = Point::new(450., 200.);
    let (_, wake, _) = point(&mut ui, &mut app, here, false, false);
    for _ in 0..10 {
        point(&mut ui, &mut app, here, false, false);
    }
    assert!(on_screen(&ui, "stage-bar"));
    assert!(on_screen(&ui, "stage-card"), "a new take: its title");
    assert!(wake.is_some(), "the editor asks to be woken: {wake:?}");
    // Resting: the card and the transport go; then nothing more is asked.
    let mut last = (None, true);
    for _ in 0..(5 * 60) {
        let (_, wake, moving) = point(&mut ui, &mut app, here, false, false);
        last = (wake, moving);
    }
    assert!(!on_screen(&ui, "stage-bar") && !on_screen(&ui, "stage-card"));
    assert_eq!(last, (None, false), "idle: no timer, no motion");
    // A move brings it back, with a wake-up two seconds on (a tree reads
    // the pointer the frame before it).
    point(&mut ui, &mut app, Point::new(460., 210.), false, false);
    let (_, wake, _) = point(&mut ui, &mut app, Point::new(460., 210.), false, false);
    let wake = wake.map(|w| w.as_secs_f64()).unwrap_or_default();
    assert!((1.9..=2.1).contains(&wake), "{wake}");
    for _ in 0..5 {
        point(&mut ui, &mut app, Point::new(460., 210.), false, false);
    }
    assert!(on_screen(&ui, "stage-bar"), "faded in");
    // Paused, it stays.
    song_mut(&mut app).transport.paused = true;
    for _ in 0..(3 * 60) {
        point(&mut ui, &mut app, Point::new(460., 210.), false, false);
    }
    assert!(on_screen(&ui, "stage-bar"));
    // Played again from its end: a new take, the card again.
    song_mut(&mut app).transport.take = 2;
    for _ in 0..10 {
        point(&mut ui, &mut app, Point::new(460., 210.), false, false);
    }
    assert!(on_screen(&ui, "stage-card"));
}

/// A drag along the Stage's seek bar moves the song; begun with Shift, it
/// marks a loop over what it covered.
#[test]
fn a_drag_on_the_seek_bar_moves_the_song_or_marks_a_loop() {
    let mut ui = kontra_ui::ui();
    let mut app = loaded();
    song_mut(&mut app).transport.paused = true;
    let mut out = Vec::new();
    stage::shortcut(&mut app, Shortcut::Stage, &mut out);
    settle(&mut ui, &mut app);
    let f = ui
        .scene()
        .and_then(|s| s.surface("stage-seek"))
        .map(|s| s.frame)
        .unwrap();
    let at = |t: f64| Point::new(f.x + f.size.width * t, f.y + f.size.height / 2.);
    let drag = |ui: &mut Ui, app: &mut App, shift: bool| {
        let mut out = Vec::new();
        for (t, pressed) in [
            (0.25, true),
            (0.4, true),
            (0.6, true),
            (0.75, true),
            (0.75, false),
        ] {
            out.extend(point(ui, app, at(t), pressed, shift).0);
        }
        out.extend(point(ui, app, at(0.75), false, false).0);
        commands(out)
    };
    let seeks = drag(&mut ui, &mut app, false);
    let times: Vec<f64> = (seeks.iter())
        .map(|c| match c {
            Command::Seek(t) => *t,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(
        times.len() >= 3 && times.windows(2).all(|w| w[1] > w[0]),
        "{times:?}"
    );
    assert!((times[0] - 30.).abs() < 1. && (times.last().unwrap() - 90.).abs() < 1.);
    // Again, from where the last drag left the pointer: it still takes it.
    let again = drag(&mut ui, &mut app, false);
    assert_eq!(again, seeks, "a second drag moves it too");
    let looped = drag(&mut ui, &mut app, true);
    assert!(
        matches!(looped[..], [Command::Marks(Some(a), Some(b)), Command::Loop(true)]
            if (a - 30.).abs() < 1. && (b - 90.).abs() < 1.),
        "{looped:?}"
    );
    assert!(app.view.stage.scrub.is_none());
}
