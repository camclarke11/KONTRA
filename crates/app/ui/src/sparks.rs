//! Sparks thrown up by a struck key and embers rising from a held one.
//!
//! Stateless, as piano-viz draws its particles (kyan0s, MIT OR
//! Apache-2.0; THIRD_PARTY.md), whose hash, flight and curves this ports
//! from its WGSL to Rust: a particle is a closed-form function of its
//! note, its number and the song's time. Nothing is stored or stepped
//! from frame to frame, so a moment of the song looks the same however it
//! was reached and at any frame rate, a render is reproducible, and the
//! cost is bounded by what is on screen ([`MAX`]), never by the song.

use mui::prelude::Color;

use crate::design::{Particles, SparkColor};
use crate::waterfall::Note;

/// The most particles drawn at once, the newest notes' first.
pub const MAX: usize = 600;

/// One particle where it is now, its colour carrying how faded it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spark {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub color: Color,
}

/// PCG hash: well spread, cheap, the same everywhere.
fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}

/// A stream of numbers in 0..1 from a seed.
struct Seed(u32);

impl Seed {
    fn next(&mut self) -> f64 {
        self.0 = pcg(self.0);
        f64::from(self.0 >> 8) / 16_777_216.
    }
}

/// A note's own seed: by when and where it is, not where it is in a list.
fn identity(n: &Note) -> u32 {
    let bits = n.start.to_bits();
    let (low, high) = (bits as u32, (bits >> 32) as u32);
    pcg(low ^ pcg(high) ^ u32::from(n.key).wrapping_mul(0x9E37_79B1))
}

/// Where a particle is `t` seconds after leaving with velocity `v0`,
/// under gravity `g` and linear drag `k`: the integral of dv/dt = g - kv,
/// with no time step.
fn ballistic(v0: (f64, f64), g: (f64, f64), k: f64, t: f64) -> (f64, f64) {
    if k < 1e-4 {
        return (v0.0 * t + 0.5 * g.0 * t * t, v0.1 * t + 0.5 * g.1 * t * t);
    }
    let e = 1. - (-k * t).exp();
    (
        (v0.0 + g.0 / k) * (e / k) - g.0 * t / k,
        (v0.1 + g.1 / k) * (e / k) - g.1 * t / k,
    )
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// How a burst flies; the embers a held key gives off fly as a gentler,
/// buoyant version of the same.
#[derive(Clone, Copy)]
struct Flight {
    lifetime: f64,
    speed: f64,
    spread: f64,
    gravity: f64,
    size: f64,
    turbulence: f64,
    white_hot: f64,
}

impl Flight {
    fn hit(p: &Particles) -> Self {
        Self {
            lifetime: p.lifetime,
            speed: p.speed,
            spread: p.spread,
            gravity: p.gravity,
            size: p.size,
            turbulence: p.turbulence,
            white_hot: p.white_hot,
        }
    }

    fn hold(p: &Particles) -> Self {
        Self {
            lifetime: p.lifetime * 1.6,
            speed: p.speed * 0.3,
            spread: p.spread * 0.5,
            gravity: p.gravity.abs() * 0.35,
            size: p.size * 0.75,
            turbulence: p.turbulence * 1.6,
            white_hot: p.white_hot * 0.4,
        }
    }
}

/// What sparks are drawn on: where each key is and how big a key is.
pub struct Ground<'a> {
    /// A key's centre and width, if it is shown.
    pub place: &'a dyn Fn(u8) -> Option<(f64, f64)>,
    /// The line the notes strike, where sparks start.
    pub y: f64,
    /// A natural key's width: sparks' lengths are in these.
    pub unit: f64,
}

