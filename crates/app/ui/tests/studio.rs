//! Recording, practice and exporting driven by the pointer, headless: the
//! keyboard bar's Record and Practice and the header's Export ask for what
//! they name, and the practice marks over the keys leave them playable.

use kontra_ui::app::{self, Action, App, Song};
use kontra_ui::export::{self, Exporting};
use kontra_ui::practice::{Hands, Practice};
use kontra_ui::record::{self, Recording, Take};
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

/// A click on the middle of `id`, or at `at` (0..1 of its size) in it.
fn click_at(ui: &mut Ui, app: &mut App, id: &str, at: (f64, f64)) -> Vec<Action> {
    let Some(s) = ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame) else {
        return Vec::new();
    };
    let pos = Point::new(s.x + s.size.width * at.0, s.y + s.size.height * at.1);
    let mut out = Vec::new();
    for buttons in [Buttons::PRIMARY, Buttons::default()] {
        let pointer = PointerInput {
            pos: Some(pos),
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

fn click(ui: &mut Ui, app: &mut App, id: &str) -> Vec<Action> {
    click_at(ui, app, id, (0.5, 0.5))
}

fn editor() -> App {
    let notes = Notes::new(vec![Note {
        start: 1.0,
        end: 2.0,
        key: 60,
        velocity: 100,
        voice: 0,
    }]);
    App {
        song: Some(Song {
            name: "a.mid".into(),
            position: 1.0,
            length: 2.0,
            notes,
            ..Default::default()
        }),
        export: Exporting {
            song: Some("a.mid".into()),
            ..Exporting::default()
        },
        record: Recording::default(),
        ..App::default()
    }
}

#[test]
fn record_export_and_practice_ask_for_what_they_name() {
    let mut ui = kontra_ui::ui();
    let mut app = editor();
    settle(&mut ui, &mut app);
    assert_eq!(
        click(&mut ui, &mut app, "record"),
        [Action::Record(record::Do::Start)]
    );
    // Its menu: the count-in.
    assert!(click(&mut ui, &mut app, "record-menu").is_empty());
    assert_eq!(
        click(&mut ui, &mut app, "record-menu-0"),
        [Action::Record(record::Do::CountIn(true))]
    );
    // Recording: the same place stops it.
    app.record.take = Take::Recording;
    settle(&mut ui, &mut app);
    assert_eq!(
        click(&mut ui, &mut app, "record"),
        [Action::Record(record::Do::Stop)]
    );
    // Export: audio, then 4K at 60.
    assert!(click(&mut ui, &mut app, "export").is_empty());
    assert_eq!(
        click(&mut ui, &mut app, "export-0"),
        [Action::Export(export::Do::Audio)]
    );
    click(&mut ui, &mut app, "export");
    assert_eq!(
        click(&mut ui, &mut app, "export-4"),
        [Action::Export(export::Do::Video {
            height: 2160,
            fps: 60
        })]
    );
    // Under way: Cancel.
    app.export.job = Some(("Exporting audio".into(), 0.5));
    settle(&mut ui, &mut app);
    assert_eq!(
        click(&mut ui, &mut app, "export-cancel"),
        [Action::Export(export::Do::Cancel)]
    );
    // Practice: the left hand.
    click(&mut ui, &mut app, "practice");
    assert_eq!(
        click(&mut ui, &mut app, "practice-2"),
        [Action::Practice(Hands::Left)]
    );
    assert_eq!(app.practice.hands, Hands::Left);
}

/// The marks lie over the keys, and the keys still play under them.
#[test]
fn the_practice_marks_leave_the_keys_playable() {
    let mut ui = kontra_ui::ui();
    let mut app = editor();
    app.practice = Practice {
        hands: Hands::Right,
        waiting: true,
        want: vec![60],
        struck: Vec::new(),
        wrong: vec![62],
    };
    settle(&mut ui, &mut app);
    let named = |ui: &Ui, id: &str| ui.scene().is_some_and(|s| s.surface(id).is_some());
    // The lane's marks have an id to find them by; the keys' have none,
    // or they would take the pointer from the keys.
    assert!(named(&ui, "practice-lane"));
    // Pressing the keybed plays a key, marks or not.
    let pressed = click_at(&mut ui, &mut app, "keybed", (0.5, 0.9));
    assert!(
        pressed.iter().any(|a| matches!(a, Action::Note(_, true))),
        "{pressed:?}"
    );
    // Not practising: no marks.
    app.practice.hands = Hands::Off;
    settle(&mut ui, &mut app);
    assert!(!named(&ui, "practice-lane"));
}
