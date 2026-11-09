//! The transport through the whole plugin, `process` to the first output,
//! into song::render's click instrument (one full-scale frame [`LEAD`]
//! frames after each note starts): speed, seeks, loops, the count-in and
//! the metronome land on their frames at any host block size.

use moose::prelude::*;

use super::super::render::{LEAD, RATE, Scratch, clicks, render, song};
use crate::{Dsp, Kontra, KontraParams, Tick};

/// The frames a note sounds on, the song's notes at `speed`.
fn heard(speed: f64) -> Vec<usize> {
    (song().events.iter())
        .filter(|&&(_, [status, _, velocity])| status & 0xF0 == 0x90 && velocity > 0)
        .map(|&(time, _)| ((time * RATE).round() / speed).floor() as usize + LEAD)
        .collect()
}

/// Where the output is not silent.
fn onsets(out: &[f32]) -> Vec<usize> {
    (out.iter().enumerate())
        .filter(|(_, x)| x.abs() > 1e-3)
        .map(|(n, _)| n)
        .collect()
}

const SIZES: [&[usize]; 4] = [&[64], &[333], &[4096], &[471, 470, 512, 33]];

#[test]
fn a_slowed_or_hurried_song_plays_each_note_on_its_frame() {
    let scratch = Scratch::new("transport-speed");
    for speed in [0.5, 0.75, 2.] {
        let total = ((song().length + 0.2) * RATE / speed) as usize;
        for sizes in SIZES {
            let (params, mut dsp) = clicks(&scratch.0);
            params.shared.song.transport().set_speed(speed);
            params.shared.song.play(song());
            let out = render(&params, &mut dsp, sizes, total);
            assert_eq!(
                onsets(&out),
                heard(speed),
                "{speed}x in blocks of {sizes:?}"
            );
        }
    }
}

#[test]
fn a_seek_plays_on_from_its_place_and_a_loop_comes_round() {
    let scratch = Scratch::new("transport-seek");
    for sizes in SIZES {
        let (params, mut dsp) = clicks(&scratch.0);
        let songs = &params.shared.song;
        songs.play(song());
        let mut out = render(&params, &mut dsp, sizes, 10_000);
        // Back to the third note's time: from there on, as the file has it.
        let third = heard(1.).get(2).copied().unwrap() - LEAD;
        songs.transport().seek(third as f64 / RATE);
        out.extend(render(&params, &mut dsp, sizes, 120_000));
        // The first two notes sound before it.
        let want: Vec<usize> = (heard(1.).into_iter().take(2))
            .chain(heard(1.).into_iter().skip(2).map(|f| f - third + 10_000))
            .collect();
        assert_eq!(onsets(&out), want, "blocks of {sizes:?}");

        // A loop round the second and third notes, twice.
        let (params, mut dsp) = clicks(&scratch.0);
        let songs = &params.shared.song;
        let notes = heard(1.);
        let (a, b) = (notes[1] - LEAD, notes[3] - LEAD);
        // Set once it plays: a new song starts without the last one's loop.
        songs.play(song());
        songs
            .transport()
            .set_marks(Some(a as f64 / RATE), Some(b as f64 / RATE));
        songs.transport().set_looping(true);
        let out = render(&params, &mut dsp, sizes, b + 2 * (b - a) - 1);
        let turn = b - a;
        let want = [
            notes[0],
            notes[1],
            notes[2],
            notes[1] + turn,
            notes[2] + turn,
            notes[1] + 2 * turn,
            notes[2] + 2 * turn,
        ];
        assert_eq!(onsets(&out), want, "blocks of {sizes:?}");
    }
}

/// The plugin with nothing loaded: what it puts out is the metronome.
fn bare() -> (KontraParams, Dsp) {
    let params = KontraParams::new();
    let mut dsp = Dsp::default();
    <Kontra as PluginLogic>::reset(&mut dsp, &params, &AudioConfig::new(RATE, 4096));
    Tick.run(&params);
    (params, dsp)
}

/// Where each click starts: a sine from its frame, so silent there and
/// sounding the frame after, out of a silence.
fn ticks(out: &[f32]) -> Vec<usize> {
    (1..out.len())
        .filter(|&n| out[n].abs() > 1e-6 && out[n.saturating_sub(200)..n].iter().all(|x| *x == 0.))
        .map(|n| n - 1)
        .collect()
}

#[test]
fn the_metronome_and_count_in_click_on_the_beat_into_the_output() {
    // song(): 90 BPM, a beat 32000 frames; four beats to its end.
    let beat = 32_000;
    for sizes in SIZES {
        let (params, mut dsp) = bare();
        params.shared.song.transport().set_metronome(true, 1.);
        params.shared.song.play(song());
        let out = render(&params, &mut dsp, sizes, 4 * beat);
        assert_eq!(
            ticks(&out),
            [0, beat, 2 * beat, 3 * beat],
            "blocks of {sizes:?}"
        );
        assert!(out.iter().any(|x| x.abs() > 0.3), "loud enough to hear");
        // A bar's count-in (the file's 4/4 default), then the first note.
        let scratch = Scratch::new("transport-count");
        let (params, mut dsp) = clicks(&scratch.0);
        params.shared.song.transport().set_count_in(1);
        params.shared.song.play(song());
        let out = render(&params, &mut dsp, sizes, 5 * beat);
        let first = onsets(&out).into_iter().find(|n| out[*n] > 0.9);
        assert_eq!(first, Some(4 * beat + LEAD), "blocks of {sizes:?}");
        assert_eq!(
            ticks(&out).get(..4),
            Some(&[0, beat, 2 * beat, 3 * beat][..])
        );
    }
}