/// The particles alive `at` seconds into the song, from `notes` (in start
/// order: the newest win when there are more than [`MAX`]), coloured by
/// `color`, added to `out`.
pub fn sparks<'n>(
    notes: impl DoubleEndedIterator<Item = &'n Note>,
    at: f64,
    ground: &Ground<'_>,
    p: &Particles,
    color: &dyn Fn(&Note) -> Color,
    out: &mut Vec<Spark>,
) {
    if p.strength <= 0. || ground.unit <= 0. {
        return;
    }
    let (hit, hold) = (Flight::hit(p), Flight::hold(p));
    let start = out.len();
    let full = |out: &Vec<Spark>| out.len() - start >= MAX;
    for n in notes.rev() {
        if full(out) {
            break;
        }
        if n.start > at {
            continue;
        }
        let Some((cx, kw)) = (ground.place)(n.key) else {
            continue;
        };
        let v = f64::from(n.velocity.min(127)) / 127.;
        // A louder note throws more, faster and bigger.
        let vk = 0.35 + 0.65 * v;
        let id = identity(n);
        let base = match p.source {
            SparkColor::Note => color(n),
            SparkColor::Fixed => p.color.color(),
        };
        let one = |flight: &Flight, k: u32, spawn: f64, out: &mut Vec<Spark>| {
            let mut seed = Seed(pcg(id ^ pcg(k)));
            let life = flight.lifetime * (1. - p.lifetime_variance * seed.next());
            let age = at - spawn;
            if age < 0. || age > life || life <= 0. {
                return;
            }
            let s = age / life;
            let x0 = cx + (seed.next() - 0.5) * kw * 0.8;
            let angle = (90. + (seed.next() - 0.5) * 2. * flight.spread).to_radians();
            let unit = ground.unit;
            let speed = flight.speed * unit * (1. - p.speed_variance * seed.next()) * vk;
            let v0 = (angle.cos() * speed, angle.sin() * speed);
            let (dx, dy) = ballistic(v0, (0., flight.gravity * unit), p.drag, age);
            // A drift that grows with age: a burst leaves together and
            // wanders apart.
            let (px, py) = (
                seed.next() * std::f64::consts::TAU,
                seed.next() * std::f64::consts::TAU,
            );
            let rate = 1. + seed.next() * 2.;
            let wander = flight.turbulence * unit * age;
            let x = x0 + dx + (px + age * rate).sin() * wander;
            // Up the screen is up: y grows downwards.
            let y = ground.y - dy - (py + age * rate * 0.7).cos() * wander;
            let radius =
                flight.size * unit * (1. - p.size_variance * seed.next()) * (1. - s) * vk.sqrt();
            let alpha = (1. - smoothstep(0.5, 1., s)) * p.strength;
            if radius < 0.05 || alpha < 0.004 {
                return;
            }
            // Heat and fade in steps: sparks alike share one gradient.
            let heat = ((flight.white_hot * (1. - s) * (1. - s)) * 4.).round() / 4.;
            let alpha = ((alpha * 10.).ceil() / 10.).min(1.);
            let white = Color::oklch(0.98, 0.02, base.hue());
            out.push(Spark {
                x,
                y,
                radius,
                color: base.mix(white, heat as f32).with_alpha(alpha as f32),
            });
        };
        // The burst as it strikes.
        let count = (p.hit * vk).round().max(0.) as u32;
        for k in 0..count {
            if full(out) {
                break;
            }
            one(&hit, k * 2, n.start, out);
        }
        // Embers while it is held, the ones still alive: born at a steady
        // rate from its start, so each is the same ember every frame.
        if p.hold > 0. {
            let rate = p.hold;
            let first = ((at - hold.lifetime - n.start) * rate).ceil().max(0.) as u32;
            let last = (n.end.min(at) - n.start) * rate;
            let mut k = first;
            while f64::from(k) <= last && !full(out) {
                one(&hold, k * 2 + 1, n.start + f64::from(k) / rate, out);
                k += 1;
            }
        }
    }
}

