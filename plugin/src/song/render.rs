//! A song played through the whole plugin, `process` to the rack's output,
//! into an instrument built from a generated sample: every note sounds on
//! the frame its time in the file falls on, at any host block size.

use std::path::{Path, PathBuf};

use moose::prelude::*;

use super::Song;
use super::tests::{file, track};
use crate::{Dsp, Kontra, KontraParams, Tick};

pub(super) const RATE: f64 = 48_000.0;
/// Frames into the click sample its one full-scale frame lies.
pub(super) const LEAD: usize = 16;

pub(super) struct Scratch(pub(super) PathBuf);

impl Scratch {
    pub(super) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kontra-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// A mono 16-bit WAV at [`RATE`].
fn wav(path: &Path, samples: &[i16]) {
    let data = 2 * samples.len() as u32;
    let mut out = Vec::new();
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(1u16.to_le_bytes()); // mono
    out.extend((RATE as u32).to_le_bytes());
    out.extend((RATE as u32 * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    std::fs::write(path, out).unwrap();
}

/// The plugin, activated for blocks up to 4096, with part 1 a click on
/// every key: one full-scale frame [`LEAD`] frames in, unpitched, at one
/// level whatever the velocity.
pub(super) fn clicks(dir: &Path) -> (KontraParams, Dsp) {
    let params = KontraParams::new();
    let mut dsp = Dsp::default();
    <Kontra as PluginLogic>::reset(&mut dsp, &params, &AudioConfig::new(RATE, 4096));
    Tick.run(&params);
    let mut click = vec![0i16; 4800];
    click[LEAD] = i16::MAX;
    wav(&dir.join("click.wav"), &click);
    let sfz = "<region> sample=click.wav lokey=0 hikey=127 pitch_keycenter=60 \
               pitch_keytrack=0 amp_veltrack=0 ampeg_attack=0 ampeg_release=0\n";
    let instrument = kontra_format_sfz::parse(sfz, dir);
    let mut rack = crate::lock(&params.shared.rack);
    rack.as_mut().unwrap().load_ir(0, instrument, None).unwrap();
    drop(rack);
    (params, dsp)
}

/// `total` frames of the first output's left side, in blocks of `sizes`
/// in turn, as a live host asks for them.
pub(super) fn render(
    params: &KontraParams,
    dsp: &mut Dsp,
    sizes: &[usize],
    total: usize,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(total);
    let mut channels = vec![vec![0.0f32; 4096]; 2 * crate::OUTPUTS];
    let events = EventList::with_capacity(16);
    for &size in sizes.iter().cycle() {
        let n = size.min(total - out.len());
        let mut outs: Vec<&mut [f32]> = channels.iter_mut().map(|c| &mut c[..n]).collect();
        let mut buffer = AudioBuffer::from_slices_checked(&[], &mut outs, n);
        let transport = TransportInfo::default();
        let mut out_events = EventList::with_capacity(16);
        let mut cx = ProcessContext::new(&transport, RATE, n, &mut out_events);
        <Kontra as PluginLogic>::process(dsp, params, &mut buffer, &events, &mut cx);
        out.extend_from_slice(&channels[0][..n]);
        if out.len() == total {
            break;
        }
    }
    out
}

/// At 90 BPM a tick of 480 to the quarter is 66.67 frames: notes one tick
/// apart, on ticks no block size divides, in two tracks and channels.
pub(super) fn song() -> Song {
    // 666,666 µs a quarter.
    let tempo = track(&[(0, &[0xFF, 0x51, 3, 0x0A, 0x2C, 0x2A])]);
    let a = track(&[
        (0, &[0x90, 60, 100]),
        (7, &[0x80, 60, 0]),
        (100, &[0x90, 62, 1]),
        (333, &[0x80, 62, 0]),
        (480, &[0x90, 64, 127]),
        (481, &[0x80, 64, 0]),
        (1000, &[0x90, 65, 80]),
        (1100, &[0x80, 65, 0]),
    ]);
    let b = track(&[
        (481, &[0x91, 67, 90]),
        (1001, &[0x91, 69, 90]),
        (1500, &[0x81, 69, 0]),
    ]);
    Song::read("odd.mid".into(), &file(1, 480, &[tempo, a, b])).unwrap()
}

/// The frames a note starts on, by the file.
fn expected(song: &Song) -> Vec<usize> {
    (song.events.iter())
        .filter(|&&(_, [status, _, velocity])| status & 0xF0 == 0x90 && velocity > 0)
        .map(|&(time, _)| (time * RATE).round() as usize + LEAD)
        .collect()
}

#[test]
fn a_song_plays_every_note_on_its_frame_at_any_block_size() {
    let scratch = Scratch::new("song-render");
    let want = expected(&song());
    assert_eq!(want.len(), 6);
    let total = ((song().length + 0.2) * RATE) as usize;
    let mut first: Option<Vec<f32>> = None;
    for sizes in [&[64][..], &[333], &[1024], &[4096], &[471, 470, 512, 33]] {
        let (params, mut dsp) = clicks(&scratch.0);
        params.shared.song.play(song());
        let out = render(&params, &mut dsp, sizes, total);
        let onsets: Vec<usize> = (out.iter().enumerate())
            .filter(|(_, x)| x.abs() > 1e-3)
            .map(|(n, _)| n)
            .collect();
        assert_eq!(onsets, want, "blocks of {sizes:?}");
        match &first {
            None => first = Some(out),
            Some(first) => assert!(*first == out, "blocks of {sizes:?} differ"),
        }
        assert!(params.shared.song.status().is_none(), "it ended");
    }
}
