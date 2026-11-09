//! Designs: how a song's falling notes look, as data. A Design is a plain
//! struct of colours and strengths ([`Design`]), written as TOML a person
//! can read and edit, after piano-viz's Designs (kyan0s, MIT OR
//! Apache-2.0; THIRD_PARTY.md): a file lists only what it changes from
//! the default, unknown keys and values out of range come back as
//! warnings, never as a failure, and one file ([`MIDNIGHT`]) writes out
//! every setting as the reference.
//!
//! Sizes are not pixels: they are shares of a key's width or of the
//! view's height, so a Design looks the same in the editor's lane and on
//! a full-screen stage.
//!
//! Four ship with KONTRA ([`presets`]): Midnight (the default), Neon,
//! Ivory and Ember. Ember is warm by request: it is the one place an
//! orange is drawn, opted into by name; the default keeps DESIGN.md's No
//! Orange Rule.

use std::fmt;
use std::sync::{Arc, OnceLock};

use mui::prelude::Color;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The reference Design, every setting written out.
pub const MIDNIGHT: &str = include_str!("../designs/midnight.toml");

/// The Designs that ship, in the order the picker lists them.
const PRESETS: [&str; 4] = [
    MIDNIGHT,
    include_str!("../designs/neon.toml"),
    include_str!("../designs/ivory.toml"),
    include_str!("../designs/ember.toml"),
];

/// An sRGB colour with alpha, written `"#rrggbb"` or `"#rrggbbaa"` (or
/// the short `"#rgb"`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hex(pub [u8; 4]);

impl Hex {
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.trim().strip_prefix('#')?;
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let nibble = |i: usize| u8::from_str_radix(hex.get(i..=i)?, 16).ok().map(|v| v * 17);
        match hex.len() {
            3 => Some(Self([nibble(0)?, nibble(1)?, nibble(2)?, 255])),
            6 => Some(Self([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Some(Self([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => None,
        }
    }

    /// As the renderer takes it.
    #[must_use]
    pub fn color(self) -> Color {
        let [r, g, b, a] = self.0.map(|c| f32::from(c) / 255.);
        Color::srgba(r, g, b, a)
    }
}

impl fmt::Display for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            [r, g, b, 255] => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            [r, g, b, a] => write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}"),
        }
    }
}

impl fmt::Debug for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Serialize for Hex {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!("{s:?} is not a colour like \"#rrggbb\""))
        })
    }
}

/// A Design: the falling notes' whole look; its default is Midnight.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Design {
    pub meta: Meta,
    pub background: Background,
    pub notes: Bars,
    pub glow: Glow,
    pub beams: Beams,
    pub strike: Strike,
    pub particles: Particles,
    pub pedal: Pedal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    /// What the picker calls it.
    pub name: String,
    pub description: String,
}

/// The field the notes fall through.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Background {
    /// The field's colour at the top, and down at the keys.
    pub top: Hex,
    pub bottom: Hex,
    /// Soft light drifting slowly across the field; none for a plain one.
    pub aurora: Vec<Hex>,
    /// How strong that light is, 0..1.
    pub aurora_strength: f64,
    /// How fast it drifts: turns a second of the song, so it stands still
    /// when the song does.
    pub drift: f64,
    /// How much the edges darken, 0..1.
    pub vignette: f64,
    /// The faint line at each octave's C, and how strong; the sharps'
    /// lanes are shaded by `sharp_lanes`.
    pub lanes: Hex,
    pub lane_strength: f64,
    pub sharp_lanes: f64,
}

/// How a note's bar is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shading {
    /// One colour.
    None,
    /// Lighter at the end that reaches the keys first.
    Along,
    /// Lighter down the middle, like a rod lit from the front.
    #[default]
    Across,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bars {
    /// One colour a voice (a track and channel), over again past the last.
    pub palette: Vec<Hex>,
    /// A song one voice plays (most piano files) coloured by hand instead:
    /// the right hand the first colour, below middle C the second.
    pub hands: bool,
    /// How much lighter a loud note is than a soft one, 0..1.
    pub velocity_brightness: f64,
    /// The share of its width a soft note gives up, 0..0.5.
    pub velocity_width: f64,
    /// A sharp's bar darker by this factor, as its key is.
    pub sharp_shade: f64,
    /// A loud note's bar as a share of its key's width.
    pub width: f64,
    /// Its corners' radius as a share of its width: 0.5 rounds its ends.
    pub roundness: f64,
    pub shading: Shading,
    pub shading_amount: f64,
    /// The bar's lit edge, and how strong, 0..1.
    pub rim: Hex,
    pub rim_strength: f64,
    /// A glint down the bar's left side, 0..1.
    pub gloss: f64,
    /// Lighter while it sounds, by this much lightness.
    pub active_boost: f64,
}

