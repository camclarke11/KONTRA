//! The transport driven the way the editor and the audio thread drive it:
//! settings and seeks set between blocks, events read back at their frame
//! from the song's start in output frames.

use std::time::{Duration, Instant};

use kontra_host::rack::{Event, HostPattern};

use super::super::tests::{file, song, track};
use super::super::{PORT, Song, Songs};
use super::{Playing, SHORTEST};

const RATE: f64 = 48_000.;

/// The output so far: each event at its frame from the first block, and
/// each click with whether it is a bar's first.
#[derive(Default)]
struct Heard {
    frame: u64,
    events: Vec<(u64, Event)>,
    clicks: Vec<(u64, bool)>,
}

impl Heard {
    /// `blocks` blocks of `frames`.
    fn run(&mut self, songs: &Songs, now: &mut Option<Playing>, frames: usize, blocks: usize) {
        let mut out = Vec::with_capacity(256);
        for _ in 0..blocks {
            out.clear();
            songs.process(now, RATE, frames, &mut out);
            assert_eq!(out.capacity(), 256, "the audio thread never grows it");
            super::super::sort_by_frame(&mut out);
            let at = self.frame;
            self.events
                .extend(out.iter().map(|&(f, e)| (at + u64::from(f), e)));
            if let Some(p) = now.as_ref() {
                let list = p.clicks.list.get(..p.clicks.len).unwrap_or_default();
                self.clicks
                    .extend(list.iter().map(|&(f, a)| (at + u64::from(f), a)));
            }
            self.frame += frames as u64;
        }
    }

    /// Until `frame` in blocks of `frames` (the last one shorter).
    fn until(&mut self, songs: &Songs, now: &mut Option<Playing>, frames: usize, frame: u64) {
        while self.frame < frame {
            let n = (frame - self.frame).min(frames as u64) as usize;
            self.run(songs, now, n, 1);
        }
    }

    /// The note-ons heard, as (frame, key).
    fn ons(&self) -> Vec<(u64, u8)> {
        (self.events.iter())
            .filter_map(|(f, e)| match e {
                Event::NoteOn { note, .. } => Some((*f, note.key)),
                _ => None,
            })
            .collect()
    }

    /// Everything heard from `frame` on.
    fn since(&self, frame: u64) -> Vec<(u64, Event)> {
        (self.events.iter())
            .filter(|(f, _)| *f >= frame)
            .copied()
            .collect()
    }
}

fn off(channel: u8, key: u8) -> Event {
    Event::NoteOff(HostPattern {
        port: i32::from(PORT),
        channel: i32::from(channel),
        key: i32::from(key),
        id: -1,
        clap: false,
    })
}

fn pedal(channel: u8, value: u8) -> Event {
    Event::midi1(0xB0 | channel, 64, value)
}

fn songs(song: Song) -> Songs {
    let songs = Songs::default();
    songs.play(song);
    songs
}

/// song(): pedal down, C D E F a quarter each at 120 BPM (frames 0,
/// 24000, 48000, 72000), G at 60 BPM (96000) to 3 s.
fn scale() -> Song {
    Song::read("a.mid".into(), &song()).unwrap()
}

/// At 120 BPM: pedal down at 0, up at 1 s, down at 2 s; C held 0..3 s,
/// E 1.5..1.75 s, G 2.5..3 s.
fn pedalled() -> Song {
    let t = track(&[
        (0, &[0xB0, 64, 127]),
        (0, &[0x90, 60, 90]),
        (960, &[0xB0, 64, 0]),
        (1440, &[0x90, 64, 90]),
        (1680, &[0x80, 64, 0]),
        (1920, &[0xB0, 64, 127]),
        (2400, &[0x90, 67, 90]),
        (2880, &[0x80, 60, 0]),
        (2880, &[0x80, 67, 0]),
    ]);
    Song::read("p.mid".into(), &file(0, 480, &[t])).unwrap()
}

