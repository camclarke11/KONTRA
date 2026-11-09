//! Exporting the song, as the header shows it: an "Export" menu (the song
//! rendered through the rack to a WAV, or its falling notes to a video),
//! then the export's progress and "Cancel", then where it went.
//!
//! [`Frames`] draws the video: the falling notes over a keybed, frame by
//! frame, headless, by vello's CPU renderer.

use mui::prelude::*;
use mui::vello;

use crate::keyboard::{self, Keys};
use crate::kit::{self, button};
use crate::theme::{TEXT, TIGHT, secondary};
use crate::waterfall::{self, Notes};

/// A video's frame sizes and rates on offer.
pub const VIDEOS: [(u32, u32); 4] = [(1080, 30), (1080, 60), (2160, 30), (2160, 60)];

/// What the header shows of exporting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exporting {
    /// A song to export: the one playing or played last.
    pub song: Option<String>,
    /// The export under way: what it makes, and how far it is, 0 to 1.
    pub job: Option<(String, f64)>,
    /// What the last export did: where it wrote, or why it did not.
    pub last: Option<String>,
}

/// What the user asked of exporting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Do {
    /// The song through the rack to a WAV file.
    Audio,
    /// The song's falling notes to an MP4, `height` lines (1080 or 2160)
    /// at `fps` frames a second, with the song's WAV when there is one.
    Video {
        height: u32,
        fps: u32,
    },
    Cancel,
    /// Show the last file exported.
    Reveal,
}

const MENU: &str = "export";

/// The header's export controls; what the user asked, into `out`.
pub fn control(ui: &mut Ui, open: &mut Option<String>, ex: &Exporting, out: &mut Vec<Do>) -> El {
    if let Some((what, done)) = &ex.job {
        let cancel = button(ui, "export-cancel", "Cancel", false);
        if cancel.changed {
            out.push(Do::Cancel);
        }
        let pct = format!("{what} {:.0}%", (done * 100.).clamp(0., 100.));
        return row![
            caption(pct).reserve("Exporting video 100%").lines(1),
            kit::meter(*done, TEXT * 4.),
            cancel.el.tip("Stop the export; nothing is kept"),
        ]
        .gap(TIGHT)
        .align(Align::Center)
        .shrink(0)
        .id("exporting");
    }
    let mut items: Vec<String> = vec!["Audio (WAV, 48 kHz 24-bit)".into()];
    items.extend(VIDEOS.iter().map(|&(h, fps)| {
        let size = if h >= 2160 { "4K" } else { "1080p" };
        format!("Video ({size}, {fps} fps)")
    }));
    if ex.last.is_some() {
        items.push("Show last export".into());
    }
    let names: Vec<&str> = items.iter().map(String::as_str).collect();
    let tip = match (&ex.song, &ex.last) {
        (None, _) => "Play a MIDI file first, then export it".to_owned(),
        (Some(song), Some(last)) => format!("Export {song} · last: {last}"),
        (Some(song), None) => format!("Export {song} through the rack"),
    };
    let r = kit::dropdown(ui, open, MENU, (None, "Export", &tip), &names, None);
    match r.changed {
        Some(0) if ex.song.is_some() => out.push(Do::Audio),
        Some(n) if n <= VIDEOS.len() && ex.song.is_some() => {
            if let Some(&(height, fps)) = VIDEOS.get(n - 1) {
                out.push(Do::Video { height, fps });
            }
        }
        Some(_) if ex.last.is_some() => out.push(Do::Reveal),
        _ => {}
    }
    r.el
}

/// The logical size a video frame is laid out at: 1080p, whatever it is
/// drawn at, so a 4K frame is the same picture, sharper.
pub const FRAME: (f64, f64) = (1920., 1080.);

/// One frame of a song's video: its falling notes over a keybed showing
/// `range` (whole octaves), its keys lit as they sound, `at` seconds in,
/// its name in the corner.
#[must_use]
pub fn frame_view(ui: &mut Ui, notes: &Notes, at: f64, range: (u8, u8), name: &str) -> El {
    let (w, h) = FRAME;
    // Keys a little wider than tall, as a piano's look on a 16:9 screen.
    let keys_tall = (h * 0.17).round();
    let shown = Keys {
        range,
        lit: notes.lit(at),
        ..Keys::default()
    };
    let mut none = None;
    let bed = keyboard::keyboard(ui, "video-keys", &shown, &mut none, keys_tall);
    let lane = waterfall::waterfall(notes, at, range, h - keys_tall - 2.);
    // Its name in the top corner, over the lane.
    let title = col![
        row![
            caption(name.to_owned())
                .text_size(TEXT * 1.5)
                .fill(secondary())
                .lines(1)
        ]
        .pad((TEXT * 1.5, TEXT * 1.5)),
        spacer()
    ]
    .w(Len::Pct(100.))
    .h(Len::Pct(100.));
    col![
        stack![lane, title].w(Len::Pct(100.)),
        block(Len::Pct(100.), 2).fill(Role::Background),
        bed.el.w(Len::Pct(100.)).grow(0),
    ]
    .w(w)
    .h(h)
    .fill(Role::Field)
}

