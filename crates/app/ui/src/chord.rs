//! Chord names: the chord sounding at a moment of a song ([`chord_at`]),
//! and the template match it rests on ([`identify`]), which a song's
//! auto-pedal uses to hear where the harmony changes.
//!
//! Each pitch class is weighed by how much it sounds: a note counts more
//! the longer it has rung (so a passing tone just struck weighs little
//! against the chord it passes over) and less as it dies away, louder and
//! lower notes more. The best of the chord templates (triads, suspensions,
//! sixths and sevenths) on each root wins: what it holds counts for it,
//! what it lacks and what sounds outside it against it, and the bass votes
//! for the root. A bass that is not the root is named after a slash: an
//! inversion (C/E) or a slash chord (C/D).

use crate::waterfall::Notes;

/// The kinds of chord named, simplest first: the first of equal matches
/// wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Major,
    Minor,
    Seventh,
    Major7,
    Minor7,
    Sus4,
    Sus2,
    Dim,
    Aug,
    HalfDim7,
    Dim7,
    Sixth,
    Minor6,
}

impl Kind {
    pub const ALL: [Self; 13] = [
        Self::Major,
        Self::Minor,
        Self::Seventh,
        Self::Major7,
        Self::Minor7,
        Self::Sus4,
        Self::Sus2,
        Self::Dim,
        Self::Aug,
        Self::HalfDim7,
        Self::Dim7,
        Self::Sixth,
        Self::Minor6,
    ];

    /// Its notes, in semitones above the root.
    #[must_use]
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Self::Major => &[0, 4, 7],
            Self::Minor => &[0, 3, 7],
            Self::Seventh => &[0, 4, 7, 10],
            Self::Major7 => &[0, 4, 7, 11],
            Self::Minor7 => &[0, 3, 7, 10],
            Self::Sus4 => &[0, 5, 7],
            Self::Sus2 => &[0, 2, 7],
            Self::Dim => &[0, 3, 6],
            Self::Aug => &[0, 4, 8],
            Self::HalfDim7 => &[0, 3, 6, 10],
            Self::Dim7 => &[0, 3, 6, 9],
            Self::Sixth => &[0, 4, 7, 9],
            Self::Minor6 => &[0, 3, 7, 9],
        }
    }

    /// What follows the root in its name.
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Major => "",
            Self::Minor => "m",
            Self::Seventh => "7",
            Self::Major7 => "maj7",
            Self::Minor7 => "m7",
            Self::Sus4 => "sus4",
            Self::Sus2 => "sus2",
            Self::Dim => "dim",
            Self::Aug => "aug",
            Self::HalfDim7 => "m7b5",
            Self::Dim7 => "dim7",
            Self::Sixth => "6",
            Self::Minor6 => "m6",
        }
    }

    /// What naming it costs: a rarer chord needs more evidence, and a
    /// suspension, having no third, loses to a triad that has one (C E G
    /// over D is C/D, not Gsus4/D).
    fn cost(self) -> f64 {
        match self {
            Self::Major | Self::Minor => 0.0,
            Self::Seventh | Self::Major7 | Self::Minor7 => 0.05,
            Self::Dim | Self::HalfDim7 | Self::Dim7 | Self::Sixth => 0.1,
            Self::Minor6 => 0.12,
            Self::Aug => 0.15,
            Self::Sus4 => 0.25,
            Self::Sus2 => 0.3,
        }
    }

    /// Its third is minor: its root is spelled as a minor key's.
    fn minor(self) -> bool {
        matches!(
            self,
            Self::Minor | Self::Minor7 | Self::Minor6 | Self::Dim | Self::HalfDim7 | Self::Dim7
        )
    }
}

/// A chord: its root and bass as pitch classes (0 is C) and its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub root: u8,
    pub kind: Kind,
    pub bass: u8,
}

impl Chord {
    /// Whether pitch class `pc` (0 is C) is one of its notes.
    #[must_use]
    pub fn contains(self, pc: u8) -> bool {
        (self.kind.intervals().iter()).any(|i| (self.root + i) % 12 == pc % 12)
    }

    /// It is spelled with flats: its root is, or its key has them.
    fn flats(self) -> bool {
        let minor = self.kind.minor();
        let key = match self.root {
            1 | 8 => !minor,
            3 | 5 | 10 => true,
            0 | 2 | 7 => minor,
            _ => false,
        };
        key || matches!(self.kind, Kind::Dim | Kind::Dim7 | Kind::HalfDim7)
            || (self.root == 0 && self.kind == Kind::Seventh)
    }
}

