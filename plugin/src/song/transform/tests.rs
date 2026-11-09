//! The touch-ups on files made here: a rough one (on a grid, never
//! pedalled), the same played (off the grid, pedalled) and one at a single
//! velocity. What each touch-up changes, and that nothing else changes.

use kontra_library::Touch;

use super::*;
use crate::song::Song;
use crate::song::tests::{file, track};

/// Ticks to a quarter note; 120 BPM, so a beat is half a second.
const DIVISION: u16 = 480;
const BEAT: f64 = 0.5;

/// A note: its beat, length in beats, key and velocity.
type Written = (f64, f64, u8, u8);

/// A standard MIDI file of `notes` on channel 1 and `pedal` (beat, value)
/// presses and lifts, each moved by `jitter(n)` ticks.
fn smf(notes: &[Written], pedal: &[(f64, u8)], jitter: impl Fn(usize) -> i64) -> Vec<u8> {
    let tick = |beat: f64, n: usize| {
        let t = (beat * f64::from(DIVISION)).round() as i64 + jitter(n);
        t.max(0) as usize
    };
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    for (n, &(beat, length, key, velocity)) in notes.iter().enumerate() {
        let on = tick(beat, n);
        let off = tick(beat + length, n + 7).max(on + 1);
        events.push((on, vec![0x90, key, velocity]));
        events.push((off, vec![0x80, key, 0]));
    }
    for (n, &(beat, value)) in pedal.iter().enumerate() {
        events.push((tick(beat, n + 3), vec![0xB0, 64, value]));
    }
    events.sort_by_key(|e| e.0);
    let tempo = track(&[(0, &[0xFF, 0x51, 3, 0x07, 0xA1, 0x20])]);
    let body: Vec<(usize, &[u8])> = events.iter().map(|(t, b)| (*t, b.as_slice())).collect();
    file(1, DIVISION, &[tempo, track(&body)])
}

/// Am, F, C and G, a bar each, then two beats' rest and Am again: a left
/// hand of broken chords in eighths, a right hand's chord on each bar's
/// first beat and two melody notes, the first passing.
fn piece(velocity: impl Fn(usize) -> u8) -> Vec<Written> {
    // A bar: its first beat, the left hand's figure, the chord, the melody.
    type Bar = (f64, [u8; 4], [u8; 3], [u8; 2]);
    let bars: [Bar; 5] = [
        (0.0, [45, 52, 57, 60], [69, 72, 76], [74, 76]),
        (4.0, [41, 48, 53, 57], [69, 72, 77], [79, 77]),
        (8.0, [48, 55, 60, 64], [67, 72, 76], [74, 76]),
        (12.0, [43, 50, 55, 59], [67, 71, 74], [72, 74]),
        (18.0, [45, 52, 57, 60], [69, 72, 76], [74, 76]),
    ];
    let mut notes = Vec::new();
    for (start, left, chord, melody) in bars {
        for (n, figure) in [0, 1, 2, 3, 1, 2, 3, 1].into_iter().enumerate() {
            notes.push((start + 0.5 * n as f64, 0.5, left[figure], 0));
        }
        for key in chord {
            notes.push((start, 2.0, key, 0));
        }
        notes.push((start + 2.0, 1.0, melody[0], 0));
        notes.push((start + 3.0, 1.0, melody[1], 0));
    }
    notes.sort_by(|a, b| a.0.total_cmp(&b.0));
    (notes.into_iter().enumerate())
        .map(|(n, (beat, length, key, _))| (beat, length, key, velocity(n)))
        .collect()
}

fn varied(n: usize) -> u8 {
    50 + (n * 13 % 40) as u8
}

/// The piece as a sequencer writes it: on the grid, never pedalled.
fn rough() -> Vec<u8> {
    smf(&piece(varied), &[], |_| 0)
}

/// The piece as played: off the grid by up to 9 ticks either way, the
/// pedal changed on each bar.
fn played() -> Vec<u8> {
    let pedal = [
        (0.1, 127),
        (3.95, 0),
        (4.1, 100),
        (7.9, 0),
        (8.1, 127),
        (16.0, 0),
    ];
    smf(&piece(varied), &pedal, |n| (n as i64 * 7919 % 19) - 9)
}

/// The piece on the grid at one velocity.
fn flat() -> Vec<u8> {
    smf(&piece(|_| 90), &[], |_| 0)
}

/// The events and tempo of `bytes`, the drum channel left out, as read.
fn read(bytes: &[u8]) -> (Vec<Timed>, Tempo) {
    let (events, tempo) = crate::song::timeline(bytes).unwrap();
    (events, tempo)
}

