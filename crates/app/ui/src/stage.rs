//! The Stage, and a playing song's transport.
//!
//! The Stage gives the window over to the song: its notes falling onto the
//! keyboard and nothing else (browser, rack, mixer and header hidden), the
//! keyboard strip's waterfall and keybed drawn at the window's size. A
//! slim transport lies over its foot and hides once the pointer has rested
//! [`HIDE`] seconds; a title card shows as the song starts. The editor draws
//! only on events, so these fade on springs and on [`Ui::repaint_in`]
//! wake-ups that stop when the fade is done: nothing loops.
//!
//! The transport: play and pause, a seek bar (a click or a drag puts the
//! song there; with Shift held, a drag marks a loop), the clock, speed,
//! transpose, the loop's A and B, muted hands or tracks, the metronome and
//! the count-in. The header carries the same, compact, its settings under
//! a menu. Space plays and pauses, F opens and closes the Stage, Escape
//! closes it ([`shortcut`]); the host routes those keys here.

use mui::prelude::*;

use crate::app::{Action, App, Song};
use crate::keyboard::{self, Keys};
use crate::kit::{self, Icon, button, icon_button, latch, popup, section, vrule};
use crate::theme::{CONTROL, INSET, SPACE, TEXT, TIGHT, hairline, lift, secondary};
use crate::waterfall;

/// Seconds the pointer rests before the Stage's transport hides.
pub const HIDE: f64 = 2.;

/// Seconds the title card shows, from when a take starts to when it
/// begins to fade; its fades are springs.
const CARD: f64 = 3.2;

/// A frame a little after this one: a wake-up shorter than a frame is
/// already past when the next frame starts.
const SOON: f64 = 0.05;

/// The shortest loop a Shift-drag on the seek bar marks, in seconds.
const SHORTEST: f64 = 0.5;

/// The window width from which the header names the song it plays.
const WIDE: f64 = 1360.;

/// A loaded song's transport, as the host fills it.
#[derive(Clone, Debug, PartialEq)]
pub struct Transport {
    pub paused: bool,
    /// It played to its end and waits there: Play plays it again.
    pub ended: bool,
    /// Which start of a song this is: each new one shows the title card.
    pub take: u32,
    /// 0.25 to 2: its time scaled, its pitch not.
    pub speed: f64,
    /// Semitones, -12 to 12.
    pub transpose: i8,
    /// The loop's marks, in seconds; unset, the song's start or end.
    pub a: Option<f64>,
    pub b: Option<f64>,
    pub looping: bool,
    /// The hands muted: a two-voice song's voices (the right hand first),
    /// or a one-voice song's keys below and from middle C.
    pub left: bool,
    pub right: bool,
    /// The song's voices (a track and channel each), and those muted, a
    /// bit each: more than two are muted by track instead of by hand.
    pub voices: Vec<String>,
    pub muted: u64,
    pub metronome: bool,
    /// The click's level, 0 to 1.
    pub volume: f64,
    /// Bars of clicks before the first note: 0, 1 or 2.
    pub count_in: u8,
    /// The title card's words.
    pub title: String,
    pub composer: Option<String>,
    /// Whether the window fills the screen; None where it cannot.
    pub full_screen: Option<bool>,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            paused: false,
            ended: false,
            take: 0,
            speed: 1.,
            transpose: 0,
            a: None,
            b: None,
            looping: false,
            left: false,
            right: false,
            voices: Vec::new(),
            muted: 0,
            metronome: false,
            volume: 0.6,
            count_in: 0,
            title: String::new(),
            composer: None,
            full_screen: None,
        }
    }
}

/// What the transport asks of the host.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Play on; from its end, play it again from its start.
    Play,
    Pause,
    /// Play on from this many seconds in.
    Seek(f64),
    Speed(f64),
    Transpose(i8),
    /// The loop's marks, in seconds.
    Marks(Option<f64>, Option<f64>),
    Loop(bool),
    /// Mute (true) or bring back a hand.
    Hand(Hand, bool),
    /// Mute (true) or bring back voice `n`.
    Voice(usize, bool),
    Metronome(bool),
    /// The click's level, 0 to 1.
    Volume(f64),
    /// Bars of count-in.
    CountIn(u8),
    /// Fill the screen with the window, or give the screen back.
    FullScreen(bool),
}

