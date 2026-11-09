//! The editor, laid out as v2 laid it out: a header with the engine's load,
//! the songs' files, the master and the performance switches; the browser;
//! the Rack, Mixer and Report tabs; the keyboard strip along the foot,
//! where the playing happens (a song's transport, practice, recording and
//! the notes over the keys: [`strip`]). Built from [`crate::kit`] each
//! frame from the [`App`] the host fills; what the user does comes back as
//! [`Action`]s. Long words are cut to one line with the rest in a tooltip,
//! choices are dropdowns, menus and panels float.

mod browser;
mod controls;
mod mapping;
mod mixer;
mod part;
mod rack;
mod report;
mod settings;
mod songs;
mod sound;
mod strip;

pub use songs::{Played, SongAction, SongFile, SongFolder, SongShelf, SongView, Songbook, Touch};
pub use sound::{Envelope, Filter, Lower, Param, Pass, Sound};

use kontra_ui_model::Interface;
use mui::prelude::*;

use crate::Pictures;
use crate::kit::{self, Icon, button, icon_button, rule, section, sheet, tab, vrule};
use crate::stage;
use crate::theme::{CONTROL, SIDEBAR, SPACE, TEXT, TIGHT, secondary};
use crate::waterfall::{self, Notes};

/// Everything the editor shows. The host fills the data; `view` is the
/// editor's own state and survives a reload of the rest.
#[derive(Debug, Default)]
pub struct App {
    pub libraries: Vec<Library>,
    /// Instruments loaded lately, newest first, as (library, preset).
    pub recent: Vec<(usize, usize)>,
    pub parts: Vec<Part>,
    pub stats: Stats,
    /// Master gain in dB, and the master's level from 0 to 1.
    pub master: f64,
    pub level: f64,
    /// What did not load or play as the source meant, one entry each: its
    /// first line is its summary.
    pub report: Vec<String>,
    pub midi_thru: bool,
    /// Where new parts listen and play ([`Action::NewParts`]).
    pub next_channel: bool,
    pub own_output: bool,
    /// The build, for About: its version and source revision.
    pub build: String,
    /// The folders the browser scans.
    pub folders: Vec<String>,
    /// The editor's width in points, as the host frames it: a library's
    /// view fits what the rack has of it. 0 while unknown.
    pub width: f64,
    pub view: View,
    /// The libraries' cover pictures; the host gives a store whose worker
    /// reads [`Library::art`] and replaces it when the list changes.
    pub covers: Option<Pictures>,
    /// The same art as the rack header's banners ([`crate::artwork::banner`]).
    pub banners: Option<Pictures>,
    /// A load the host has been asked for and not finished.
    // ponytail: one load shown at a time; a list if loads queue up.
    pub loading: Option<Loading>,
    /// The MIDI file playing into the rack ([`Action::PlaySong`]), if any.
    pub song: Option<Song>,
    /// The browser's Songs tab: folders of MIDI files and their touch-ups.
    pub songbook: Songbook,
    /// Recording what the player plays ([`crate::record`]).
    pub record: crate::record::Recording,
    /// Practice mode: the song waits for the player ([`crate::practice`]).
    pub practice: crate::practice::Practice,
    /// Exporting the song to audio or video ([`crate::export`]).
    pub export: crate::export::Exporting,
    /// What the player plays: rising from the keys with no song, lighting
    /// the keys they strike under one ([`waterfall::Played`]).
    pub played: waterfall::Played,
}

impl App {
    /// Whether the keyboard follows a playing song, its notes falling and
    /// its keys lit as they sound: the host asks for a frame every display
    /// refresh while it does, and for none once the song ends.
    #[must_use]
    pub fn follows_song(&self) -> bool {
        self.song.is_some() && (!self.view.keyboard_hidden || self.view.stage.open)
    }
}

/// A MIDI file playing, as the keyboard bar shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Song {
    /// Its file name.
    pub name: String,
    /// Where it is, in seconds.
    pub position: f64,
    /// How long it is, in seconds.
    pub length: f64,
    /// Its notes, falling onto the keyboard as it plays.
    pub notes: Notes,
    /// Paused or not, its speed, loop and the rest ([`stage`]).
    pub transport: stage::Transport,
}

