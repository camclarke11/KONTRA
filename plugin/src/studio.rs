//! The editor's side of recording, practice and exporting: what it shows of
//! them each frame, and the header's and keyboard's actions for them
//! carried out ([`crate::record`], [`crate::practice`], [`crate::export`]).

use std::time::Instant;

use kontra_host::rack::Event;
use kontra_ui::app::{Action, App};
use kontra_ui::practice::Hands;
use kontra_ui::{export, record};

use crate::{Request, Shared};

/// Fills what the editor shows of recording, practice and exporting.
pub fn fill(shared: &Shared, app: &mut App) {
    app.record = shared.record.shown();
    app.practice = shared.song.practice.shown();
    app.export = shared.export.shown(shared.song.path().as_deref());
    // Practising, the song stops and starts at the player's notes: shown
    // where the audio thread has it, not eased (which runs on past a stop
    // and falls behind a start).
    if app.practice.hands != Hands::Off
        && let Some(song) = app.song.as_mut()
        && let Some(at) = shared.song.position_at(Instant::now())
    {
        song.position = at.min(song.length);
    }
}

/// The editor's keyboard played `event`: a take records it, practice
/// hears it.
pub fn played(shared: &Shared, event: &Event) {
    shared.record.keyboard(event);
    match *event {
        Event::NoteOn { note, .. } => shared.song.practice.press(note.key, true),
        Event::NoteOff(p) => {
            if let Ok(key) = u8::try_from(p.key) {
                shared.song.practice.press(key, false);
            }
        }
        _ => {}
    }
}

/// Carries out `action` if it is recording's, practice's or exporting's;
/// whether it was.
pub fn act(shared: &Shared, action: &Action) -> bool {
    match action {
        Action::Record(r) => match r {
            record::Do::Start => shared.record.start(),
            record::Do::Stop => shared.record.stop(),
            record::Do::CountIn(on) => shared.record.set_count_in(*on),
            record::Do::Folder => shared.record.dialog.ask(Some(shared.record.folder())),
            record::Do::PlayLast => {
                if let Some(path) = shared.record.saved() {
                    shared.request(Request::Song(path));
                }
            }
            record::Do::Reveal => {
                let folder = shared.record.folder();
                if std::fs::create_dir_all(&folder).is_ok() {
                    crate::export::reveal(&folder);
                }
            }
        },
        Action::Practice(hands) => shared.song.practice.set_hands(*hands),
        Action::Export(e) => match *e {
            export::Do::Audio => shared.request(Request::Export(crate::export::Kind::Audio)),
            export::Do::Video { height, fps } => {
                let kind = crate::export::Kind::Video { height, fps };
                shared.request(Request::Export(kind));
            }
            export::Do::Cancel => shared.export.cancel(),
            export::Do::Reveal => {
                if let Some(path) = shared.export.last() {
                    crate::export::reveal(&path);
                }
            }
        },
        _ => return false,
    }
    true
}