/// A hand at the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hand {
    Left,
    Right,
}

/// The Stage's own state, kept in the editor's view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stage {
    pub open: bool,
    /// When the pointer last moved, in the `Ui`'s seconds (none since the
    /// Stage opened: it shows its transport at once), and where it was.
    pub moved: Option<f64>,
    pub pointer: Option<(f64, f64)>,
    /// The take the title card last showed for, and when.
    pub card: Option<(u32, f64)>,
    /// A drag on a seek bar under way.
    pub scrub: Option<Scrub>,
    /// The Stage filled the screen: closing it gives the screen back.
    pub full: bool,
}

/// A drag on a seek bar: where it began and is, in seconds, and whether it
/// marks a loop (Shift held as it began) or moves the song.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrub {
    pub from: f64,
    pub to: f64,
    pub marking: bool,
}

/// The keys the transport and the Stage answer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shortcut {
    /// Space.
    PlayPause,
    /// F.
    Stage,
    /// Escape.
    Leave,
}

/// Carries out `key`: whether it meant anything now. Space needs a song,
/// F a song or the Stage open, Escape the Stage open.
pub fn shortcut(app: &mut App, key: Shortcut, out: &mut Vec<Action>) -> bool {
    match key {
        Shortcut::PlayPause => {
            let Some(t) = app.song.as_ref().map(|s| &s.transport) else {
                return false;
            };
            let play = t.paused || t.ended;
            out.push(Action::Song(if play {
                Command::Play
            } else {
                Command::Pause
            }));
            true
        }
        Shortcut::Stage if app.view.stage.open => {
            leave(app, out);
            true
        }
        Shortcut::Stage if app.song.is_some() => {
            open(app);
            true
        }
        Shortcut::Leave if app.view.stage.open => {
            leave(app, out);
            true
        }
        Shortcut::Stage | Shortcut::Leave => false,
    }
}

fn open(app: &mut App) {
    app.view.stage.open = true;
    app.view.stage.moved = None;
    app.view.menu = None;
}

fn leave(app: &mut App, out: &mut Vec<Action>) {
    app.view.stage.open = false;
    app.view.stage.scrub = None;
    if std::mem::take(&mut app.view.stage.full) {
        out.push(Action::Song(Command::FullScreen(false)));
    }
}

/// `seconds` as minutes and seconds: 2:05; a count-in reads 0:00.
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The title card's opacity to head for `since` seconds after its take
/// started, and how long until that changes, if it will: 0 on the take's
/// first frame (its fade in starts from nothing), 1 while it shows, 0 after.
fn card(since: f64) -> (f64, Option<f64>) {
    if since <= 0. {
        (0., Some(SOON))
    } else if since < CARD {
        (1., Some(CARD - since))
    } else {
        (0., None)
    }
}

