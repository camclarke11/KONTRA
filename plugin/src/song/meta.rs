//! What a MIDI file says besides its channel messages: its beats, by its
//! tempo map and time signatures, for the metronome and the count-in; and
//! its title and composer, for the Stage's title card, from its text or,
//! failing that, from a file name such as `01 Chopin - Nocturne.mid`.

use super::{Reader, big_endian};

/// A beat of the song's metre, in seconds from its start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beat {
    pub time: f64,
    /// The first beat of a bar: the metronome accents it.
    pub downbeat: bool,
}

/// What the file says about itself.
#[derive(Debug, Default)]
pub struct Meta {
    /// Every beat from the song's start to its last event.
    pub beats: Vec<Beat>,
    pub title: Option<String>,
    pub composer: Option<String>,
    /// Each track's name, where it gives one, by track.
    pub tracks: Vec<Option<String>>,
}

/// More beats than this (a day at 120 BPM) are not kept.
const BEATS: usize = 200_000;

/// A meta event or the end of a track's channel events, at its tick.
enum Item {
    /// Microseconds per quarter note from here on.
    Tempo(u32),
    /// A time signature: beats a bar and the beat's note, as a power of 2.
    Metre(u8, u8),
    /// A text event of `kind` (1 text, 2 copyright, 3 track name).
    Text(u8, String),
    /// Any other event: how far the song runs.
    Event,
}

/// The file's beats and words. A file [`super::timeline`] cannot read has
/// none of either.
pub fn read(bytes: &[u8]) -> Meta {
    let Some((division, items)) = items(bytes) else {
        return Meta::default();
    };
    let last = items.iter().map(|&(tick, ..)| tick).max().unwrap_or(0);
    let mut tracks = Vec::new();
    let mut title = None;
    let mut composer = None;
    for (_, track, item) in &items {
        let Item::Text(kind, text) = item else {
            continue;
        };
        let track = usize::from(*track);
        if *kind == 3 {
            if tracks.len() <= track {
                tracks.resize(track + 1, None);
            }
            if let Some(slot) = tracks.get_mut(track).filter(|s| s.is_none()) {
                *slot = Some(text.clone());
            }
            // The first track's name is the song's (format 0's only
            // track, format 1's conductor track).
            if track == 0 && title.is_none() && titled(text) {
                title = Some(text.clone());
            }
        }
        if *kind == 1 && composer.is_none() {
            composer = credited(text);
        }
    }
    Meta {
        beats: beats(division, &items, last),
        title,
        composer,
        tracks,
    }
}

/// The song's title and composer from its file `name`: `01 Chopin -
/// Nocturne.mid` is Chopin's Nocturne; a name without " - " is a title.
#[must_use]
pub fn from_name(name: &str) -> (String, Option<String>) {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    // A leading track number: "01 ", "1. ", "03_".
    let digits = stem.len() - stem.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let stem = match stem.get(digits..) {
        Some(rest) if digits > 0 && rest.starts_with([' ', '.', '_', '-']) => {
            rest.trim_start_matches([' ', '.', '_', '-'])
        }
        _ => stem,
    };
    match stem.split_once(" - ") {
        Some((who, what)) if !who.trim().is_empty() && !what.trim().is_empty() => {
            (what.trim().to_owned(), Some(who.trim().to_owned()))
        }
        _ => (stem.trim().to_owned(), None),
    }
}

/// Whether a track name reads as a song's title, not a sequencer's label.
fn titled(text: &str) -> bool {
    let lower = text.to_lowercase();
    let generic = [
        "track",
        "untitled",
        "sequence",
        "tempo",
        "conductor",
        "piano",
    ];
    text.chars().filter(|c| c.is_alphabetic()).count() >= 2
        && text.chars().count() <= 80
        && !generic.iter().any(|g| lower.starts_with(g))
}

/// The composer a text event credits: "Composer: Chopin", "Composed by
/// Chopin", "Music by Chopin".
fn credited(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    ["composer:", "composer -", "composed by", "music by"]
        .iter()
        .find_map(|p| lower.starts_with(p).then(|| text.get(p.len()..)))
        .flatten()
        .map(|s| s.trim_matches([' ', ':', '-']).to_owned())
        .filter(|s| !s.is_empty())
}

/// The ticks a second runs at for a tick of `division`: ticks per quarter,
/// or SMPTE frames (whose beats are taken at 120 BPM).
#[derive(Clone, Copy)]
enum Division {
    Quarter(f64),
    Smpte(f64),
}