fn pedal(events: &[Timed]) -> Vec<(f64, bool)> {
    (events.iter())
        .filter(|e| is_pedal(e.2))
        .map(|e| ((e.0 * 1e6).round() / 1e6, e.2[2] >= 64))
        .collect()
}

fn without_pedal(events: &[Timed]) -> Vec<Timed> {
    events.iter().filter(|e| !is_pedal(e.2)).copied().collect()
}

/// Each note as (start, end, channel, key, velocity), by channel, key and
/// start: a note before and after a touch-up that moves it by a little are
/// at one place in the list.
fn notes_of(events: &[Timed]) -> Vec<(f64, f64, u8, u8, u8)> {
    let length = events.last().map_or(0.0, |e| e.0);
    let velocity = |s: &Span| events.get(s.on).map_or(0, |e| e.2[2]);
    let mut notes: Vec<_> = spans(events, length)
        .iter()
        .map(|s| (s.start, s.end, s.channel, s.key, velocity(s)))
        .collect();
    notes.sort_by(|a, b| (a.2, a.3).cmp(&(b.2, b.3)).then(a.0.total_cmp(&b.0)));
    notes
}

#[test]
fn a_rough_file_needs_a_pedal_and_a_hand_and_a_played_one_nothing() {
    let (events, tempo) = read(&rough());
    let a = analyse(&events, &tempo);
    assert_eq!(a.notes, piece(varied).len());
    assert_eq!(a.tracks, 1);
    assert!(a.unpedalled() && !a.pedal);
    assert!((a.on_grid - 1.0).abs() < 1e-9, "{}", a.on_grid);
    assert!(a.quantised());
    assert!(!a.flat(), "{}", a.one_velocity);
    assert_eq!(
        a.applies(&Touch::default()),
        Touch {
            pedal: true,
            humanise: true,
            roll: false,
            velocity: false
        }
    );
    let all = Touch {
        roll: true,
        velocity: true,
        ..Touch::default()
    };
    assert!(a.applies(&all).roll, "rolled chords come with the hand");
    assert_eq!(a.applies(&Touch::NONE), Touch::NONE);

    let (events, tempo) = read(&played());
    let a = analyse(&events, &tempo);
    assert!(a.pedal && !a.unpedalled());
    assert!(a.on_grid < 0.3, "{}", a.on_grid);
    assert!(!a.quantised() && !a.flat());
    assert_eq!(a.applies(&all), Touch::NONE);

    let (events, tempo) = read(&flat());
    let a = analyse(&events, &tempo);
    assert!(a.flat() && a.quantised());
    assert!((a.level_chords - 1.0).abs() < 1e-9);
    assert!(a.applies(&all).velocity);
}

#[test]
fn a_played_file_reads_exactly_as_written_whatever_is_asked() {
    let all = Touch {
        pedal: true,
        humanise: true,
        roll: true,
        velocity: true,
    };
    let bytes = played();
    let plain = Song::read("p.mid".into(), &bytes).unwrap();
    let (touched, what) = Song::read_with("p.mid".into(), &bytes, &all).unwrap();
    assert_eq!(what.applied, Touch::NONE);
    assert_eq!(touched.events, plain.events);
    assert_eq!(touched.length, plain.length);
}

#[test]
fn the_pedal_changes_with_the_harmony_and_lifts_in_rests() {
    let (events, tempo) = read(&rough());
    let out = auto_pedal(&events, &tempo);
    // Down just after each bar's new bass (0, 2, 4 and 6 s; and after the
    // rest, 9 s), up as the next bar strikes, and up a quarter of a second
    // into the rest (8 s to 9 s) and after the last note (11 s).
    assert_eq!(
        pedal(&out),
        [
            (0.06, true),
            (2.0, false),
            (2.06, true),
            (4.0, false),
            (4.06, true),
            (6.0, false),
            (6.06, true),
            (8.25, false),
            (9.06, true),
            (11.25, false),
        ]
    );
    // Not changed under a bar's broken chord climbing from its bass (its
    // second E3 at 1 s is the lowest note around, but the same chord).
    assert!(pedal(&out).iter().all(|&(t, _)| (t - 1.0).abs() > 0.01));
    // Nothing else changes: the same events in the same order.
    assert_eq!(without_pedal(&out), events);
    // A lift comes before the notes struck with it.
    let at_two: Vec<Message> = (out.iter()).filter(|e| e.0 == 2.0).map(|e| e.2).collect();
    assert_eq!(at_two.first(), Some(&[0xB0, 64, 0]));
}