const SHARPS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const FLATS: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

/// A root's name: as its key spells it, which for F# and B is sharp and
/// for Eb and Bb flat whatever the chord, and for Db/C# and Ab/G# depends
/// on whether the chord is major or minor.
fn root_name(pc: u8, minor: bool) -> &'static str {
    let name = match pc % 12 {
        1 | 8 if minor => SHARPS.get(usize::from(pc % 12)),
        6 => SHARPS.get(6),
        _ => FLATS.get(usize::from(pc % 12)),
    };
    name.copied().unwrap_or("?")
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let root = root_name(self.root, self.kind.minor());
        write!(f, "{root}{}", self.kind.suffix())?;
        if self.bass != self.root {
            let names = if self.flats() { &FLATS } else { &SHARPS };
            let bass = names
                .get(usize::from(self.bass % 12))
                .copied()
                .unwrap_or("?");
            write!(f, "/{bass}")?;
        }
        Ok(())
    }
}

/// A pitch class sounds when it weighs this share of the heaviest.
const PRESENT: f64 = 0.15;
/// What a pitch class sounding outside a chord costs it, for its weight.
const OUTSIDE: f64 = 0.6;
/// What a chord's note that does not sound costs it.
const MISSING: f64 = 0.55;
/// What the bass being the root, or in the chord, adds; what a bass
/// outside the chord costs, a slash chord, its own weight not counted
/// against the chord.
const ROOT_BASS: f64 = 0.35;
const IN_BASS: f64 = 0.2;
const SLASH: f64 = 0.3;
/// The least score named, and the least share of what sounds a chord
/// must hold: below them the notes are too far from any chord.
const NAMED: f64 = 1.6;
const HOLDS: f64 = 0.6;

/// The chord that best matches pitch-class `weights` (any scale; 0 is C)
/// over `bass`: `None` while fewer than three pitch classes sound, or no
/// chord fits them.
#[must_use]
pub fn identify(weights: &[f64; 12], bass: Option<u8>) -> Option<Chord> {
    let max = weights.iter().copied().fold(0.0, f64::max);
    if max <= 0.0 || !max.is_finite() {
        return None;
    }
    let w = weights.map(|x| x / max);
    let heard = |pc: usize| w.get(pc).copied().filter(|&x| x >= PRESENT);
    if (0..12).filter_map(heard).count() < 3 {
        return None;
    }
    let total: f64 = (0..12).filter_map(heard).sum();
    let bass = bass.map(|b| b % 12);
    let mut best: Option<(f64, Chord)> = None;
    for root in 0..12u8 {
        for kind in Kind::ALL {
            let tones = kind
                .intervals()
                .iter()
                .map(|i| usize::from((root + i) % 12));
            let (mut inside, mut missing) = (0.0, 0);
            for t in tones {
                match heard(t) {
                    Some(x) => inside += x,
                    None => missing += 1,
                }
            }
            // A chord missing two of its notes, or holding too little of
            // what sounds, is not heard.
            if missing > 1 || inside < HOLDS * total {
                continue;
            }
            let has = |pc: u8| kind.intervals().iter().any(|i| (root + i) % 12 == pc);
            let (voting, excused) = match bass {
                Some(b) if b == root => (ROOT_BASS, 0.0),
                Some(b) if has(b) => (IN_BASS, 0.0),
                Some(b) => (-SLASH, heard(usize::from(b)).unwrap_or(0.0)),
                None => (0.0, 0.0),
            };
            let outside = total - inside - excused;
            let score =
                inside - OUTSIDE * outside - MISSING * f64::from(missing) - kind.cost() + voting;
            if best.is_none_or(|(s, _)| score > s + 1e-9) {
                let chord = Chord {
                    root,
                    kind,
                    bass: bass.unwrap_or(root),
                };
                best = Some((score, chord));
            }
        }
    }
    best.filter(|&(score, _)| score >= NAMED).map(|(_, c)| c)
}

/// Seconds a note's weight builds over: struck, it weighs this much of
/// what it will after ringing that long.
const ATTACK: f64 = 0.2;
/// Seconds a note rings before it no longer counts.
const HORIZON: f64 = 6.0;
/// A note lower than this weighs as a bass; a note the bass needs this
/// share of the heaviest note's weight.
const BASS_SHARE: f64 = 0.2;

