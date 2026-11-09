//! The browser's Songs tab, beside Kontakt and Falcon / UVI: the folders
//! of MIDI files the player adds (with +, or by dropping one), their
//! favourites and what played lately, a search across them, and the
//! touch-ups a rough file gets as it is read (a pedal, a human hand, rolled
//! chords, a velocity curve), with what each did to the song playing. A
//! click on a song plays it.

use std::path::PathBuf;

use mui::prelude::*;

use super::{Action, App, Format};
use crate::kit::{self, Icon, glyph, icon_button, interactive, latch, rule, section};
use crate::theme::{CONTROL, SPACE, TEXT, TIGHT, accent, secondary};
use crate::waterfall::Notes;

/// What the Songs tab lists; the host fills it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Songbook {
    pub folders: Vec<SongFolder>,
    /// Every song in the folders, by folder and then path inside it, then
    /// those played or starred from elsewhere.
    pub songs: Vec<SongFile>,
    /// Played lately, newest first, as indices into `songs`.
    pub recent: Vec<usize>,
    /// The touch-ups asked for.
    pub touch: Touch,
    /// The song last played, as it was read.
    pub playing: Option<Played>,
    /// The folders are being read.
    pub scanning: bool,
}

/// A folder of songs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongFolder {
    pub name: String,
    pub path: PathBuf,
}

/// A MIDI file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongFile {
    pub path: PathBuf,
    /// Its file name without the extension.
    pub name: String,
    /// The folder it is in, by index into the folders.
    pub folder: Option<usize>,
    /// The folder inside that it is in, `""` at the top.
    pub dir: String,
    /// Seconds long, and the tracks that play notes.
    pub length: f64,
    pub tracks: u16,
    /// The touch-ups it would get, were each asked for.
    pub needs: Touch,
    pub starred: bool,
    /// Why it cannot be read, if it cannot.
    pub problem: Option<String>,
}

/// The touch-ups a song gets as it is read, each only where it needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Touch {
    /// A sustain pedal for a file that never presses one.
    pub pedal: bool,
    /// Chords spread by a few milliseconds and velocities varied, for a
    /// file quantised to a grid.
    pub humanise: bool,
    /// Big chords rolled from the bottom up, slightly; with `humanise`.
    pub roll: bool,
    /// A velocity curve for a file at one velocity.
    pub velocity: bool,
}

impl Default for Touch {
    fn default() -> Self {
        Self {
            pedal: true,
            humanise: true,
            roll: false,
            velocity: false,
        }
    }
}

/// The song last played, as it was read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Played {
    pub path: PathBuf,
    /// Its file name, as the header shows it.
    pub name: String,
    /// The touch-ups it needs, and those it got.
    pub needs: Touch,
    pub applied: Touch,
    /// Its notes as they ring, pedal and all: what its chord is heard from
    /// ([`crate::chord::chord_at`]).
    pub ringing: Notes,
}

/// The Songs tab's own state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongView {
    pub shelf: SongShelf,
    pub search: String,
    /// Folders folded shut, by their heading.
    pub folded: Vec<String>,
}

/// Which songs the tab lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SongShelf {
    #[default]
    All,
    Favorites,
    Recent,
}

/// What the player asks of the Songs tab.
#[derive(Clone, Debug, PartialEq)]
pub enum SongAction {
    /// Play this MIDI file.
    Play(PathBuf),
    Star(PathBuf, bool),
    /// List a folder of songs: this one, or one the player picks.
    AddFolder(Option<PathBuf>),
    RemoveFolder(PathBuf),
    /// Read songs with these touch-ups from now on, the playing one again.
    Touch(Touch),
    /// Read the folders again.
    Rescan,
}

/// The song last read by the Songs tab, if it is `song`.
fn read<'a>(song: &super::Song, book: &'a Songbook) -> Option<&'a Played> {
    book.playing.as_ref().filter(|p| p.name == song.name)
}

