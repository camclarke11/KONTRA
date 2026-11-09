//! The song rendered offline through a rack of its own: the instruments
//! loaded again from the rack's saved state, the song played into it as
//! Play MIDI plays it ([`crate::song::Songs::process`]), each block waiting
//! for the disk as a host's bounce does ([`RackAudio::settle`]), as fast as
//! the machine renders. The parts' outputs are summed into one stereo pair
//! at the plugin's master gain, and rendering runs on past the song's last
//! event until its release tails and effects have died away.

use kontra_host::rack::{Event, OUTPUTS, Rack, RackAudio, Store, StoreConfig};

use super::Progress;
use crate::song::{Song, Songs, sort_by_frame};

/// The export's rate.
pub const RATE: u32 = 48_000;

/// Frames rendered at once, as the plugin renders a block.
const BLOCK: usize = 1024;

/// A level below this (-96 dBFS) is silence.
const SILENCE: f32 = 1.6e-5;

/// Silence this long after the song ends: its tails are done.
const QUIET: f64 = 0.5;

/// The longest tail rendered after the song's last event.
const TAIL: f64 = 30.0;

/// Kept after the last sound above silence, so a tail fades out whole.
const PAD: f64 = 0.05;

/// How the rack plays the export.
#[derive(Clone, Copy, Debug)]
pub struct Mix {
    /// The plugin's master gain, linear.
    pub gain: f32,
    /// Auto-align: hold the earlier parts so every part sounds on the beat,
    /// and start the file the latency later.
    pub align: bool,
}

/// One block of `rack`: commands carried out, the disk waited for, then
/// rendered with `events`; its outputs summed into `out` at `gain`.
fn block(
    rack: &mut Rack,
    audio: &mut RackAudio,
    events: &[(u32, Event)],
    gain: f32,
    out: &mut Vec<[f32; 2]>,
) -> f32 {
    rack.tick();
    audio.settle();
    audio.process(BLOCK, events);
    let start = out.len();
    out.resize(start + BLOCK, [0.0; 2]);
    let mut peak = 0.0f32;
    if let Some(mix) = out.get_mut(start..) {
        for n in 0..OUTPUTS {
            for (m, x) in mix.iter_mut().zip(audio.output(n, BLOCK)) {
                m[0] += x[0];
                m[1] += x[1];
            }
        }
        for m in mix.iter_mut() {
            *m = [m[0] * gain, m[1] * gain];
            peak = peak.max(m[0].abs()).max(m[1].abs());
        }
    }
    peak
}