/// Half a bar each: Am, F, C, G, Am, Am over C, Dm. The pedal changes
/// with each new chord, two beats apart (sooner than its refresh), but
/// not for Am's new bass.
#[test]
fn the_pedal_follows_a_quicker_harmony_but_not_an_inversion() {
    let halves: [([u8; 3], [u8; 2]); 7] = [
        ([45, 52, 57], [72, 76]),
        ([41, 48, 53], [72, 77]),
        ([48, 55, 60], [76, 79]),
        ([43, 50, 55], [74, 79]),
        ([45, 52, 57], [72, 76]),
        ([48, 52, 57], [72, 76]),
        ([50, 57, 62], [74, 77]),
    ];
    let mut notes: Vec<Written> = Vec::new();
    for (n, (left, right)) in halves.iter().enumerate() {
        let start = 2.0 * n as f64;
        for (step, figure) in [0, 1, 2, 1].into_iter().enumerate() {
            notes.push((start + 0.5 * step as f64, 0.5, left[figure], 70));
        }
        notes.extend(right.map(|key| (start, 2.0, key, 64)));
    }
    notes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (events, tempo) = read(&smf(&notes, &[], |_| 0));
    let lifts: Vec<f64> = (pedal(&auto_pedal(&events, &tempo)).into_iter())
        .filter(|p| !p.1)
        .map(|p| p.0)
        .collect();
    // Up at each change (a beat is half a second), not at 5 s, and a
    // quarter second after the last note.
    assert_eq!(lifts, [1.0, 2.0, 3.0, 4.0, 6.0, 7.25]);
}

#[test]
fn the_pedal_reaches_every_channel_that_plays() {
    // The right hand on channel 2, in its own track.
    let left = track(&[(0, &[0x90, 48, 80]), (960, &[0x80, 48, 0])]);
    let right = track(&[
        (0, &[0x91, 64, 80]),
        (0, &[0x91, 67, 80]),
        (960, &[0x81, 64, 0]),
        (960, &[0x81, 67, 0]),
    ]);
    let (events, tempo) = read(&file(1, DIVISION, &[left, right]));
    let out = auto_pedal(&events, &tempo);
    let channels: Vec<(u8, u16)> = (out.iter())
        .filter(|e| is_pedal(e.2))
        .map(|e| (e.2[0] & 15, e.1))
        .collect();
    assert_eq!(channels, [(0, 0), (1, 1), (0, 0), (1, 1)]);
}

#[test]
fn a_file_with_no_notes_gets_no_pedal() {
    let (events, tempo) = read(&file(0, DIVISION, &[track(&[(0, &[0xB0, 7, 100])])]));
    assert_eq!(auto_pedal(&events, &tempo), events);
}

#[test]
fn a_hand_spreads_chords_by_a_few_milliseconds_top_note_first() {
    let (events, _) = read(&rough());
    let out = humanise(&events, false);
    assert_eq!(out, humanise(&events, false), "the same every time");
    let before = notes_of(&events);
    let after = notes_of(&out);
    assert_eq!(before.len(), after.len());
    let mut moved = 0;
    for (b, a) in before.iter().zip(&after) {
        assert_eq!((a.2, a.3), (b.2, b.3), "the same keys");
        let d = a.0 - b.0;
        assert!((0.0..=SPREAD + 1e-9).contains(&d), "{b:?} -> {a:?}");
        // It keeps its length, unless its key strikes again sooner.
        assert!(a.1 - a.0 <= b.1 - b.0 + 1e-9 && a.1 - a.0 >= b.1 - b.0 - SPREAD - 1e-9);
        assert!(
            (i32::from(a.4) - i32::from(b.4)).abs() <= 9,
            "{b:?} -> {a:?}"
        );
        moved += usize::from(d > 0.0);
    }
    assert!(moved > 20, "{moved}");
    // A chord's top note keeps its time; the rest follow it.
    let chord = |at: f64, notes: &[(f64, f64, u8, u8, u8)]| -> Vec<(u8, f64)> {
        let mut c: Vec<(u8, f64)> = (before.iter().zip(notes))
            .filter(|(b, _)| b.0 == at)
            .map(|(_, a)| (a.3, a.0))
            .collect();
        c.sort_by_key(|n| n.0);
        c
    };
    let first = chord(0.0, &after);
    assert_eq!(first.len(), 4, "A2 under A4 C5 E5");
    assert_eq!(first.last(), Some(&(76, 0.0)));
    assert!(first.iter().take(3).all(|n| n.1 >= SPREAD_LEAST - 1e-9));
    // A note alone keeps its time.
    let alone = before
        .iter()
        .zip(&after)
        .find(|(b, _)| b.0 == 0.25)
        .unwrap();
    assert_eq!(alone.1.0, 0.25);
    // Every note-off still follows its note-on; nothing hangs.
    assert_eq!(after.iter().filter(|n| n.1 < n.0).count(), 0);
    let ons = out.iter().filter(|e| is_on(e.2)).count();
    let offs = out.iter().filter(|e| is_off(e.2)).count();
    assert_eq!(ons, offs);
    // Only notes change.
    let others = |e: &[Timed]| e.iter().filter(|e| !is_on(e.2) && !is_off(e.2)).count();
    assert_eq!(others(&out), others(&events));
}