/// The chord sounding in `song` now, for the keyboard bar, heard from its
/// notes as they ring when the Songs tab read it, else as its keys hold
/// them.
pub(super) fn chord_label(song: &super::Song, book: &Songbook) -> El {
    let notes = read(song, book).map_or(&song.notes, |p| &p.ringing);
    let chord = crate::chord::chord_at(notes, song.position).unwrap_or_default();
    caption(chord)
        .text_weight(Weight::SEMIBOLD)
        .lines(1)
        .reserve("C#m7b5/G#")
        .tip("The chord sounding now")
        .id("song-chord")
}

/// The touch-ups `song` got as the Songs tab read it, dim, for the song's
/// menu; none when it got none.
pub(super) fn touched_label(song: &super::Song, book: &Songbook) -> Option<El> {
    let applied = read(song, book)?.applied;
    let done: Vec<&str> = [
        (applied.pedal, "auto-pedal"),
        (applied.humanise && !applied.roll, "humanised"),
        (applied.roll, "humanised, rolled"),
        (applied.velocity, "velocity curve"),
    ]
    .into_iter()
    .filter_map(|(on, word)| on.then_some(word))
    .collect();
    if done.is_empty() {
        return None;
    }
    Some(
        row![
            section("Touch-ups").w(TEXT * 6.5).shrink(0),
            caption(done.join(" · "))
                .fill(secondary())
                .lines(1)
                .min_w(0)
                .tip(
                    "Touch-ups this song got as it was read; change them in the browser's Songs tab"
                )
                .id("song-touched"),
        ]
        .gap(TIGHT)
        .align(Align::Center),
    )
}

