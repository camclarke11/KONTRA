//! A playing song's notes falling onto the keybed: each note a bar in its
//! key's lane that reaches the keys as it sounds, a piano roll turned on
//! its side. Drawn only while a song plays.
//!
//! The look follows MIDIVisualizer (Simon Rodriguez, MIT; THIRD_PARTY.md):
//! bars a little narrower than their key with softly rounded corners, a
//! sharp's bar darker than a natural's, the keys a note sounds lit in its
//! colour, and a halo where a sounding note meets the keys, brightest as
//! it strikes. A louder note is lighter, as Magenta.js's waterfall
//! (Apache-2.0) makes it more opaque.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mui::prelude::*;

use crate::keyboard::{black, layout};

/// Seconds of the song above the keys: how soon a note shows before it
/// sounds.
pub const AHEAD: f64 = 3.;

/// The keybed's octaves while it follows a song, at the least: the width
/// it has without one, so its keys keep their size.
pub const OCTAVES: u8 = 7;

/// One note of a song.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    /// When it starts and ends, in seconds from the song's start.
    pub start: f64,
    pub end: f64,
    pub key: u8,
    /// 1..=127.
    pub velocity: u8,
    /// Which of the song's voices plays it (a track and channel), its
    /// colour.
    pub voice: u8,
}

/// A song's notes in start order, shared without copying.
#[derive(Clone, Debug, Default)]
pub struct Notes {
    notes: Arc<[Note]>,
    /// The longest note's length: how long before a moment a note that
    /// still sounds at it can have started.
    longest: f64,
    /// The lowest and highest key played.
    keys: Option<(u8, u8)>,
}

/// One song's notes are the same notes: compared by identity, not by
/// walking thousands of them each frame.
impl PartialEq for Notes {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.notes, &other.notes)
    }
}

