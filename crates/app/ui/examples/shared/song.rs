//! What the song examples draw: a made song, a real one read from a MIDI
//! file, the editor playing it, and the falling notes alone over a keybed
//! filling a window, as a full-screen view would show them.

use std::error::Error;
use std::sync::Arc;

use kontra_ui::app::{App, Part, Song};
use kontra_ui::design::Design;
use kontra_ui::keyboard::{self, Keys};
use kontra_ui::waterfall::{self, Note, Notes, Pedal, Show};
use mui::prelude::*;

/// Seconds a beat lasts at `bar`.
fn beat(bar: usize) -> f64 {
    if bar < 8 { 0.5 } else { 0.75 }
}

pub fn song() -> Notes {
    let mut notes = Vec::new();
    let mut pedal = Vec::new();
    let chords: [[u8; 3]; 4] = [[48, 55, 64], [45, 52, 60], [41, 48, 57], [43, 50, 59]];
    let scale = [72u8, 74, 76, 77, 79, 81, 83, 84];
    let mut at = 0.;
    for bar in 0..16 {
        let b = beat(bar);
        // The left hand: a chord held the bar, the pedal changed under it,
        // half down in the slower bars.
        for &key in chords.get(bar % 4).into_iter().flatten() {
            notes.push(Note {
                start: at,
                end: at + 4. * b - 0.02,
                key,
                velocity: 60,
                voice: 1,
            });
        }
        pedal.push((at + 0.03, 0, 0));
        pedal.push((at + 0.12, 0, if bar < 8 { 127 } else { 70 }));
        // The right hand: sixteenths up and back down, swelling.
        for s in 0..16usize {
            let step = if s < 8 { s } else { 15 - s };
            let key = scale.get(step).copied().unwrap_or(72) + if bar % 2 == 1 { 12 } else { 0 };
            notes.push(Note {
                start: at + s as f64 * b / 4.,
                end: at + (s + 1) as f64 * b / 4. - 0.015,
                key: if s % 4 == 3 { key + 1 } else { key },
                velocity: 40 + (70 * s / 15) as u8,
                voice: 0,
            });
        }
        at += 4. * b;
    }
    Notes::new(notes).with_pedal(Pedal::new(pedal))
}

// MARK: a small Standard MIDI File reader, for real performances

