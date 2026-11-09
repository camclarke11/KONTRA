//! Turning a piano recording into a MIDI song, as the Songs tab shows it:
//! "Transcribe a recording…" and whether to isolate its piano first (from
//! a voice and a band), then its progress and "Cancel", then the song it
//! made (which the Songs tab lists and plays) or why it made none.

use mui::prelude::*;

use crate::kit::{self, button, latch};
use crate::theme::{SPACE, TEXT, TIGHT, secondary};

/// What the Songs tab shows of transcribing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcribing {
    /// What it is doing to which recording ("Isolating the piano in …",
    /// "Transcribing …"), and how far it is, 0 to 1.
    pub job: Option<(String, f64)>,
    /// What the last one did: the song it made, or why it made none.
    pub last: Option<String>,
    /// Whether the transcription model is installed.
    pub ready: bool,
    /// Whether the separation model, which isolates the piano, is.
    pub separator: bool,
}

/// What the player asked of transcribing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Do {
    /// Pick a recording to transcribe.
    Pick,
    Cancel,
    /// Isolate the piano before transcribing (true), or not.
    Isolate(bool),
}

/// The tooltip on the button: what it does and what it takes.
const TIP: &str = "Turn a piano recording (WAV, AIFF, MP3, M4A, FLAC, Ogg) into a MIDI song with its notes, \
    dynamics and pedal, then play it. You can also drop a recording on the window. \
    Transcription: Kong et al., ByteDance, CC BY 4.0";

/// The Songs tab's transcribing row, `isolate` the piano or not; what the
/// player asked, into `out`.
pub fn control(ui: &mut Ui, t: &Transcribing, isolate: bool, out: &mut Vec<Do>) -> El {
    if let Some((what, done)) = &t.job {
        let cancel = button(ui, "transcribe-cancel", "Cancel", false);
        if cancel.changed {
            out.push(Do::Cancel);
        }
        let pct = format!("{:.0}%", (done * 100.).clamp(0., 100.));
        return col![
            row![
                caption(what.clone())
                    .lines(1)
                    .min_w(0)
                    .grow(1)
                    .tip(what.clone()),
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
    let tip_isolate = if t.separator {
        "Isolate the piano first: voices, drums, bass and other instruments are separated out \
         (BS-RoFormer, on the GPU), so only the piano becomes notes. Turn it off for a solo piano recording; it is faster."
    } else {
        "Isolating the piano needs its model, piano-separation.safetensors, which is not installed (see tools/separate-model)"
    };
    let on = isolate && t.separator;
    let piano = latch(ui, "transcribe-isolate", "Piano only", tip_isolate, on);
    if piano.changed && t.separator {
        out.push(Do::Isolate(!isolate));
    }
    let mut items = vec![row![go.el.tip(tip).grow(1), piano.el.tip(tip_isolate)].gap(TIGHT)];
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