/// Where a load lands: a new part, or part `n` it replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loading {
    New,
    Part(usize),
}

/// A library in the browser.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    pub name: String,
    pub format: Format,
    /// Its instruments by path inside the library: `Folder/Name`.
    pub presets: Vec<String>,
    pub starred: bool,
    /// Pictures that may be its cover, best first, as the host's cover
    /// store reads them; the first that decodes is shown.
    pub art: Vec<String>,
}

/// Which family of libraries the browser lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Format {
    #[default]
    Kontakt,
    Uvi,
    /// Not libraries: the player's MIDI files ([`Songbook`]).
    Songs,
}

/// One loaded instrument.
#[derive(Debug, Default)]
pub struct Part {
    pub name: String,
    pub library: String,
    /// MIDI channel 1..=16, or 0 for every channel.
    pub channel: u8,
    /// Stereo output 1..=16, or 0 to let the host pick.
    pub output: u8,
    /// Gain in dB, pan from -1 (left) to 1, tune in semitones.
    pub volume: f64,
    pub pan: f64,
    pub tune: f64,
    /// Its left and right peaks from 0 to 1, for the meters: its output's,
    /// then each instrument bus's.
    pub level: Vec<[f64; 2]>,
    pub solo: bool,
    pub mute: bool,
    /// Velocity it plays at: 0 as played, or a fixed one.
    pub velocity: u8,
    /// The articulations and the key that switches to each.
    pub articulations: Vec<(String, Option<u8>)>,
    pub articulation: usize,
    /// The one it starts on.
    pub default_articulation: usize,
    /// The keys it plays.
    pub keys: (u8, u8),
    /// What it needs to play fully (a missing file, a host feature).
    pub needs: Vec<String>,
    /// What the Info tab lists, as (label, value).
    pub info: Vec<(String, String)>,
    /// Its scripts' names.
    pub scripts: Vec<String>,
    /// Its groups' names, and its zones.
    pub groups: Vec<String>,
    pub zones: Vec<Zone>,
    /// Each group's sound, by group.
    pub sounds: Vec<Sound>,
    /// The library's own interface, when it has one.
    pub face: Option<Face>,
    /// Shown below its header.
    pub open: bool,
    pub tab: PartTab,
    /// How its interface is drawn; `None` follows [`View::face_look`].
    pub look: Option<FaceLook>,
}

/// One zone of a part's mapping.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zone {
    /// Index into the part's groups.
    pub group: usize,
    pub sample: String,
    /// The keys and velocities it plays, inclusive, and its root key.
    pub keys: (u8, u8),
    pub velocities: (u8, u8),
    pub root: u8,
    /// Tune in cents, volume in dB, pan from -1 to 1.
    pub tune: f64,
    pub volume: f64,
    pub pan: f64,
}

/// A library's interface, its decoded pictures and the gesture on it.
pub struct Face {
    pub tree: Interface,
    pub pictures: Pictures,
    pub gesture: crate::input::Gesture,
    /// Its level meters as they fall and hold.
    pub meters: crate::meter::Meters,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Face")
            .field("widgets", &self.tree.widgets.len())
            .finish_non_exhaustive()
    }
}

/// A part's views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PartTab {
    #[default]
    Interface,
    Articulations,
    Mapping,
    Sound,
    Info,
    Scripts,
}

/// How a part draws its library's interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FaceLook {
    /// The library's own pictures, as its sampler draws them.
    #[default]
    Original,
    /// The same controls in the same places, without the pictures.
    Vector,
    /// The controls one after another in KONTRA's own look.
    Kontra,
}

impl FaceLook {
    pub const ALL: [Self; 3] = [Self::Original, Self::Vector, Self::Kontra];
    pub const LABELS: [&str; 3] = ["Original", "Vector", "KONTRA"];
}

/// The engine's load, for the header.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub cpu: f64,
    pub voices: u32,
    /// The whole process; 0 where the host cannot read it.
    pub ram_mb: u32,
    /// The samples' preloaded pages.
    pub samples_mb: u32,
    pub disk_mbs: f64,
    /// The streamed page cache's size.
    pub cache_mb: u32,
}

/// The main area's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Rack,
    Mixer,
    Report,
}