/// Two hands at 120 BPM: the right (track 0, channel 1) plays C5 and D5 a
/// beat each, the left (track 1, channel 2) holds C3 for the bar.
fn hands() -> Song {
    let right = track(&[
        (0, &[0x90, 72, 90]),
        (480, &[0x80, 72, 0]),
        (480, &[0x90, 74, 90]),
        (960, &[0x80, 74, 0]),
    ]);
    let left = track(&[(0, &[0x91, 48, 70]), (1920, &[0x81, 48, 0])]);
    Song::read("h.mid".into(), &file(1, 480, &[right, left])).unwrap()
}

#[test]
fn pausing_ends_the_notes_and_resuming_brings_back_the_pedal() {
    let songs = songs(scale());
    let mut now = None;
    let mut heard = Heard::default();
    // Into C, the pedal down.
    heard.run(&songs, &mut now, 512, 10);
    let at = heard.frame;
    songs.transport().pause(true);
    let paused_at = songs.position_at(Instant::now()).unwrap();
    heard.run(&songs, &mut now, 512, 1);
    assert_eq!(heard.since(at), [(at, off(0, 60)), (at, pedal(0, 0))]);
    // Paused: silent, the playhead still, the song still loaded.
    heard.run(&songs, &mut now, 512, 100);
    assert_eq!(heard.since(at).len(), 2);
    let still = songs.position_at(Instant::now() + Duration::from_secs(1));
    assert!(
        still.is_some_and(|p| (p - 5120. / RATE).abs() < 1e-9),
        "{still:?}"
    );
    assert!(paused_at <= 5120. / RATE + 1e-9);
    assert!(songs.now().is_some_and(|n| n.paused && !n.ended));
    // Resumed: the pedal goes down again at once, C is not struck again,
    // and D comes as many frames on as the song had left to it.
    songs.transport().pause(false);
    let resumed = heard.frame;
    heard.until(&songs, &mut now, 512, resumed + 30_000);
    assert_eq!(
        heard.since(resumed).first(),
        Some(&(resumed, pedal(0, 127)))
    );
    let d = resumed + 24_000 - 5120;
    assert!(heard.ons().contains(&(d, 62)), "{:?}", heard.ons());
    assert_eq!(heard.ons().iter().filter(|o| o.1 == 60).count(), 1);
    // C's own note-off, which it has no note for, is not sent.
    assert!(!heard.since(resumed).iter().any(|(_, e)| *e == off(0, 60)));
}

#[test]
fn seeking_silences_what_sounds_and_sets_the_pedal_as_it_is_there() {
    let songs = songs(pedalled());
    let mut now = None;
    let mut heard = Heard::default();
    heard.run(&songs, &mut now, 512, 20);
    // To 1.5 s, where the pedal is up and E starts.
    songs.transport().seek(1.5);
    let at = heard.frame;
    heard.run(&songs, &mut now, 512, 1);
    let struck = |key| matches!(heard.since(at).last(), Some((_, Event::NoteOn { note, .. })) if note.key == key);
    assert_eq!(
        heard.since(at).get(..2),
        Some(&[(at, off(0, 60)), (at, pedal(0, 0))][..])
    );
    assert!(struck(64), "E on the seek's frame: {:?}", heard.since(at));
    assert_eq!(heard.since(at).len(), 3, "the pedal was up there");
    // To 2.2 s: the pedal is down there again; G follows 0.3 s on.
    songs.transport().seek(2.2);
    let at = heard.frame;
    heard.until(&songs, &mut now, 512, at + 20_000);
    assert_eq!(
        heard.since(at).get(..3),
        Some(&[(at, off(0, 64)), (at, pedal(0, 0)), (at, pedal(0, 127))][..])
    );
    assert_eq!(heard.ons().last(), Some(&(at + 14_400, 67)));
    // Paused, a seek moves the playhead and nothing sounds.
    songs.transport().pause(true);
    heard.run(&songs, &mut now, 512, 1);
    songs.transport().seek(0.25);
    let at = heard.frame;
    heard.run(&songs, &mut now, 512, 4);
    let position = songs.position_at(Instant::now() + Duration::from_secs(1));
    assert_eq!(position, Some(0.25));
    assert!(heard.ons().iter().all(|o| o.0 < at), "silent while paused");
    // Resumed there: the pedal as it is at 0.25 s, down.
    songs.transport().pause(false);
    let at = heard.frame;
    heard.run(&songs, &mut now, 512, 1);
    assert_eq!(heard.since(at).first(), Some(&(at, pedal(0, 127))));
}