/// `seconds` as minutes and seconds: 3:07.
fn minutes(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn matches(song: &SongFile, search: &str) -> bool {
    let search = search.to_lowercase();
    search.split_whitespace().all(|word| {
        song.name.to_lowercase().contains(word) || song.dir.to_lowercase().contains(word)
    })
}

/// The Songs tab: the whole browser column.
pub(super) fn view(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let search = kit::field(
        ui,
        "song-search",
        "Search songs",
        &mut app.view.songs.search,
    );
    let add = icon_button(
        ui,
        "songs-add",
        Icon::Plus,
        "Add a folder of MIDI files",
        false,
    );
    if add.changed {
        out.push(Action::Songs(SongAction::AddFolder(None)));
    }
    let hide = icon_button(ui, "browser-hide", Icon::Left, "Hide the browser", false);
    if hide.changed {
        app.view.browser_hidden = true;
    }
    let count = app.songbook.songs.len();
    let tabs = super::browser::formats(
        ui,
        &mut app.view,
        |f| match f {
            Format::Songs => count,
            _ => 0,
        },
        out,
    );
    let shelves = shelves(ui, app);
    let list = list(ui, app, out);
    let touch = touch_ups(ui, app, out);
    col![
        row![search.el.grow(1).min_w(0), add.el, hide.el]
            .gap(TIGHT)
            .align(Align::Center)
            .pad(kit::edges(SPACE, SPACE, 0., SPACE))
            .shrink(0),
        tabs.shrink(0),
        rule(),
        shelves.shrink(0),
        rule(),
        list.grow(1).min_h(CONTROL * 3.),
        rule(),
        touch,
    ]
    .h(Len::Pct(100.))
    .id("songs-column")
}

fn shelves(ui: &mut Ui, app: &mut App) -> El {
    let book = &app.songbook;
    let starred = book.songs.iter().filter(|s| s.starred).count();
    let mut rows = Vec::new();
    for (shelf, icon, label, n) in [
        (SongShelf::Favorites, Icon::Star, "Favorites", starred),
        (SongShelf::Recent, Icon::Clock, "Recent", book.recent.len()),
    ] {
        let id = format!("song-shelf-{label}");
        let on = app.view.songs.shelf == shelf;
        if ui.get(id.clone()).activated() {
            app.view.songs.shelf = if on { SongShelf::All } else { shelf };
        }
        rows.push(
            interactive(
                row![
                    stack![glyph(icon, TEXT, secondary()).centered()]
                        .square(CONTROL)
                        .fill(Role::Field),
                    body(label).text_size(TEXT).grow(1),
                    caption(n.to_string()).fill(secondary()),
                ]
                .gap(SPACE)
                .align(Align::Center)
                .h(CONTROL + TIGHT)
                .pad((0., SPACE))
                .when(on, |e| e.fill(Role::Raised)),
            )
            .focusable()
            .a11y(A11y::Button)
            .named(label)
            .id(id),
        );
    }
    col(rows).pad((TIGHT, 0.))
}

/// What the song list shows now: matches of a search, a shelf, or every
/// folder in turn.
fn list(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let book = &app.songbook;
    let v = &app.view.songs;
    let playing = (app.song.as_ref())
        .and_then(|s| book.playing.as_ref().filter(|p| p.name == s.name))
        .map(|p| p.path.clone());
    let mut rows: Vec<El> = Vec::new();
    let mut star = None;
    let mut fold = None;
    let mut remove = None;
    let mut row_of = |ui: &mut Ui, n: usize, song: &SongFile, rows: &mut Vec<El>| {
        let on = playing.as_ref() == Some(&song.path);
        let (el, starred) = song_row(ui, n, song, on, out);
        if starred {
            star = Some(n);
        }
        rows.push(el);
    };
    let searching = !v.search.trim().is_empty();
    if searching || v.shelf != SongShelf::All {
        let shown: Vec<usize> = match v.shelf {
            _ if searching => (0..book.songs.len()).collect(),
            SongShelf::Favorites => (0..book.songs.len())
                .filter(|&n| book.songs.get(n).is_some_and(|s| s.starred))
                .collect(),
            SongShelf::Recent => book.recent.clone(),
            SongShelf::All => Vec::new(),
        };
        for n in shown {
            let Some(song) = book.songs.get(n) else {
                continue;
            };
            let wanted = match v.shelf {
                SongShelf::Favorites => song.starred,
                SongShelf::Recent => book.recent.contains(&n),
                SongShelf::All => true,
            };
            if wanted && (!searching || matches(song, &v.search)) {
                row_of(ui, n, song, &mut rows);
            }
        }
    } else {
        for (f, folder) in book.folders.iter().enumerate() {
            let inside: Vec<(usize, &SongFile)> = (book.songs.iter().enumerate())
                .filter(|(_, s)| s.folder == Some(f))
                .collect();
            // The folder first, counting all it holds; folded, it hides all.
            let (el, folded, removed) = folder_row(ui, f, &folder.name, true, inside.len(), v);
            fold = fold.or(folded);
            remove = remove.or(removed.then(|| folder.path.clone()));
            rows.push(el);
            if v.folded.contains(&folder.name) {
                continue;
            }
            let mut heading: Option<&str> = None;
            for &(n, song) in &inside {
                let sub = format!("{} / {}", folder.name, song.dir);
                if !song.dir.is_empty() && heading != Some(song.dir.as_str()) {
                    heading = Some(song.dir.as_str());
                    let count = inside.iter().filter(|(_, s)| s.dir == song.dir).count();
                    let (el, folded, _) = folder_row(ui, f, &sub, false, count, v);
                    fold = fold.or(folded);
                    rows.push(el);
                }
                if song.dir.is_empty() || !v.folded.contains(&sub) {
                    row_of(ui, n, song, &mut rows);
                }
            }
        }
    }
    if let Some(n) = star
        && let Some(song) = app.songbook.songs.get_mut(n)
    {
        song.starred = !song.starred;
        out.push(Action::Songs(SongAction::Star(
            song.path.clone(),
            song.starred,
        )));
    }
    if let Some(heading) = fold {
        let folded = &mut app.view.songs.folded;
        match folded.iter().position(|f| *f == heading) {
            Some(at) => {
                folded.remove(at);
            }
            None => folded.push(heading),
        }
    }
    if let Some(path) = remove {
        out.push(Action::Songs(SongAction::RemoveFolder(path)));
    }
    if rows.is_empty() {
        let book = &app.songbook;
        let why = if book.scanning {
            "Reading the folders…"
        } else if book.folders.is_empty() && app.view.songs.shelf == SongShelf::All {
            "Add a folder of MIDI files with +, or drop one here"
        } else {
            match app.view.songs.shelf {
                _ if !app.view.songs.search.trim().is_empty() => "Nothing matches",
                SongShelf::Favorites => "No favourites yet: star a song",
                SongShelf::Recent => "Nothing played yet",
                SongShelf::All => "No MIDI files in these folders",
            }
        };
        return caption(why)
            .fill(secondary())
            .pad(SPACE * 2.)
            .id("songs-empty");
    }
    col(rows)
        .gap(1)
        .pad(TIGHT)
        .scroll()
        .min_h(0)
        .id("song-list")
}

/// A folder's heading: fold, its name and count; the top one of a folder
/// can stop listing it. Whether it was folded or removed this frame.
fn folder_row(
    ui: &mut Ui,
    f: usize,
    heading: &str,
    top: bool,
    count: usize,
    v: &SongView,
) -> (El, Option<String>, bool) {
    let id = format!("song-folder-{f}-{heading}");
    let shut = v.folded.iter().any(|h| h == heading);
    let folded = ui.get(id.clone()).activated().then(|| heading.to_owned());
    let mut items = vec![
        glyph(
            if shut { Icon::Right } else { Icon::Down },
            TEXT * 0.8,
            secondary(),
        ),
        body(heading.to_owned())
            .text_size(TEXT)
            .text_weight(Weight::SEMIBOLD)
            .lines(1)
            .min_w(0)
            .grow(1),
        caption(count.to_string()).fill(secondary()),
    ];
    let mut removed = false;
    let mut el_items = Vec::new();
    if top {
        let r = icon_button(
            ui,
            format!("song-folder-remove-{f}"),
            Icon::Close,
            "Stop listing this folder",
            false,
        );
        removed = r.changed;
        el_items.push(r.el);
    }
    let head = interactive(
        row(std::mem::take(&mut items))
            .gap(TIGHT)
            .align(Align::Center)
            .h(CONTROL)
            .pad((0., SPACE)),
    )
    .focusable()
    .a11y(A11y::Button)
    .named(heading.to_owned())
    .tip(format!("{heading}\nFold or unfold"))
    .id(id)
    .grow(1)
    .min_w(0);
    let mut line = vec![head];
    line.extend(el_items);
    (
        row(line).align(Align::Center).w(Len::Pct(100.)),
        folded,
        removed,
    )
}

/// A song: its name, length and tracks, and its star; raised with an
/// accent edge while it plays. A click plays it. Whether its star was
/// clicked this frame.
fn song_row(
    ui: &mut Ui,
    n: usize,
    song: &SongFile,
    playing: bool,
    out: &mut Vec<Action>,
) -> (El, bool) {
    let id = format!("song-{n}");
    if ui.get(id.clone()).activated() && song.problem.is_none() {
        out.push(Action::Songs(SongAction::Play(song.path.clone())));
    }
    let detail = match &song.problem {
        Some(_) => "unreadable".to_owned(),
        None if song.tracks == 1 => format!("{} · 1 track", minutes(song.length)),
        None => format!("{} · {} tracks", minutes(song.length), song.tracks),
    };
    let mut needs = Vec::new();
    if song.needs.pedal {
        needs.push("no pedal");
    }
    if song.needs.humanise {
        needs.push("quantised");
    }
    if song.needs.velocity {
        needs.push("one velocity");
    }
    let mut tip = song.path.display().to_string();
    if let Some(problem) = &song.problem {
        tip.push_str(&format!("\nIt cannot play: {problem}"));
    } else if !needs.is_empty() {
        tip.push_str(&format!("\n{}: touch-ups apply", needs.join(", ")));
    }
    let mark = block(2, Len::Pct(100.))
        .fill(if playing {
            Fill::from(accent())
        } else {
            Role::Ink.alpha(0.)
        })
        .shrink(0);
    let name = body(song.name.clone())
        .text_size(TEXT)
        .fill(if song.problem.is_some() {
            secondary()
        } else {
            Fill::from(Role::Ink)
        })
        .lines(1)
        .min_w(0)
        .grow(1);
    let el = interactive(
        row![
            mark,
            name,
            caption(detail).fill(secondary()).lines(1).shrink(0)
        ]
        .gap(SPACE)
        .align(Align::Center)
        .pad(kit::edges(0., TIGHT, 0., 0.))
        .h(CONTROL)
        .when(playing, |e| e.fill(Role::Raised)),
    )
    .focusable()
    .a11y(A11y::Button)
    .named(song.name.clone())
    .tip(tip)
    .id(id)
    .grow(1)
    .min_w(0);
    let star = icon_button(
        ui,
        format!("song-star-{n}"),
        Icon::Star,
        if song.starred {
            "Unstar"
        } else {
            "Star: a favourite"
        },
        song.starred,
    );
    let line = row![el, star.el].align(Align::Center).w(Len::Pct(100.));
    (line, star.changed)
}

/// The touch-ups: a switch each, two to a row, and a line on what they
/// did to the song playing (or what each is for, in its tooltip).
fn touch_ups(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let book = &app.songbook;
    let playing =
        (app.song.as_ref()).and_then(|s| book.playing.as_ref().filter(|p| p.name == s.name));
    let mut touch = book.touch;
    let status = match playing {
        None => "Each applies as a song is read, only where it needs it".to_owned(),
        Some(p) => status(p),
    };
    type Pick = fn(&mut Touch) -> &mut bool;
    let switches: [(&str, &str, &str, Pick); 4] = [
        (
            "touch-pedal",
            "Auto-pedal",
            "For files that never press the sustain pedal: down after each new harmony, lifted at the next and in rests",
            |t| &mut t.pedal,
        ),
        (
            "touch-humanise",
            "Humanise",
            "For files quantised to a grid: each chord's notes spread over a few milliseconds after its top note, velocities varied",
            |t| &mut t.humanise,
        ),
        (
            "touch-roll",
            "Roll chords",
            "With Humanise: chords of four notes or more rolled from the bottom up, slightly",
            |t| &mut t.roll,
        ),
        (
            "touch-velocity",
            "Velocity curve",
            "For files whose notes share one velocity: louder on the beat and up the line, the melody over the rest",
            |t| &mut t.velocity,
        ),
    ];
    let mut changed = false;
    let mut cells = Vec::new();
    for (id, label, tip, pick) in switches {
        let on = *pick(&mut touch);
        let r = latch(ui, id, label, tip, on);
        if r.changed {
            *pick(&mut touch) = !on;
            changed = true;
        }
        cells.push(r.el.grow(1).w(0.));
    }
    if changed {
        app.songbook.touch = touch;
        out.push(Action::Songs(SongAction::Touch(touch)));
    }
    let mut grid = Vec::new();
    let mut cells = cells.into_iter();
    while let (Some(a), b) = (cells.next(), cells.next()) {
        grid.push(row(std::iter::once(a).chain(b).collect::<Vec<_>>()).gap(TIGHT));
    }
    col![
        row![section("Touch-ups"), spacer()],
        col(grid).gap(TIGHT),
        row![
            caption(status.clone())
                .fill(secondary())
                .lines(1)
                .min_w(0)
                .tip(status)
                .id("touch-status"),
            spacer()
        ],
    ]
    .gap(TIGHT)
    .pad(kit::edges(TIGHT, SPACE, SPACE, SPACE))
    .shrink(0)
    .id("touch-ups")
}

/// What the touch-ups did to the song `p`: those it got, else those it
/// would get were they on, else that it needs none.
fn status(p: &Played) -> String {
    let names = |t: Touch| -> Vec<&str> {
        [
            (t.pedal, "auto-pedal"),
            (t.humanise, "humanise"),
            (t.roll && t.humanise, "rolled chords"),
            (t.velocity, "velocity curve"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
    };
    let got = names(p.applied);
    if !got.is_empty() {
        return format!("This song got {}", got.join(", "));
    }
    let could = names(p.needs);
    if could.is_empty() {
        "This song is played by hand: it needs none".into()
    } else {
        format!("Off for this song: {}", could.join(", "))
    }
}