/// The window given over to the playing song: its notes falling onto the
/// keyboard, with the transport and the title card over them.
pub fn view(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let Some(song) = app.song.clone() else {
        app.view.stage.open = false;
        return spacer();
    };
    let t = &song.transport;
    let size = ui.scene().map_or(Size::new(1180., 780.), |s| s.layout.size);
    let now = ui.time();
    let pointer = ui.pointer().pos.map(|p| (p.x, p.y));
    let over = |id: &str| {
        let frame = ui.scene().and_then(|s| s.surface(id)).map(|s| s.frame);
        frame.zip(pointer).is_some_and(|(f, (x, y))| {
            x >= f.x && x <= f.x + f.size.width && y >= f.y && y <= f.y + f.size.height
        })
    };
    let on_bar = over("stage-bar");
    let stage = &mut app.view.stage;
    if stage.moved.is_none() || pointer != stage.pointer {
        stage.pointer = pointer;
        stage.moved = Some(now);
    }
    // The transport shows while the pointer moves, rests on it, drags a
    // seek bar, or the song waits; else it hides, a timed wake-up away.
    let rested = now - stage.moved.unwrap_or(now);
    let held = t.paused || t.ended || stage.scrub.is_some() || on_bar;
    if !held && rested < HIDE {
        ui.repaint_in(HIDE - rested + 0.02);
    }
    let shown = held || rested < HIDE;
    let fade = ui.tween("stage-bar-fade", if shown { 1. } else { 0. });
    // A new take: the title card, its fades springs, a wake-up as it
    // starts and one as it ends.
    if stage.card.map(|c| c.0) != Some(t.take) {
        stage.card = Some((t.take, now));
    }
    let (target, change) = card(now - stage.card.map_or(now, |c| c.1));
    if let Some(after) = change {
        ui.repaint_in(after + 0.01);
    }
    let lit = ui.tween_with("stage-card-fade", target, Spring::new(0.45, 1.));

    // The keyboard at the window's size: a sixth of it for the keys.
    let bed = (size.height * 0.17).clamp(TEXT * 6., TEXT * 14.);
    let range =
        (song.notes.keys()).map_or((24, 107), |keys| waterfall::fit(keys, waterfall::OCTAVES));
    // Each part's keys tinted, as the keyboard strip draws them.
    let spans = (app.parts.iter().enumerate())
        .map(|(n, p)| (p.keys.0, p.keys.1, crate::theme::part_color(n)))
        .collect();
    let keys = Keys {
        range,
        lit: song.notes.lit(song.position),
        held: app.view.playing.into_iter().collect(),
        spans,
        ..Keys::default()
    };
    let keybed = keyboard::keyboard(ui, "stage-keybed", &keys, &mut app.view.playing, bed);
    out.extend(
        keybed
            .changed
            .into_iter()
            .map(|(n, on)| Action::Note(n, on)),
    );
    let lane = waterfall::waterfall(
        &song.notes,
        song.position,
        range,
        (size.height - bed).max(0.),
    )
    .id("stage-falling");
    let mut layers = vec![
        col![lane, keybed.el.grow(0).w(Len::Pct(100.))]
            .w(Len::Pct(100.))
            .h(Len::Pct(100.))
            .fill(Role::Field),
    ];
    if lit > 0.01 {
        layers.push(
            // A third of the way down the falling notes.
            col![
                spacer().grow(1),
                title_card(t).opacity(lit as f32),
                spacer().grow(2),
                spacer().h(bed).grow(0)
            ]
            .align(Align::Center)
            .w(Len::Pct(100.))
            .h(Len::Pct(100.)),
        );
    }
    if fade > 0.01 {
        let bar = bar(ui, app, &song, out);
        layers.push(
            col![spacer(), bar.opacity(fade as f32)]
                .w(Len::Pct(100.))
                .h(Len::Pct(100.)),
        );
    }
    stack(layers)
        .w(Len::Pct(100.))
        .h(Len::Pct(100.))
        .fill(Role::Field)
        .tracks_pointer()
        .named("Stage")
        .id("stage")
}

/// The song's name and composer, large, as it starts.
fn title_card(t: &Transport) -> El {
    let mut lines = vec![
        body(t.title.clone())
            .text_size(TEXT * 2.4)
            .text_weight(Weight::SEMIBOLD)
            .fill(Role::Ink)
            .lines(2)
            .max_size((TEXT * 50., TEXT * 8.)),
    ];
    if let Some(composer) = &t.composer {
        lines.push(
            body(composer.clone())
                .text_size(TEXT * 1.4)
                .fill(secondary())
                .lines(1),
        );
    }
    col(lines)
        .align(Align::Center)
        .gap(TIGHT * 2.)
        .pad((INSET * 2., INSET * 1.5))
        .fill(lift(1))
        .stroke(hairline())
        .stroke_width(1)
        .id("stage-card")
}