/// Which libraries the browser lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shelf {
    #[default]
    All,
    Favorites,
    Recent,
}

/// The editor's own state; its default is a fresh editor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub tab: Tab,
    pub browser_hidden: bool,
    /// The library list's share of the browser's height; 0 is the default.
    pub split: f64,
    pub format: Format,
    pub shelf: Shelf,
    /// Libraries listed Z to A.
    pub descending: bool,
    pub search: String,
    /// The library whose presets the browser lists, and their search.
    pub library: Option<usize>,
    pub preset_search: String,
    /// Preset folders folded shut.
    pub folded: Vec<String>,
    pub sheet: Option<Sheet>,
    /// The id of the open popup menu.
    pub menu: Option<String>,
    pub mixer_wide: bool,
    pub report_search: String,
    /// The report entry shown whole.
    pub report_open: Option<usize>,
    /// The articulation grip being dragged, and the row it started on.
    pub grip: Option<(String, usize)>,
    /// The browser row a click selected, as (library, preset): a click
    /// selects, a double-click or a drag loads.
    pub picked: Option<(usize, usize)>,
    /// When that row was clicked, for the second click of a double-click.
    pub clicked: Option<std::time::Instant>,
    /// The Mapping tab: the group shown (all when none), the group filter,
    /// the cell last picked on the grid (key, velocity) and the zone picked.
    // ponytail: one mapping state for the editor; per part if two open at once matters.
    pub mapping_group: Option<usize>,
    pub mapping_search: String,
    pub mapping_cell: Option<(u8, u8)>,
    pub mapping_zone: Option<usize>,
    /// The Sound tab: the group shown, whether edits set it alone, what
    /// shows under the graphs, the handle held (graph, handle, the axis
    /// it was pressed on) and each graph's last handle.
    pub sound_group: usize,
    pub sound_one: bool,
    pub sound_lower: Lower,
    pub sound_grab: Option<(usize, usize, f64)>,
    pub sound_focus: [usize; 2],
    /// Articulations listed without their keys.
    pub keys_hidden: bool,
    pub keyboard_hidden: bool,
    /// A playing song's notes not shown falling onto the keyboard.
    pub notes_hidden: bool,
    /// The [`design`] the falling notes are drawn in, by name; empty is
    /// the default.
    pub design: String,
    /// Octaves the keybed is moved from C-2 to B4.
    pub octave: i8,
    pub playing: Option<u8>,
    /// The pitch wheel (springs back to 0) and the mod wheel, -1..1 and 0..1.
    pub pitch: f64,
    pub modulation: f64,
    /// Which of [`SCALES`] the editor is drawn at.
    pub scale: usize,
    /// The Stage: the window given over to a playing song.
    pub stage: stage::Stage,
    /// Which of [`FACE_SCALES`] a library's own view is drawn at.
    pub face_scale: usize,
    /// How a part that chose none draws its interface.
    pub face_look: FaceLook,
    /// The browser's Songs tab.
    pub songs: SongView,
}

/// The editor sizes on offer.
pub const SCALES: [f64; 4] = [1., 1.25, 1.5, 2.];

/// A library's view sizes on offer: as wide as the rack (0, the default),
/// or a fixed scale that scrolls when it is wider.
pub const FACE_SCALES: [f64; 4] = [0., 1., 1.5, 2.];

/// A panel opened on demand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sheet {
    Settings,
    About,
}