/// Draws video frames headless at `width` by `height` pixels, reusing its
/// renderer between them.
pub struct Frames {
    ui: Ui,
    /// No frame drawn yet: a first frame's text is drawn a little apart
    /// from later ones' (its glyphs not yet cached), so one is drawn and
    /// dropped, and every frame is the same whichever `Frames` draws it.
    cold: bool,
    width: u16,
    height: u16,
    ctx: vello::vello_cpu::RenderContext,
    resources: vello::vello_cpu::Resources,
    cache: vello::Cache,
}

impl Frames {
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            ui: crate::ui(),
            cold: true,
            width,
            height,
            ctx: vello::vello_cpu::RenderContext::new(width, height),
            resources: vello::vello_cpu::Resources::default(),
            cache: vello::Cache::default(),
        }
    }

    /// The frame `at` seconds into the song, as straight RGBA, row by row.
    ///
    /// # Errors
    /// When the frame cannot be laid out or painted.
    pub fn draw(
        &mut self,
        notes: &Notes,
        at: f64,
        range: (u8, u8),
        name: &str,
    ) -> Result<Vec<u8>, String> {
        if std::mem::take(&mut self.cold) {
            self.draw(notes, at, range, name)?;
        }
        let el = frame_view(&mut self.ui, notes, at, range, name);
        let size = Size::new(FRAME.0, FRAME.1);
        self.ui
            .frame(el, Some(size), Input::default(), 1. / 60.)
            .map_err(|e| format!("{e:?}"))?;
        let scene = self.ui.scene().ok_or("no scene")?;
        self.ctx.reset();
        let scale = f64::from(self.height) / FRAME.1;
        let cpu = &mut vello::Cpu {
            ctx: &mut self.ctx,
            resources: &mut self.resources,
            cache: &mut self.cache,
        };
        vello::paint(cpu, scene, vello::kurbo::Affine::scale(scale))
            .map_err(|e| format!("{e:?}"))?;
        self.ctx.flush();
        let mut pix = vello::vello_cpu::Pixmap::new(self.width, self.height);
        self.ctx.render(&mut pix, &mut self.resources);
        Ok(pix
            .take_unpremultiplied()
            .iter()
            .flat_map(|p| [p.r, p.g, p.b, p.a])
            .collect())
    }
}

/// The keys a song's video shows: whole octaves holding its notes, seven
/// at least, as the editor's keybed shows them.
#[must_use]
pub fn range(notes: &Notes) -> (u8, u8) {
    notes
        .keys()
        .map_or((24, 107), |keys| waterfall::fit(keys, waterfall::OCTAVES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waterfall::Note;

    #[test]
    fn a_frame_draws_the_notes_falling_and_the_keys_they_light() {
        let notes = Notes::new(vec![
            Note {
                start: 0.5,
                end: 1.5,
                key: 60,
                velocity: 100,
                voice: 0,
            },
            Note {
                start: 0.0,
                end: 2.0,
                key: 48,
                velocity: 80,
                voice: 1,
            },
        ]);
        let mut frames = Frames::new(192, 108);
        let range = range(&notes);
        assert_eq!(range, (24, 107));
        let a = frames.draw(&notes, 0.0, range, "test.mid").unwrap();
        assert_eq!(a.len(), 192 * 108 * 4);
        let b = frames.draw(&notes, 1.0, range, "test.mid").unwrap();
        assert_ne!(a, b, "the notes moved");
        // The same moment draws the same picture, whoever draws it.
        assert_eq!(b, frames.draw(&notes, 1.0, range, "test.mid").unwrap());
        let mut fresh = Frames::new(192, 108);
        assert_eq!(b, fresh.draw(&notes, 1.0, range, "test.mid").unwrap());
        // Not all one colour.
        assert!(a.chunks(4).any(|p| p != &a[..4]));
    }
}
