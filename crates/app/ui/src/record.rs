//! Recording what the player plays, as the header shows it: "Record", a
//! count-in's beats, then the take's clock and "Stop"; beside it a menu for
//! the count-in, the folder takes are saved in and the last take.

use mui::prelude::*;

use crate::kit::{self, Icon, button, icon_button};
use crate::theme::{TEXT, TIGHT, secondary};

/// Where a take is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Take {
    #[default]
    Idle,
    /// Counting in: the beat sounding, from 1.
    CountIn(u8),
    Recording,
}

/// What the header shows of recording.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recording {
    pub take: Take,
    /// Seconds into the take.
    pub seconds: f64,
    /// A bar of clicks before the take starts.
    pub count_in: bool,
    /// The folder takes go to.
    pub folder: String,
    /// The last take saved: its file name.
    pub saved: Option<String>,
}

/// What the user asked of recording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Do {
    /// Start a take (after the count-in, when it is on).
    Start,
    /// Stop the take and save it.
    Stop,
    CountIn(bool),
    /// Pick the folder takes go to.
    Folder,
    /// Play the last take saved.
    PlayLast,
    /// Show the folder takes go to.
    Reveal,
}

/// The menu's id.
const MENU: &str = "record-menu";

/// `seconds` as minutes and seconds: 2:05.
fn minutes(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A red dot: what records.
fn dot() -> El {
    canvas(|s| {
        let r = s.width.min(s.height) * 0.3;
        let (cx, cy) = (s.width / 2., s.height / 2.);
        let k = r * 0.552_284_75;
        let p = |x: f64, y: f64| Point::new(cx + x, cy + y);
        let path = Path::default()
            .move_to(p(r, 0.))
            .cubic_to(p(r, k), p(k, r), p(0., r))
            .cubic_to(p(-k, r), p(-r, k), p(-r, 0.))
            .cubic_to(p(-r, -k), p(-k, -r), p(0., -r))
            .cubic_to(p(k, -r), p(r, -k), p(r, 0.))
            .close();
        vec![Draw::fill(path, Fill::from(Color::oklch(0.63, 0.2, 25.)))]
    })
    .square(TEXT)
    .shrink(0)
}

/// The header's recording controls; what the user asked, into `out`.
pub fn control(ui: &mut Ui, open: &mut Option<String>, rec: &Recording, out: &mut Vec<Do>) -> El {
    let main = match rec.take {
        Take::Idle => {
            let r = button(ui, "record", "Record", false);
            if r.changed {
                out.push(Do::Start);
            }
            let tip = if rec.count_in {
                "Record what you play, after a bar of clicks"
            } else {
                "Record what you play: the take starts at your first note"
            };
            row![dot(), r.el].align(Align::Center).tip(tip)
        }
        Take::CountIn(beat) => {
            let r = button(ui, "record", &format!("Count-in {beat}"), true);
            if r.changed {
                out.push(Do::Stop);
            }
            row![dot(), r.el.tip("Counting in: click to stop")].align(Align::Center)
        }
        Take::Recording => {
            let r = button(ui, "record", "Stop", true);
            if r.changed {
                out.push(Do::Stop);
            }
            row![
                dot(),
                caption(minutes(rec.seconds)).reserve("00:00"),
                r.el.tip("Stop the take and save it as a MIDI file"),
            ]
            .gap(TIGHT)
            .align(Align::Center)
        }
    };
    let more = icon_button(ui, MENU, Icon::Down, "Recording options", false);
    let mut el = vec![main.gap(TIGHT).shrink(0).id("recording"), more.el];
    if kit::toggle(ui, open, MENU, more.changed) {
        let count = if rec.count_in {
            "Count-in: on"
        } else {
            "Count-in: off"
        };
        let mut items = vec![count, "Recordings folder…", "Show recordings"];
        let last = rec.saved.as_deref().map(|name| format!("Play {name}"));
        if let Some(last) = &last {
            items.push(last);
        }
        let picked = kit::menu(ui, MENU, &items, 1, None);
        if let Some(n) = picked.changed {
            out.extend(match n {
                0 => Some(Do::CountIn(!rec.count_in)),
                1 => Some(Do::Folder),
                2 => Some(Do::Reveal),
                3 => Some(Do::PlayLast),
                _ => None,
            });
            *open = None;
        }
        el.push(picked.el.tip(format!("Takes go to {}", rec.folder)));
    }
    // The last take, once saved: its name, so the player sees it landed.
    if let (Take::Idle, Some(saved)) = (rec.take, &rec.saved) {
        el.push(
            caption(saved.clone())
                .fill(secondary())
                .lines(1)
                .min_w(0)
                .w(TEXT * 8.)
                .tip(format!("Saved in {} · the menu plays it", rec.folder))
                .id("recording-saved"),
        );
    }
    row(el).gap(1).align(Align::Center).shrink(0)
}