/// Items at their tick, with their track.
type Items = Vec<(usize, u16, Item)>;

/// Every meta event and the tick of every other, by track, in tick order.
fn items(bytes: &[u8]) -> Option<(Division, Items)> {
    if bytes.get(..4) != Some(b"MThd".as_slice()) {
        return None;
    }
    let header = big_endian(bytes, 4, 4)?;
    let division = big_endian(bytes, 12, 2)?;
    let division = if division & 0x8000 != 0 {
        let (fps, ticks) = (256 - (division >> 8), division & 0xFF);
        Division::Smpte((fps * ticks).max(1) as f64)
    } else {
        Division::Quarter(division.max(1) as f64)
    };
    let mut out = Vec::new();
    let mut n = 0u16;
    let mut at = header.checked_add(8)?;
    while let (Some(id), Some(len)) = (bytes.get(at..at + 4), big_endian(bytes, at + 4, 4)) {
        let start = at + 8;
        let end = start.saturating_add(len).min(bytes.len());
        at = end.max(start);
        if id == b"MTrk" {
            // What a track holds up to where it breaks off.
            let _ = track(bytes.get(start..end).unwrap_or_default(), n, &mut out);
            n = n.saturating_add(1);
        }
    }
    // Stable: what one tick holds keeps the file's order.
    out.sort_by_key(|&(tick, ..)| tick);
    Some((division, out))
}

/// Track `n`'s meta events, and a mark at each other event, into `out`.
fn track(bytes: &[u8], n: u16, out: &mut Vec<(usize, u16, Item)>) -> Result<(), String> {
    let mut r = Reader { bytes, at: 0 };
    let mut tick = 0usize;
    let mut running = 0u8;
    while r.at < bytes.len() {
        tick = tick.saturating_add(r.number()?);
        let mut status = r.byte()?;
        let mut first = false;
        if status < 0x80 {
            if running == 0 {
                return Err("no status".into());
            }
            first = true;
            status = running;
        }
        match status {
            0xFF => {
                running = 0;
                let kind = r.byte()?;
                let len = r.number()?;
                let from = r.at;
                r.skip(len)?;
                let data = bytes.get(from..r.at).unwrap_or_default();
                match (kind, data) {
                    (0x2F, _) => break,
                    (0x51, &[a, b, c]) => out.push((
                        tick,
                        n,
                        Item::Tempo(u32::from(a) << 16 | u32::from(b) << 8 | u32::from(c)),
                    )),
                    (0x58, &[num, den, ..]) => out.push((tick, n, Item::Metre(num, den))),
                    (1..=3, text) => {
                        let text = String::from_utf8_lossy(text).trim().to_owned();
                        if !text.is_empty() {
                            out.push((tick, n, Item::Text(kind, text)));
                        }
                    }
                    _ => {}
                }
            }
            0xF0 | 0xF7 => {
                running = 0;
                let len = r.number()?;
                r.skip(len)?;
            }
            0x80..=0xEF => {
                running = status;
                let data = usize::from(!matches!(status & 0xF0, 0xC0 | 0xD0)) + 1;
                r.skip(data - usize::from(first))?;
                out.push((tick, n, Item::Event));
            }
            _ => return Err("unexpected byte".into()),
        }
    }
    Ok(())
}

/// The beats from tick 0 to `last`: a beat each time signature's note, a
/// bar starting at each change of signature, timed by the tempo map.
fn beats(division: Division, items: &[(usize, u16, Item)], last: usize) -> Vec<Beat> {
    let (per_quarter, smpte) = match division {
        Division::Quarter(q) => (q, None),
        // Time runs in frames; beats are taken at 120 BPM.
        Division::Smpte(per_second) => (per_second / 2., Some(per_second)),
    };
    let tempos: Vec<(usize, f64)> = (items.iter())
        .filter_map(|(tick, _, i)| match i {
            Item::Tempo(t) if *t > 0 => Some((*tick, f64::from(*t) / 1e6)),
            _ => None,
        })
        .collect();
    let mut metres: Vec<(usize, u8, u8)> = (items.iter())
        .filter_map(|(tick, _, i)| match i {
            Item::Metre(num, den) if *num > 0 && *den < 7 => Some((*tick, *num, *den)),
            _ => None,
        })
        .collect();
    if metres.first().is_none_or(|m| m.0 > 0) {
        metres.insert(0, (0, 4, 2));
    }
    // Seconds at a tick, walked forward: the tempo map's pieces in turn.
    let mut clock = Clock {
        tempos: &tempos,
        next: 0,
        tick: 0,
        time: 0.,
        tempo: 0.5,
        per_quarter,
        smpte,
    };
    let mut out = Vec::new();
    for (n, &(from, num, den)) in metres.iter().enumerate() {
        let until = metres.get(n + 1).map_or(last, |m| m.0);
        let step = (per_quarter * 4. / f64::from(1u32 << den)).max(1.);
        let mut beat = 0u32;
        loop {
            let tick = from + (f64::from(beat) * step).round() as usize;
            if tick > until || (tick == until && n + 1 < metres.len()) || out.len() >= BEATS {
                break;
            }
            out.push(Beat {
                time: clock.at(tick),
                downbeat: beat.is_multiple_of(u32::from(num)),
            });
            beat += 1;
        }
    }
    out
}