/// What the user asked for this frame, for the host to carry out.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Load {
        library: usize,
        preset: usize,
    },
    /// A browser preset dropped on a part: it plays that instead.
    Replace {
        part: usize,
        library: usize,
        preset: usize,
    },
    /// Replace a part's instrument with another of its library's.
    Switch {
        part: usize,
        preset: usize,
    },
    /// The previous (-1) or next (1) preset of a part's library.
    Step {
        part: usize,
        by: i32,
    },
    Volume(usize, f64),
    Pan(usize, f64),
    Tune(usize, f64),
    Solo(usize, bool),
    Mute(usize, bool),
    Channel(usize, u8),
    Output(usize, u8),
    Velocity(usize, u8),
    Articulation(usize, usize),
    /// A library's own control set by the user: the widget of the part's
    /// interface, its value and the keys held at the click, for the host to
    /// write into the engine.
    Control {
        part: usize,
        widget: usize,
        /// The cell of a table set; `None` for any other control.
        cell: Option<u32>,
        value: f64,
        keys: kontra_ui_model::UiModifiers,
    },
    /// Text the user typed into a library's text edit.
    ControlText {
        part: usize,
        widget: usize,
        text: String,
    },
    /// A value of a part's sound, for one group or all of them.
    Sound {
        part: usize,
        group: Option<usize>,
        param: Param,
        value: f64,
    },
    /// Play a zone of a part's mapping.
    Audition {
        part: usize,
        zone: usize,
    },
    DefaultArticulation(usize, usize),
    /// Move an articulation from one place in the list to another.
    MoveArticulation {
        part: usize,
        from: usize,
        to: usize,
    },
    Duplicate(usize),
    Remove(usize),
    Master(f64),
    Note(u8, bool),
    Pitch(f64),
    Modulation(f64),
    Panic,
    MidiThru(bool),
    /// Pick a MIDI file and play it into the rack.
    PlaySong,
    /// Stop the MIDI file playing.
    StopSong,
    /// The playing song's transport: pause, seek, speed, loop and the rest.
    Song(stage::Command),
    /// The browser's Songs tab: play, star, a folder, the touch-ups.
    Songs(SongAction),
    /// Recording: start, stop, its options ([`crate::record`]).
    Record(crate::record::Do),
    /// Practice with these hands, or not at all ([`crate::practice`]).
    Practice(crate::practice::Hands),
    /// Export the song, or cancel the export ([`crate::export`]).
    Export(crate::export::Do),
    Star(usize, bool),
    AddFolder,
    /// New parts take the next free MIDI channel; their own output pair.
    NewParts {
        next_channel: bool,
        own_output: bool,
    },
    /// Stop scanning the browser's folder `n`.
    RemoveFolder(usize),
    Scale(f64),
    CopyDiagnostics,
    ExportReport,
    LogFolder,
    Refresh,
}

/// The editor for `app`, and what the user did.
pub fn view(ui: &mut Ui, app: &mut App) -> Response<Vec<Action>> {
    let mut out = Vec::new();
    // The window's width as last laid out: what a library's view fits.
    if let Some(s) = ui.scene() {
        app.width = s.layout.size.width;
    }
    // The Stage takes the whole window while a song is loaded.
    if app.view.stage.open && app.song.is_some() {
        let el = stage::view(ui, app, &mut out);
        return Response { el, changed: out };
    }
    // Closed because the song went: the screen it filled goes back too.
    if std::mem::take(&mut app.view.stage.open) && std::mem::take(&mut app.view.stage.full) {
        out.push(Action::Song(stage::Command::FullScreen(false)));
    }
    let header = header(ui, app, &mut out);
    let width = ui.tween(
        "browser-width",
        if app.view.browser_hidden { 0. } else { SIDEBAR },
    );
    let mut middle = Vec::new();
    if width > 0.5 {
        middle.push(
            browser::view(ui, app, &mut out)
                .w(width)
                .shrink(0)
                .fill(Role::Surface)
                .clip(),
        );
        middle.push(vrule());
    }
    let mut main = vec![main(ui, app, &mut out).grow(1).min_w(0).min_h(0)];
    if let Some(s) = app.view.sheet {
        main.push(sheet_view(ui, app, s, &mut out).centered());
    }
    middle.push(stack(main).grow(1).min_w(0).min_h(0).id("main"));
    // v2's two tones: the frame around the work (header, browser, tab
    // rows, keyboard) one step up on Surface, the work on Background.
    // Only the middle gives way to a short window: the header and the
    // keyboard keep their height, as v2's dock did.
    let el = col![
        header.shrink(0).fill(Role::Surface),
        loading_bar(app),
        row(middle).grow(1).min_h(0).align(Align::Stretch),
        rule(),
        strip::view(ui, app, &mut out).shrink(0).fill(Role::Surface),
    ]
    .w(Len::Pct(100.))
    .h(Len::Pct(100.))
    .fill(Role::Background);
    Response { el, changed: out }
}

