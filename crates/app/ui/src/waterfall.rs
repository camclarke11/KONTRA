//! A playing song's notes falling onto the keybed: each note a bar in its
//! key's lane that reaches the keys as it sounds, a piano roll turned on
//! its side. Drawn only while a song plays, from the song's time alone,
//! so a paused song is a still picture and costs nothing.
//!
//! How it looks is a [`Design`] (crate::design). Back to front: the field
//! (a gradient, soft light drifting with the song), the lanes, the
//! sustain pedal's lane, light rising from each sounding key, a bloom
//! around each sounding bar, the bars, the line they strike with a halo
//! where each lands, sparks, and a vignette. Every length is a share of a
//! key's width or of the view's height, so it scales with its rect.
//!
//! The bars and the strike's flash follow MIDIVisualizer (Simon
//! Rodriguez, MIT): bars a little narrower than their key with rounded
//! ends and a lit edge, a sharp's bar darker, a flash that is brightest as
//! a note strikes. The halo's falloff, the strike line, a key's light
//! fading after release and the sparks ([`crate::sparks`]) follow
//! piano-viz (kyan0s, MIT OR Apache-2.0). Provenance: THIRD_PARTY.md.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mui::prelude::*;

use crate::design::{self, Blend, Design, Shading};
use crate::keyboard::{black, layout};
use crate::sparks::{self, Ground, Spark};

/// Seconds of the song above the keys: how soon a note shows before it
/// sounds.
pub const AHEAD: f64 = 3.;

/// The keybed's octaves while it follows a song, at the least: the width
/// it has without one, so its keys keep their size.
pub const OCTAVES: u8 = 7;

/// Seconds a key's light takes to fade once its note ends (piano-viz).
const RELEASE: f64 = 0.35;

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

/// The sustain pedal through a song: how far down it is from each moment
/// on, 0 (up) to 1 (down), half-pedalling in between.
#[derive(Clone, Debug, Default)]
pub struct Pedal {
    /// (seconds, depth) where the depth changes, in time order.
    steps: Arc<[(f64, f32)]>,
}

impl Pedal {
    /// From a song's sustain messages (CC64): (seconds, channel, value)
    /// in time order. With pedals on several channels, the deepest shows.
    #[must_use]
    pub fn new(events: impl IntoIterator<Item = (f64, u8, u8)>) -> Self {
        let mut held = [0u8; 16];
        let mut steps: Vec<(f64, f32)> = Vec::new();
        for (time, channel, value) in events {
            if let Some(h) = held.get_mut(usize::from(channel & 15)) {
                *h = value.min(127);
            }
            let depth = f32::from(held.iter().copied().max().unwrap_or(0)) / 127.;
            if let Some(last) = steps.last_mut()
                && last.0 >= time
            {
                last.1 = depth;
            } else if steps.last().is_none_or(|last| last.1 != depth) {
                steps.push((time, depth));
            }
        }
        Self {
            steps: steps.into(),
        }
    }

    /// Whether it is never pressed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.iter().all(|s| s.1 <= 0.)
    }

    /// How far down it is at `at` seconds.
    #[must_use]
    pub fn depth(&self, at: f64) -> f32 {
        let after = self.steps.partition_point(|s| s.0 <= at);
        after
            .checked_sub(1)
            .and_then(|i| self.steps.get(i))
            .map_or(0., |s| s.1)
    }

    /// How far down it is from `from` to `to` seconds, as the corners of
    /// a line through (seconds, depth). A pedal that moves in small steps
    /// close together (a pianist's foot, sent continuously) slopes from
    /// one to the next; one that jumps, holds, then jumps steps.
    #[must_use]
    pub fn profile(&self, from: f64, to: f64) -> Vec<(f64, f32)> {
        /// Messages closer than this are one movement of the foot.
        const GLIDE: f64 = 0.08;
        let first = self.steps.partition_point(|s| s.0 <= from);
        let mut held = self.depth(from);
        let mut out = vec![(from, held)];
        let mut before = first
            .checked_sub(1)
            .and_then(|i| self.steps.get(i))
            .map(|s| s.0);
        for &(t, depth) in self.steps.get(first..).unwrap_or_default() {
            if t >= to {
                break;
            }
            if before.is_none_or(|b| t - b >= GLIDE) {
                out.push((t, held));
            }
            out.push((t, depth));
            (held, before) = (depth, Some(t));
        }
        out.push((to, held));
        out
    }

    /// The stretches from `from` to `to` seconds it is down, cut to that
    /// window, in order: (start, end, depth).
    pub fn down(&self, from: f64, to: f64) -> impl Iterator<Item = (f64, f64, f32)> + '_ {
        let first = self
            .steps
            .partition_point(|s| s.0 <= from)
            .saturating_sub(1);
        let steps = self.steps.get(first..).unwrap_or_default();
        steps
            .iter()
            .enumerate()
            .map_while(move |(i, &(start, depth))| {
                let end = steps.get(i + 1).map_or(f64::INFINITY, |s| s.0);
                (start < to).then_some((start.max(from), end.min(to), depth))
            })
            .filter(|s| s.2 > 0. && s.1 > s.0)
    }
}