#[test]
fn big_chords_roll_from_the_bottom_up() {
    let (events, _) = read(&rough());
    let out = humanise(&events, true);
    let before = notes_of(&events);
    let after = notes_of(&out);
    let mut first: Vec<(u8, f64)> = (before.iter().zip(&after))
        .filter(|(b, _)| b.0 == 0.0)
        .map(|(_, a)| (a.3, a.0))
        .collect();
    first.sort_by_key(|n| n.0);
    let times: Vec<f64> = first.iter().map(|n| n.1).collect();
    assert!(times.windows(2).all(|w| w[0] < w[1]), "{times:?}");
    assert!(
        times[0] < 0.004 && times[3] <= ROLL_SPAN + 0.3 * ROLL_STEP,
        "{times:?}"
    );
    // Two notes together are spread, not rolled: the top first.
    let pair: Vec<(u8, f64)> = (before.iter().zip(&after))
        .filter(|(b, _)| b.0 == 1.0)
        .map(|(_, a)| (a.3, a.0))
        .collect();
    assert_eq!(pair.len(), 2, "E3 and D5");
    assert!(pair.iter().any(|n| n == &(74, 1.0)));
}

#[test]
fn a_note_struck_again_at_once_is_let_go_first() {
    // The chord's lower note repeats at once: delayed, its first note
    // must not end after the repeat starts.
    let t = track(&[
        (0, &[0x90, 60, 80]),
        (0, &[0x90, 72, 80]),
        (480, &[0x80, 60, 0]),
        (480, &[0x90, 60, 80]),
        (480, &[0x90, 76, 80]),
        (960, &[0x80, 60, 0]),
        (960, &[0x80, 72, 0]),
        (960, &[0x80, 76, 0]),
    ]);
    let (events, _) = read(&file(0, DIVISION, &[t]));
    let out = humanise(&events, false);
    let keyed: Vec<(f64, Message)> = (out.iter())
        .filter(|e| e.2[1] == 60)
        .map(|e| (e.0, e.2))
        .collect();
    let kinds: Vec<bool> = keyed.iter().map(|e| is_on(e.1)).collect();
    assert_eq!(kinds, [true, false, true, false], "{keyed:?}");
    assert!(keyed.windows(2).all(|w| w[0].0 <= w[1].0));
}

#[test]
fn one_velocity_becomes_a_curve() {
    let (events, tempo) = read(&flat());
    let out = velocity_curve(&events, &tempo);
    assert_eq!(out, velocity_curve(&events, &tempo));
    let after = notes_of(&out);
    let mut seen: Vec<u8> = after.iter().map(|n| n.4).collect();
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() >= 10, "{seen:?}");
    assert!(seen.iter().all(|&v| (70..=115).contains(&v)), "{seen:?}");
    // On the beat louder than off it, a chord's top over its others.
    let mean = |pick: &dyn Fn(f64) -> bool| {
        let v: Vec<f64> = (after.iter())
            .filter(|n| pick(n.0))
            .map(|n| f64::from(n.4))
            .collect();
        v.iter().sum::<f64>() / v.len() as f64
    };
    let on = mean(&|t| (t / BEAT).fract() == 0.0);
    let off = mean(&|t| (t / BEAT).fract() != 0.0);
    assert!(on > off + 3.0, "{on} {off}");
    let first: Vec<(u8, u8)> = (after.iter())
        .filter(|n| n.0 == 0.0)
        .map(|n| (n.3, n.4))
        .collect();
    let top = first.iter().max_by_key(|n| n.0).unwrap().1;
    assert!(first.iter().all(|n| n.1 <= top), "{first:?}");
    // Only velocities change.
    let strip = |e: &[Timed]| -> Vec<(f64, u16, u8, u8)> {
        e.iter().map(|e| (e.0, e.1, e.2[0], e.2[1])).collect()
    };
    assert_eq!(strip(&out), strip(&events));
}