/// The line under the header, lit while a load is under way (v2's
/// `src/ui/header.rs` `loading_bar`, without its sweep: a sweep needs a
/// frame every display refresh for as long as the load runs).
fn loading_bar(app: &App) -> El {
    if app.loading.is_none() {
        return rule();
    }
    block(Len::Pct(100.), 1)
        .fill(Role::Ink.alpha(0.7))
        .shrink(0)
        .named("Loading")
}

fn main(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let mut tabs = vec![block(SPACE, 1).shrink(0)];
    for (which, label) in [
        (Tab::Rack, "Rack"),
        (Tab::Mixer, "Mixer"),
        (Tab::Report, "Report"),
    ] {
        let t = tab(ui, format!("tab-{label}"), label, app.view.tab == which);
        if t.changed {
            app.view.tab = which;
        }
        tabs.push(t.el);
    }
    let shown = match app.view.tab {
        Tab::Rack => rack::view(ui, app, out),
        Tab::Mixer => mixer::view(ui, app, out),
        Tab::Report => report::view(ui, app, out),
    };
    col![
        row(tabs).gap(TIGHT).shrink(0).fill(Role::Surface),
        shown.grow(1).min_h(0).w(Len::Pct(100.))
    ]
    .w(Len::Pct(100.))
    .h(Len::Pct(100.))
}

/// The songs' files: "Play MIDI…" picks one to play along with the rack
/// (a song playing is replaced), and while there is one to export (the
/// song playing, or played last) Export, or the export's progress.
fn song_control(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let play = button(ui, "song-play", "Play MIDI…", false);
    if play.changed {
        out.push(Action::PlaySong);
    }
    let mut items = vec![
        play.el
            .tip("Play a MIDI file along with the rack, or drop one on the window"),
    ];
    // Exporting the song ([`crate::export`]).
    if app.export.song.is_some() || app.export.job.is_some() {
        let mut export = Vec::new();
        items.push(crate::export::control(
            ui,
            &mut app.view.menu,
            &app.export,
            &mut export,
        ));
        out.extend(export.into_iter().map(Action::Export));
    }
    row(items)
        .gap(TIGHT)
        .align(Align::Center)
        .shrink(0)
        .id("song-files")
}

/// From this window width the header shows the engine's load, CPU and
/// voices; from [`ROOMY`], its memory and disk too. Narrower, the
/// readouts give way so the controls fit (and so does a first frame,
/// whose width is not known yet). An export's progress takes their room.
const LOADED: f64 = 900.;
const ROOMY: f64 = 1280.;
/// The room an export's progress and its Cancel take beside Export.
const EXPORTING: f64 = 260.;

/// Whether the window is `wide` enough for a readout, with room left for
/// an export's progress while one runs.
fn fits(app: &App, wide: f64) -> bool {
    let busy = if app.export.job.is_some() {
        EXPORTING
    } else {
        0.
    };
    app.width >= wide + busy
}

/// The engine's load: CPU and voices, and memory and disk where there is
/// room; each in full in its tooltip.
fn load(app: &App) -> El {
    let s = app.stats;
    let ram = if s.ram_mb > 0 { s.ram_mb } else { s.samples_mb };
    let stat = |label: &str, value: String, widest: &str, tip: String| {
        row![section(label), caption(value).reserve(widest)]
            .gap(TIGHT)
            .align(Align::Center)
            .shrink(0)
            .tip(tip)
            .id(format!("stat-{label}"))
    };
    let mut stats = vec![
        stat(
            "CPU",
            format!("{:.0}%", s.cpu),
            "100%",
            "The audio thread's time spent rendering".into(),
        ),
        stat(
            "Voices",
            s.voices.to_string(),
            "000",
            "Voices playing".into(),
        ),
    ];
    if fits(app, ROOMY) {
        stats.extend([
            stat(
                "RAM",
                format!("{ram} MB"),
                "00000 MB",
                format!(
                    "{} MB of samples in memory, of {ram} MB in all",
                    s.samples_mb
                ),
            ),
            stat(
                "Disk",
                format!("{:.1} MB/s", s.disk_mbs),
                "000.0 MB/s",
                format!("Streamed from disk, through a {} MB cache", s.cache_mb),
            ),
        ]);
    }
    row(stats).gap(SPACE * 1.5).shrink(0).id("load")
}