/// A song's notes in start order, and its pedal, shared without copying.
#[derive(Clone, Debug, Default)]
pub struct Notes {
    notes: Arc<[Note]>,
    /// The longest note's length: how long before a moment a note that
    /// still sounds at it can have started.
    longest: f64,
    /// The lowest and highest key played.
    keys: Option<(u8, u8)>,
    /// How many voices play them.
    voices: u8,
    pedal: Pedal,
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
        let voices = notes.iter().map(|n| n.voice).max();
        Self {
            notes: notes.into(),
            longest,
            keys: low.zip(high),
            voices: voices.map_or(0, |v| v.saturating_add(1)),
            pedal: Pedal::default(),
        }
    }

    /// These notes with the song's sustain pedal.
    #[must_use]
    pub fn with_pedal(mut self, pedal: Pedal) -> Self {
        self.pedal = pedal;
        self
    }

    /// The song's sustain pedal.
    #[must_use]
    pub fn pedal(&self) -> &Pedal {
        &self.pedal
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

    /// Whether `design` colours these notes by hand: it asks to, and one
    /// voice plays them all, as in most piano files.
    fn by_hand(&self, design: &Design) -> bool {
        design.notes.hands && self.voices <= 1
    }

    /// The notes sounding at some moment from `from` to `to` seconds, in
    /// start order: those starting before `to` and ending after `from`.
    pub fn between(&self, from: f64, to: f64) -> impl DoubleEndedIterator<Item = &Note> {
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

    /// The keys sounding at `at`, each in its note's colour by the default
    /// Design: the last struck wins a key two voices hold.
    #[must_use]
    pub fn lit(&self, at: f64) -> Vec<(u8, Color)> {
        self.lit_in(at, &design::named(""))
    }

    /// [`Notes::lit`] in `design`'s colours.
    #[must_use]
    pub fn lit_in(&self, at: f64, design: &Design) -> Vec<(u8, Color)> {
        let mut lit: Vec<(u8, Color)> = Vec::new();
        let hands = self.by_hand(design);
        for n in self.sounding(at) {
            let c = design.voice(voice(n, hands), 110, false);
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

/// A note's colour by the default Design: its voice's colour, lighter the
/// harder it is played; a sharp's darker, as its key is.
#[must_use]
pub fn color(voice: u8, velocity: u8, sharp: bool) -> Color {
    design::presets()
        .first()
        .map_or(Color::oklch(0.7, 0.1, 250.), |d| {
            d.voice(voice, velocity, sharp)
        })
}

/// A key lit by a sounding note, as the keybed paints it: brightest at
/// the top, where the note strikes it, deepening towards the player.
#[must_use]
pub fn lit_key(c: Color, sharp: bool) -> Fill {
    let tone = |l: f32, chroma: f32| c.with_lightness(l).scale_chroma(chroma);
    let stops = if sharp {
        [
            (0., tone(0.8, 0.7)),
            (0.3, tone(0.62, 1.)),
            (1., tone(0.48, 1.)),
        ]
    } else {
        [
            (0., tone(0.96, 0.35)),
            (0.22, tone(0.84, 0.8)),
            (1., tone(0.72, 1.)),
        ]
    };
    Gradient::linear(180., stops).into()
}

/// How far a bar runs on past the canvas's edges, out of sight, in key
/// widths: its rounded ends show only where the note itself starts and
/// ends.
const BLEED: f64 = 0.5;

/// A note's bar over a keybed `size` big showing `range`, `at` seconds into
/// the song, `share` of its key's width: (x, y, width, height), None when
/// it is off the keybed or not in view. Time runs up the canvas, [`AHEAD`]
/// seconds of it, the bottom edge being now; a bar under way runs on under
/// the keys.
fn bar(
    keys: &[(u8, f64, f64)],
    range: (u8, u8),
    note: &Note,
    at: f64,
    size: Size,
    share: f64,
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
    let bleed = BLEED * w.max(2.);
    let (top, bottom) = (top.max(-bleed), bottom.min(size.height + bleed));
    // A gap either side, so neighbours read apart.
    let bw = w * share.clamp(0.05, 1.);
    // Even the shortest note is a bar, not a line.
    let height = (bottom - top).max((w * 0.25).max(2.));
    Some((x + (w - bw) / 2., bottom - height, bw, height))
}

/// How much of its key's width a note's bar takes: a loud note its
/// Design's width, a soft one a little less.
fn share(d: &Design, n: &Note) -> f64 {
    let v = f64::from(n.velocity.min(127)) / 127.;
    let soft = d.notes.velocity_width * (1. - v);
    // A sharp's key is narrower already: its bar fills more of it.
    let sharp = if black(n.key) { 1.08 } else { 1. };
    (d.notes.width * (1. - soft) * sharp).min(0.98)
}

/// Middle C: below it the left hand, when a song is coloured by hand.
const SPLIT: u8 = 60;

/// The voice whose colour `n` takes: its own, or with `hands` its hand's
/// (the right hand the first colour, the left the second).
fn voice(n: &Note, hands: bool) -> u8 {
    match hands {
        true => u8::from(n.key < SPLIT),
        false => n.voice,
    }
}

/// A note's tone in `d`, its velocity in steps, so neighbours share a
/// gradient the renderer has already made; with `hands`, by hand.
fn tone(d: &Design, n: &Note, hands: bool) -> Color {
    let velocity = (n.velocity.min(127) / 8) * 8 + 4;
    d.voice(voice(n, hands), velocity, black(n.key))
}

/// How bright a strike's flash still is `age` seconds after it.
fn flash(age: f64) -> f64 {
    (-age.max(0.) / 0.12).exp()
}

/// A key lit by a note.
#[derive(Clone, Copy, Debug)]
struct Press<'a> {
    note: &'a Note,
    /// 1 while it sounds, fading for [`RELEASE`] after.
    level: f64,
    /// Seconds since it struck.
    age: f64,
}

/// The keys lit `at` seconds into the song, one press each: a held note
/// beats a fading one, the latest struck among equals (piano-viz's key
/// states).
fn presses<'a>(notes: impl Iterator<Item = &'a Note>, at: f64) -> Vec<Press<'a>> {
    let mut out: Vec<Press<'a>> = Vec::new();
    for n in notes {
        if n.start > at || at >= n.end + RELEASE {
            continue;
        }
        let level = if at < n.end {
            1.
        } else {
            (-(at - n.end) * 12.).exp()
        };
        let press = Press {
            note: n,
            level,
            age: at - n.start,
        };
        match out.iter_mut().find(|p| p.note.key == n.key) {
            Some(p)
                if level > p.level + 1e-9
                    || (level >= p.level - 1e-9 && n.start > p.note.start) =>
            {
                *p = press;
            }
            Some(_) => {}
            None => out.push(press),
        }
    }
    out
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

/// A disc; a tiny one is a square, which no one can tell from a disc.
fn disc(cx: f64, cy: f64, r: f64) -> Path {
    if r < 0.75 {
        return rect(cx - r, cy - r, 2. * r, 2. * r);
    }
    const K: f64 = 0.552_284_749_8;
    let (p, k) = (Point::new, r * K);
    Path::default()
        .move_to(p(cx + r, cy))
        .cubic_to(p(cx + r, cy + k), p(cx + k, cy + r), p(cx, cy + r))
        .cubic_to(p(cx - k, cy + r), p(cx - r, cy + k), p(cx - r, cy))
        .cubic_to(p(cx - r, cy - k), p(cx - k, cy - r), p(cx, cy - r))
        .cubic_to(p(cx + k, cy - r), p(cx + r, cy - k), p(cx + r, cy))
        .close()
}

/// How light falls off past a lit shape's edge, as a blur's edge does: at
/// each share of the way out, the share of its strength left (a Gaussian,
/// gone at the end).
const FALLOFF: [(f32, f32); 6] = [
    (0., 1.),
    (0.2, 0.85),
    (0.4, 0.53),
    (0.6, 0.24),
    (0.8, 0.08),
    (1., 0.),
];

/// [`FALLOFF`] of `c` at strength `a`, from `core` (full) out to 1.
fn ramp(c: Color, a: f32, core: f32) -> Vec<(f32, Color)> {
    // In steps, so a light that holds steady, or two alike, share one ramp
    // the renderer has made already: it keeps 64.
    let (a, core) = (step(a, 40.), step(core, 64.));
    FALLOFF
        .iter()
        .map(|&(u, k)| (core + (1. - core) * u, c.with_alpha(a * k)))
        .collect()
}

/// `v` to the nearest `1 / steps`.
fn step(v: f32, steps: f32) -> f32 {
    (v * steps).round() / steps
}

/// Light around a vertical pill from `top` down past `bottom`, `width`
/// wide, reaching `spread` past its edge: a blurred pill, drawn as a
/// straight side ramp and a round cap that share one falloff and meet on
/// a whole pixel, so no seam shows.
fn bloom(
    out: &mut Vec<Draw>,
    (cx, top, bottom): (f64, f64, f64),
    width: f64,
    spread: f64,
    c: Color,
    a: f32,
) {
    let (half, reach) = (width / 2., width / 2. + spread);
    if a <= 0.002 || reach <= 0. || bottom <= top {
        return;
    }
    let core = (half / reach) as f32;
    let side: Vec<(f32, Color)> = ramp(c, a, core)
        .into_iter()
        .rev()
        .map(|(t, c)| (0.5 - 0.5 * t, c))
        .chain(
            ramp(c, a, core)
                .into_iter()
                .map(|(t, c)| (0.5 + 0.5 * t, c)),
        )
        .collect();
    let joint = (top + half).min((top + bottom) / 2.).round();
    out.push(Draw::fill(
        rect(cx - reach, joint, 2. * reach, bottom - joint),
        Gradient::linear(90., side),
    ));
    out.push(Draw::fill(
        rect(cx - reach, joint - reach, 2. * reach, reach),
        Gradient::radial((0.5, 1.), 0.5, ramp(c, a, core)),
    ));
}

/// A halo rising from the line at `ground`, centred on `cx`, `r` in
/// radius: the top half of a soft disc.
fn halo(out: &mut Vec<Draw>, cx: f64, ground: f64, r: f64, c: Color, a: f32) {
    if a > 0.002 && r > 0. {
        out.push(Draw::fill(
            rect(cx - r, ground - r, 2. * r, r),
            Gradient::radial((0.5, 1.), 0.5, ramp(c, a, 0.)),
        ));
    }
}

/// A flare along the line at `ground`: light `length` either side of
/// `cx`, `thick` deep, its ends round; only its top half shows.
fn flare(out: &mut Vec<Draw>, (cx, ground): (f64, f64), length: f64, thick: f64, c: Color, a: f32) {
    if a <= 0.002 || thick <= 0. {
        return;
    }
    let (left, right) = ((cx - length).round(), (cx + length).round());
    let up: Vec<(f32, Color)> = ramp(c, a, 0.)
        .into_iter()
        .rev()
        .map(|(t, c)| (1. - t, c))
        .collect();
    out.push(Draw::fill(
        rect(left, ground - thick, right - left, thick),
        Gradient::linear(180., up),
    ));
    for (x, centre) in [(left - thick, 1.), (right, 0.)] {
        out.push(Draw::fill(
            rect(x, ground - thick, thick, thick),
            Gradient::radial((centre, 1.), 1., ramp(c, a, 0.)),
        ));
    }
}

/// What the falling notes show, and how.
#[derive(Clone, Debug)]
pub struct Show {
    pub notes: Notes,
    /// Seconds into the song.
    pub at: f64,
    /// The keys shown, as the keybed under it shows them.
    pub range: (u8, u8),
    pub design: Arc<Design>,
}

/// Everything the falling notes draw on a canvas of `size`, in two
/// layers: what is lit (the field, the bars, the lines) and the light
/// itself (beams, bloom, halos, sparks), which composites over it through
/// its Design's blend, so light adds to light.
fn draw(show: &Show, size: Size) -> (Vec<Draw>, Vec<Draw>) {
    let (d, at) = (&*show.design, show.at);
    let (w, h) = (size.width, size.height);
    let keys = layout(show.range, w);
    // The length every size is a share of: a natural key's width.
    let unit = keys
        .iter()
        .find(|k| !black(k.0))
        .map_or(w / 52., |k| k.2)
        .max(1.);
    let mut out = Vec::with_capacity(256);
    let mut light = Vec::with_capacity(256);
    field(&mut out, d, at, size);
    lanes(&mut out, d, &keys, show.range, size);
    let hands = show.notes.by_hand(d);
    let back = sparks::reach(&d.particles).max(RELEASE);
    let near: Vec<&Note> = show.notes.between(at - back, at + AHEAD).collect();
    let lit = presses(near.iter().copied(), at);
    let place = |key: u8| {
        let &(_, x, kw) = keys.get(usize::from(key.checked_sub(show.range.0)?))?;
        (key <= show.range.1).then_some((x + kw / 2., kw))
    };
    let glow = |c: Color| {
        if d.light() {
            c
        } else {
            c.lighten(0.16).scale_chroma(0.9)
        }
    };
    // Light rising from each lit key, behind everything that falls.
    if d.beams.strength > 0. {
        for p in &lit {
            let Some((cx, kw)) = place(p.note.key) else {
                continue;
            };
            let v = f64::from(p.note.velocity) / 127.;
            let a = (d.beams.strength * p.level * (0.45 + 0.55 * v) * (1. + 0.5 * flash(p.age)))
                .min(1.) as f32;
            let c = glow(tone(d, p.note, hands));
            let (tall, half) = (h * d.beams.height, kw * 0.45);
            let spread = unit * 0.9;
            let cone = Path::polyline(
                [
                    (cx - half, h),
                    (cx + half, h),
                    (cx + half + spread, h - tall),
                    (cx - half - spread, h - tall),
                ]
                .map(|(x, y)| Point::new(x, y)),
                true,
            );
            let stops = [
                (0., c.with_alpha(0.)),
                (0.55, c.with_alpha(a * 0.12)),
                (0.85, c.with_alpha(a * 0.32)),
                (1., c.with_alpha(a * 0.5)),
            ];
            light.push(Draw::fill(cone, Gradient::linear(180., stops)));
        }
    }
    // Bars: naturals first, so a sharp's lies over its neighbours', as its
    // key does; each sounding one in a bloom.
    let mut bars = Vec::with_capacity(near.len());
    for sharp in [false, true] {
        for n in near.iter().filter(|n| black(n.key) == sharp) {
            let Some(b) = bar(&keys, show.range, n, at, size, share(d, n)) else {
                continue;
            };
            bars.push((*n, b));
        }
    }
    if d.glow.bloom > 0. {
        for &(n, (x, y, bw, bh)) in &bars {
            if n.start <= at && n.end > at {
                let v = f64::from(n.velocity) / 127.;
                let a = d.glow.bloom * (0.5 + 0.5 * v) * (0.7 + 0.3 * flash(at - n.start));
                bloom(
                    &mut light,
                    (x + bw / 2., y, (y + bh).min(h + unit)),
                    bw,
                    unit * 0.55,
                    glow(tone(d, n, hands)),
                    (a * 0.42) as f32,
                );
            }
        }
    }
    for &(n, b) in &bars {
        note(&mut out, (d, hands), n, at, b, unit);
    }
    // The line the notes strike, and the light where each lands.
    strike(&mut out, d, size, unit, show.notes.pedal().depth(at));
    if d.glow.strength > 0. {
        for p in &lit {
            let Some((cx, _)) = place(p.note.key) else {
                continue;
            };
            let v = f64::from(p.note.velocity) / 127.;
            let f = flash(p.age) * d.glow.flash;
            let c = glow(tone(d, p.note, hands));
            let a = d.glow.strength * p.level * (0.4 + 0.6 * v);
            let r = d.glow.radius * unit * (1. + 0.22 * f);
            halo(
                &mut light,
                cx,
                h,
                r,
                c,
                (a * (0.55 + 0.3 * f).min(0.85)) as f32,
            );
            // The heart of it, white-hot as it strikes.
            let white = Color::oklch(0.98, 0.03, c.hue());
            let hot = if d.light() {
                c
            } else {
                c.mix(
                    white,
                    (d.glow.white_hot * (0.35 + 0.65 * flash(p.age))) as f32,
                )
            };
            halo(
                &mut light,
                cx,
                h,
                unit * 0.75,
                hot,
                (a * (0.55 + 0.45 * f).min(1.)) as f32,
            );
            flare(
                &mut light,
                (cx, h),
                unit * (0.4 + 1.1 * f),
                unit * 0.32,
                hot,
                (a * 0.85 * f.min(1.)) as f32,
            );
        }
    }
    // Sparks over it all.
    let mut flying: Vec<Spark> = Vec::new();
    let color = |n: &Note| glow(tone(d, n, hands));
    let ground = Ground {
        place: &place,
        y: h,
        unit,
    };
    sparks::sparks(
        near.iter().copied(),
        at,
        &ground,
        &d.particles,
        &color,
        &mut flying,
    );
    for s in &flying {
        // A hot point in a soft glow: one shape, its ramp shared by every
        // spark of its colour and brightness.
        let reach = s.radius * if d.light() { 1.6 } else { 3.2 };
        let core = (s.radius / reach) as f32;
        let c = s.color;
        let a = c.alpha();
        let stops = [
            (0., c.with_alpha(a)),
            (core * 0.6, c.with_alpha(a)),
            (core, c.with_alpha(a * 0.55)),
            (core + (1. - core) * 0.35, c.with_alpha(a * 0.16)),
            (1., c.with_alpha(0.)),
        ];
        light.push(Draw::fill(
            disc(s.x, s.y, reach),
            Gradient::radial((0.5, 0.5), 0.5, stops),
        ));
    }
    vignette(&mut out, d, size);
    // The pedal over the vignette: it is read, not just seen.
    pedal(&mut out, d, show.notes.pedal(), at, size, unit);
    (out, light)
}

/// The field: its gradient, and soft light drifting with the song.
fn field(out: &mut Vec<Draw>, d: &Design, at: f64, size: Size) {
    let (w, h) = (size.width, size.height);
    let b = &d.background;
    out.push(Draw::fill(
        rect(0., 0., w, h),
        Gradient::vertical(b.top.color(), b.bottom.color()),
    ));
    let turn = at * b.drift * std::f64::consts::TAU;
    for (i, c) in b.aurora.iter().enumerate() {
        let phase = i as f64 * 2.4;
        // Slow, out of step with each other, never still for long.
        let cx = w * (0.5 + 0.42 * (turn + phase).sin());
        let cy = h * (0.42 + 0.3 * (turn * 0.73 + phase * 1.7).sin());
        let r = w.max(h) * (0.34 + 0.06 * (turn * 0.51 + phase).cos());
        out.push(Draw::fill(
            rect(cx - r, cy - r, 2. * r, 2. * r),
            Gradient::radial(
                (0.5, 0.5),
                0.5,
                ramp(c.color(), b.aurora_strength as f32, 0.),
            ),
        ));
    }
}

/// The lanes: the sharps' shaded, a faint line at each C.
fn lanes(out: &mut Vec<Draw>, d: &Design, keys: &[(u8, f64, f64)], range: (u8, u8), size: Size) {
    let b = &d.background;
    let c = b.lanes.color();
    if b.sharp_lanes > 0. {
        for &(_, x, w) in keys.iter().filter(|k| black(k.0)) {
            out.push(Draw::fill(
                rect(x, 0., w, size.height),
                c.with_alpha(b.sharp_lanes as f32),
            ));
        }
    }
    if b.lane_strength > 0. {
        let a = b.lane_strength as f32;
        let fade = Gradient::vertical(c.with_alpha(a * 0.3), c.with_alpha(a));
        for &(n, x, _) in keys.iter().filter(|k| k.0 % 12 == 0 && k.0 != range.0) {
            let _ = n;
            out.push(Draw::fill(
                rect(x.round() - 0.5, 0., 1., size.height),
                fade.clone(),
            ));
        }
    }
}

/// The sustain pedal's lane down the left edge: how far down it is, as
/// the notes fall beside it.
fn pedal(out: &mut Vec<Draw>, d: &Design, pedal: &Pedal, at: f64, size: Size, unit: f64) {
    let p = &d.pedal;
    if p.strength <= 0. || pedal.is_empty() {
        return;
    }
    let h = size.height;
    let per_second = h / AHEAD;
    let lane = (p.width * unit).max(3.);
    let c = p.color.color();
    let a = p.strength as f32;
    out.push(Draw::fill(rect(0., 0., lane, h), c.with_alpha(a * 0.08)));
    let point =
        |(t, depth): (f64, f32)| Point::new(lane * f64::from(depth), h - (t - at) * per_second);
    let profile = pedal.profile(at - AHEAD * 0.1, at + AHEAD);
    let (Some(&first), Some(&last)) = (profile.first(), profile.last()) else {
        return;
    };
    let mut area = vec![Point::new(0., point(first).y)];
    area.extend(profile.iter().map(|&v| point(v)));
    area.push(Point::new(0., point(last).y));
    out.push(Draw::fill(
        Path::polyline(area, true),
        Gradient::linear(
            90.,
            [(0., c.with_alpha(a * 0.25)), (1., c.with_alpha(a * 0.6))],
        ),
    ));
    // Its edge, where it is down.
    let width = (unit * 0.05).clamp(1., 2.5);
    for run in profile.split(|v| v.1 <= 0.) {
        if run.len() > 1 {
            out.push(Draw::stroke(
                Path::polyline(run.iter().map(|&v| point(v)), false),
                c.with_alpha(a * 0.95),
                width,
            ));
        }
    }
    // Lit at the keys while it is down.
    let now = pedal.depth(at);
    if now > 0. {
        halo(out, lane / 2., h, lane * 1.8, c, a * 0.7 * now);
    }
}

/// One note's bar: its fill, a glint, its lit edge.
fn note(
    out: &mut Vec<Draw>,
    (d, hands): (&Design, bool),
    n: &Note,
    at: f64,
    (x, y, bw, bh): (f64, f64, f64, f64),
    unit: f64,
) {
    let s = &d.notes;
    let sounding = n.start <= at && n.end > at;
    let base = tone(d, n, hands);
    let face = if sounding {
        base.lighten(step(
            (s.active_boost + 0.08 * flash(at - n.start)) as f32,
            50.,
        ))
    } else {
        base
    };
    let r = (bw * s.roundness).min(bh / 2.);
    let amount = s.shading_amount as f32;
    let fill: Fill = match s.shading {
        Shading::None => face.into(),
        Shading::Across => Gradient::linear(
            90.,
            [
                (0., face.darken(amount * 0.2)),
                (0.5, face.lighten(amount * 0.08)),
                (1., face.darken(amount * 0.24)),
            ],
        )
        .into(),
        Shading::Along => {
            Gradient::vertical(face.darken(amount * 0.22), face.lighten(amount * 0.06)).into()
        }
    };
    out.push(Draw::fill(rounded(x, y, bw, bh, r), fill));
    let glint = bw * 0.1;
    if s.gloss > 0. && glint >= 0.75 && bh > 2. * r + 2. {
        let white = Color::oklch(1., 0., 0.).with_alpha((s.gloss * 0.6) as f32);
        out.push(Draw::fill(
            rounded(
                x + bw * 0.16,
                y + r * 0.8 + 1.,
                glint,
                bh - 1.6 * r - 2.,
                glint / 2.,
            ),
            white,
        ));
    }
    if s.rim_strength > 0. {
        let lw = (unit * 0.045).clamp(1., 3.);
        let rim = s.rim.color();
        let rim = if d.light() { rim } else { rim.mix(face, 0.35) };
        out.push(Draw::stroke(
            rounded(
                x + lw / 2.,
                y + lw / 2.,
                bw - lw,
                bh - lw,
                (r - lw / 2.).max(0.),
            ),
            rim.with_alpha((s.rim_strength * if sounding { 1. } else { 0.75 }) as f32),
            lw,
        ));
    }
}

/// The line along the top of the keys, a band of light above it, a little
/// brighter while the pedal is down.
fn strike(out: &mut Vec<Draw>, d: &Design, size: Size, unit: f64, pedal: f32) {
    let s = &d.strike;
    if s.strength <= 0. {
        return;
    }
    let (w, h) = (size.width, size.height);
    let c = s.color.color();
    let a = (s.strength as f32) * (0.85 + 0.15 * pedal);
    let band = unit * 0.7;
    out.push(Draw::fill(
        rect(0., h - band, w, band),
        Gradient::vertical(c.with_alpha(0.), c.with_alpha(a * 0.2)),
    ));
    let line = (unit * 0.06).clamp(1., 3.);
    out.push(Draw::fill(
        rect(0., h - line, w, line),
        c.with_alpha(a * 0.85),
    ));
}

/// The edges darkened, and the top fading into the field, so a note comes
/// into view gently.
fn vignette(out: &mut Vec<Draw>, d: &Design, size: Size) {
    let (w, h) = (size.width, size.height);
    let b = &d.background;
    let top = b.top.color();
    let fade = h * 0.1;
    out.push(Draw::fill(
        rect(0., 0., w, fade),
        Gradient::vertical(top.with_alpha(1.), top.with_alpha(0.)),
    ));
    if b.vignette > 0. {
        let dark = if d.light() {
            b.bottom.color().darken(0.3)
        } else {
            Color::oklch(0., 0., 0.)
        };
        let a = b.vignette as f32;
        let side = w * 0.14;
        let edge = [(0., dark.with_alpha(a * 0.7)), (1., dark.with_alpha(0.))];
        out.push(Draw::fill(
            rect(0., 0., side, h),
            Gradient::linear(90., edge),
        ));
        out.push(Draw::fill(
            rect(w - side, 0., side, h),
            Gradient::linear(270., edge),
        ));
        out.push(Draw::fill(
            rect(0., 0., w, h * 0.35),
            Gradient::vertical(dark.with_alpha(a * 0.45), dark.with_alpha(0.)),
        ));
    }
}

/// The falling notes filling their parent, which should be as wide as
/// the keybed under them; any size, as every length scales with it.
#[must_use]
pub fn falling(show: Show) -> El {
    let blend = match show.design.glow.blend {
        Blend::Normal => Mix::Normal,
        Blend::Screen => Mix::Screen,
        Blend::Multiply => Mix::Multiply,
    };
    let lit = show.clone();
    let face = canvas(move |s| draw(&lit, s).0)
        .w(Len::Pct(100.))
        .h(Len::Pct(100.));
    let light = canvas(move |s| draw(&show, s).1)
        .w(Len::Pct(100.))
        .h(Len::Pct(100.))
        .blend(blend);
    // A node clips what is inside it, not its own drawing: hence the stack.
    stack![face, light]
        .w(Len::Pct(100.))
        .h(Len::Pct(100.))
        .clip()
        .named("Falling notes")
}

/// The falling notes over a keybed showing `range`, `at` seconds into the
/// song, `height` tall and as wide as the keybed under it, as the default
/// Design draws them.
#[must_use]
pub fn waterfall(notes: &Notes, at: f64, range: (u8, u8), height: f64) -> El {
    waterfall_in(notes, at, range, height, design::named(""))
}

/// [`waterfall`] as `design` draws it.
#[must_use]
pub fn waterfall_in(
    notes: &Notes,
    at: f64,
    range: (u8, u8),
    height: f64,
    design: Arc<Design>,
) -> El {
    falling(Show {
        notes: notes.clone(),
        at,
        range,
        design,
    })
    .h(height)
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
        // Each Design lights them in its own colours.
        let ember = design::named("Ember");
        assert_ne!(notes.lit_in(2.2, &ember), notes.lit(2.2));
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
        // Seven naturals 10 wide; 100 px a second; 0.86 of its key.
        let c = bar(&keys, range, &note(1., 1.5, 48), 0., size, 0.86).unwrap();
        assert!(
            (c.0 - 0.7).abs() < 1e-9 && (c.2 - 8.6).abs() < 1e-9,
            "{c:?}"
        );
        assert!(
            (c.1 - 150.).abs() < 1e-9 && (c.3 - 50.).abs() < 1e-9,
            "{c:?}"
        );
        // A sharp's bar is narrower, over the seam between its naturals.
        let cs = bar(&keys, range, &note(1., 1.5, 49), 0., size, 0.86).unwrap();
        assert!(cs.0 > 7. && cs.0 + cs.2 < 13., "{cs:?}");
        // As it sounds, its foot is under the keys, out of sight.
        let now = bar(&keys, range, &note(1., 1.5, 48), 1.2, size, 0.86).unwrap();
        assert!((now.1 - 270.).abs() < 1e-9, "{now:?}");
        assert!((now.1 + now.3 - 300. - BLEED * 10.).abs() < 1e-9, "{now:?}");
        // A note longer than the view runs past its top, out of sight.
        let long = bar(&keys, range, &note(0., 9., 48), 1., size, 0.86).unwrap();
        assert!((long.1 + BLEED * 10.).abs() < 1e-9, "{long:?}");
        // Off the keybed, past, or not yet in view: nothing.
        assert!(bar(&keys, range, &note(1., 1.5, 47), 0., size, 0.86).is_none());
        assert!(bar(&keys, range, &note(1., 1.5, 60), 0., size, 0.86).is_none());
        assert!(bar(&keys, range, &note(4., 5., 50), 0., size, 0.86).is_none());
        // A blink of a note still shows, a quarter of its key tall.
        let blip = bar(&keys, range, &note(1., 1., 50), 0., size, 0.86).unwrap();
        assert!(blip.3 >= 2.5);
    }

    #[test]
    fn a_bar_scales_with_its_view() {
        // The same moment twice the size is the same picture twice as big.
        let range = (48, 59);
        let small = Size::new(70., 300.);
        let big = Size::new(140., 600.);
        let n = note(1., 1.5, 50);
        let a = bar(&layout(range, small.width), range, &n, 0.4, small, 0.8).unwrap();
        let b = bar(&layout(range, big.width), range, &n, 0.4, big, 0.8).unwrap();
        for (x, y) in [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)] {
            assert!((2. * x - y).abs() < 1e-9, "{a:?} {b:?}");
        }
    }

    #[test]
    fn a_loud_note_is_wider_and_lighter() {
        let d = Design::default();
        let soft = Note {
            velocity: 20,
            ..note(0., 1., 60)
        };
        let loud = Note {
            velocity: 127,
            ..note(0., 1., 60)
        };
        assert!(share(&d, &loud) > share(&d, &soft));
        assert!((share(&d, &loud) - d.notes.width).abs() < 1e-9);
        assert!(share(&d, &soft) > d.notes.width * 0.8, "slightly");
        assert!(tone(&d, &loud, false).lightness() > tone(&d, &soft, false).lightness());
        // The colours come in steps, so near velocities share one.
        let near = Note {
            velocity: 121,
            ..loud
        };
        assert_eq!(tone(&d, &near, false), tone(&d, &loud, false));
        // A one-voice song in hands: the left hand its own colour.
        let left = Note { key: 48, ..loud };
        assert_ne!(tone(&d, &left, true).hue(), tone(&d, &loud, true).hue());
        assert_eq!(tone(&d, &left, false).hue(), tone(&d, &loud, false).hue());
        let one = Notes::new(vec![loud, left]);
        assert!(one.by_hand(&d));
        let two = Notes::new(vec![loud, Note { voice: 1, ..left }]);
        assert!(!two.by_hand(&d), "a song with voices keeps them");
    }

    #[test]
    fn a_key_lights_while_its_note_sounds_and_fades_after() {
        let notes = [note(0., 1., 60), note(0.5, 2., 60), note(0.2, 0.4, 62)];
        let at = |t: f64| presses(notes.iter(), t);
        // Struck twice: the later note lights it.
        let p = at(0.6);
        let key60 = p.iter().find(|p| p.note.key == 60).unwrap();
        assert!((key60.note.start - 0.5).abs() < 1e-9 && key60.level == 1.);
        assert!((key60.age - 0.1).abs() < 1e-9);
        // Just after its note ends, the key still glows, dimmer...
        let fading = at(0.45);
        let key62 = fading.iter().find(|p| p.note.key == 62).unwrap();
        assert!(key62.level < 1. && key62.level > 0.3, "{}", key62.level);
        // ...and is dark once the release has passed.
        assert!(at(0.4 + RELEASE + 0.01).iter().all(|p| p.note.key != 62));
        assert!(at(5.).is_empty());
        // The strike's flash dies away quickly.
        assert!(flash(0.) == 1. && flash(0.5) < 0.02);
    }

    #[test]
    fn the_pedal_holds_its_depth_until_it_moves() {
        // Down at 1 s, half up at 2, up at 3; a second channel down at 2.5.
        let pedal = Pedal::new([
            (1., 0, 127),
            (2., 0, 64),
            (2.5, 1, 100),
            (3., 0, 0),
            (3., 1, 0),
            (4., 0, 0),
        ]);
        assert!(!pedal.is_empty());
        assert_eq!(pedal.depth(0.5), 0.);
        assert_eq!(pedal.depth(1.), 1.);
        assert!((pedal.depth(2.2) - 64. / 127.).abs() < 1e-6);
        assert!(
            (pedal.depth(2.7) - 100. / 127.).abs() < 1e-6,
            "the deeper channel"
        );
        assert_eq!(pedal.depth(3.5), 0.);
        let down: Vec<(f64, f64, f32)> = pedal.down(0., 10.).collect();
        assert_eq!(down.len(), 3, "{down:?}");
        assert_eq!((down[0].0, down[0].1), (1., 2.));
        assert_eq!((down[2].0, down[2].1), (2.5, 3.));
        // Cut to the window asked for.
        let cut: Vec<(f64, f64, f32)> = pedal.down(1.5, 2.2).collect();
        assert_eq!(cut.len(), 2, "{cut:?}");
        assert_eq!((cut[0].0, cut[1].1), (1.5, 2.2));
        assert!(Pedal::default().is_empty());
        assert!(Pedal::new([(1., 0, 0)]).is_empty());
        assert_eq!(Pedal::default().down(0., 5.).count(), 0);
        // As a line: a pedal that jumps steps, held flat between.
        let line = pedal.profile(0., 5.);
        assert_eq!(line.first(), Some(&(0., 0.)));
        assert_eq!(line.get(1..3), Some(&[(1., 0.), (1., 1.)][..]), "{line:?}");
        assert_eq!(line.last(), Some(&(5., 0.)));
        // A foot sent continuously, every 20 ms, slopes from one to the next.
        let foot = Pedal::new((0..=10).map(|i| (f64::from(i) * 0.02, 0, (i * 12) as u8)));
        let slope = foot.profile(-1., 1.);
        assert_eq!(slope.len(), 2 + 1 + 11, "{slope:?}");
        assert!(slope.windows(2).all(|w| w[1].0 >= w[0].0), "in time order");
        assert!(
            slope.windows(2).skip(2).all(|w| w[1].0 > w[0].0),
            "no steps"
        );
        // Notes carry their song's pedal.
        let notes = Notes::new(vec![note(0., 1., 60)]).with_pedal(pedal);
        assert_eq!(notes.pedal().depth(1.5), 1.);
    }

    /// What a frame draws of `notes` at `at` in `design`, on `size`.
    fn frame(notes: &Notes, at: f64, design: &str, size: Size) -> Vec<Draw> {
        let show = Show {
            notes: notes.clone(),
            at,
            range: (36, 107),
            design: design::named(design),
        };
        let (mut lit, light) = draw(&show, size);
        lit.extend(light);
        lit
    }

    #[test]
    fn a_frame_is_reproducible_and_bounded() {
        let mut many = Vec::new();
        for i in 0..4000 {
            let t = f64::from(i) * 0.005;
            many.push(Note {
                start: t,
                end: t + 0.6,
                key: 36 + (i % 72) as u8,
                velocity: 30 + (i % 97) as u8,
                voice: (i % 5) as u8,
            });
        }
        let pedal =
            Pedal::new((0..40).map(|i| (f64::from(i) * 0.5, 0, if i % 2 == 0 { 127 } else { 0 })));
        let notes = Notes::new(many).with_pedal(pedal);
        let size = Size::new(1180., 260.);
        for name in ["Midnight", "Neon", "Ivory", "Ember"] {
            let once = frame(&notes, 10., name, size);
            let again = frame(&notes, 10., name, size);
            assert_eq!(once.len(), again.len(), "{name}");
            assert!(
                once.iter()
                    .zip(&again)
                    .all(|(a, b)| a.fill == b.fill && a.path == b.path),
                "{name}: the same moment draws the same"
            );
            // At 200 notes a second, still a bounded picture: at most three
            // shapes a bar, two a spark, eight a lit key, and the field's.
            let bars = notes.between(10., 10. + AHEAD).count();
            let bound = 3 * bars + 2 * sparks::MAX + 8 * 128 + 64;
            assert!(once.len() <= bound, "{name}: {} of {bound}", once.len());
        }
        // Nothing playing yet: the field and its lines alone.
        let quiet = frame(&Notes::default(), 0., "Midnight", size);
        assert!(quiet.len() < 64, "{}", quiet.len());
    }

    #[test]
    fn the_field_drifts_with_the_song_and_stands_still_with_it() {
        let size = Size::new(800., 300.);
        let notes = Notes::default();
        let a = frame(&notes, 10., "Midnight", size);
        let b = frame(&notes, 10., "Midnight", size);
        let c = frame(&notes, 12., "Midnight", size);
        let paths = |v: &[Draw]| v.iter().map(|d| d.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&a), paths(&b), "a still song is a still picture");
        assert_ne!(paths(&a), paths(&c), "a moving one drifts");
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