#[test]
fn speed_scales_time_on_every_frame_at_any_block_size() {
    let notes = [0u64, 24_000, 48_000, 72_000, 96_000];
    for speed in [0.25, 0.5, 0.75, 1., 1.5, 2.] {
        let mut first: Option<Vec<(u64, Event)>> = None;
        for frames in [64, 333, 512, 4096] {
            let songs = songs(scale());
            songs.transport().set_speed(speed);
            let mut now = None;
            let mut heard = Heard::default();
            heard.until(&songs, &mut now, frames, (150_000. / speed) as u64 + 8192);
            let want: Vec<(u64, u8)> = (notes.iter().zip([60, 62, 64, 65, 67]))
                .map(|(&f, k)| ((f as f64 / speed).floor() as u64, k))
                .collect();
            assert_eq!(heard.ons(), want, "{speed}x in blocks of {frames}");
            // The pedal lifts where G ends, 3 s of the song on.
            let up = heard.events.iter().rev().find(|e| e.1 == pedal(0, 0));
            assert_eq!(up.map(|e| e.0), Some((144_000. / speed).floor() as u64));
            match &first {
                None => first = Some(heard.events),
                Some(first) => assert_eq!(*first, heard.events, "{speed}x, {frames}"),
            }
        }
    }
}

#[test]
fn a_speed_change_takes_the_song_on_from_where_it_is() {
    let songs = songs(scale());
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 1000, 30_000);
    songs.transport().set_speed(0.5);
    heard.until(&songs, &mut now, 1000, 90_000);
    // E at song frame 48000: 18000 song frames after the change, at half speed.
    assert!(heard.ons().contains(&(66_000, 64)), "{:?}", heard.ons());
    // The playhead runs at half speed between blocks.
    let head = &songs.transport().head;
    let (base, end) = (head.at(Instant::now()), head.end());
    assert!((end - (30_000. + 30_000.) / RATE).abs() < 1e-9, "{end}");
    let later = head.at(Instant::now() + Duration::from_millis(10));
    assert!(later >= base && later <= end);
    let at = songs
        .position_at(Instant::now() + Duration::from_secs(5))
        .unwrap();
    assert!((at - end).abs() < 1e-9, "never past what is rendered");
}

#[test]
fn the_playhead_moves_at_the_songs_speed_between_blocks() {
    let songs = songs(scale());
    songs.transport().set_speed(2.);
    let mut now = None;
    let mut heard = Heard::default();
    // A tenth of a second of output: a fifth of the song.
    heard.run(&songs, &mut now, 4800, 1);
    let rendered = Instant::now();
    let at = |ms| {
        songs
            .position_at(rendered + Duration::from_millis(ms))
            .unwrap()
    };
    let half = at(50);
    assert!((0.1..=0.2).contains(&half), "{half}");
    assert!(
        (half - 0.1).abs() < 0.01,
        "50 ms is 0.1 s of the song: {half}"
    );
    assert_eq!(at(1000), 0.2, "held where the block ended");
}

#[test]
fn a_loop_wraps_from_b_to_a_on_its_frame_at_any_block_size() {
    let mut first: Option<Vec<(u64, Event)>> = None;
    for frames in [64, 333, 512, 1000, 4096] {
        let songs = songs(scale());
        songs.transport().set_marks(Some(0.5), Some(1.5));
        songs.transport().set_looping(true);
        let mut now = None;
        let mut heard = Heard::default();
        heard.until(&songs, &mut now, frames, 200_000);
        // C D E, then from 72000 on D E again, a second a turn.
        assert_eq!(
            heard.ons(),
            [
                (0, 60),
                (24_000, 62),
                (48_000, 64),
                (72_000, 62),
                (96_000, 64),
                (120_000, 62),
                (144_000, 64),
                (168_000, 62),
                (192_000, 64),
            ],
            "blocks of {frames}"
        );
        // At the wrap: E ends, the pedal lifts and goes down again (it is
        // down at A), then D.
        let wrap = heard.since(72_000);
        assert_eq!(
            wrap.get(..3),
            Some(
                &[
                    (72_000, off(0, 64)),
                    (72_000, pedal(0, 0)),
                    (72_000, pedal(0, 127))
                ][..]
            ),
            "blocks of {frames}"
        );
        assert!(
            now.as_ref().is_some_and(|p| !p.ended()),
            "a loop never ends"
        );
        match &first {
            None => first = Some(heard.events),
            Some(first) => assert_eq!(*first, heard.events, "blocks of {frames}"),
        }
    }
}