/// The master gain, its readout and its level.
fn master(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let mut master = app.master;
    let gain = kit::slider(
        ui,
        "master",
        "Master gain · double-click for 0 dB",
        &mut master,
        -60.0..=6.0,
        0.,
        (TEXT * 6., CONTROL),
    );
    if gain.changed {
        app.master = master;
        out.push(Action::Master(master));
    }
    row![
        section("Master"),
        gain.el,
        caption(db(app.master)).reserve("-inf dB"),
        kit::meter(app.level, TEXT * 3.),
    ]
    .gap(TIGHT)
    .align(Align::Center)
    .shrink(0)
}

fn header(ui: &mut Ui, app: &mut App, out: &mut Vec<Action>) -> El {
    let side = icon_button(
        ui,
        "browser",
        Icon::Sidebar,
        "Browser",
        !app.view.browser_hidden,
    );
    if side.changed {
        app.view.browser_hidden = !app.view.browser_hidden;
    }
    let songs = song_control(ui, app, out);
    let master = master(ui, app, out);
    let thru = button(ui, "midi-thru", "MIDI Thru", app.midi_thru);
    if thru.changed {
        app.midi_thru = !app.midi_thru;
        out.push(Action::MidiThru(app.midi_thru));
    }
    let panic = button(ui, "panic", "Panic", false);
    if panic.changed {
        out.push(Action::Panic);
    }
    // Who it is and how hard it works on the left; what the player does
    // on the right, which never gives way: the left does.
    let mut left = vec![side.el, body("KONTRA").text_weight(Weight::SEMIBOLD)];
    if fits(app, LOADED) {
        left.push(load(app));
    }
    let mut right = vec![
        songs,
        vrule().h(CONTROL * 0.6),
        master,
        vrule().h(CONTROL * 0.6),
        thru.el.tip("Pass MIDI through to the host"),
        panic.el.tip("Stop every note, and the song"),
    ];
    if !app.report.is_empty() {
        let n = app.report.len();
        let warn = icon_button(
            ui,
            "warnings",
            Icon::Warning,
            &format!("{n} things did not load as written: open the report"),
            false,
        );
        if warn.changed {
            app.view.tab = Tab::Report;
        }
        right.push(warn.el);
    }
    let more = icon_button(ui, "app-menu", Icon::Menu, "Settings and About", false);
    right.push(more.el);
    if kit::toggle(ui, &mut app.view.menu, "app-menu", more.changed) {
        let picked = kit::menu(ui, "app-menu", &["Settings", "About"], 1, None);
        if let Some(n) = picked.changed {
            app.view.sheet = [Sheet::Settings, Sheet::About].get(n).copied();
            app.view.menu = None;
        }
        right.push(picked.el);
    }
    row![
        row(left)
            .gap(SPACE * 1.5)
            .align(Align::Center)
            .grow(1)
            .min_w(0)
            .clip(),
        row(right).gap(SPACE).align(Align::Center).shrink(0),
    ]
    .gap(SPACE)
    .align(Align::Center)
    .pad((SPACE, TIGHT))
    .shrink(0)
}

fn sheet_view(ui: &mut Ui, app: &mut App, which: Sheet, out: &mut Vec<Action>) -> El {
    let (title, content) = match which {
        Sheet::Settings => ("Settings", settings::view(ui, app, out)),
        Sheet::About => (
            "About",
            col![
                title("KONTRA"),
                body(app.build.clone()).id("about-build"),
                caption(
                    "One engine that plays Kontakt, UVI and SFZ libraries as they were written."
                )
                .fill(secondary()),
            ]
            .gap(SPACE),
        ),
    };
    // No taller than the room under the header as last laid out, its
    // content scrolling, so a short window keeps its keyboard in view.
    let room = (ui.scene())
        .and_then(|s| s.surface("main"))
        .map_or(TEXT * 40., |s| s.frame.size.height - 2. * SPACE)
        .clamp(TEXT * 10., TEXT * 40.);
    let r = sheet(ui, "sheet", title, content.scroll().min_h(0));
    if r.changed {
        app.view.sheet = None;
    }
    r.el.max_size((TEXT * 36., room))
}

/// A gain for a readout.
fn db(v: f64) -> String {
    if v <= -60. {
        "-inf dB".into()
    } else {
        format!("{v:.1} dB")
    }
}