impl Notes {
    #[must_use]
    pub fn new(mut notes: Vec<Note>) -> Self {
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        let longest = notes.iter().map(|n| n.end - n.start).fold(0., f64::max);
        let low = notes.iter().map(|n| n.key).min();
        let high = notes.iter().map(|n| n.key).max();
        Self {
            notes: notes.into(),
            longest,
            keys: low.zip(high),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// The lowest and highest key played.
    #[must_use]
    pub fn keys(&self) -> Option<(u8, u8)> {
        self.keys
    }

    /// The notes sounding at some moment from `from` to `to` seconds, in
    /// start order: those starting before `to` and ending after `from`.
    pub fn between(&self, from: f64, to: f64) -> impl Iterator<Item = &Note> {
        let first = self
            .notes
            .partition_point(|n| n.start < from - self.longest);
        let last = self.notes.partition_point(|n| n.start < to);
        let span = self.notes.get(first..last.max(first)).unwrap_or_default();
        span.iter().filter(move |n| n.end > from)
    }

    /// The notes sounding at `at` seconds.
    pub fn sounding(&self, at: f64) -> impl Iterator<Item = &Note> {
        // A note starting exactly at `at` sounds: it is under way.
        self.between(at, at + 1e-9).filter(move |n| n.start <= at)
    }

    /// The keys sounding at `at`, each in its note's colour: the last
    /// struck wins a key two voices hold.
    #[must_use]
    pub fn lit(&self, at: f64) -> Vec<(u8, Color)> {
        let mut lit: Vec<(u8, Color)> = Vec::new();
        for n in self.sounding(at) {
            let c = color(n.voice, 127, false);
            match lit.iter_mut().find(|k| k.0 == n.key) {
                Some(k) => k.1 = c,
                None => lit.push((n.key, c)),
            }
        }
        lit
    }
}

/// Whole octaves from a C holding `keys`, at least `least` of them, with
/// the room spread either side: the keybed's range while it follows a song.
#[must_use]
pub fn fit((low, high): (u8, u8), least: u8) -> (u8, u8) {
    // MIDI's 128 keys make ten whole octaves and a part one.
    const ALL: i32 = 11;
    let (first, last) = (i32::from(low.min(high) / 12), i32::from(low.max(high) / 12));
    let have = last - first + 1;
    let want = i32::from(least).max(have).min(ALL);
    let start = (first - (want - have) / 2).clamp(0, ALL - want);
    let at = |octave: i32| u8::try_from(octave.clamp(0, 127)).unwrap_or(127);
    (at(start * 12), at((start + want) * 12 - 1))
}

/// The voices' hues, far apart, over again past the sixth: blue (the first
/// part's), then teal, violet, cyan, magenta and green. None is amber,
/// which reads as a warning, nor coral, the error colour, as a part's hue
/// may be.
const VOICES: [f32; 6] = [250., 165., 305., 205., 340., 135.];

/// A note's colour: its voice's hue, lighter and fuller the harder it is
/// played; a sharp's darker, as its key is.
#[must_use]
pub fn color(voice: u8, velocity: u8, sharp: bool) -> Color {
    let v = f32::from(velocity.min(127)) / 127.;
    let lightness = 0.56 + 0.2 * v;
    let lightness = if sharp { lightness * 0.84 } else { lightness };
    let hue = VOICES
        .get(usize::from(voice) % VOICES.len())
        .copied()
        .unwrap_or_default();
    Color::oklch(lightness, 0.09 + 0.06 * v, hue)
}

/// How far a bar runs on past the canvas's edges, out of sight: its
/// rounded ends show only where the note itself starts and ends.
const BLEED: f64 = 6.;

/// A note's bar over a keybed `size` big showing `range`, `at` seconds into
/// the song: (x, y, width, height), None when it is off the keybed or not
/// in view. Time runs up the canvas, [`AHEAD`] seconds of it, the bottom
/// edge being now; a bar under way runs on under the keys.
fn bar(
    keys: &[(u8, f64, f64)],
    range: (u8, u8),
    note: &Note,
    at: f64,
    size: Size,
) -> Option<(f64, f64, f64, f64)> {
    let &(_, x, w) = keys.get(usize::from(note.key.checked_sub(range.0)?))?;
    if note.key > range.1 {
        return None;
    }
    let per_second = size.height / AHEAD;
    let top = size.height - (note.end - at) * per_second;
    let bottom = size.height - (note.start - at) * per_second;
    if bottom < 0. || top > size.height {
        return None;
    }
    let (top, bottom) = (top.max(-BLEED), bottom.min(size.height + BLEED));
    // A gap either side, so neighbours read apart (MIDIVisualizer's bars
    // are 0.9 of their key).
    let inset = w * if black(note.key) { 0.08 } else { 0.07 };
    // Even the shortest note is a bar, not a line.
    let height = (bottom - top).max(3.);
    Some((x + inset, bottom - height, w - 2. * inset, height))
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Path {
    Path::polyline(
        [(x, y), (x + w, y), (x + w, y + h), (x, y + h)].map(|(x, y)| Point::new(x, y)),
        true,
    )
}

/// A rectangle with its corners rounded by `r`.
fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> Path {
    let r = r.min(w / 2.).min(h / 2.).max(0.);
    let p = Point::new;
    Path::default()
        .move_to(p(x + r, y))
        .line_to(p(x + w - r, y))
        .quad_to(p(x + w, y), p(x + w, y + r))
        .line_to(p(x + w, y + h - r))
        .quad_to(p(x + w, y + h), p(x + w - r, y + h))
        .line_to(p(x + r, y + h))
        .quad_to(p(x, y + h), p(x, y + h - r))
        .line_to(p(x, y + r))
        .quad_to(p(x, y), p(x + r, y))
        .close()
}

/// Everything the falling notes draw on a canvas of `size`.
fn draw(notes: &Notes, at: f64, range: (u8, u8), size: Size) -> Vec<Draw> {
    let (w, h) = (size.width, size.height);
    let keys = layout(range, w);
    let mut out = vec![Draw::fill(rect(0., 0., w, h), Role::Field)];
    // An octave's lanes between faint lines at each C, fainter at each F.
    for &(n, x, _) in keys.iter().filter(|k| k.0 % 12 == 0 || k.0 % 12 == 5) {
        if n != range.0 {
            let alpha = if n % 12 == 0 { 0.07 } else { 0.035 };
            out.push(Draw::fill(rect(x, 0., 1., h), Role::Ink.alpha(alpha)));
        }
    }
    let shown: Vec<&Note> = notes.between(at, at + AHEAD).collect();
    // Naturals first, so a sharp's bar lies over a neighbour's, as its key does.
    let mut halos = Vec::new();
    for sharp in [false, true] {
        for n in shown.iter().filter(|n| black(n.key) == sharp) {
            let Some((x, y, bw, bh)) = bar(&keys, range, n, at, size) else {
                continue;
            };
            let sounding = n.start <= at;
            let c = color(n.voice, n.velocity, sharp);
            let face = if sounding {
                c.with_lightness((c.lightness() + 0.12).min(0.94))
            } else {
                c
            };
            let r = (bw * 0.22).min(4.);
            out.push(Draw::fill(rounded(x, y, bw, bh, r), Fill::from(face)));
            // A lighter rim, as MIDIVisualizer brightens a bar's edge.
            out.push(Draw::stroke(
                rounded(x + 0.5, y + 0.5, bw - 1., bh - 1., r),
                Fill::from(
                    face.with_lightness((face.lightness() + 0.1).min(0.97))
                        .with_alpha(0.55),
                ),
                1.,
            ));
            if sounding {
                halos.push((x + bw / 2., bw, c, at - n.start));
            }
        }
    }
    // Where a sounding note meets the keys, a halo rising from them,
    // brightest as it strikes (MIDIVisualizer's flash: full inside 0.14 of
    // its radius, gone at its edge).
    for (cx, bw, c, since) in halos {
        let strike = (-since / 0.18).exp() as f32;
        let r = (bw * 1.6).max(10.) * (1. + 0.4 * f64::from(strike));
        let glow = c.with_lightness((c.lightness() + 0.18).min(0.97));
        let stops = [
            (0., glow.with_alpha(0.5 + 0.4 * strike)),
            (0.14, glow.with_alpha(0.32 + 0.3 * strike)),
            (1., glow.with_alpha(0.)),
        ];
        // The upper half of a disc on the keys' edge: the gradient's
        // radius is a share of the box's longer side, its width.
        out.push(Draw::fill(
            rect(cx - r, h - r, 2. * r, r),
            Gradient::radial((0.5, 1.), 0.5, stops),
        ));
    }
    // The top fades into the field, so a note comes into view gently.
    let fade = (h * 0.12).min(24.);
    out.push(Draw::fill(
        rect(0., 0., w, fade),
        Gradient::linear(
            180.,
            [(0., Role::Field.alpha(1.)), (1., Role::Field.alpha(0.))],
        ),
    ));
    out
}

/// The falling notes over a keybed showing `range`, `at` seconds into the
/// song, `height` tall and as wide as the keybed under it.
#[must_use]
pub fn waterfall(notes: &Notes, at: f64, range: (u8, u8), height: f64) -> El {
    let notes = notes.clone();
    let face = canvas(move |s| draw(&notes, at, range, s))
        .w(Len::Pct(100.))
        .h(Len::Pct(100.));
    // A node clips what is inside it, not its own drawing: hence the stack.
    stack![face]
        .w(Len::Pct(100.))
        .h(height)
        .clip()
        .named("Falling notes")
}

/// Where a song is, moving smoothly between the audio thread's reports.
/// It reports once a block, so in steps of 5 to 20 ms, which a display
/// showing a frame every 8 or 16 ms would show as a judder: this runs on
/// the display's own clock and leans gently towards each report, jumping
/// only when far out (a new song). Once the reports stop moving (the audio
/// thread stalled, or its host stopped it), so does this.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    /// What it last showed, and when.
    last: Option<(f64, Instant)>,
    /// The last report, and when it first said so.
    heard: Option<(f64, Instant)>,
}

impl Clock {
    /// Further out than this, it jumps to the report.
    const JUMP: f64 = 0.25;
    /// The share of its error it makes up each frame.
    const LEAN: f64 = 0.08;
    /// Reports that hold still this long have stopped.
    const STALL: Duration = Duration::from_millis(250);

    /// Whether the reports still move: the frames stop when they do, so
    /// a stalled audio thread cannot keep the editor drawing.
    #[must_use]
    pub fn moving(&self, now: Instant) -> bool {
        self.heard
            .is_some_and(|(_, at)| now.saturating_duration_since(at) < Self::STALL)
    }

    /// The position to show at `now`, the audio thread having last
    /// reported `reported` seconds.
    pub fn at(&mut self, reported: f64, now: Instant) -> f64 {
        if self.heard.is_none_or(|(r, _)| r != reported) {
            self.heard = Some((reported, now));
        }
        let shown = match self.last {
            // Stopped: where the audio thread is, not where it would be.
            Some(_) if !self.moving(now) => reported,
            Some((shown, then)) => {
                let ahead = shown + now.saturating_duration_since(then).as_secs_f64();
                let error = reported - ahead;
                if error.abs() > Self::JUMP {
                    reported
                } else {
                    // Never backwards: a bar that climbs back jitters.
                    (ahead + error * Self::LEAN).max(shown)
                }
            }
            None => reported,
        };
        self.last = Some((shown, now));
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f64, end: f64, key: u8) -> Note {
        Note {
            start,
            end,
            key,
            velocity: 100,
            voice: 0,
        }
    }

    #[test]
    fn notes_in_view_and_sounding() {
        let notes = Notes::new(vec![
            note(2., 2.5, 64),
            note(0., 4., 48), // long: still sounding late
            note(1., 1.5, 60),
            note(5., 6., 72),
        ]);
        assert_eq!(notes.keys(), Some((48, 72)));
        let keys = |it: Vec<&Note>| it.iter().map(|n| n.key).collect::<Vec<_>>();
        // In start order; the long one found though it started long before.
        assert_eq!(keys(notes.between(1.2, 4.2).collect()), [48, 60, 64]);
        assert_eq!(keys(notes.between(3.0, 4.5).collect()), [48]);
        assert_eq!(keys(notes.sounding(2.0).collect()), [48, 64]);
        assert_eq!(keys(notes.sounding(1.5).collect()), [48], "ended at 1.5");
        assert!(notes.sounding(6.0).next().is_none());
        assert_eq!(
            notes.lit(2.2).iter().map(|k| k.0).collect::<Vec<_>>(),
            [48, 64]
        );
        assert_eq!(Notes::default().keys(), None);
        assert!(Notes::default().between(0., 10.).next().is_none());
    }

    #[test]
    fn the_keybed_fits_a_song_in_whole_octaves() {
        // C1..C6 needs six octaves; seven shown, the spare above.
        assert_eq!(fit((36, 96), OCTAVES), (36, 119));
        // A piano's 88 keys: A-1..C6 by these names, nine octaves.
        assert_eq!(fit((21, 108), OCTAVES), (12, 119));
        // A small song sits in the middle of seven.
        assert_eq!(fit((60, 64), OCTAVES), (24, 107));
        // Room runs out at the ends.
        assert_eq!(fit((0, 5), OCTAVES), (0, 83));
        assert_eq!(fit((120, 127), OCTAVES), (48, 127));
        assert_eq!(fit((0, 127), OCTAVES), (0, 127));
    }

    #[test]
    fn a_bar_stands_on_its_key_and_reaches_the_keys_as_it_sounds() {
        let range = (48, 59);
        let size = Size::new(70., 300.);
        let keys = layout(range, size.width);
        // Seven naturals 10 wide; 100 px a second.
        let c = bar(&keys, range, &note(1., 1.5, 48), 0., size).unwrap();
        assert!(
            (c.0 - 0.7).abs() < 1e-9 && (c.2 - 8.6).abs() < 1e-9,
            "{c:?}"
        );
        assert!(
            (c.1 - 150.).abs() < 1e-9 && (c.3 - 50.).abs() < 1e-9,
            "{c:?}"
        );
        // A sharp's bar is narrower, over the seam between its naturals.
        let cs = bar(&keys, range, &note(1., 1.5, 49), 0., size).unwrap();
        assert!(cs.0 > 7. && cs.0 + cs.2 < 13., "{cs:?}");
        // As it sounds, its foot is under the keys, out of sight.
        let now = bar(&keys, range, &note(1., 1.5, 48), 1.2, size).unwrap();
        assert!((now.1 - 270.).abs() < 1e-9, "{now:?}");
        assert!((now.1 + now.3 - 300. - BLEED).abs() < 1e-9, "{now:?}");
        // A note longer than the view runs past its top, out of sight.
        let long = bar(&keys, range, &note(0., 9., 48), 1., size).unwrap();
        assert!((long.1 + BLEED).abs() < 1e-9, "{long:?}");
        // Off the keybed, past, or not yet in view: nothing.
        assert!(bar(&keys, range, &note(1., 1.5, 47), 0., size).is_none());
        assert!(bar(&keys, range, &note(1., 1.5, 60), 0., size).is_none());
        assert!(bar(&keys, range, &note(4., 5., 50), 0., size).is_none());
        // A blink of a note still shows.
        let blip = bar(&keys, range, &note(1., 1., 50), 0., size).unwrap();
        assert!(blip.3 >= 3.);
    }

    #[test]
    fn the_harder_a_note_the_lighter_and_sharps_darker() {
        let soft = color(0, 30, false);
        let hard = color(0, 120, false);
        assert!(hard.lightness() > soft.lightness());
        assert!(color(0, 120, true).lightness() < hard.lightness());
        assert_ne!(color(0, 100, false), color(1, 100, false));
        // No amber (a warning) and no coral (an error), as DESIGN.md asks.
        for voice in 0..=40 {
            let hue = color(voice, 100, false).hue();
            assert!(!(15.0..105.0).contains(&hue), "voice {voice}: {hue}");
        }
    }

    #[test]
    fn the_clock_moves_smoothly_between_reports() {
        let t0 = Instant::now();
        let mut clock = Clock::default();
        let frame = Duration::from_micros(8_333);
        let block = 0.0107; // 512 frames at 48 kHz
        let mut shown = Vec::new();
        for n in 0..240u32 {
            let now = t0 + frame * n;
            let real = (now - t0).as_secs_f64();
            // Reported in steps, at each block's end.
            let reported = (real / block).floor() * block;
            shown.push(clock.at(reported, now));
        }
        let steps: Vec<f64> = shown.windows(2).map(|w| w[1] - w[0]).collect();
        let next = t0 + frame * 241;
        let frame = frame.as_secs_f64();
        // Every frame moves on about a frame: no stalls, no leaps back.
        assert!(steps.iter().all(|s| *s >= 0.), "{steps:?}");
        let late: Vec<f64> = steps.iter().skip(20).copied().collect();
        assert!(
            late.iter().all(|s| (s - frame).abs() < frame * 0.15),
            "{late:?}"
        );
        // And it keeps up with the reports.
        let last = shown.last().copied().unwrap_or_default();
        assert!((last - 239. * frame).abs() < 0.015, "{last}");
        // A new song from its start: a jump, not a slide.
        assert_eq!(clock.at(0.0, next), 0.0);
        assert!(clock.moving(next));
        // Reports that stop moving: it stops where they are, and says so.
        let still = next + Duration::from_millis(300);
        assert_eq!(clock.at(0.0, still), 0.0);
        assert!(!clock.moving(still));
        assert!(!Clock::default().moving(still));
    }
}