/// The notes and sustain pedal of the Standard MIDI File `bytes`: each
/// note from its note-on to its key's next note-off on that channel,
/// coloured by track and channel; the drum channel skipped.
pub fn midi(bytes: &[u8]) -> Result<Notes, Box<dyn Error>> {
    let be = |at: usize, len: usize| -> Result<usize, Box<dyn Error>> {
        let field = bytes.get(at..at + len).ok_or("truncated")?;
        Ok(field.iter().fold(0, |v, &b| v << 8 | usize::from(b)))
    };
    if bytes.get(..4) != Some(b"MThd".as_slice()) {
        return Err("not a MIDI file".into());
    }
    let division = be(12, 2)?;
    if division & 0x8000 != 0 || division == 0 {
        return Err("SMPTE time is not read here".into());
    }
    // (tick, order, track, message); tempo changes as (tick, µs a beat).
    let mut events: Vec<(u64, usize, u16, [u8; 3])> = Vec::new();
    let mut tempos: Vec<(u64, f64)> = vec![(0, 500_000.)];
    let mut at = 8 + be(4, 4)?;
    let mut track = 0u16;
    while at + 8 <= bytes.len() {
        let len = be(at + 4, 4)?;
        let body = bytes.get(at + 8..at + 8 + len).ok_or("truncated track")?;
        if bytes.get(at..at + 4) == Some(b"MTrk".as_slice()) {
            let (mut i, mut tick, mut status) = (0usize, 0u64, 0u8);
            let number = |i: &mut usize| -> Result<u64, Box<dyn Error>> {
                let mut v = 0u64;
                for _ in 0..4 {
                    let b = *body.get(*i).ok_or("truncated")?;
                    *i += 1;
                    v = v << 7 | u64::from(b & 0x7F);
                    if b & 0x80 == 0 {
                        return Ok(v);
                    }
                }
                Err("bad length".into())
            };
            while i < body.len() {
                tick += number(&mut i)?;
                let first = *body.get(i).ok_or("truncated")?;
                if first == 0xFF {
                    let kind = *body.get(i + 1).ok_or("truncated")?;
                    i += 2;
                    let len = number(&mut i)? as usize;
                    if kind == 0x51 && len == 3 {
                        let t = body.get(i..i + 3).ok_or("truncated")?;
                        let us = t.iter().fold(0u32, |v, &b| v << 8 | u32::from(b));
                        tempos.push((tick, f64::from(us)));
                    }
                    i += len;
                    continue;
                }
                if first == 0xF0 || first == 0xF7 {
                    i += 1;
                    let len = number(&mut i)? as usize;
                    i += len;
                    continue;
                }
                if first & 0x80 != 0 {
                    status = first;
                    i += 1;
                }
                let two = matches!(status & 0xF0, 0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0);
                let d1 = *body.get(i).ok_or("truncated")?;
                let d2 = if two {
                    *body.get(i + 1).ok_or("truncated")?
                } else {
                    0
                };
                i += if two { 2 } else { 1 };
                events.push((tick, events.len(), track, [status, d1, d2]));
            }
            track += 1;
        }
        at += 8 + len;
    }
    events.sort_by_key(|e| (e.0, e.1));
    tempos.sort_by_key(|t| t.0);
    let seconds = |tick: u64| {
        let (mut time, mut last, mut tempo) = (0., 0u64, 500_000.);
        for &(t, us) in tempos.iter().take_while(|t| t.0 <= tick) {
            time += (t - last) as f64 * tempo / 1e6 / division as f64;
            (last, tempo) = (t, us);
        }
        time + (tick - last) as f64 * tempo / 1e6 / division as f64
    };
    let mut voices: Vec<(u16, u8)> = events
        .iter()
        .filter(|e| e.3[0] & 0xF0 == 0x90 && e.3[2] > 0)
        .map(|e| (e.2, e.3[0] & 15))
        .collect();
    voices.sort_unstable();
    voices.dedup();
    let mut open = vec![None; 16 * 128];
    let mut notes: Vec<Note> = Vec::new();
    let mut pedal = Vec::new();
    let end = events.last().map_or(0., |e| seconds(e.0));
    for &(tick, _, track, [status, key, value]) in &events {
        let channel = status & 15;
        if channel == 9 {
            continue;
        }
        let time = seconds(tick);
        match status & 0xF0 {
            0xB0 if key == 64 => pedal.push((time, channel, value)),
            0x80 | 0x90 => {
                let slot = open
                    .get_mut(usize::from(channel) * 128 + usize::from(key))
                    .ok_or("key")?;
                if let Some(n) = slot.take().and_then(|i: usize| notes.get_mut(i)) {
                    n.end = time;
                }
                if status & 0xF0 == 0x90 && value > 0 {
                    *slot = Some(notes.len());
                    let voice = voices.binary_search(&(track, channel)).unwrap_or(0);
                    notes.push(Note {
                        start: time,
                        end,
                        key,
                        velocity: value,
                        voice: voice as u8,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(Notes::new(notes).with_pedal(Pedal::new(pedal)))
}

pub fn editor(notes: Notes, name: &str, design: &str) -> App {
    let mut app = App {
        parts: vec![Part {
            name: "Una Corda Cotton".into(),
            library: "Una Corda".into(),
            output: 1,
            keys: (21, 108),
            ..Part::default()
        }],
        song: Some(Song {
            name: name.into(),
            position: 0.,
            length: 40.,
            notes,
            ..Default::default()
        }),
        ..App::default()
    };
    app.view.design = design.into();
    app
}

/// The falling notes over a keybed filling a window `size` big, `at`
/// seconds into the song: what a full-screen view draws.
pub fn stage(ui: &mut Ui, notes: &Notes, d: &Arc<Design>, at: f64, (w, h): (f64, f64)) -> El {
    let range = notes
        .keys()
        .map_or((21, 108), |k| waterfall::fit(k, waterfall::OCTAVES));
    let whites = (range.0..=range.1)
        .filter(|n| ![1, 3, 6, 8, 10].contains(&(n % 12)))
        .count();
    let keys_tall = (w / whites as f64 * 5.).min(h * 0.22);
    let show = Show {
        notes: notes.clone(),
        at,
        range,
        design: d.clone(),
        ..Show::default()
    };
    let keys = Keys {
        range,
        lit: notes.lit_in(at, d),
        ..Keys::default()
    };
    let bed = keyboard::keyboard(ui, "keybed", &keys, &mut None, keys_tall);
    col![
        waterfall::falling(show).h(h - keys_tall),
        bed.el.grow(0).w(Len::Pct(100.))
    ]
    .w(Len::Pct(100.))
    .h(Len::Pct(100.))
    .fill(Color::oklch(0.08, 0., 0.))
}