/// Practice, recording and exporting through the whole plugin, as a host
/// drives it: MIDI in through `process`, the tick between blocks, a click
/// instrument (one full-scale frame per note) to hear what played when.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use moose::prelude::*;

    use crate::record::smf::{Track, write};
    use crate::song::Song;
    use crate::{Dsp, Kontra, KontraParams, Tick};

    const RATE: f64 = 48_000.0;
    const BLOCK: usize = 480;
    /// Frames into the click sample its full-scale frame lies.
    const LEAD: usize = 16;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("kontra-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// The plugin with part 1 a click on every key, in `dir`.
    fn clicks(dir: &Path) -> (KontraParams, Dsp) {
        let params = KontraParams::new();
        let mut dsp = Dsp::default();
        <Kontra as PluginLogic>::reset(&mut dsp, &params, &AudioConfig::new(RATE, BLOCK));
        Tick.run(&params);
        let mut click = vec![0i16; 4800];
        click[LEAD] = i16::MAX;
        let data = 2 * click.len() as u32;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend((RATE as u32).to_le_bytes());
        wav.extend((RATE as u32 * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data.to_le_bytes());
        wav.extend(click.iter().flat_map(|s| s.to_le_bytes()));
        std::fs::write(dir.join("click.wav"), wav).unwrap();
        let sfz = "<region> sample=click.wav lokey=0 hikey=127 pitch_keycenter=60 \
                   pitch_keytrack=0 amp_veltrack=0 ampeg_attack=0 ampeg_release=0\n";
        let instrument = kontra_format_sfz::parse(sfz, dir);
        let mut rack = crate::lock(&params.shared.rack);
        rack.as_mut().unwrap().load_ir(0, instrument, None).unwrap();
        drop(rack);
        (params, dsp)
    }

    fn note(on: bool, key: u8) -> Event {
        let body = if on {
            EventBody::NoteOn {
                group: 0,
                channel: 0,
                note: key,
                velocity: 100,
            }
        } else {
            EventBody::NoteOff {
                group: 0,
                channel: 0,
                note: key,
                velocity: 0,
            }
        };
        Event::new(0, body)
    }

    /// `blocks` blocks of 10 ms, `input(n)` the host's events in block `n`,
    /// a tick every fifth; the first output's left side.
    fn render(
        params: &KontraParams,
        dsp: &mut Dsp,
        blocks: std::ops::Range<usize>,
        input: impl Fn(usize) -> Vec<Event>,
    ) -> Vec<f32> {
        let mut out = Vec::new();
        let mut channels = vec![vec![0.0f32; BLOCK]; 2 * crate::OUTPUTS];
        for n in blocks {
            let mut list = EventList::with_capacity(16);
            for e in input(n) {
                list.push(e);
            }
            let mut outs: Vec<&mut [f32]> = channels.iter_mut().map(Vec::as_mut_slice).collect();
            let mut buffer = AudioBuffer::from_slices_checked(&[], &mut outs, BLOCK);
            let transport = TransportInfo::default();
            let mut out_events = EventList::with_capacity(16);
            let mut cx = ProcessContext::new(&transport, RATE, BLOCK, &mut out_events);
            <Kontra as PluginLogic>::process(dsp, params, &mut buffer, &list, &mut cx);
            out.extend_from_slice(&channels[0]);
            if n % 5 == 0 {
                Tick.run(params);
            }
        }
        out
    }

    fn onsets(audio: &[f32], from: usize) -> Vec<usize> {
        (audio.iter().enumerate())
            .filter(|(_, x)| x.abs() > 0.5)
            .map(|(n, _)| from + n)
            .collect()
    }

    /// Right hand C at 0.5 s and D at 1 s; left hand G2 at 0.5 s.
    fn song() -> Song {
        let right = Track {
            name: "Right".into(),
            events: vec![
                (0.5, [0x90, 60, 100]),
                (0.6, [0x80, 60, 0]),
                (1.0, [0x90, 62, 100]),
                (1.1, [0x80, 62, 0]),
            ],
        };
        let left = Track {
            name: "Left".into(),
            events: vec![(0.5, [0x90, 43, 100]), (0.9, [0x80, 43, 0])],
        };
        Song::read("hands.mid".into(), &write("hands", 120.0, &[right, left])).unwrap()
    }

    /// Practising the left hand: the song stands at 0.5 s until G2 comes
    /// from the MIDI keyboard (2 s in), then plays its right hand from
    /// there, on time after the wait; it never plays G2 itself.
    #[test]
    fn practice_waits_for_the_player_through_the_plugin() {
        let scratch = Scratch::new("studio-practice");
        let (params, mut dsp) = clicks(&scratch.0);
        params
            .shared
            .song
            .practice
            .set_hands(kontra_ui::practice::Hands::Left);
        params.shared.song.play(song());
        let audio = render(&params, &mut dsp, 0..400, |n| match n {
            200 => vec![note(true, 43)],
            210 => vec![note(false, 43)],
            _ => Vec::new(),
        });
        // G2 from the player at 2 s with C from the song on the same frame,
        // then D half a second after.
        assert_eq!(onsets(&audio, 0), [96_000 + LEAD, 120_000 + LEAD]);
        // Two clicks on one frame: the player's and the song's, summed.
        let ratio = audio[96_000 + LEAD] / audio[120_000 + LEAD];
        assert!((ratio - 2.0).abs() < 0.01, "{ratio}");
        let shown = params.shared.song.practice.shown();
        assert!(!shown.waiting && shown.want.is_empty(), "{shown:?}");
        assert!(params.shared.song.status().is_none(), "the song ended");
    }

    #[test]
    fn practice_marks_the_keys_to_press_and_wrong_ones() {
        let scratch = Scratch::new("studio-marks");
        let (params, mut dsp) = clicks(&scratch.0);
        params
            .shared
            .song
            .practice
            .set_hands(kontra_ui::practice::Hands::Right);
        params.shared.song.play(song());
        render(&params, &mut dsp, 0..60, |n| {
            if n == 55 {
                vec![note(true, 61)]
            } else {
                Vec::new()
            }
        });
        let shown = params.shared.song.practice.shown();
        assert!(shown.waiting);
        assert_eq!((shown.want, shown.wrong), (vec![60], vec![61]));
        let (_, at, _) = params.shared.song.status().unwrap();
        assert!((at - 0.5).abs() < 1e-9, "stands at C: {at}");
        // The editor keyboard plays it.
        let editor_key = kontra_host::rack::Event::NoteOn {
            note: kontra_host::rack::HostNote {
                port: 255,
                channel: 0,
                key: 60,
                id: -1,
                clap: false,
            },
            velocity: 0.8,
            tune: 0.0,
        };
        super::played(&params.shared, &editor_key);
        render(&params, &mut dsp, 60..61, |_| Vec::new());
        assert_eq!(params.shared.song.practice.shown().want, [62]);
    }

    /// Recording through the plugin: a count-in's four clicks on the
    /// output, then the take from the MIDI keyboard into the folder, where
    /// Play MIDI plays it back.
    #[test]
    fn a_take_records_the_host_keyboard_after_a_count_in() {
        let scratch = Scratch::new("studio-record");
        let (params, mut dsp) = clicks(&scratch.0);
        let r = &params.shared.record;
        r.set_folder(scratch.0.join("takes"));
        r.set_count_in(true);
        r.start();
        // The count-in is 2 s (200 blocks); the take is played 2.5 s in.
        let audio = render(&params, &mut dsp, 0..300, |n| match n {
            250 => vec![note(true, 64)],
            260 => vec![note(false, 64)],
            _ => Vec::new(),
        });
        let count_in: Vec<usize> = (1..96_000)
            .filter(|&n| audio[n - 1] == 0.0 && audio[n] != 0.0)
            .collect();
        assert_eq!(count_in.len(), 4, "{count_in:?}");
        r.stop();
        render(&params, &mut dsp, 300..301, |_| Vec::new());
        let take = r.saved().expect("saved on the tick");
        assert_eq!(take, scratch.0.join("takes/Recording 1.mid"));
        let song = Song::open(&take).unwrap();
        let n = song.notes.between(0.0, 10.0).next().copied().unwrap();
        assert_eq!((n.key, n.velocity), (64, 100));
        assert!(
            (n.start - 0.5).abs() < 1e-3 && (n.end - 0.6).abs() < 1e-3,
            "{n:?}"
        );
        // Play the last take: it plays.
        assert!(super::act(
            &params.shared,
            &kontra_ui::app::Action::Record(kontra_ui::record::Do::PlayLast)
        ));
        Tick.run(&params);
        assert_eq!(
            params.shared.song.status().map(|s| s.0),
            Some("Recording 1.mid".into())
        );
    }

    /// The header's Export asks the tick, which exports the song played
    /// last on a thread of its own; with nothing played it says so.
    #[test]
    fn export_runs_from_the_header_through_the_tick() {
        let scratch = Scratch::new("studio-export");
        let (params, _dsp) = clicks(&scratch.0);
        params.shared.record.set_folder(scratch.0.join("out"));
        let export = kontra_ui::app::Action::Export(kontra_ui::export::Do::Audio);
        assert!(super::act(&params.shared, &export));
        Tick.run(&params);
        assert!(params.shared.export.shown(None).job.is_none());
        let song = scratch.0.join("two.mid");
        let t = Track {
            name: "t".into(),
            events: vec![(0.0, [0x90, 60, 100]), (0.75, [0x80, 60, 0])],
        };
        std::fs::write(&song, write("t", 120.0, &[t])).unwrap();
        params.shared.song.open(&song).unwrap();
        assert!(super::act(&params.shared, &export));
        Tick.run(&params);
        let wav = scratch.0.join("out/two.wav");
        for _ in 0..2000 {
            Tick.run(&params);
            if params.shared.export.last().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(params.shared.export.last(), Some(wav.clone()));
        // The click part is built in memory, so the export's rack (loaded
        // from the saved state) is empty: 0.75 s of silence.
        assert_eq!(std::fs::metadata(&wav).unwrap().len(), 44 + 6 * 36_000);
    }
}
