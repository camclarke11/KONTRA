//! The editor driven by the pointer, headless: a click drills into a
//! library, a click on a preset asks to load it, a menu pick lands, and
//! once motion settles the editor asks for no more frames.

use kontra_ui::app::{
    self, Action, App, Envelope, Face, Format, Library, Param, Part, PartTab, Sound,
};
use kontra_ui::waterfall::{Note, Notes};
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

/// "Play MIDI" asks for a file; while a song plays, the header shows it and
/// its Stop stops it.
#[test]
fn the_header_plays_and_stops_a_midi_file() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    settle(&mut ui, &mut app);
    assert_eq!(click(&mut ui, &mut app, "song-play"), [Action::PlaySong]);
    app.song = Some(app::Song {
        name: "Clair de Lune.mid".into(),
        position: 65.0,
        length: 312.0,
        notes: Notes::default(),
    });
    settle(&mut ui, &mut app);
    let scene = ui.scene().unwrap();
    assert!(scene.surface("song-play").is_none());
    assert!(scene.surface("song").is_some());
    assert_eq!(click(&mut ui, &mut app, "song-stop"), [Action::StopSong]);
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
        click(&mut ui, &mut app, "song-notes").is_empty(),
        "the editor's own"
    );
    assert!(app.view.notes_hidden);
    assert!(frame_of(&ui, "song-falling").is_none());
    assert!(frame_of(&ui, "octave-up").is_some());
    assert!(app.follows_song(), "its keys still light as they sound");
    click(&mut ui, &mut app, "song-notes");
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

/// While a song's notes fall, a dropdown beside "Notes" picks their
/// Design; the pick is the editor's own, kept in its view.
#[test]
fn the_falling_notes_take_the_design_picked() {
    let mut ui = kontra_ui::ui();
    let mut app = App::default();
    settle(&mut ui, &mut app);
    let shown = |ui: &Ui, id: &str| ui.scene().and_then(|s| s.surface(id)).is_some();
    assert!(!shown(&ui, "song-design"), "no song, no picker");
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
    });
    settle(&mut ui, &mut app);
    assert!(shown(&ui, "song-design"));
    assert_eq!(app.view.design, "", "the default until one is picked");
    assert!(click(&mut ui, &mut app, "song-design").is_empty());
    assert!(shown(&ui, "song-design-2"), "the menu lists the Designs");
    assert!(click(&mut ui, &mut app, "song-design-2").is_empty());
    assert_eq!(app.view.design, "Ivory");
    assert!(!shown(&ui, "song-design-2"), "the pick closes the menu");
    // Hidden notes need no Design.
    click(&mut ui, &mut app, "song-notes");
    assert!(!shown(&ui, "song-design"));
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
