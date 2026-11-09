//! The keyboard strip along the editor's foot, where the playing happens:
//! its bar, the notes over the keys (a song's falling onto them, or with
//! none what the player plays rising from them), the keys and the wheels.
//!
//! The bar holds everything about playing, in one place. With no song it
//! names the keys shown; with a song loaded it is the song's transport
//! (play, stop, its name and chord, the seek bar, practice, its settings
//! under a menu, the Stage), in the same order as the Stage's. Record and
//! how the notes show follow either way. Folded, the bar stays, so the
//! transport does and the keyboard comes back from the same place.

use mui::prelude::*;

use super::{Action, App, View, songs};
use crate::design;
use crate::keyboard::{self, Keys, note_name};
use crate::kit::{self, Icon, icon_button, popup, section, strip, vrule};
use crate::stage::{self, Labels};
use crate::theme::{CONTROL, SPACE, TEXT, TIGHT, quick, secondary};
use crate::waterfall;

/// A piano's 88 keys, A-1 to C7 (middle C as C3).
const PIANO: (u8, u8) = (21, 108);

/// What the strip shows this frame.
struct Lane {
    /// A loaded song's notes fall onto the keys (it has notes).
    song: bool,
    /// With no song, what the player plays rises from the keys, once they
    /// have played.
    live: bool,
    /// Those notes show: not hidden, and there are some.
    shows: bool,
    /// The keys the keybed shows.
    range: (u8, u8),
}

fn lane(app: &App) -> Lane {
    let v = &app.view;
    let song = app.song.as_ref().filter(|s| !s.notes.is_empty());
    let live = song.is_none() && app.played.begun;
    let shows = !v.notes_hidden && (song.is_some() || live);
    // While notes show, the keybed shows the keys they play, centred in
    // whole octaves; its own octaves wait until they stop.
    let low = 12 * v.octave.clamp(0, 5) as u8;
    // What the player plays rises over a piano's keys, and any past them.
    let piano = |(a, b): (u8, u8)| (a.min(PIANO.0), b.max(PIANO.1));
    let range = match song.filter(|_| shows).and_then(|s| s.notes.keys()) {
        Some(keys) => waterfall::fit(keys, waterfall::OCTAVES),
        None if shows => {
            let keys = app.played.notes.keys().map_or(PIANO, piano);
            waterfall::fit(keys, waterfall::OCTAVES)
        }
        None => (low, low + 83),
    };
    Lane {
        song: song.is_some(),
        live,
        shows,
        range,
    }
}