/// Renders `song` through a rack that `setup` fills (from a saved state,
/// or built in memory), and the release tails after it. Frame 0 is the
/// song's start. `progress` shows how far it is and may cancel it.
///
/// # Errors
/// When the rack cannot start, `setup` fails, or the export is cancelled.
pub fn render(
    setup: impl FnOnce(&mut Rack) -> Result<(), String>,
    song: Song,
    mix: Mix,
    progress: &Progress,
) -> Result<Vec<[f32; 2]>, String> {
    let store = Store::new(StoreConfig::DEFAULT).map_err(|e| e.to_string())?;
    let (mut rack, mut audio) = Rack::new(RATE, store, BLOCK);
    setup(&mut rack)?;
    let latency = crate::align::update(mix.align, &mut rack) as usize;
    let rate = f64::from(RATE);
    // The parts arrive and their scripts settle, as a player gives a
    // program a moment before playing: a quarter second, not kept.
    let mut scratch = Vec::with_capacity(BLOCK);
    for _ in 0..(RATE as usize / 4).div_ceil(BLOCK) {
        scratch.clear();
        block(&mut rack, &mut audio, &[], mix.gain, &mut scratch);
    }
    let length = song.length;
    let songs = Songs::default();
    songs.play(song);
    let mut now = None;
    let mut events = Vec::with_capacity(4096);
    let mut out = Vec::with_capacity(((length + 2.0) * rate) as usize + latency);
    // Frames since the song ended, and since the last sound in them.
    let (mut tail, mut quiet) = (0usize, 0usize);
    let mut ended = false;
    loop {
        if progress.cancelled() {
            return Err("cancelled".into());
        }
        events.clear();
        if !ended {
            songs.process(&mut now, rate, BLOCK, &mut events);
            sort_by_frame(&mut events);
            ended = now.is_none() && songs.status().is_none();
        }
        let peak = block(&mut rack, &mut audio, &events, mix.gain, &mut out);
        let at = out.len().saturating_sub(latency) as f64 / rate;
        progress.set(at / (length + 1.0));
        if ended {
            tail += BLOCK;
            quiet = if peak < SILENCE { quiet + BLOCK } else { 0 };
            if quiet as f64 >= QUIET * rate || tail as f64 >= TAIL * rate {
                break;
            }
        }
    }
    // Auto-align's latency is the file's lead-in: cut, so the song is on
    // time; then the silence after its last sound, but for a breath.
    out.drain(..latency.min(out.len()));
    let last = out
        .iter()
        .rposition(|m| m[0].abs().max(m[1].abs()) >= SILENCE)
        .map_or(0, |n| n + 1);
    let keep = (last + (PAD * rate) as usize).max((length * rate).ceil() as usize);
    out.truncate(keep);
    progress.set(1.0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::smf::{Track, write};
    use std::path::{Path, PathBuf};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
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

    /// A mono 16-bit WAV at 48 kHz of `samples`.
    fn wav(path: &Path, samples: &[i16]) {
        let data = 2 * samples.len() as u32;
        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend((36 + data).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(RATE.to_le_bytes());
        out.extend((RATE * 2).to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(data.to_le_bytes());
        out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        std::fs::write(path, out).unwrap();
    }

    /// A synthetic organ: a looped 480 Hz sine on every key, released over
    /// `release` seconds.
    fn organ(dir: &Path, release: f64) -> impl FnOnce(&mut Rack) -> Result<(), String> {
        // Exactly 100 cycles, so the loop joins without a click.
        let sine: Vec<i16> = (0..10_000)
            .map(|n| (8000.0 * (std::f64::consts::TAU * 480.0 * n as f64 / 48_000.0).sin()) as i16)
            .collect();
        wav(&dir.join("sine.wav"), &sine);
        let sfz = format!(
            "<region> sample=sine.wav lokey=0 hikey=127 pitch_keycenter=60 \
             pitch_keytrack=0 amp_veltrack=0 loop_mode=loop_continuous \
             loop_start=0 loop_end=9999 ampeg_attack=0 ampeg_release={release}\n"
        );
        let instrument = kontra_format_sfz::parse(&sfz, dir);
        move |rack: &mut Rack| rack.load_ir(0, instrument, None).map_err(|e| e.to_string())
    }

    /// Two notes: C from 0 to 1 s, E from 0.5 to 1.5 s.
    fn song() -> Song {
        let t = Track {
            name: "two".into(),
            events: vec![
                (0.0, [0x90, 60, 100]),
                (0.5, [0x90, 64, 100]),
                (1.0, [0x80, 60, 0]),
                (1.5, [0x80, 64, 0]),
            ],
        };
        Song::read("two.mid".into(), &write("two", 120.0, &[t])).unwrap()
    }

    fn level(audio: &[[f32; 2]], from: f64, to: f64) -> f32 {
        let at = |s: f64| ((s * f64::from(RATE)) as usize).min(audio.len());
        audio[at(from)..at(to)]
            .iter()
            .map(|m| m[0].abs().max(m[1].abs()))
            .fold(0.0, f32::max)
    }

    /// The song from its first frame, then its release: the file runs on
    /// past the last note-off (1.5 s) by the release, then stops at the
    /// silence after it.
    #[test]
    fn an_export_runs_on_through_the_release_tail_and_stops_after_it() {
        let scratch = Scratch::new("export-tail");
        let progress = Progress::default();
        let mix = Mix {
            gain: 1.0,
            align: false,
        };
        let audio = render(organ(&scratch.0, 0.4), song(), mix, &progress).unwrap();
        let seconds = audio.len() as f64 / f64::from(RATE);
        // 1.5 s of notes, 0.4 s of release (to -96 dB it runs a little
        // past), a breath of padding.
        assert!((1.85..2.4).contains(&seconds), "{seconds} s");
        // It sounds from the start, through the notes and into the tail.
        assert!(level(&audio, 0.0, 0.01) > 0.05);
        assert!(level(&audio, 1.0, 1.4) > 0.05);
        assert!(level(&audio, 1.55, 1.65) > 0.001, "the release tail");
        // And dies away by the end.
        assert!(level(&audio, seconds - 0.02, seconds) < 0.001);
        assert!((progress.done() - 1.0).abs() < 1e-9);
    }

    /// The same song through the same instrument renders the same samples,
    /// sample for sample; a longer release makes a longer file.
    #[test]
    fn an_export_is_the_same_every_time() {
        let scratch = Scratch::new("export-same");
        let mix = Mix {
            gain: 0.5,
            align: false,
        };
        let a = render(organ(&scratch.0, 0.2), song(), mix, &Progress::default()).unwrap();
        let b = render(organ(&scratch.0, 0.2), song(), mix, &Progress::default()).unwrap();
        assert!(a == b, "two renders differ");
        let long = render(organ(&scratch.0, 1.0), song(), mix, &Progress::default()).unwrap();
        assert!(long.len() > a.len() + RATE as usize / 2);
        // Half the gain is half the level, near enough.
        let full = render(
            organ(&scratch.0, 0.2),
            song(),
            Mix { gain: 1.0, ..mix },
            &Progress::default(),
        )
        .unwrap();
        let ratio = level(&a, 0.0, 1.0) / level(&full, 0.0, 1.0);
        assert!((ratio - 0.5).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn a_cancelled_export_stops_and_keeps_nothing() {
        let scratch = Scratch::new("export-cancel");
        let progress = Progress::default();
        progress.cancel();
        let mix = Mix {
            gain: 1.0,
            align: false,
        };
        let out = render(organ(&scratch.0, 0.2), song(), mix, &progress);
        assert_eq!(out, Err("cancelled".into()));
    }

    /// An empty rack renders the song's length of silence, and stops.
    #[test]
    fn an_empty_rack_renders_silence_as_long_as_the_song() {
        let mix = Mix {
            gain: 1.0,
            align: true,
        };
        let audio = render(|_| Ok(()), song(), mix, &Progress::default()).unwrap();
        assert_eq!(audio.len(), (1.5 * f64::from(RATE)) as usize);
        assert!(audio.iter().all(|m| *m == [0.0; 2]));
    }
}