/// The light where a note meets the keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Glow {
    /// 0..1; 0 for none.
    pub strength: f64,
    /// The halo's radius, in key widths.
    pub radius: f64,
    /// How much brighter and wider it flares as the note strikes.
    pub flash: f64,
    /// How white its heart burns at the strike, 0..1.
    pub white_hot: f64,
    /// The light around a sounding bar, 0..1.
    pub bloom: f64,
}

/// Light rising from a key while it sounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beams {
    /// 0..1; 0 for none.
    pub strength: f64,
    /// How high it reaches, a share of the view's height.
    pub height: f64,
}

/// The line the notes strike, along the top of the keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Strike {
    pub color: Hex,
    /// 0..1; 0 for none.
    pub strength: f64,
}

/// Where a spark takes its colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SparkColor {
    /// Its note's.
    #[default]
    Note,
    /// [`Particles::color`].
    Fixed,
}

/// Sparks thrown up from a struck key and embers rising while it is held
/// ([`crate::sparks`]). Lengths are in key widths, times in seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Particles {
    /// Sparks a note struck at full velocity throws.
    pub hit: f64,
    /// Embers a second while a note is held.
    pub hold: f64,
    pub lifetime: f64,
    /// The share of its lifetime a particle may lose at random, 0..1.
    pub lifetime_variance: f64,
    /// Key widths a second, up.
    pub speed: f64,
    pub speed_variance: f64,
    /// Degrees either side of straight up it may fly.
    pub spread: f64,
    /// Key widths a second squared, up; negative pulls it back down.
    pub gravity: f64,
    /// How quickly the air slows it, per second.
    pub drag: f64,
    /// Its radius, in key widths.
    pub size: f64,
    pub size_variance: f64,
    /// How far it wanders, in key widths a second.
    pub turbulence: f64,
    /// How white it is when new, 0..1.
    pub white_hot: f64,
    /// 0..1; 0 for none.
    pub strength: f64,
    pub source: SparkColor,
    pub color: Hex,
}

/// The sustain pedal's lane down the view's left edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pedal {
    pub color: Hex,
    /// 0..1; 0 hides the lane.
    pub strength: f64,
    /// Its width, in key widths.
    pub width: f64,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            name: "Midnight".into(),
            description: "Cool light on a night-blue field".into(),
        }
    }
}

impl Default for Background {
    fn default() -> Self {
        Self {
            top: Hex::rgb(0x0b, 0x10, 0x22),
            bottom: Hex::rgb(0x05, 0x07, 0x10),
            aurora: vec![Hex::rgb(0x2f, 0x5b, 0xff), Hex::rgb(0x14, 0xb8, 0xa6)],
            aurora_strength: 0.16,
            drift: 0.02,
            vignette: 0.45,
            lanes: Hex::rgb(0xc8, 0xd6, 0xff),
            lane_strength: 0.07,
            sharp_lanes: 0.012,
        }
    }
}

impl Default for Bars {
    fn default() -> Self {
        Self {
            // OKLCH hues 250, 165, 305, 205, 340 and 135: blue (the first
            // part's), teal, violet, cyan, magenta, green. No amber.
            palette: vec![
                Hex::rgb(0x5b, 0x9b, 0xff),
                Hex::rgb(0x2f, 0xd3, 0xae),
                Hex::rgb(0xb3, 0x88, 0xff),
                Hex::rgb(0x2e, 0xc4, 0xf0),
                Hex::rgb(0xf0, 0x6f, 0xc4),
                Hex::rgb(0x6c, 0xd6, 0x6a),
            ],
            hands: true,
            velocity_brightness: 0.45,
            velocity_width: 0.14,
            sharp_shade: 0.82,
            width: 0.84,
            roundness: 0.32,
            shading: Shading::Across,
            shading_amount: 0.32,
            rim: Hex::rgb(0xff, 0xff, 0xff),
            rim_strength: 0.3,
            gloss: 0.16,
            active_boost: 0.1,
        }
    }
}