/// The Stage's transport: the seek bar across, the controls under it.
fn bar(ui: &mut Ui, app: &mut App, song: &Song, out: &mut Vec<Action>) -> El {
    let t = &song.transport;
    let labels = if app.width >= LABELLED {
        Labels::Inline
    } else {
        Labels::Hidden
    };
    let seek = seek(
        ui,
        "stage-seek",
        song,
        &mut app.view.stage.scrub,
        out,
        (0., CONTROL * 0.6),
    );
    let shown = app
        .view
        .stage
        .scrub
        .filter(|s| !s.marking)
        .map_or(song.position, |s| s.to);
    let line = row![
        caption(clock(shown)).reserve("00:00").fill(secondary()),
        seek,
        caption(clock(song.length))
            .reserve("00:00")
            .fill(secondary()),
    ]
    .gap(SPACE)
    .align(Align::Center)
    .w(Len::Pct(100.));
    let play = play_button(ui, "stage-play", t, out);
    let mut controls = vec![play, vrule().h(CONTROL * 0.6)];
    for (n, group) in settings(ui, "stage", song, labels, out)
        .into_iter()
        .enumerate()
    {
        if n > 0 {
            controls.push(vrule().h(CONTROL * 0.6));
        }
        controls.push(group);
    }
    controls.push(spacer());
    if let Some(full) = t.full_screen {
        let (icon, name) = if full {
            (Icon::Contract, "Give the screen back")
        } else {
            (Icon::Expand, "Fill the screen")
        };
        let r = icon_button(ui, "stage-full", icon, name, false);
        if r.changed {
            app.view.stage.full = !full;
            out.push(Action::Song(Command::FullScreen(!full)));
        }
        controls.push(r.el);
    }
    let close = icon_button(
        ui,
        "stage-close",
        Icon::Close,
        "Leave the Stage (F or Esc)",
        false,
    );
    if close.changed {
        leave(app, out);
    }
    controls.push(close.el);
    col![
        line,
        row(controls)
            .gap(SPACE)
            .align(Align::Center)
            .w(Len::Pct(100.))
    ]
    .gap(TIGHT)
    .pad((SPACE * 1.5, TIGHT * 1.5))
    .w(Len::Pct(100.))
    .fill(Role::Surface)
    .stroke(hairline())
    .stroke_width(1)
    .named("Transport")
    .id("stage-bar")
}

/// Play, or pause; at its end, play again.
fn play_button(ui: &mut Ui, id: &str, t: &Transport, out: &mut Vec<Action>) -> El {
    let play = t.paused || t.ended;
    let (icon, name) = match (play, t.ended) {
        (true, true) => (Icon::Play, "Play again (Space)"),
        (true, false) => (Icon::Play, "Play (Space)"),
        _ => (Icon::Pause, "Pause (Space)"),
    };
    let r = icon_button(ui, id, icon, name, false);
    if r.changed {
        out.push(Action::Song(if play {
            Command::Play
        } else {
            Command::Pause
        }));
    }
    r.el
}