#[test]
fn reading_a_rough_file_touches_it_and_says_so() {
    let bytes = rough();
    let plain = Song::read("r.mid".into(), &bytes).unwrap();
    let (song, what) = Song::read_with("r.mid".into(), &bytes, &Touch::default()).unwrap();
    assert!(what.applied.pedal && what.applied.humanise);
    assert!(!what.applied.roll && !what.applied.velocity);
    assert_eq!(song.notes.len(), plain.notes.len());
    assert!(song.events.iter().any(|e| is_pedal(e.1)));
    assert!(!plain.events.iter().any(|e| is_pedal(e.1)));
    // The pedal holds the last chord a quarter second past its keys.
    assert!(
        (song.length - plain.length - 0.25).abs() < 0.02,
        "{} {}",
        song.length,
        plain.length
    );
    // As it rings, the first bar's broken chord sounds whole: Am.
    assert_eq!(
        kontra_ui::chord::chord_at(&what.ringing, 1.6).as_deref(),
        Some("Am")
    );
    assert_eq!(
        kontra_ui::chord::chord_at(&what.ringing, 3.0).as_deref(),
        Some("F")
    );
    assert_eq!(
        kontra_ui::chord::chord_at(&song.notes, 1.6),
        None,
        "keys alone"
    );
    // Read the same way twice, it is the same song.
    let (again, _) = Song::read_with("r.mid".into(), &bytes, &Touch::default()).unwrap();
    assert_eq!(again.events, song.events);
}

#[test]
fn about_a_file_is_what_it_holds_as_written() {
    let a = about(&rough()).unwrap();
    assert!((a.length - 11.0).abs() < 1e-9, "{}", a.length);
    assert!(a.analysis.quantised() && a.analysis.unpedalled());
    assert!(about(b"MThd").is_err());
    let drums = file(
        0,
        DIVISION,
        &[track(&[(0, &[0x99, 36, 100]), (60, &[0x89, 36, 0])])],
    );
    assert!(about(&drums).is_err());
}

#[test]
fn notes_ring_to_the_pedals_lift() {
    let t = track(&[
        (0, &[0xB0, 64, 127]),
        (0, &[0x90, 60, 80]),
        (240, &[0x80, 60, 0]),
        (480, &[0x90, 62, 80]),
        (600, &[0x80, 62, 0]),
        (960, &[0xB0, 64, 0]),
        (960, &[0x90, 64, 80]),
        (1200, &[0x80, 64, 0]),
        (1440, &[0x90, 60, 80]),
        (1500, &[0xB0, 64, 127]),
        (1680, &[0x90, 60, 70]),
        (1920, &[0x80, 60, 0]),
    ]);
    let (events, _) = read(&file(0, DIVISION, &[t]));
    let rung: Vec<(f64, f64, u8)> = ringing(&events, 2.0)
        .between(0.0, 10.0)
        .map(|n| (n.start, n.end, n.key))
        .collect();
    assert_eq!(
        rung,
        [
            // Let go under the pedal, they ring until it lifts.
            (0.0, 1.0, 60),
            (0.5, 1.0, 62),
            (1.0, 1.25, 64),
            // Held as the pedal goes down, it ends when struck again; the
            // last, let go under it, rings to the song's end.
            (1.5, 1.75, 60),
            (1.75, 2.0, 60),
        ]
    );
}

#[test]
fn the_tempo_map_finds_beats_and_grids() {
    let mut tempo = Tempo::new(Some(480), 0.5 / 480.0);
    tempo.change(2.0, 1920, 1.0 / 480.0);
    assert!((tempo.beat(1.0) - 0.5).abs() < 1e-12);
    assert!((tempo.beat(3.0) - 1.0).abs() < 1e-12);
    assert!((tempo.tick(3.0) - 2400.0).abs() < 1e-9);
    assert!(tempo.on_grid(2.25), "a sixteenth after the change");
    assert!(!tempo.on_grid(2.0 + 7.0 / 480.0));
    assert_eq!(tempo.in_beat(2.5), Some(0.5));
    let smpte = Tempo::new(None, 0.001);
    assert_eq!(smpte.beat(1.0), 0.5);
    assert!(!smpte.on_grid(0.0));
    assert_eq!(smpte.in_beat(0.3), None);
}