impl Default for Glow {
    fn default() -> Self {
        Self {
            strength: 0.85,
            radius: 2.4,
            flash: 1.0,
            white_hot: 0.6,
            bloom: 0.45,
        }
    }
}

impl Default for Beams {
    fn default() -> Self {
        Self {
            strength: 0.4,
            height: 0.6,
        }
    }
}

impl Default for Strike {
    fn default() -> Self {
        Self {
            color: Hex::rgb(0x9e, 0xc0, 0xff),
            strength: 0.55,
        }
    }
}

impl Default for Particles {
    fn default() -> Self {
        Self {
            hit: 10.,
            hold: 5.,
            lifetime: 1.1,
            lifetime_variance: 0.45,
            speed: 5.,
            speed_variance: 0.6,
            spread: 38.,
            gravity: -2.4,
            drag: 1.1,
            size: 0.1,
            size_variance: 0.5,
            turbulence: 0.5,
            white_hot: 0.55,
            strength: 0.9,
            source: SparkColor::Note,
            color: Hex::rgb(0xd6, 0xe4, 0xff),
        }
    }
}

impl Default for Pedal {
    fn default() -> Self {
        Self {
            color: Hex::rgb(0x8f, 0xb0, 0xff),
            strength: 0.75,
            width: 0.9,
        }
    }
}

impl Design {
    /// The Design in `src`, and what was wrong with it but survived:
    /// settings it does not know, values out of range (clamped). Err only
    /// when it is not TOML or a value has the wrong type.
    pub fn from_toml(src: &str) -> Result<(Self, Vec<String>), String> {
        let raw: toml::Table = src.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let mut design: Self = toml::from_str(src).map_err(|e| e.to_string())?;
        let mut warnings = Vec::new();
        // What does not survive a trip through the struct was not understood.
        if let Ok(known) = toml::Table::try_from(&design) {
            unknown(&raw, &known, "", &mut warnings);
        }
        warnings.extend(design.sanitize());
        Ok((design, warnings))
    }