/// The song's timeline `w` by `h` (0 wide grows): a click or drag puts the
/// song there; a drag begun with Shift marks a loop. It shows where the
/// song is, how far it has played and its loop.
fn seek(
    ui: &mut Ui,
    id: &str,
    song: &Song,
    scrub: &mut Option<Scrub>,
    out: &mut Vec<Action>,
    (w, h): (f64, f64),
) -> El {
    let r = ui.get(id);
    let length = song.length.max(1e-3);
    let width = ui
        .scene()
        .and_then(|s| s.surface(id))
        .map(|s| s.frame.size.width);
    let at = (ui.local(id).zip(width)).map(|(p, w)| (p.x / w.max(1.)).clamp(0., 1.) * length);
    if r.pressed
        && let Some(at) = at
    {
        let marking = r.press_mods.shift;
        *scrub = Some(Scrub {
            from: at,
            to: at,
            marking,
        });
        if !marking {
            out.push(Action::Song(Command::Seek(at)));
        }
    } else if r.held
        && let (Some(s), Some(at)) = (scrub.as_mut(), at)
        && (s.to - at).abs() > 1e-3
    {
        s.to = at;
        if !s.marking {
            out.push(Action::Song(Command::Seek(at)));
        }
    }
    if r.released
        && let Some(s) = scrub.take()
        && s.marking
    {
        let (a, b) = (s.from.min(s.to), s.from.max(s.to));
        if b - a >= SHORTEST {
            out.push(Action::Song(Command::Marks(Some(a), Some(b))));
            out.push(Action::Song(Command::Loop(true)));
        }
    }
    let t = &song.transport;
    let marking = scrub.filter(|s| s.marking);
    let (position, marks) = match (*scrub, marking) {
        (_, Some(s)) => (song.position, Some((s.from.min(s.to), s.from.max(s.to)))),
        (Some(s), None) => (s.to, mark_span(t, length)),
        (None, None) => (song.position, mark_span(t, length)),
    };
    let (played, looping) = ((position / length).clamp(0., 1.), t.looping);
    let marks = marks.map(|(a, b)| (a / length, b / length));
    let hover = ui.state(id).hover as f32;
    let face = canvas(move |s| {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
            Path::polyline(
                [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| Point::new(x, y)),
                true,
            )
        };
        let (w, mid) = (s.width, s.height / 2.);
        let x = |t: f64| t * w;
        let half = 1.5 + f64::from(hover);
        let mut draw = vec![Draw::fill(rect(0., mid - half, w, mid + half), lift(3))];
        if let Some((a, b)) = marks {
            let band = if looping { 0.22 } else { 0.1 };
            draw.push(Draw::fill(
                rect(x(a), mid - 5., x(b), mid + 5.),
                Role::Ink.alpha(band),
            ));
            for edge in [a, b] {
                draw.push(Draw::fill(
                    rect(x(edge) - 0.5, mid - 6., x(edge) + 0.5, mid + 6.),
                    Role::Ink.alpha(0.7),
                ));
            }
        }
        draw.push(Draw::fill(
            rect(0., mid - half, x(played), mid + half),
            Fill::from(Color::oklch(0.8 + 0.08 * hover, 0., 0.)),
        ));
        let head = x(played).clamp(2., (w - 2.).max(2.));
        draw.push(Draw::fill(
            rect(head - 2., mid - 5., head + 2., mid + 5.),
            Fill::from(Color::oklch(0.92, 0., 0.)),
        ));
        draw
    });
    // 0 wide: it takes the room its row has.
    let face = if w > 0. {
        face.w(w).shrink(0)
    } else {
        face.w(0.).grow(1).min_w(TEXT * 4.)
    };
    face.h(h)
        .cursor(Cursor::Hand)
        .focusable()
        .named("Song position")
        .tip("Click or drag to move through the song · Shift-drag to loop a passage")
        .id(id.to_owned())
}

/// The loop's span as its marks set it, in seconds, while any is set.
fn mark_span(t: &Transport, length: f64) -> Option<(f64, f64)> {
    if t.a.is_none() && t.b.is_none() {
        return None;
    }
    Some((t.a.unwrap_or(0.), t.b.unwrap_or(length)))
}

/// How a group of settings is labelled.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Labels {
    /// Unlabelled, where a row has no room: each control's tooltip names it.
    Hidden,
    /// Before its controls.
    Inline,
    /// Before its controls, this wide, so groups in a column line up.
    Column(f64),
}

/// The window width from which the Stage's transport labels its groups.
const LABELLED: f64 = 1100.;

