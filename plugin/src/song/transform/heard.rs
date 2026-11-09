//! The touch-ups as heard: the player's own MIDI files played through the
//! whole plugin into an instrument made here (a decaying tone on every
//! key that follows velocity and releases in 0.3 s), as written and as
//! touched. Run with `--ignored`, naming the files in `KONTRA_ROUGH` and
//! `KONTRA_PLAYED` (`:` between), and a folder for the WAVs in
//! `KONTRA_RENDER`. A performance must render to the same samples; a rough
//! file rings longer under its pedal, its onsets a few milliseconds late at
//! most.

use std::path::{Path, PathBuf};

use kontra_library::Touch;
use moose::prelude::*;

use crate::song::Song;
use crate::{Dsp, Kontra, KontraParams, Tick};

const RATE: f64 = 48_000.0;

fn files(var: &str) -> Vec<PathBuf> {
    (std::env::var(var).unwrap_or_default().split(':'))
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// A mono 16-bit WAV at [`RATE`].
fn wav(path: &Path, samples: &[f32]) {
    let data = 2 * samples.len() as u32;
    let mut out = Vec::new();
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend((RATE as u32).to_le_bytes());
    out.extend((RATE as u32 * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    for s in samples {
        out.extend(((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

/// The plugin with part 1 on every key: a tone (A440 at key 69, dying
/// away over a few seconds, following velocity, released in 0.3 s), or a
/// click (one full-scale frame, at one level whatever the velocity).
fn piano(dir: &Path, click: bool) -> (KontraParams, Dsp) {
    let params = KontraParams::new();
    let mut dsp = Dsp::default();
    <Kontra as PluginLogic>::reset(&mut dsp, &params, &AudioConfig::new(RATE, 1024));
    Tick.run(&params);
    let sample: Vec<f32> = if click {
        let mut c = vec![0.0; 480];
        c[0] = 1.0;
        c
    } else {
        (0..(RATE as usize * 6))
            .map(|n| {
                let t = n as f64 / RATE;
                let ring = (-t / 1.5).exp() * (1.0 - (-t / 0.002).exp());
                let wave = (2.0 * std::f64::consts::PI * 440.0 * t).sin()
                    + 0.3 * (4.0 * std::f64::consts::PI * 440.0 * t).sin();
                (0.25 * ring * wave) as f32
            })
            .collect()
    };
    let file = if click { "click.wav" } else { "tone.wav" };
    wav(&dir.join(file), &sample);
    let sfz = if click {
        "<region> sample=click.wav lokey=0 hikey=127 pitch_keycenter=60 pitch_keytrack=0 amp_veltrack=0 ampeg_attack=0 ampeg_release=0\n"
    } else {
        "<region> sample=tone.wav lokey=0 hikey=127 pitch_keycenter=69 ampeg_release=0.3\n"
    };
    let instrument = kontra_format_sfz::parse(sfz, dir);
    let mut rack = crate::lock(&params.shared.rack);
    rack.as_mut().unwrap().load_ir(0, instrument, None).unwrap();
    drop(rack);
    (params, dsp)
}

/// `song` rendered whole, and a second after, from the first output's left.
fn render(dir: &Path, song: Song, click: bool) -> Vec<f32> {
    let (params, mut dsp) = piano(dir, click);
    let total = ((song.length + 1.0) * RATE) as usize;
    params.shared.song.play(song);
    let mut out = Vec::with_capacity(total);
    let mut channels = vec![vec![0.0f32; 1024]; 2 * crate::OUTPUTS];
    let events = EventList::with_capacity(16);
    while out.len() < total {
        let n = 1024.min(total - out.len());
        let mut outs: Vec<&mut [f32]> = channels.iter_mut().map(|c| &mut c[..n]).collect();
        let mut buffer = AudioBuffer::from_slices_checked(&[], &mut outs, n);
        let transport = TransportInfo::default();
        let mut out_events = EventList::with_capacity(16);
        let mut cx = ProcessContext::new(&transport, RATE, n, &mut out_events);
        <Kontra as PluginLogic>::process(&mut dsp, &params, &mut buffer, &events, &mut cx);
        out.extend_from_slice(&channels[0][..n]);
    }
    out
}

/// Each 20 ms window's RMS.
fn envelope(x: &[f32]) -> Vec<f64> {
    x.chunks(960)
        .map(|w| (w.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / w.len() as f64).sqrt())
        .collect()
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-9).log10()
}

/// The frames a click render sounds on.
fn clicks(x: &[f32]) -> Vec<usize> {
    (x.iter().enumerate())
        .filter(|(_, s)| s.abs() > 1e-3)
        .map(|(n, _)| n)
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kontra-heard-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn name(path: &Path) -> String {
    path.file_stem()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

#[test]
#[ignore = "renders the player's own MIDI files"]
fn performances_render_to_the_same_samples() {
    let all = Touch {
        pedal: true,
        humanise: true,
        roll: true,
        velocity: true,
    };
    let dir = scratch("played");
    let played = files("KONTRA_PLAYED");
    assert!(!played.is_empty(), "name them in KONTRA_PLAYED");
    for path in played {
        let bytes = std::fs::read(&path).unwrap();
        let (plain, _) = Song::read_with(name(&path), &bytes, &Touch::NONE).unwrap();
        let (touched, what) = Song::read_with(name(&path), &bytes, &all).unwrap();
        let (a, b) = (render(&dir, plain, false), render(&dir, touched, false));
        let peak = a.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        println!(
            "{}: {:.0} s rendered, peak {:.1} dB, touch-ups {:?}, samples identical: {}",
            name(&path),
            a.len() as f64 / RATE,
            db(f64::from(peak)),
            what.applied,
            a == b
        );
        assert!(a == b, "{}", name(&path));
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "renders the player's own MIDI files"]
fn a_rough_file_rings_under_its_pedal_and_keeps_its_time() {
    let dir = scratch("rough");
    let out = std::env::var_os("KONTRA_RENDER").map(PathBuf::from);
    let rough = files("KONTRA_ROUGH");
    assert!(!rough.is_empty(), "name them in KONTRA_ROUGH");
    for path in rough {
        let bytes = std::fs::read(&path).unwrap();
        let read = |touch: &Touch| Song::read_with(name(&path), &bytes, touch).unwrap().0;
        let (_, what) = Song::read_with(name(&path), &bytes, &Touch::default()).unwrap();
        assert!(what.applied.pedal && what.applied.humanise);

        // Clicks: when each note starts, nothing else.
        let (ca, cb) = (
            clicks(&render(&dir, read(&Touch::NONE), true)),
            clicks(&render(&dir, read(&Touch::default()), true)),
        );
        let late = |t: usize| ca.iter().rev().find(|&&s| s <= t).map(|&s| t - s);
        let latest = cb.iter().filter_map(|&t| late(t)).max().unwrap_or(0);
        let kept = ca.iter().filter(|s| cb.binary_search(s).is_ok()).count();
        println!(
            "{}: {} note starts as written, {} touched (chords spread); {} of the written frames still sound; latest {:.2} ms after its written frame",
            name(&path),
            ca.len(),
            cb.len(),
            kept,
            latest as f64 / RATE * 1e3
        );
        assert_eq!(
            kept,
            ca.len(),
            "each chord's top note and each lone note keep their frame"
        );
        assert!(
            cb.iter().all(|&t| late(t).is_some()),
            "nothing sooner than written"
        );
        assert!(
            latest as f64 / RATE <= 0.0105,
            "a few milliseconds late at most"
        );

        // Tones: how it rings.
        let (a, b) = (
            render(&dir, read(&Touch::NONE), false),
            render(&dir, read(&Touch::default()), false),
        );
        if let Some(out) = &out {
            std::fs::create_dir_all(out).unwrap();
            wav(&out.join(format!("{} - as written.wav", name(&path))), &a);
            wav(&out.join(format!("{} - touched.wav", name(&path))), &b);
        }
        // Where the written file holds one key or none, by 20 ms window.
        let (events, _) = crate::song::timeline(&bytes).unwrap();
        let held = super::spans(&events, f64::MAX);
        let (ea, eb) = (envelope(&a), envelope(&b));
        let sparse: Vec<usize> = (0..ea.len())
            .filter(|&w| {
                let t = w as f64 * 0.02 + 0.01;
                held.iter().filter(|s| s.start <= t && t < s.end).count() <= 1
            })
            .collect();
        let mean = |e: &[f64], at: &[usize]| {
            at.iter().filter_map(|&w| e.get(w)).sum::<f64>() / at.len().max(1) as f64
        };
        let all: Vec<usize> = (0..ea.len()).collect();
        let peak = |x: &[f32]| db(f64::from(x.iter().fold(0.0f32, |m, s| m.max(s.abs()))));
        println!(
            "  level {:.1} -> {:.1} dB; where it holds one key or none ({:.0}% of the time) {:.1} -> {:.1} dB; peak {:.1} -> {:.1} dB",
            db(mean(&ea, &all)),
            db(mean(&eb, &all)),
            100.0 * sparse.len() as f64 / ea.len() as f64,
            db(mean(&ea, &sparse)),
            db(mean(&eb, &sparse)),
            peak(&a),
            peak(&b)
        );
        assert!(
            mean(&eb, &sparse) > 1.5 * mean(&ea, &sparse),
            "the pedal holds what the keys let go"
        );
        assert!(peak(&b) < 0.0, "no clipping");
    }
    std::fs::remove_dir_all(&dir).ok();
}