/// The strip: its bar, then (unless folded) the notes, the keys and the
/// wheels.
pub(super) fn view(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let lane = lane(app);
    let bar = bar(ui, app, &lane, out);
    if app.view.keyboard_hidden {
        return col![bar].animate_layout_with(quick());
    }
    let range = lane.range;
    let falls = lane.song && lane.shows;
    // As tall as a third of the window, within reason, easing in and out.
    let window = ui.scene().map_or(0., |s| s.layout.size.height);
    let tall = if lane.shows {
        (window * 0.3).clamp(TEXT * 8., TEXT * 22.)
    } else {
        0.
    };
    let tall = ui.tween("song-notes-height", tall);
    let v = &mut app.view;
    // The focused part: the first one open, else the first.
    let focused = app.parts.iter().find(|p| p.open).or(app.parts.first());
    let script: Vec<_> = focused
        .and_then(|p| p.face.as_ref())
        .map(|f| (f.tree.keybed.iter()).map(|&(n, c)| (n, crate::view::color(c))))
        .into_iter()
        .flatten()
        .collect();
    let look = design::named(&v.design);
    let song = app.song.as_ref().filter(|s| !s.notes.is_empty());
    let lit = song
        .map(|s| s.notes.lit_in(s.position, &look))
        .unwrap_or_default();
    let shown = Keys {
        range,
        lit: waterfall::with_played(lit, &app.played, &look, song.is_none()),
        held: match focused.and_then(|p| p.face.as_ref()?.tree.key_pressed.as_ref()) {
            Some(pressed) => pressed.clone(),
            None => v.playing.into_iter().collect(),
        },
        spans: app
            .parts
            .iter()
            .enumerate()
            .map(|(n, p)| (p.keys.0, p.keys.1, crate::theme::part_color(n)))
            .collect(),
        // A script that colours its keys shows its own keyswitches.
        switches: focused
            .filter(|_| script.is_empty())
            .map(|p| {
                p.articulations
                    .iter()
                    .enumerate()
                    .filter_map(|(a, (_, key))| Some(((*key)?, crate::theme::part_color(a))))
                    .collect()
            })
            .unwrap_or_default(),
        script,
        names: focused
            .and_then(|p| p.face.as_ref())
            .map(|f| f.tree.key_names.clone())
            .unwrap_or_default(),
    };
    let height = TEXT * 6.;
    let mut pitch = v.pitch;
    let bend = kit::slider(
        ui,
        "pitch",
        "Pitch bend",
        &mut pitch,
        -1.0..=1.0,
        0.,
        (TEXT * 1.5, height),
    );
    if !ui.get("pitch").held {
        pitch = 0.;
    }
    if pitch != v.pitch {
        v.pitch = pitch;
        out.push(Action::Pitch(pitch));
    }
    let mut modulation = v.modulation;
    let wheel = kit::slider(
        ui,
        "modulation",
        "Modulation",
        &mut modulation,
        0.0..=1.0,
        0.,
        (TEXT * 1.5, height),
    );
    if wheel.changed {
        v.modulation = modulation;
        out.push(Action::Modulation(modulation));
    }
    let bed = keyboard::keyboard(ui, "keybed", &shown, &mut v.playing, height);
    out.extend(bed.changed.into_iter().map(|(n, on)| Action::Note(n, on)));
    // Practice marks the keys to press over the notes and the keys.
    let marks = falls && app.practice.marks();
    // The notes fall in a lane as wide as the keybed, right above it, so
    // each lands on its key, the keys the player strikes lit under them;
    // with no song, what the player plays rises from the keys instead.
    // The wheels sit beside the keys alone.
    let mut keys = Vec::new();
    if tall > 0.5 {
        let lane = match song {
            Some(s) => waterfall::falling(waterfall::Show {
                notes: s.notes.clone(),
                at: s.position,
                range,
                design: look,
                played: Some(app.played.clone()),
                ..waterfall::Show::default()
            })
            .h(tall)
            .id("song-falling"),
            None => waterfall::falling(waterfall::Show::rising(&app.played, range, look))
                .h(tall)
                .id("played-rising"),
        };
        keys.push(if marks {
            let over = crate::practice::lane(&app.practice, range, tall);
            stack![lane, over].w(Len::Pct(100.))
        } else {
            lane
        });
    }
    let bed = bed.el.grow(0).w(Len::Pct(100.));
    keys.push(if marks {
        let over = crate::practice::keys(&app.practice, range, height);
        stack![bed, over].w(Len::Pct(100.))
    } else {
        bed
    });
    col![
        bar,
        row![bend.el, wheel.el, col(keys).grow(1).w(0.).min_w(TEXT * 20.)]
            .gap(TIGHT)
            .align(Align::End)
            .pad(kit::edges(0., SPACE, SPACE, SPACE))
    ]
    .animate_layout_with(quick())
}

/// The bar: the song's transport or the keys' range, then Record, how the
/// notes show, the octaves while they are the keybed's, and the fold.
fn bar(ui: &mut Ui, app: &mut App, lane: &Lane, out: &mut Vec<Action>) -> El {
    let mut items = if app.song.is_some() {
        transport(ui, app, out)
    } else {
        let (low, high) = lane.range;
        let label = caption(format!("{} – {}", note_name(low), note_name(high)))
            .lines(1)
            .tip("Every part · the keys play channel 1 or Omni · click a part to focus it")
            .id("keys-range");
        vec![section("Keyboard"), label, spacer()]
    };
    items.push(vrule().h(CONTROL * 0.6));
    // Recording what the player plays ([`crate::record`]).
    let v = &mut app.view;
    let mut record = Vec::new();
    items.push(crate::record::control(
        ui,
        &mut v.menu,
        &app.record,
        &mut record,
    ));
    out.extend(record.into_iter().map(Action::Record));
    let folded = v.keyboard_hidden;
    if !folded && (lane.song || lane.live) {
        items.push(vrule().h(CONTROL * 0.6));
        items.push(notes_picker(ui, v));
    }
    if !folded && !lane.shows {
        let down = icon_button(ui, "octave-down", Icon::Left, "Octave down", false);
        let up = icon_button(ui, "octave-up", Icon::Right, "Octave up", false);
        if down.changed {
            v.octave = (v.octave - 1).max(0);
        }
        if up.changed {
            v.octave = (v.octave + 1).min(5);
        }
        items.extend([down.el, up.el]);
    }
    // The keyboard's own switch, raised while it shows: not a chevron,
    // which would read as one more dropdown's.
    let name = if folded {
        "Show the keyboard"
    } else {
        "Hide the keyboard"
    };
    let fold = icon_button(ui, "keys-fold", Icon::Piano, name, !folded);
    if fold.changed {
        v.keyboard_hidden = !folded;
    }
    items.push(fold.el);
    strip(items).id("keys-bar")
}