#[test]
fn a_loop_set_behind_the_playhead_takes_it_back_and_a_short_one_plays_through() {
    let songs = songs(scale());
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 80_000);
    songs.transport().set_marks(Some(0.0), Some(1.0));
    songs.transport().set_looping(true);
    let at = heard.frame;
    heard.run(&songs, &mut now, 512, 1);
    assert!(
        heard.ons().contains(&(at, 60)),
        "back to A: {:?}",
        heard.ons()
    );
    // Too short to loop: it plays on through B.
    let songs = self::songs(scale());
    songs
        .transport()
        .set_marks(Some(0.5), Some(0.5 + SHORTEST / 2.));
    songs.transport().set_looping(true);
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 100_000);
    assert_eq!(heard.ons().len(), 5);
    // On with no marks: the whole song.
    let songs = self::songs(scale());
    songs.transport().set_looping(true);
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 150_000);
    assert!(heard.ons().contains(&(144_000, 60)), "{:?}", heard.ons());
}

#[test]
fn transpose_moves_each_note_and_ends_the_key_it_struck() {
    let songs = songs(scale());
    songs.transport().set_transpose(3);
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 12_000);
    // Moved while C sounds: C still ends on the key it struck.
    songs.transport().set_transpose(-2);
    heard.until(&songs, &mut now, 512, 30_000);
    assert_eq!(heard.ons(), [(0, 63), (24_000, 60)]);
    assert!(heard.events.contains(&(24_000, off(0, 63))));
    assert!(
        !heard.events.iter().any(|e| e.1 == off(0, 60)),
        "D sounds on"
    );
    // Out of range: not played.
    songs.transport().set_transpose(100);
    assert_eq!(songs.transport().settings().transpose, 12);
}

#[test]
fn a_muted_voice_or_hand_plays_nothing_and_what_it_held_ends() {
    // The left hand, a track of its own: its notes go.
    let songs = songs(hands());
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 6_000);
    songs.transport().set_mute(1 << 1);
    let at = heard.frame;
    heard.until(&songs, &mut now, 512, 60_000);
    assert_eq!(
        heard.since(at).first(),
        Some(&(at, off(1, 48))),
        "ended at once"
    );
    assert_eq!(heard.ons(), [(0, 72), (0, 48), (24_000, 74)]);
    // Its note-off later has nothing to end.
    assert_eq!(heard.events.iter().filter(|e| e.1 == off(1, 48)).count(), 1);
    // One voice: the hands split at middle C.
    let songs = self::songs(pedalled());
    songs.transport().set_hands(false, true, 62);
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 150_000);
    assert_eq!(heard.ons(), [(0, 60)], "E and G are the right hand's");
}

#[test]
fn the_metronome_clicks_on_the_tempo_maps_beats() {
    for speed in [1., 2.] {
        let songs = songs(scale());
        songs.transport().set_metronome(true, 0.8);
        songs.transport().set_speed(speed);
        let mut now = None;
        let mut heard = Heard::default();
        heard.until(&songs, &mut now, 333, (150_000. / speed) as u64);
        // 120 BPM to 2 s, then 60 BPM: the bar's first beat accented.
        let want: Vec<(u64, bool)> = [0, 24_000, 48_000, 72_000, 96_000, 144_000]
            .iter()
            .zip([true, false, false, false, true, false])
            .map(|(&f, a)| ((f as f64 / speed).floor() as u64, a))
            .collect();
        assert_eq!(heard.clicks, want, "{speed}x");
    }
}