/// How long after a note starts its particles may still be alive.
#[must_use]
pub fn reach(p: &Particles) -> f64 {
    Flight::hold(p).lifetime.max(Flight::hit(p).lifetime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::Design;

    fn note(start: f64, end: f64, key: u8, velocity: u8) -> Note {
        Note {
            start,
            end,
            key,
            velocity,
            voice: 0,
        }
    }

    fn at(notes: &[Note], t: f64, p: &Particles) -> Vec<Spark> {
        let place = |key: u8| Some((f64::from(key) * 10., 10.));
        let ground = Ground {
            place: &place,
            y: 300.,
            unit: 10.,
        };
        let mut out = Vec::new();
        let white = |_: &Note| Color::oklch(0.7, 0.1, 250.);
        sparks(notes.iter(), t, &ground, p, &white, &mut out);
        out
    }

    #[test]
    fn the_hash_is_pcg() {
        // PCG's output for these inputs, as piano-viz's shader computes it.
        assert_eq!(pcg(0), 129_708_002);
        assert_eq!(pcg(1), 2_831_084_092);
        let mut seed = Seed(7);
        let draws: Vec<f64> = (0..1000).map(|_| seed.next()).collect();
        assert!(draws.iter().all(|d| (0.0..1.0).contains(d)));
        let mean = draws.iter().sum::<f64>() / 1000.;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
    }

    #[test]
    fn flight_is_closed_form() {
        // No drag: the schoolbook parabola.
        let (x, y) = ballistic((1., 2.), (0., -10.), 0., 0.5);
        assert!((x - 0.5).abs() < 1e-12 && (y - (1. - 1.25)).abs() < 1e-12);
        // With drag it travels less far, and tends to a terminal drift.
        let (_, slow) = ballistic((0., 10.), (0., 0.), 2., 1.);
        assert!(slow < 10. && slow > 0.);
        let (_, far) = ballistic((0., 10.), (0., 0.), 2., 100.);
        assert!((far - 5.).abs() < 1e-9, "v0/k");
        assert_eq!(ballistic((3., 4.), (1., 1.), 1.5, 0.), (0., 0.));
    }

    #[test]
    fn sparks_are_the_same_however_the_moment_is_reached() {
        let p = Design::default().particles;
        let notes = [note(1., 2., 60, 100), note(1.2, 1.5, 64, 60)];
        let once = at(&notes, 1.4, &p);
        assert!(!once.is_empty());
        // Other moments drawn in between change nothing.
        for t in [0.5, 3., 1.39, 1.41] {
            let _ = at(&notes, t, &p);
        }
        assert_eq!(at(&notes, 1.4, &p), once);
        // A note's sparks are its own: another note added elsewhere in
        // the song leaves them as they were.
        let more = [note(0.1, 0.2, 40, 90), notes[0], notes[1]];
        assert_eq!(at(&more, 1.4, &p), once);
    }

    #[test]
    fn a_strike_throws_sparks_up_that_fade_and_are_gone() {
        let mut p = Design::default().particles;
        p.hold = 0.;
        let notes = [note(1., 1.05, 60, 127)];
        assert!(at(&notes, 0.99, &p).is_empty(), "not before it strikes");
        let born = at(&notes, 1.0, &p);
        assert_eq!(born.len() as f64, p.hit, "all of them at full velocity");
        for s in &born {
            assert!((s.y - 300.).abs() < 1e-9, "at the strike line");
            assert!((s.x - 600.).abs() <= 4. + 1e-9, "within the key: {}", s.x);
        }
        let flying = at(&notes, 1.3, &p);
        assert!(flying.iter().any(|s| s.y < 300. - 5.), "they rise");
        let mean = |v: &[Spark]| v.iter().map(|s| f64::from(s.color.alpha())).sum::<f64>();
        assert!(mean(&at(&notes, 1.9, &p)) < mean(&flying), "and fade");
        assert!(at(&notes, 1. + p.lifetime + 0.01, &p).is_empty(), "and go");
        // A soft note throws fewer.
        let soft = at(&[note(1., 1.05, 60, 20)], 1.0, &p);
        assert!(soft.len() < born.len() && !soft.is_empty());
    }

    #[test]
    fn a_held_note_smoulders_and_stops_when_released() {
        let mut p = Design::default().particles;
        p.hit = 0.;
        let held = [note(0., 10., 60, 100)];
        let rate = p.hold * Flight::hold(&p).lifetime;
        let alive = at(&held, 5., &p).len() as f64;
        assert!(alive > 0. && alive <= rate.ceil() + 1., "{alive} of {rate}");
        let released = [note(0., 2., 60, 100)];
        let after = at(&released, 2. + reach(&p) + 0.01, &p);
        assert!(after.is_empty());
    }

    #[test]
    fn however_dense_the_song_the_count_is_bounded() {
        let p = Design::default().particles;
        let notes: Vec<Note> = (0..2000)
            .map(|i| note(f64::from(i) * 0.001, 5., 21 + (i % 88) as u8, 127))
            .collect();
        let all = at(&notes, 2.05, &p);
        assert_eq!(all.len(), MAX);
        // The newest notes' sparks are the ones kept.
        let newest = at(&notes[1990..], 2.05, &p);
        assert!(newest.iter().all(|s| all.contains(s)));
        // And none when they are switched off.
        let off = Particles {
            strength: 0.,
            ..p.clone()
        };
        assert!(at(&notes, 2.05, &off).is_empty());
    }
}