    /// This Design as TOML, every setting written out.
    #[must_use]
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Clamps every number into its range; what it changed.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let mut clamp = |name: &str, v: &mut f64, lo: f64, hi: f64| {
            let c = if v.is_finite() { v.clamp(lo, hi) } else { lo };
            if c != *v {
                out.push(format!("{name} = {v} is out of range; using {c}"));
                *v = c;
            }
        };
        let b = &mut self.background;
        clamp("background.aurora_strength", &mut b.aurora_strength, 0., 1.);
        clamp("background.drift", &mut b.drift, 0., 1.);
        clamp("background.vignette", &mut b.vignette, 0., 1.);
        clamp("background.lane_strength", &mut b.lane_strength, 0., 1.);
        clamp("background.sharp_lanes", &mut b.sharp_lanes, 0., 1.);
        let n = &mut self.notes;
        clamp(
            "notes.velocity_brightness",
            &mut n.velocity_brightness,
            0.,
            1.,
        );
        clamp("notes.velocity_width", &mut n.velocity_width, 0., 0.5);
        clamp("notes.sharp_shade", &mut n.sharp_shade, 0.2, 1.5);
        clamp("notes.width", &mut n.width, 0.2, 1.);
        clamp("notes.roundness", &mut n.roundness, 0., 0.5);
        clamp("notes.shading_amount", &mut n.shading_amount, 0., 1.);
        clamp("notes.rim_strength", &mut n.rim_strength, 0., 1.);
        clamp("notes.gloss", &mut n.gloss, 0., 1.);
        clamp("notes.active_boost", &mut n.active_boost, 0., 0.5);
        let g = &mut self.glow;
        clamp("glow.strength", &mut g.strength, 0., 1.);
        clamp("glow.radius", &mut g.radius, 0., 12.);
        clamp("glow.flash", &mut g.flash, 0., 3.);
        clamp("glow.white_hot", &mut g.white_hot, 0., 1.);
        clamp("glow.bloom", &mut g.bloom, 0., 1.);
        clamp("beams.strength", &mut self.beams.strength, 0., 1.);
        clamp("beams.height", &mut self.beams.height, 0., 1.);
        clamp("strike.strength", &mut self.strike.strength, 0., 1.);
        let p = &mut self.particles;
        clamp("particles.hit", &mut p.hit, 0., 64.);
        clamp("particles.hold", &mut p.hold, 0., 60.);
        clamp("particles.lifetime", &mut p.lifetime, 0.05, 6.);
        clamp(
            "particles.lifetime_variance",
            &mut p.lifetime_variance,
            0.,
            1.,
        );
        clamp("particles.speed", &mut p.speed, 0., 40.);
        clamp("particles.speed_variance", &mut p.speed_variance, 0., 1.);
        clamp("particles.spread", &mut p.spread, 0., 180.);
        clamp("particles.gravity", &mut p.gravity, -100., 100.);
        clamp("particles.drag", &mut p.drag, 0., 50.);
        clamp("particles.size", &mut p.size, 0., 1.);
        clamp("particles.size_variance", &mut p.size_variance, 0., 1.);
        clamp("particles.turbulence", &mut p.turbulence, 0., 10.);
        clamp("particles.white_hot", &mut p.white_hot, 0., 1.);
        clamp("particles.strength", &mut p.strength, 0., 1.);
        clamp("pedal.strength", &mut self.pedal.strength, 0., 1.);
        clamp("pedal.width", &mut self.pedal.width, 0., 4.);
        if self.notes.palette.is_empty() {
            out.push("notes.palette is empty; using the default".into());
            self.notes.palette = Bars::default().palette;
        }
        out
    }

    /// The colour of a note voice `voice` plays at `velocity`: its colour
    /// in the palette, lighter the harder it is played; a sharp's darker,
    /// as its key is.
    #[must_use]
    pub fn voice(&self, voice: u8, velocity: u8, sharp: bool) -> Color {
        let n = &self.notes;
        let base = n
            .palette
            .get(usize::from(voice) % n.palette.len().max(1))
            .copied()
            .unwrap_or(Hex::rgb(0x80, 0x80, 0x80))
            .color();
        // Mezzo-forte (about 90) is the palette's own colour.
        let v = f64::from(velocity.min(127)) / 127. - 0.7;
        let shift = n.velocity_brightness * v * 0.42;
        let lightness = (f64::from(base.lightness()) + shift).clamp(0.05, 0.97);
        let lightness = if sharp {
            lightness * n.sharp_shade
        } else {
            lightness
        };
        let chroma = (1. + n.velocity_brightness * v * 0.6).max(0.2);
        base.with_lightness(lightness.clamp(0.02, 0.98) as f32)
            .scale_chroma(chroma as f32)
    }

    /// Whether its field is light: glows then tint rather than shine.
    #[must_use]
    pub fn light(&self) -> bool {
        self.background.bottom.color().lightness() > 0.6
    }
}

/// Settings in `raw` that `known` (the same, read and written back) lost.
fn unknown(raw: &toml::Table, known: &toml::Table, prefix: &str, out: &mut Vec<String>) {
    for (k, v) in raw {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match (v, known.get(k)) {
            (_, None) => out.push(format!("unknown setting `{path}` was ignored")),
            (toml::Value::Table(a), Some(toml::Value::Table(b))) => unknown(a, b, &path, out),
            _ => {}
        }
    }
}

/// The Designs that ship, read once.
#[must_use]
pub fn presets() -> &'static [Arc<Design>] {
    static PRESET: OnceLock<Vec<Arc<Design>>> = OnceLock::new();
    PRESET.get_or_init(|| {
        PRESETS
            .iter()
            .map(|src| Arc::new(Design::from_toml(src).map(|d| d.0).unwrap_or_default()))
            .collect()
    })
}

