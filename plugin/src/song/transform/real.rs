//! The touch-ups on the player's own MIDI files, which are not in the
//! repository: run with `--ignored`, naming the files in
//! `KONTRA_ROUGH` (files that should be touched) and `KONTRA_PLAYED`
//! (performances that must not be), separated by `:`. Prints what each
//! touch-up changed.

use kontra_library::Touch;

use super::*;
use crate::song::Song;

fn files(var: &str) -> Vec<std::path::PathBuf> {
    std::env::var(var)
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .collect()
}

fn name(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

#[test]
#[ignore = "reads the player's own MIDI files"]
fn performances_play_exactly_as_written() {
    let all = Touch {
        pedal: true,
        humanise: true,
        roll: true,
        velocity: true,
    };
    let played = files("KONTRA_PLAYED");
    assert!(!played.is_empty(), "name them in KONTRA_PLAYED");
    for path in played {
        let bytes = std::fs::read(&path).unwrap();
        let plain = Song::read(name(&path), &bytes).unwrap();
        let (touched, what) = Song::read_with(name(&path), &bytes, &all).unwrap();
        let a = what.analysis;
        println!(
            "{}: {} notes, pedal {}, on grid {:.3}, level chords {:.3} of {}, one velocity {:.3} -> {:?}",
            name(&path),
            a.notes,
            a.pedal,
            a.on_grid,
            a.level_chords,
            a.chords,
            a.one_velocity,
            what.applied
        );
        assert_eq!(what.applied, Touch::NONE, "{}", name(&path));
        assert_eq!(touched.events, plain.events, "{}", name(&path));
    }
}

#[test]
#[ignore = "reads the player's own MIDI files"]
fn rough_files_get_a_pedal_and_a_hand_and_nothing_else() {
    let rough = files("KONTRA_ROUGH");
    assert!(!rough.is_empty(), "name them in KONTRA_ROUGH");
    for path in rough {
        let bytes = std::fs::read(&path).unwrap();
        let (events, tempo) = crate::song::timeline(&bytes).unwrap();
        let events: Vec<Timed> = events
            .into_iter()
            .filter(|e| e.2[0] & 15 != crate::song::DRUMS)
            .collect();
        let a = analyse(&events, &tempo);
        println!(
            "{}: {} notes in {} tracks, pedal {}, on grid {:.3}, level chords {:.3} of {}, one velocity {:.3}",
            name(&path),
            a.notes,
            a.tracks,
            a.pedal,
            a.on_grid,
            a.level_chords,
            a.chords,
            a.one_velocity
        );
        let applies = a.applies(&Touch::default());
        assert!(applies.pedal && applies.humanise, "{applies:?}");

        // The hand: only notes' times and velocities move, by a little.
        let hand = humanise(&events, false);
        let (before, after) = (spans(&events, f64::MAX), spans(&hand, f64::MAX));
        let note = |e: &[Timed], s: &Span| {
            (
                s.channel,
                s.key,
                s.start,
                s.end,
                e.get(s.on).map_or(0, |e| e.2[2]),
            )
        };
        let mut b: Vec<_> = before.iter().map(|s| note(&events, s)).collect();
        let mut h: Vec<_> = after.iter().map(|s| note(&hand, s)).collect();
        let order = |x: &(u8, u8, f64, f64, u8), y: &(u8, u8, f64, f64, u8)| {
            (x.0, x.1).cmp(&(y.0, y.1)).then(x.2.total_cmp(&y.2))
        };
        b.sort_by(order);
        h.sort_by(order);
        assert_eq!(b.len(), h.len());
        let (mut moved, mut most, mut louder, mut softer) = (0, 0.0f64, 0, 0);
        for (x, y) in b.iter().zip(&h) {
            assert_eq!((x.0, x.1), (y.0, y.1));
            let d = y.2 - x.2;
            assert!((0.0..=SPREAD + 1e-9).contains(&d), "{x:?} {y:?}");
            assert!(y.3 >= y.2, "a note ends after it starts");
            let dv = i32::from(y.4) - i32::from(x.4);
            assert!(dv.abs() <= 9);
            moved += usize::from(d > 0.0);
            most = most.max(d);
            louder += usize::from(dv > 0);
            softer += usize::from(dv < 0);
        }
        let others = |e: &[Timed]| -> Vec<Timed> {
            e.iter()
                .filter(|e| !is_on(e.2) && !is_off(e.2))
                .copied()
                .collect()
        };
        assert_eq!(others(&hand), others(&events), "only notes change");
        println!(
            "  hand: {moved} of {} notes later, by {:.1} ms at most; {louder} louder, {softer} softer",
            b.len(),
            most * 1e3
        );

        // The pedal: added, and nothing else changed.
        let pedalled = auto_pedal(&hand, &tempo);
        let pedal: Vec<(f64, bool)> = (pedalled.iter())
            .filter(|e| is_pedal(e.2))
            .map(|e| (e.0, e.2[2] >= 64))
            .collect();
        let rest: Vec<Timed> = pedalled
            .iter()
            .filter(|e| !is_pedal(e.2))
            .copied()
            .collect();
        assert_eq!(rest, hand, "only the pedal is added");
        let presses = pedal.iter().filter(|p| p.1).count();
        let length = hand.last().map_or(0.0, |e| e.0);
        let down: f64 = pedal
            .chunks(2)
            .filter_map(|p| match p {
                [(on, true), (off, false)] => Some(off - on),
                _ => None,
            })
            .sum();
        println!(
            "  pedal: {presses} presses, down {:.0}% of {:.0} s, a press every {:.2} beats",
            100.0 * down / length,
            length,
            (length / tempo.beat(length / 2.0)) / presses as f64
        );
        let first: Vec<String> = (pedal.iter())
            .filter(|p| !p.1)
            .take(24)
            .map(|p| {
                format!(
                    "{:.2}",
                    tempo.tick(p.0) / tempo.division.unwrap_or(1) as f64
                )
            })
            .collect();
        println!("  first lifts, in beats: {}", first.join(" "));

        // The chords heard as it rings, two beats apart.
        let (song, what) = Song::read_with(name(&path), &bytes, &Touch::default()).unwrap();
        let mut heard = Vec::new();
        let per = tempo.beat(0.0);
        for n in 0..24 {
            let t = (n as f64 * 2.0 + 1.5) * per;
            heard.push(kontra_ui::chord::chord_at(&what.ringing, t).unwrap_or_else(|| "-".into()));
        }
        println!("  chords every two beats: {}", heard.join(" "));
        assert!(song.events.iter().any(|e| is_pedal(e.1)));
    }
}