/// The transport's settings, a group each: speed, transpose, loop, mutes,
/// metronome, count-in. Ids start with `place`.
fn settings(
    ui: &mut Ui,
    place: &str,
    song: &Song,
    labels: Labels,
    out: &mut Vec<Action>,
) -> Vec<El> {
    let t = &song.transport;
    let id = |what: &str| format!("{place}-{what}");
    let mut send = |c: Command| out.push(Action::Song(c));
    let group = |label: &str, items: Vec<El>| group(label, labels, items);

    // Speed, in steps of 5%; a double click puts back 100%.
    let mut percent = t.speed * 100.;
    let speed = kit::slider(
        ui,
        id("speed"),
        "Speed",
        &mut percent,
        25.0..=200.0,
        100.,
        (TEXT * 6., CONTROL),
    );
    if speed.changed {
        send(Command::Speed((percent / 5.).round() * 5. / 100.));
    }
    let speed = group(
        "Speed",
        vec![
            speed.el,
            caption(format!("{:.0}%", t.speed * 100.))
                .reserve("200%")
                .id(id("speed-shown")),
        ],
    );

    let down = icon_button(
        ui,
        id("transpose-down"),
        Icon::Left,
        "Transpose down a semitone",
        false,
    );
    let up = icon_button(
        ui,
        id("transpose-up"),
        Icon::Right,
        "Transpose up a semitone",
        false,
    );
    if down.changed && t.transpose > -12 {
        send(Command::Transpose(t.transpose - 1));
    }
    if up.changed && t.transpose < 12 {
        send(Command::Transpose(t.transpose + 1));
    }
    let shown = match t.transpose {
        0 => "0".to_owned(),
        n => format!("{n:+}"),
    };
    let transpose = group(
        "Transpose",
        vec![
            down.el,
            caption(shown).reserve("+12").id(id("transpose-shown")),
            up.el,
        ],
    );

    // A and B mark the loop at the playhead; B also starts it looping.
    let here = song.position.clamp(0., song.length);
    let a = latch(ui, id("loop-a"), "A", "Loop from here", t.a.is_some());
    let b = latch(ui, id("loop-b"), "B", "Loop to here", t.b.is_some());
    let on = latch(
        ui,
        id("loop"),
        "Loop",
        "Loop the passage from A to B",
        t.looping,
    );
    if a.changed {
        let b = t.b.filter(|b| *b > here + SHORTEST);
        send(Command::Marks(Some(here), b));
    }
    if b.changed {
        let a = t.a.filter(|a| *a < here - SHORTEST);
        send(Command::Marks(a, Some(here)));
        send(Command::Loop(true));
    }
    if on.changed {
        send(Command::Loop(!t.looping));
    }
    let mut looping = vec![a.el, b.el, on.el];
    if t.a.is_some() || t.b.is_some() {
        let clear = icon_button(ui, id("loop-clear"), Icon::Close, "Clear the loop", false);
        if clear.changed {
            send(Command::Marks(None, None));
            send(Command::Loop(false));
        }
        looping.push(clear.el);
    }
    let looping = group("Loop", looping);

    // Two voices or one: the hands; more: each track.
    let mutes = if t.voices.len() > 2 {
        let tracks: Vec<El> = (t.voices.iter().enumerate().take(12))
            .map(|(n, name)| {
                let muted = t.muted >> n & 1 == 1;
                let tip = format!("Mute {name}");
                let r = latch(
                    ui,
                    id(&format!("track-{n}")),
                    &(n + 1).to_string(),
                    &tip,
                    muted,
                );
                if r.changed {
                    send(Command::Voice(n, !muted));
                }
                r.el
            })
            .collect();
        group("Mute", tracks)
    } else {
        let by = if t.voices.len() == 2 {
            "its own track"
        } else {
            "the keys below middle C"
        };
        let left_tip = format!("Mute the left hand ({by})");
        let right_tip = if t.voices.len() == 2 {
            "Mute the right hand (its own track)".to_owned()
        } else {
            "Mute the right hand (middle C and up)".to_owned()
        };
        let left = latch(ui, id("left"), "L", &left_tip, t.left);
        let right = latch(ui, id("right"), "R", &right_tip, t.right);
        if left.changed {
            send(Command::Hand(Hand::Left, !t.left));
        }
        if right.changed {
            send(Command::Hand(Hand::Right, !t.right));
        }
        group("Mute", vec![left.el, right.el])
    };

    let click = latch(
        ui,
        id("metronome"),
        "Click",
        "Metronome, on the file's beats",
        t.metronome,
    );
    if click.changed {
        send(Command::Metronome(!t.metronome));
    }
    let mut metronome = vec![click.el];
    if t.metronome {
        let mut volume = t.volume;
        let level = kit::slider(
            ui,
            id("volume"),
            "Metronome level",
            &mut volume,
            0.0..=1.0,
            0.6,
            (TEXT * 4., CONTROL),
        );
        if level.changed {
            send(Command::Volume(volume));
        }
        metronome.push(level.el);
    }
    let metronome = group("Metronome", metronome);

    let counts: Vec<El> = (1..=2u8)
        .map(|bars| {
            let tip = format!(
                "Count in {bars} bar{} before the first note",
                if bars > 1 { "s" } else { "" }
            );
            let r = latch(
                ui,
                id(&format!("count-{bars}")),
                &bars.to_string(),
                &tip,
                t.count_in == bars,
            );
            if r.changed {
                send(Command::CountIn(if t.count_in == bars { 0 } else { bars }));
            }
            r.el
        })
        .collect();
    let count = group("Count-in", counts);
    vec![speed, transpose, looping, mutes, metronome, count]
}