#[test]
fn a_click_sounds_from_its_frame_and_a_bars_first_is_louder() {
    let songs = songs(scale());
    songs.transport().set_metronome(true, 1.);
    let mut now = None;
    let mut out = Vec::with_capacity(64);
    let mut audio = vec![0f32; 100_000];
    let mut frame = 0;
    while frame < audio.len() {
        let n = 512.min(audio.len() - frame);
        out.clear();
        songs.process(&mut now, RATE, n, &mut out);
        if let Some(p) = now.as_mut() {
            p.click(RATE, n, |at, click| {
                let to = &mut audio[frame + at..frame + at + click.len()];
                to.iter_mut().zip(click).for_each(|(o, c)| *o += c);
            });
        }
        frame += n;
    }
    let peak = |from: usize, to: usize| audio[from..to].iter().fold(0f32, |m, x| m.max(x.abs()));
    assert_eq!(audio[0], 0., "a sine from its frame");
    assert!(audio[1] != 0.);
    assert!(
        peak(0, 2400) > peak(24_000, 26_400) * 1.3,
        "the downbeat is louder"
    );
    assert!(peak(24_000, 26_400) > 0.1);
    assert_eq!(peak(2_500, 24_000), 0., "silent between beats");
    assert_eq!(peak(23_000, 24_001), 0.);
}

#[test]
fn a_count_in_clicks_up_to_the_first_note() {
    let songs = Songs::default();
    songs.transport().set_count_in(1);
    songs.play(scale());
    // Shown from the count-in's start before a block plays.
    assert_eq!(songs.position_at(Instant::now()), Some(-2.0));
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 160_000);
    // A bar of 4/4 at 120 BPM: four clicks, then C on the next beat.
    assert_eq!(
        heard.clicks,
        [(0, true), (24_000, false), (48_000, false), (72_000, false)]
    );
    assert_eq!(heard.ons().first(), Some(&(96_000, 60)));
    // With the metronome on as well: one click a beat, never two.
    let songs = Songs::default();
    songs.transport().set_count_in(2);
    songs.transport().set_metronome(true, 0.5);
    songs.play(scale());
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 4 * 48_000 + 30_000);
    let frames: Vec<u64> = heard.clicks.iter().map(|c| c.0).collect();
    let want: Vec<u64> = (0..10).map(|n| n * 24_000).collect();
    assert_eq!(frames, want);
    assert_eq!(heard.ons().first(), Some(&(192_000, 60)));
}

#[test]
fn a_count_in_skips_a_long_silence_before_the_first_note() {
    // The first note at 3 s (120 BPM): a bar's count-in starts at 1 s,
    // the pedal pressed at 0 still down when the note comes.
    let t = track(&[
        (0, &[0xB0, 64, 127]),
        (2880, &[0x90, 60, 90]),
        (3360, &[0x80, 60, 0]),
    ]);
    let song = Song::read("late.mid".into(), &file(0, 480, &[t])).unwrap();
    let songs = Songs::default();
    songs.transport().set_count_in(1);
    songs.play(song);
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 120_000);
    assert_eq!(heard.events.first(), Some(&(0, pedal(0, 127))));
    assert_eq!(heard.ons(), [(96_000, 60)]);
}

#[test]
fn an_ended_song_waits_at_its_end_and_plays_again() {
    let songs = songs(scale());
    let take = songs.transport().take();
    let mut now = None;
    let mut heard = Heard::default();
    heard.until(&songs, &mut now, 512, 150_000);
    let n = songs.now().unwrap();
    assert!(
        n.ended && n.paused && (n.position - 3.0).abs() < 1e-9,
        "{n:?}"
    );
    assert!(songs.status().is_none(), "the header's song has ended");
    // Nothing more while it waits.
    let at = heard.frame;
    heard.run(&songs, &mut now, 512, 50);
    assert!(heard.since(at).is_empty());
    // Played again: from its start, a new take (the title card shows).
    songs.transport().pause(false);
    let at = heard.frame;
    heard.run(&songs, &mut now, 512, 1);
    assert!(heard.ons().contains(&(at, 60)));
    assert_eq!(songs.transport().take(), take + 1);
    assert!(songs.status().is_some());
    assert!(songs.now().is_some_and(|n| !n.ended && !n.paused));
    // Stopped: unloaded.
    songs.stop();
    heard.run(&songs, &mut now, 512, 1);
    assert!(now.is_none() && songs.now().is_none());
}

