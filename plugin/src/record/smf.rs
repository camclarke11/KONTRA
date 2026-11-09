//! Writing a Standard MIDI File: format 1, a first track with the tempo and
//! time signature, then one track per source, each named. [`crate::song`]
//! reads it back.

/// Ticks a quarter note: at 120 BPM a tick is about half a millisecond.
pub const DIVISION: u16 = 960;

/// One track: its name and its channel messages at their time in seconds,
/// in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub name: String,
    pub events: Vec<(f64, [u8; 3])>,
}

/// A variable-length quantity, seven bits a byte, at most four.
fn number(n: u32, out: &mut Vec<u8>) {
    let n = n.min(0x0FFF_FFFF);
    let mut shift = 21;
    while shift > 0 && n >> shift == 0 {
        shift -= 7;
    }
    while shift > 0 {
        out.push((n >> shift) as u8 & 0x7F | 0x80);
        shift -= 7;
    }
    out.push(n as u8 & 0x7F);
}

/// A meta event of `kind` holding `data`, at no time after the last.
fn meta(kind: u8, data: &[u8], out: &mut Vec<u8>) {
    out.extend([0, 0xFF, kind]);
    number(u32::try_from(data.len()).unwrap_or(u32::MAX), out);
    out.extend(data);
}

fn chunk(id: &[u8; 4], body: &[u8], out: &mut Vec<u8>) {
    out.extend(id);
    out.extend(u32::try_from(body.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend(body);
}

/// The bytes a channel message takes: one data byte for a program change
/// or channel pressure, two for the rest.
fn message([status, a, b]: [u8; 3], out: &mut Vec<u8>) {
    out.extend([status, a & 0x7F]);
    if !matches!(status & 0xF0, 0xC0 | 0xD0) {
        out.push(b & 0x7F);
    }
}

/// `tracks` as a format 1 file called `name`, at `bpm` in 4/4. Times are
/// rounded to the nearest tick; messages at one tick keep their order.
#[must_use]
pub fn write(name: &str, bpm: f64, tracks: &[Track]) -> Vec<u8> {
    let bpm = if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        120.0
    };
    let per_second = f64::from(DIVISION) * bpm / 60.0;
    let mut out = Vec::new();
    let mut header = Vec::new();
    header.extend(1u16.to_be_bytes());
    let count = u16::try_from(tracks.len() + 1).unwrap_or(u16::MAX);
    header.extend(count.to_be_bytes());
    header.extend(DIVISION.to_be_bytes());
    chunk(b"MThd", &header, &mut out);

    let mut first = Vec::new();
    meta(0x03, name.as_bytes(), &mut first);
    let tempo = (60_000_000.0 / bpm).round() as u32;
    meta(0x51, &tempo.to_be_bytes()[1..], &mut first);
    // 4/4, a click a quarter, eight 32nds a quarter.
    meta(0x58, &[4, 2, 24, 8], &mut first);
    meta(0x2F, &[], &mut first);
    chunk(b"MTrk", &first, &mut out);

    for track in tracks.iter().take(usize::from(u16::MAX) - 1) {
        let mut body = Vec::new();
        meta(0x03, track.name.as_bytes(), &mut body);
        let mut last = 0u32;
        for &(time, m) in &track.events {
            let tick = (time.max(0.0) * per_second)
                .round()
                .min(f64::from(u32::MAX)) as u32;
            // In order: a time before the last is put at the last.
            let tick = tick.max(last);
            number(tick - last, &mut body);
            message(m, &mut body);
            last = tick;
        }
        meta(0x2F, &[], &mut body);
        chunk(b"MTrk", &body, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Song;

    #[test]
    fn numbers_take_seven_bits_a_byte() {
        let enc = |n| {
            let mut out = Vec::new();
            number(n, &mut out);
            out
        };
        assert_eq!(enc(0), [0]);
        assert_eq!(enc(0x40), [0x40]);
        assert_eq!(enc(0x7F), [0x7F]);
        assert_eq!(enc(0x80), [0x81, 0x00]);
        assert_eq!(enc(0x2000), [0xC0, 0x00]);
        assert_eq!(enc(0x0FFF_FFFF), [0xFF, 0xFF, 0xFF, 0x7F]);
        assert_eq!(enc(u32::MAX), [0xFF, 0xFF, 0xFF, 0x7F], "clamped");
    }

    fn take() -> Vec<Track> {
        vec![
            Track {
                name: "MIDI input".into(),
                events: vec![
                    (0.0, [0xB0, 64, 127]),
                    (0.0, [0x90, 60, 100]),
                    (0.5, [0x80, 60, 0]),
                    (0.5, [0x90, 64, 37]),
                    (1.25, [0xB0, 64, 0]),
                    (1.5, [0x80, 64, 0]),
                    (1.75, [0xC0, 5, 0]),
                ],
            },
            Track {
                name: "On-screen keyboard".into(),
                events: vec![(0.25, [0x91, 48, 127]), (2.0, [0x81, 48, 0])],
            },
        ]
    }

    /// The file reads back through the player's own parser: every note at
    /// its time, velocity and voice (one per track).
    #[test]
    fn a_take_reads_back_as_it_was_played() {
        let bytes = write("Take", 120.0, &take());
        assert_eq!(&bytes[..4], b"MThd");
        assert_eq!(
            &bytes[8..14],
            [0, 1, 0, 3, 0x03, 0xC0],
            "format 1, 3 tracks, 960"
        );
        let song = Song::read("Take.mid".into(), &bytes).unwrap();
        let notes: Vec<_> = song
            .notes
            .between(0.0, 10.0)
            .map(|n| (n.start, n.end, n.key, n.velocity, n.voice))
            .collect();
        assert_eq!(
            notes,
            [
                (0.0, 0.5, 60, 100, 0),
                (0.25, 2.0, 48, 127, 1),
                (0.5, 1.5, 64, 37, 0)
            ]
        );
        assert!((song.length - 2.0).abs() < 1e-9, "{}", song.length);
    }

    /// At another tempo the ticks change, the seconds do not; a time that
    /// falls between ticks lands on the nearest.
    #[test]
    fn times_survive_any_tempo_to_the_nearest_tick() {
        for bpm in [60.0, 97.0, 120.0, 180.0] {
            let track = Track {
                name: "t".into(),
                events: vec![(0.333_333, [0x90, 60, 90]), (1.000_7, [0x80, 60, 0])],
            };
            let song = Song::read("t.mid".into(), &write("t", bpm, &[track])).unwrap();
            let n = song.notes.between(0.0, 10.0).next().copied().unwrap();
            let tick = 60.0 / bpm / f64::from(DIVISION);
            assert!(
                (n.start - 0.333_333).abs() <= tick / 2.0 + 1e-9,
                "{bpm}: {}",
                n.start
            );
            assert!(
                (n.end - 1.000_7).abs() <= tick / 2.0 + 1e-9,
                "{bpm}: {}",
                n.end
            );
        }
    }
}