/// How much a note `age` seconds old, on `key` at `velocity`, sounds: it
/// builds as it rings (a passing tone just struck weighs little) and dies
/// away, slower the lower it is, louder notes weighing more.
fn weight(age: f64, key: u8, velocity: u8) -> f64 {
    let decay = 2.0 + 0.04 * f64::from(60u8.saturating_sub(key));
    (age.min(2.0) + ATTACK) * (-age / decay).exp() * (0.5 + f64::from(velocity.min(127)) / 127.0)
}

/// The chord sounding `at` seconds into a song: from the `notes` that
/// sound then, by their pitch classes and its bass, ignoring passing tones
/// (see the module). Give it the notes as they ring, each ending when its
/// key is let go or, while the sustain pedal holds it, when the pedal
/// lifts; notes that end with their key work too, without the pedal.
/// `None` while fewer than three pitch classes sound or they make no
/// chord.
#[must_use]
pub fn chord_at(notes: &Notes, at: f64) -> Option<String> {
    let mut weights = [0.0; 12];
    let mut keys: Vec<(u8, f64)> = Vec::new();
    for n in notes.sounding(at) {
        let age = at - n.start;
        if age > HORIZON {
            continue;
        }
        let w = weight(age, n.key, n.velocity);
        if let Some(slot) = weights.get_mut(usize::from(n.key % 12)) {
            *slot += w;
        }
        keys.push((n.key, w));
    }
    let loudest = keys.iter().map(|k| k.1).fold(0.0, f64::max);
    let bass = (keys.iter())
        .filter(|k| k.1 >= BASS_SHARE * loudest)
        .map(|k| k.0)
        .min();
    identify(&weights, bass.map(|k| k % 12)).map(|c| c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waterfall::Note;

    /// `keys` struck together at 0 and held to 4 s.
    fn held(keys: &[u8]) -> Notes {
        Notes::new(
            keys.iter()
                .map(|&key| Note {
                    start: 0.0,
                    end: 4.0,
                    key,
                    velocity: 80,
                    voice: 0,
                })
                .collect(),
        )
    }

    fn name(keys: &[u8]) -> Option<String> {
        chord_at(&held(keys), 1.0)
    }

    // C4 is 60.
    const C: u8 = 60;

    #[test]
    fn triads_in_root_position() {
        assert_eq!(name(&[C, C + 4, C + 7]).as_deref(), Some("C"));
        assert_eq!(name(&[57, 60, 64]).as_deref(), Some("Am"), "A C E");
        assert_eq!(name(&[C, C + 3, C + 6]).as_deref(), Some("Cdim"));
        assert_eq!(name(&[C, C + 4, C + 8]).as_deref(), Some("Caug"));
        assert_eq!(name(&[C, C + 5, C + 7]).as_deref(), Some("Csus4"));
        assert_eq!(name(&[62, 66, 69]).as_deref(), Some("D"), "D F# A");
        assert_eq!(name(&[66, 69, 73]).as_deref(), Some("F#m"), "F# A C#");
        assert_eq!(name(&[63, 67, 70]).as_deref(), Some("Eb"), "Eb G Bb");
        assert_eq!(name(&[56, 60, 63]).as_deref(), Some("Ab"), "Ab C Eb");
        assert_eq!(name(&[56, 59, 63]).as_deref(), Some("G#m"), "G# B D#");
        // Spread over three octaves, doubled: still C.
        assert_eq!(name(&[36, 48, 55, 64, 72, 79]).as_deref(), Some("C"));
    }

    #[test]
    fn inversions_name_their_bass() {
        assert_eq!(name(&[64, 67, 72]).as_deref(), Some("C/E"), "E G C");
        assert_eq!(name(&[55, 64, 72]).as_deref(), Some("C/G"), "G E C");
        assert_eq!(name(&[48, 57, 64]).as_deref(), Some("Am/C"), "C A E");
        assert_eq!(name(&[56, 64, 71]).as_deref(), Some("E/G#"), "G# E B");
        assert_eq!(name(&[54, 62, 69]).as_deref(), Some("D/F#"), "F# D A");
        assert_eq!(name(&[50, 58, 65]).as_deref(), Some("Bb/D"), "D Bb F");
    }

    #[test]
    fn sevenths_and_sixths() {
        assert_eq!(name(&[55, 59, 62, 65]).as_deref(), Some("G7"), "G B D F");
        assert_eq!(name(&[C, 64, 67, 71]).as_deref(), Some("Cmaj7"));
        assert_eq!(name(&[62, 65, 69, 72]).as_deref(), Some("Dm7"), "D F A C");
        assert_eq!(name(&[59, 62, 65, 69]).as_deref(), Some("Bm7b5"), "B D F A");
        assert_eq!(
            name(&[59, 62, 65, 68]).as_deref(),
            Some("Bdim7"),
            "B D F Ab"
        );
        assert_eq!(name(&[47, 62, 65, 67]).as_deref(), Some("G7/B"), "B D F G");
        assert_eq!(
            name(&[46, 64, 67, 72]).as_deref(),
            Some("C7/Bb"),
            "Bb E G C"
        );
        // The same notes, A C E G: the bass decides between Am7 and C6.
        assert_eq!(name(&[45, 60, 64, 67]).as_deref(), Some("Am7"));
        assert_eq!(name(&[48, 57, 64, 67]).as_deref(), Some("C6"));
        // A seventh without its fifth.
        assert_eq!(name(&[48, 64, 70]).as_deref(), Some("C7"), "C E Bb");
    }

    #[test]
    fn slash_chords_keep_a_bass_outside_the_chord() {
        assert_eq!(
            name(&[50, 60, 64, 67]).as_deref(),
            Some("C/D"),
            "D under C E G"
        );
        assert_eq!(
            name(&[43, 65, 69, 72]).as_deref(),
            Some("F/G"),
            "G under F A C"
        );
        // D F# A over C is D7 over its seventh.
        assert_eq!(name(&[48, 62, 66, 69]).as_deref(), Some("D7/C"));
    }

    #[test]
    fn a_passing_tone_is_ignored() {
        let mut notes: Vec<Note> = [48, 64, 67]
            .map(|key| Note {
                start: 0.0,
                end: 3.0,
                key,
                velocity: 70,
                voice: 0,
            })
            .to_vec();
        // A melody's D, struck loud and short over the held C chord.
        notes.push(Note {
            start: 1.4,
            end: 1.6,
            key: 74,
            velocity: 110,
            voice: 1,
        });
        let song = Notes::new(notes);
        assert_eq!(chord_at(&song, 1.45).as_deref(), Some("C"));
        assert_eq!(chord_at(&song, 1.0).as_deref(), Some("C"));
    }

    #[test]
    fn too_few_notes_make_no_chord() {
        assert_eq!(name(&[]), None);
        assert_eq!(name(&[C]), None);
        assert_eq!(name(&[C, C + 7]), None, "a fifth");
        assert_eq!(name(&[C, C + 12, C + 24]), None, "octaves");
        // Before and after it sounds.
        assert_eq!(chord_at(&held(&[60, 64, 67]), -0.5), None);
        assert_eq!(chord_at(&held(&[60, 64, 67]), 4.5), None);
    }

    #[test]
    fn notes_rung_by_the_pedal_count_and_keys_let_go_do_not() {
        let short: Vec<Note> = [57, 60, 64]
            .map(|key| Note {
                start: 0.0,
                end: 0.3,
                key,
                velocity: 80,
                voice: 0,
            })
            .to_vec();
        let pedalled: Vec<Note> = short.iter().map(|n| Note { end: 2.0, ..*n }).collect();
        assert_eq!(chord_at(&Notes::new(short), 1.0), None);
        assert_eq!(chord_at(&Notes::new(pedalled), 1.0).as_deref(), Some("Am"));
    }

    #[test]
    fn identify_weighs_pitch_classes() {
        let mut w = [0.0; 12];
        for pc in [0, 4, 7] {
            w[pc] = 1.0;
        }
        let c = identify(&w, Some(0)).unwrap();
        assert_eq!((c.root, c.kind, c.bass), (0, Kind::Major, 0));
        // A faint extra pitch class does not change it.
        w[2] = 0.1;
        assert_eq!(identify(&w, Some(0)), Some(c));
        assert_eq!(identify(&[0.0; 12], None), None);
        assert_eq!(identify(&[f64::NAN; 12], None), None);
        // Eight pitch classes at once are no chord.
        let cluster = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(identify(&cluster, None), None);
    }
}