/// The Design called `name` (any case), else the default: a name saved by
/// a build that had one this build lacks still opens.
#[must_use]
pub fn named(name: &str) -> Arc<Design> {
    let all = presets();
    all.iter()
        .find(|d| d.meta.name.eq_ignore_ascii_case(name.trim()))
        .or(all.first())
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_read_and_write_as_hex() {
        assert_eq!(Hex::parse("#0b1022"), Some(Hex::rgb(0x0b, 0x10, 0x22)));
        assert_eq!(Hex::parse("#fff"), Some(Hex::rgb(255, 255, 255)));
        assert_eq!(
            Hex::parse(" #11223344 "),
            Some(Hex([0x11, 0x22, 0x33, 0x44]))
        );
        assert_eq!(Hex::parse("0b1022"), None, "a colour starts with #");
        assert_eq!(Hex::parse("#0b10"), None);
        assert_eq!(Hex::parse("#zzzzzz"), None);
        assert_eq!(Hex([1, 2, 3, 4]).to_string(), "#01020304");
        assert_eq!(Hex::rgb(255, 0, 16).to_string(), "#ff0010");
        let white = Hex::rgb(255, 255, 255).color();
        assert!(white.lightness() > 0.99 && white.alpha() > 0.99);
    }

    #[test]
    fn the_presets_are_four_and_the_reference_is_the_default() {
        let all = presets();
        let names: Vec<&str> = all.iter().map(|d| d.meta.name.as_str()).collect();
        assert_eq!(names, ["Midnight", "Neon", "Ivory", "Ember"]);
        // Every preset reads cleanly: no unknown settings, nothing clamped.
        for src in PRESETS {
            let (d, warnings) = Design::from_toml(src).unwrap();
            assert!(warnings.is_empty(), "{}: {warnings:?}", d.meta.name);
        }
        // The reference file writes out exactly the defaults.
        assert_eq!(*named("Midnight"), Design::default());
        assert_eq!(named("ivory").meta.name, "Ivory");
        assert_eq!(named("no such design").meta.name, "Midnight");
        assert!(named("Ivory").light());
        assert!(!named("Neon").light() && !named("Midnight").light());
    }

    #[test]
    fn a_design_lists_only_what_it_changes_and_round_trips() {
        let (d, warnings) =
            Design::from_toml("[meta]\nname = \"Mine\"\n[glow]\nstrength = 0.2\n").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(d.meta.name, "Mine");
        assert!((d.glow.strength - 0.2).abs() < 1e-12);
        assert_eq!(d.notes, Bars::default(), "the rest is the default");
        let (again, warnings) = Design::from_toml(&d.to_toml()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again, d);
        for d in presets() {
            assert_eq!(Design::from_toml(&d.to_toml()).unwrap().0, **d);
        }
    }

    #[test]
    fn mistakes_warn_and_wrong_types_fail() {
        let (d, warnings) = Design::from_toml(
            "[glow]\nstrength = 4.0\nsparkle = 1\n[notes]\npalette = []\n[fog]\non = true\n",
        )
        .unwrap();
        assert!((d.glow.strength - 1.).abs() < 1e-12, "clamped");
        assert!(!d.notes.palette.is_empty());
        let said = warnings.join("\n");
        for word in ["glow.sparkle", "`fog`", "glow.strength", "notes.palette"] {
            assert!(said.contains(word), "{word} in {said}");
        }
        assert!(Design::from_toml("[glow]\nstrength = \"lots\"\n").is_err());
        assert!(Design::from_toml("[notes]\nrim = \"red\"\n").is_err());
        assert!(Design::from_toml("not = toml = at all").is_err());
    }

    #[test]
    fn louder_is_lighter_sharps_darker_and_voices_differ() {
        for d in presets() {
            let soft = d.voice(0, 30, false);
            let hard = d.voice(0, 120, false);
            assert!(hard.lightness() > soft.lightness(), "{}", d.meta.name);
            assert!(d.voice(0, 120, true).lightness() < hard.lightness());
            assert_ne!(d.voice(0, 100, false), d.voice(1, 100, false));
        }
        // The default keeps DESIGN.md's No Orange Rule: no amber, no coral.
        let d = Design::default();
        for voice in 0..=40 {
            for velocity in [1, 64, 127] {
                let hue = d.voice(voice, velocity, false).hue();
                assert!(!(15.0..105.0).contains(&hue), "voice {voice}: {hue}");
            }
        }
    }
}
