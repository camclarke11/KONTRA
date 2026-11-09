//! Turning a piano recording into a MIDI song, as the Songs tab shows it:
//! "Transcribe a recording…", then its progress and "Cancel", then the
//! song it made (which the Songs tab lists and plays) or why it made none.

use mui::prelude::*;

use crate::kit::{self, button};
use crate::theme::{SPACE, TEXT, TIGHT, secondary};

/// What the Songs tab shows of transcribing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcribing {
    /// The recording being transcribed, and how far it is, 0 to 1.
    pub job: Option<(String, f64)>,
    /// What the last one did: the song it made, or why it made none.
    pub last: Option<String>,
    /// Whether the transcription model is installed.
    pub ready: bool,
}

/// What the player asked of transcribing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Do {
    /// Pick a recording to transcribe.
    Pick,
    Cancel,
}

/// The tooltip on the button: what it does and what it takes.
const TIP: &str = "Turn a piano recording (WAV, AIFF, MP3, M4A, FLAC, Ogg) into a MIDI song with its notes, \
    dynamics and pedal, then play it. You can also drop a recording on the window. \
    Transcription: Kong et al., ByteDance, CC BY 4.0";

/// The Songs tab's transcribing row; what the player asked, into `out`.
pub fn control(ui: &mut Ui, t: &Transcribing, out: &mut Vec<Do>) -> El {
    if let Some((name, done)) = &t.job {
        let cancel = button(ui, "transcribe-cancel", "Cancel", false);
        if cancel.changed {
            out.push(Do::Cancel);
        }
        let pct = format!("{:.0}%", (done * 100.).clamp(0., 100.));
        return col![
            row![
                caption(format!("Transcribing {name}"))
                    .lines(1)
                    .min_w(0)
                    .grow(1)
                    .tip(name.clone()),
                caption(pct).reserve("100%").fill(secondary()),
            ]
            .gap(TIGHT)
            .align(Align::Center),
            row![
                kit::meter(*done, TEXT * 4.).grow(1).w(0.),
                cancel.el.tip("Stop transcribing; nothing is kept"),
            ]
            .gap(TIGHT)
            .align(Align::Center),
        ]
        .gap(TIGHT)
        .pad(kit::edges(TIGHT, SPACE, TIGHT, SPACE))
        .shrink(0)
        .id("transcribing");
    }
    let go = button(ui, "transcribe", "Transcribe a recording…", false);
    if go.changed && t.ready {
        out.push(Do::Pick);
    }
    let tip = if t.ready {
        TIP
    } else {
        "Transcription needs its model, piano-transcription.onnx, which is not installed (see tools/transcribe-model)"
    };
    let mut items = vec![row![go.el.tip(tip).grow(1)]];
    if let Some(last) = &t.last {
        items.push(row![
            caption(last.clone())
                .fill(secondary())
                .lines(1)
                .min_w(0)
                .tip(last.clone())
                .id("transcribe-last"),
            spacer()
        ]);
    }
    col(items)
        .gap(TIGHT)
        .pad(kit::edges(TIGHT, SPACE, TIGHT, SPACE))
        .shrink(0)
        .id("transcribe-row")
}