/// Seconds at ticks in rising order, by the tempo map.
struct Clock<'a> {
    tempos: &'a [(usize, f64)],
    next: usize,
    tick: usize,
    time: f64,
    /// Seconds a quarter note lasts from `tick`.
    tempo: f64,
    per_quarter: f64,
    /// SMPTE ticks a second: the tempo map does not apply.
    smpte: Option<f64>,
}

impl Clock<'_> {
    fn at(&mut self, tick: usize) -> f64 {
        if let Some(per_second) = self.smpte {
            return tick as f64 / per_second;
        }
        while let Some(&(at, tempo)) = self.tempos.get(self.next).filter(|t| t.0 <= tick) {
            self.time += at.saturating_sub(self.tick) as f64 * self.tempo / self.per_quarter;
            self.tick = at.max(self.tick);
            self.tempo = tempo;
            self.next += 1;
        }
        self.time + tick.saturating_sub(self.tick) as f64 * self.tempo / self.per_quarter
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{file, song, track};
    use super::*;

    #[test]
    fn beats_follow_the_tempo_map_and_the_metre() {
        // song(): 120 BPM, 60 BPM from tick 1920 (the second bar), 4/4.
        let meta = read(&song());
        let beats: Vec<(f64, bool)> = (meta.beats.iter())
            .map(|b| ((b.time * 1000.).round() / 1000., b.downbeat))
            .collect();
        assert_eq!(
            beats,
            [
                (0.0, true),
                (0.5, false),
                (1.0, false),
                (1.5, false),
                (2.0, true),
                (3.0, false),
            ]
        );
        // 3/4 from the start, then 6/8 from tick 1440: eighths, six a bar.
        let metre = track(&[
            (0, &[0xFF, 0x58, 4, 3, 2, 24, 8]),
            (1440, &[0xFF, 0x58, 4, 6, 3, 24, 8]),
            (2880, &[0x90, 60, 100]),
        ]);
        let meta = read(&file(0, 480, &[metre]));
        let downbeats: Vec<f64> = (meta.beats.iter())
            .filter(|b| b.downbeat)
            .map(|b| b.time)
            .collect();
        assert_eq!(downbeats, [0.0, 1.5, 3.0]);
        assert_eq!(meta.beats.len(), 3 + 6 + 1);
        assert!((meta.beats[4].time - 1.75).abs() < 1e-9, "an eighth on");
    }

    #[test]
    fn words_come_from_the_file_or_its_name() {
        let text = |kind: u8, s: &str| {
            let mut out = vec![0xFF, kind, s.len() as u8];
            out.extend(s.bytes());
            out
        };
        let (name, credit) = (text(3, "Nocturne"), text(1, "Composer: Chopin"));
        let named = track(&[(0, &name), (0, &credit), (0, &[0x90, 60, 100])]);
        let meta = read(&file(0, 480, &[named]));
        assert_eq!(meta.title.as_deref(), Some("Nocturne"));
        assert_eq!(meta.composer.as_deref(), Some("Chopin"));
        assert_eq!(meta.tracks, [Some("Nocturne".to_owned())]);
        // A sequencer's label is not a title.
        let label = text(3, "Track 1");
        assert_eq!(read(&file(0, 480, &[track(&[(0, &label)])])).title, None);
        assert_eq!(
            from_name("01 Chopin - Nocturne in E-flat, Op 9 No 2.mid"),
            (
                "Nocturne in E-flat, Op 9 No 2".to_owned(),
                Some("Chopin".to_owned())
            )
        );
        assert_eq!(from_name("pedal-runs.mid"), ("pedal-runs".to_owned(), None));
        assert_eq!(from_name("2. Satie.MID"), ("Satie".to_owned(), None));
    }
}