/// `label` as `labels` says, then `items`, in a row.
fn group(label: &str, labels: Labels, items: Vec<El>) -> El {
    let mut all = match labels {
        Labels::Hidden => Vec::new(),
        Labels::Inline => vec![section(label)],
        Labels::Column(w) => vec![section(label).w(w).shrink(0)],
    };
    all.extend(items);
    row(all).gap(TIGHT).align(Align::Center).shrink(0)
}

/// The header's transport while a song is loaded: play or pause, its name,
/// a seek bar and the clock, the Stage, the settings under a menu, Stop.
pub fn compact(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let Some(song) = app.song.clone() else {
        return spacer().w(0);
    };
    let t = &song.transport;
    let play = play_button(ui, "song-pause", t, out);
    let seek = seek(
        ui,
        "song-seek",
        &song,
        &mut app.view.stage.scrub,
        out,
        (TEXT * 6., CONTROL),
    );
    let shown = app
        .view
        .stage
        .scrub
        .filter(|s| !s.marking)
        .map_or(song.position, |s| s.to);
    let stage = icon_button(
        ui,
        "song-stage",
        Icon::Expand,
        "Stage: only the falling notes and the keyboard (F)",
        false,
    );
    if stage.changed {
        open(app);
    }
    let open_menu = app.view.menu.as_deref() == Some("song-more");
    let more = icon_button(
        ui,
        "song-more",
        Icon::More,
        "Speed, loop, metronome…",
        open_menu,
    );
    let stop = button(ui, "song-stop", "Stop", true);
    if stop.changed {
        out.push(Action::StopSong);
    }
    let mut items = vec![play];
    // Its name where the header has room for it; its tooltip anywhere.
    if app.width >= WIDE {
        items.push(caption(song.name.clone()).lines(1).min_w(0).w(TEXT * 8.));
    }
    items.extend([
        seek.tip(song.name.clone()),
        caption(format!("{} / {}", clock(shown), clock(song.length))).reserve("00:00 / 00:00"),
        stage.el,
        more.el,
        stop.el.tip("Stop the MIDI file"),
    ]);
    if kit::toggle(ui, &mut app.view.menu, "song-more", more.changed) {
        let rows: Vec<El> = settings(ui, "song", &song, Labels::Column(TEXT * 6.5), out)
            .into_iter()
            .map(|g| g.h(CONTROL))
            .collect();
        // A float takes the room around its parent: it says its own height.
        let tall = rows.len() as f64 * (CONTROL + TIGHT) + TIGHT * 2.;
        items.push(popup(
            "song-more",
            col(rows).gap(TIGHT).pad(TIGHT).min_h(tall).w(TEXT * 25.),
        ));
    }
    row(items)
        .gap(TIGHT)
        .align(Align::Center)
        .shrink(0)
        .id("song")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_card_fades_in_holds_and_fades_out() {
        assert_eq!(card(0.), (0., Some(SOON)), "in from nothing");
        assert_eq!(card(1.2), (1., Some(2.)));
        assert_eq!(card(CARD + 0.1), (0., None), "out, and no more wake-ups");
        assert_eq!(clock(-2.), "0:00");
        assert_eq!(clock(125.4), "2:05");
    }
}