#[test]
fn a_new_song_starts_without_the_last_ones_loop_or_mutes() {
    let songs = songs(scale());
    let t = songs.transport();
    t.set_marks(Some(1.), Some(2.));
    t.set_looping(true);
    t.set_mute(3);
    t.set_hands(true, true, 60);
    t.set_speed(0.5);
    t.set_transpose(2);
    t.pause(true);
    songs.play(scale());
    let s = t.settings();
    assert_eq!(
        (s.a, s.b, s.looping, s.mute, s.left, s.right),
        (None, None, false, 0, false, false)
    );
    assert!(!t.paused());
    // What the player chose for every song stays.
    assert_eq!((s.speed, s.transpose), (0.5, 2));
}

#[test]
fn the_song_read_again_plays_on_from_where_it_was_with_its_loop() {
    let songs = songs(scale());
    let t = songs.transport();
    let mut now = None;
    let mut heard = Heard::default();
    heard.run(&songs, &mut now, 512, 10);
    t.set_marks(Some(1.), Some(2.));
    // Read again (other touch-ups) 1 s in, where E starts.
    let mut again = scale();
    again.from = Some(1.);
    songs.play(again);
    let s = t.settings();
    assert_eq!((s.a, s.b), (Some(1.), Some(2.)), "its loop stays");
    let at = heard.frame;
    heard.run(&songs, &mut now, 512, 1);
    assert_eq!(heard.ons().last(), Some(&(at, 64)), "E on its first frame");
    let position = songs.position_at(Instant::now());
    assert!(position.is_some_and(|p| p >= 1.), "{position:?}");
}

#[test]
fn practice_waits_for_the_players_chord_then_plays_on_at_any_speed() {
    use kontra_host::rack::HostNote;
    use kontra_ui::practice::Hands;
    let songs = Songs::default();
    songs.practice.set_hands(Hands::Right);
    songs.play(hands());
    songs.transport().set_speed(2.);
    let mut now = None;
    let mut out = Vec::with_capacity(256);
    let mut ons = Vec::new();
    let mut frame = 0u64;
    let mut block = |input: &[(u32, Event)], ons: &mut Vec<(u64, u8)>| {
        out.clear();
        out.extend_from_slice(input);
        songs.process(&mut now, RATE, 512, &mut out);
        let played = out.get(input.len()..).unwrap_or_default();
        ons.extend(played.iter().filter_map(|(f, e)| match e {
            Event::NoteOn { note, .. } => Some((frame + u64::from(*f), note.key)),
            _ => None,
        }));
        frame += 512;
    };
    // The right hand's C5 starts the song: it waits, the left's C3 too.
    for _ in 0..10 {
        block(&[], &mut ons);
    }
    assert!(ons.is_empty(), "nothing before the player: {ons:?}");
    let at = songs.position_at(Instant::now() + Duration::from_secs(1));
    assert_eq!(at, Some(0.), "the playhead waits");
    // Struck 100 frames into a block: C3 on that frame, C5 the player's.
    let c5 = HostNote {
        port: 0,
        channel: 0,
        key: 72,
        id: -1,
        clap: false,
    };
    let struck = 10 * 512 + 100;
    let on = Event::NoteOn {
        note: c5,
        velocity: 0.8,
        tune: 0.,
    };
    block(&[(100, on)], &mut ons);
    assert_eq!(ons, [(struck, 48)]);
    // At double speed it reaches D5, 0.5 s in, after 0.25 s, and waits.
    for _ in 0..60 {
        block(&[], &mut ons);
    }
    assert_eq!(ons, [(struck, 48)], "D5 is the player's");
    let at = songs.position_at(Instant::now() + Duration::from_secs(1));
    assert_eq!(at, Some(0.5), "waiting at D5");
}