/// Which [`design`] the notes over the keys are drawn in, or Off to hide
/// them: the Designs that ship, then the user's own, then Off.
fn notes_picker(ui: &mut Ui, v: &mut View) -> El {
    let all = design::all();
    let mut names: Vec<&str> = all.iter().map(|d| d.meta.name.as_str()).collect();
    let off = names.len();
    names.push("Off");
    let current = design::named(&v.design);
    let at = if v.notes_hidden {
        Some(off)
    } else {
        names.iter().position(|n| *n == current.meta.name)
    };
    let label = if v.notes_hidden {
        "Off"
    } else {
        current.meta.name.as_str()
    };
    let tip = "The notes over the keys: how they look, or Off";
    let r = kit::dropdown(
        ui,
        &mut v.menu,
        "song-notes",
        (Some(Icon::Notes), label, tip),
        &names,
        at,
    );
    match r.changed {
        Some(n) if n == off => v.notes_hidden = true,
        Some(n) => {
            v.notes_hidden = false;
            // The Design shown already keeps its name (the default's empty).
            if let Some(name) = names.get(n).filter(|n| **n != current.meta.name) {
                v.design = (*name).to_owned();
            }
        }
        None => {}
    }
    r.el
}

/// A loaded song's transport, as the bar's first items: play or pause,
/// stop, its name, the clock about the seek bar, the chord sounding,
/// practice, its settings under a menu, and the Stage.
fn transport(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> Vec<El> {
    let Some(song) = app.song.clone() else {
        return Vec::new();
    };
    let t = &song.transport;
    let play = stage::play_button(ui, "song-pause", t, out);
    let stop = icon_button(ui, "song-stop", Icon::Stop, "Stop the song", false);
    if stop.changed {
        out.push(Action::StopSong);
    }
    // Folded, no frame follows the song: wake for the clock's next second.
    if app.view.keyboard_hidden && !t.paused && !t.ended {
        ui.repaint_in(1. - song.position.fract() + 0.02);
    }
    let shown = (app.view.stage.scrub)
        .filter(|s| !s.marking)
        .map_or(song.position, |s| s.to);
    let seek = stage::seek(
        ui,
        "song-seek",
        &song,
        &mut app.view.stage.scrub,
        out,
        (0., CONTROL),
    );
    let mut about = song.name.clone();
    if let Some(composer) = &t.composer {
        about.push_str(&format!("\n{composer}"));
    }
    let title = if t.title.is_empty() {
        song.name.trim_end_matches(".mid").to_owned()
    } else {
        t.title.clone()
    };
    // Its name gives way first when the bar is short of room.
    let mut items = vec![
        play,
        stop.el,
        body(title)
            .text_weight(Weight::SEMIBOLD)
            .lines(1)
            .min_w(TEXT * 3.)
            .max_size((TEXT * 14., CONTROL))
            .shrink(1)
            .tip(about.clone())
            .id("song-title"),
        caption(stage::clock(shown))
            .reserve("00:00")
            .fill(secondary()),
        seek.tip(about),
        caption(stage::clock(song.length))
            .reserve("00:00")
            .fill(secondary()),
        songs::chord_label(&song, &app.songbook),
    ];
    // Practice ([`crate::practice`]): the song waits for the player.
    let practice = crate::practice::control(ui, &mut app.view.menu, &app.practice);
    if let Some(hands) = practice.changed {
        app.practice.hands = hands;
        out.push(Action::Practice(hands));
    }
    items.push(practice.el);
    let open_menu = app.view.menu.as_deref() == Some("song-more");
    let more = icon_button(
        ui,
        "song-more",
        Icon::More,
        "Speed, transpose, loop, metronome…",
        open_menu,
    );
    let stage_button = icon_button(
        ui,
        "song-stage",
        Icon::Expand,
        "Stage: the notes and the keys over the whole window (F)",
        false,
    );
    if stage_button.changed {
        stage::open(app);
    }
    items.extend([more.el, stage_button.el]);
    if kit::toggle(ui, &mut app.view.menu, "song-more", more.changed) {
        let mut rows: Vec<El> = stage::settings(ui, "song", &song, Labels::Column(TEXT * 6.5), out)
            .into_iter()
            .map(|g| g.h(CONTROL))
            .collect();
        let mut tall = rows.len() as f64 * (CONTROL + TIGHT) + TIGHT * 2.;
        // What the Songs tab's touch-ups did to it, where it says so.
        if let Some(done) = songs::touched_label(&song, &app.songbook) {
            rows.push(done.h(CONTROL));
            tall += CONTROL + TIGHT;
        }
        // A float takes the room around its parent: it says its own height.
        items.push(popup(
            "song-more",
            col(rows).gap(TIGHT).pad(TIGHT).min_h(tall).w(TEXT * 25.),
        ));
    }
    items
}
