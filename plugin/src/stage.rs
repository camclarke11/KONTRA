//! The editor's side of a song's transport and the Stage: the transport as
//! [`kontra_ui::stage`] shows it, filled from [`Songs`] each frame; what
//! its controls ask, carried out on [`Songs::transport`]; Space, F and
//! Escape taken from the window for it; and the window's full screen.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use kontra_ui::app;
use kontra_ui::stage::{Command, Hand, Shortcut, Transport};
use kontra_ui::waterfall::{Clock, Note, Notes};
use moose::mui::mui::host::{KeyEvent, NativeKey};
use moose::mui::mui::prelude::Key;

use crate::lock;
use crate::song::{About, SLOWEST, SPLIT, Songs};

/// How long after a transport command the editor keeps drawing, so what
/// the audio thread did with it (a seek while paused) shows.
const AFTER: Duration = Duration::from_millis(300);

/// Where the song is, as the editor draws it, and what it draws it with.
#[derive(Default)]
pub struct Player {
    /// Smooths the audio thread's reports over the display's frames, in
    /// seconds of the wall clock: the song's seconds over its speed.
    clock: Clock,
    /// The speed and report the clock last had.
    speed: f64,
    reported: f64,
    /// The song's notes moved by a transpose: the song's, the semitones,
    /// the moved notes.
    moved: Option<(Notes, i8, Notes)>,
    /// What the song is, by take.
    about: Option<(u32, About)>,
    /// When the player last asked something of the transport.
    asked: Option<Instant>,
}

impl Player {
    /// The song loaded, for the editor to show, the window `full` screen
    /// or not (None where it cannot be).
    pub fn song(&mut self, songs: &Songs, full: Option<bool>) -> Option<app::Song> {
        let Some(now) = songs.now() else {
            *self = Self {
                asked: self.asked,
                ..Self::default()
            };
            return None;
        };
        let s = now.settings;
        let position = if now.paused {
            self.clock = Clock::default();
            now.position
        } else {
            // A new speed, or a jump back (a seek, a loop's turn): the
            // clock starts again where the song is.
            if s.speed != self.speed || now.position < self.reported - 0.02 {
                self.clock = Clock::default();
            }
            let speed = s.speed.max(SLOWEST);
            self.clock.at(now.position / speed, Instant::now()) * speed
        };
        (self.speed, self.reported) = (s.speed, now.position);
        let notes = self.notes(songs.notes(), s.transpose);
        if self.about.as_ref().is_none_or(|a| a.0 != now.take) {
            self.about = Some((now.take, songs.about()));
        }
        let about = self.about.as_ref().map(|a| &a.1);
        let voices = about.map_or_else(Vec::new, |a| a.voices.clone());
        // A two-voice song's hands are its voices, the right hand first.
        let (left, right) = if voices.len() == 2 {
            (s.mute >> 1 & 1 == 1, s.mute & 1 == 1)
        } else {
            (s.left, s.right)
        };
        Some(app::Song {
            name: now.name,
            position: position.min(now.length),
            length: now.length,
            notes,
            transport: Transport {
                paused: now.paused,
                ended: now.ended,
                take: now.take,
                speed: s.speed,
                transpose: s.transpose,
                a: s.a,
                b: s.b,
                looping: s.looping,
                left,
                right,
                voices,
                muted: s.mute,
                metronome: s.metronome,
                volume: s.volume,
                count_in: s.count_in,
                title: about.map(|a| a.title.clone()).unwrap_or_default(),
                composer: about.and_then(|a| a.composer.clone()),
                full_screen: full,
            },
        })
    }

    /// `notes`, moved `by` semitones as the song sounds them: made once a
    /// transpose, not every frame.
    fn notes(&mut self, notes: Notes, by: i8) -> Notes {
        if by == 0 {
            return notes;
        }
        if let Some((from, was, moved)) = &self.moved
            && *from == notes
            && *was == by
        {
            return moved.clone();
        }
        let moved = Notes::new(
            (notes.between(f64::NEG_INFINITY, f64::INFINITY))
                .filter_map(|n| {
                    let key = u8::try_from(i16::from(n.key) + i16::from(by)).ok()?;
                    (key < 128).then_some(Note { key, ..*n })
                })
                .collect(),
        );
        self.moved = Some((notes, by, moved.clone()));
        moved
    }

    /// Whether frames should follow the song: it moves, or the player just
    /// asked the transport for something.
    pub fn moving(&self, now: Instant) -> bool {
        self.clock.moving(now)
            || (self.asked).is_some_and(|at| now.saturating_duration_since(at) < AFTER)
    }

    /// Carries out what the transport's controls asked.
    pub fn apply(&mut self, songs: &Songs, command: Command, screen: &Screen) {
        let t = songs.transport();
        let s = t.settings();
        self.asked = Some(Instant::now());
        match command {
            Command::Play => t.pause(false),
            Command::Pause => t.pause(true),
            Command::Seek(at) => {
                t.seek(at);
                self.clock = Clock::default();
            }
            Command::Speed(speed) => t.set_speed(speed),
            Command::Transpose(by) => t.set_transpose(by),
            Command::Marks(a, b) => t.set_marks(a, b),
            Command::Loop(on) => t.set_looping(on),
            Command::Hand(hand, on) => {
                if songs.about().voices.len() == 2 {
                    let bit = if hand == Hand::Left { 1 } else { 0 };
                    t.set_mute(set(s.mute, bit, on));
                } else {
                    let (left, right) = match hand {
                        Hand::Left => (on, s.right),
                        Hand::Right => (s.left, on),
                    };
                    t.set_hands(left, right, SPLIT);
                }
            }
            Command::Voice(n, on) if n < 64 => t.set_mute(set(s.mute, n, on)),
            Command::Voice(..) => {}
            Command::Metronome(on) => t.set_metronome(on, s.volume),
            Command::Volume(level) => t.set_metronome(s.metronome, level),
            Command::CountIn(bars) => t.set_count_in(bars),
            Command::FullScreen(on) => screen.ask(on),
        }
    }
}

/// `mask` with bit `n` set (`on`) or cleared.
fn set(mask: u64, n: usize, on: bool) -> u64 {
    if on { mask | 1 << n } else { mask & !(1 << n) }
}

/// The keys the transport takes from the window, and what it knows to
/// decide on the window's thread: whether a song is loaded, the Stage open.
#[derive(Default)]
pub struct Keys {
    /// Shortcuts pressed, for the next frame.
    pressed: Mutex<Vec<Shortcut>>,
    /// Keys taken while down, so their release is taken too.
    held: Mutex<Vec<u64>>,
    pub loaded: AtomicBool,
    pub stage: AtomicBool,
}

impl Keys {
    /// A key from the window, before MUI sees it: true takes it from MUI
    /// and the host. Space while a song is loaded, F while one is or the
    /// Stage is open, Escape while the Stage is; never while a text field
    /// has the keys, nor with Cmd, Ctrl or Alt.
    pub fn key(&self, focus_is_text: bool, key: &KeyEvent) -> bool {
        let mut held = lock(&self.held);
        if !key.down {
            let was = held.len();
            held.retain(|c| *c != key.code);
            return held.len() != was;
        }
        if held.contains(&key.code) {
            return true; // a repeat
        }
        if focus_is_text || key.mods.cmd || key.mods.ctrl || key.mods.alt {
            return false;
        }
        let Some(shortcut) = shortcut(&key.key) else {
            return false;
        };
        let (loaded, stage) = (
            self.loaded.load(Ordering::Acquire),
            self.stage.load(Ordering::Acquire),
        );
        let wanted = match shortcut {
            Shortcut::PlayPause => loaded,
            Shortcut::Stage => loaded || stage,
            Shortcut::Leave => stage,
        };
        if wanted {
            held.push(key.code);
            lock(&self.pressed).push(shortcut);
        }
        wanted
    }

    /// Whether a shortcut waits for a frame.
    pub fn waiting(&self) -> bool {
        !lock(&self.pressed).is_empty()
    }

    /// The shortcuts pressed since the last frame.
    pub fn take(&self) -> Vec<Shortcut> {
        std::mem::take(&mut *lock(&self.pressed))
    }
}

/// The shortcut `key` is, if any.
fn shortcut(key: &NativeKey) -> Option<Shortcut> {
    match key {
        NativeKey::Text(s) if s == " " => Some(Shortcut::PlayPause),
        NativeKey::Text(s) if s.eq_ignore_ascii_case("f") => Some(Shortcut::Stage),
        NativeKey::Named(Key::Space) => Some(Shortcut::PlayPause),
        NativeKey::Named(Key::Escape) => Some(Shortcut::Leave),
        _ => None,
    }
}

/// The window's full screen: what the window says it is, what the editor
/// asks of it.
#[derive(Default)]
pub struct Screen {
    /// 0: it cannot; 1: windowed; 2: full screen.
    now: AtomicU8,
    /// 0: nothing asked; 1: fill the screen; 2: give it back.
    asked: AtomicU8,
}

impl Screen {
    pub fn ask(&self, full: bool) {
        self.asked
            .store(if full { 1 } else { 2 }, Ordering::Release);
    }

    /// Whether the window fills the screen; None where it cannot.
    pub fn full(&self) -> Option<bool> {
        match self.now.load(Ordering::Acquire) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }

    /// The window's tick: it says what it is, and hears what is asked.
    pub fn tick(&self, now: Option<bool>) -> Option<bool> {
        let state = now.map_or(0, |full| if full { 2 } else { 1 });
        self.now.store(state, Ordering::Release);
        match self.asked.swap(0, Ordering::AcqRel) {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Song;
    use crate::song::tests::{file, song, track};
    use moose::mui::mui::prelude::Mods;

    fn press(code: u64, key: NativeKey, down: bool) -> KeyEvent {
        KeyEvent {
            code,
            key,
            down,
            mods: Mods::default(),
        }
    }

    #[test]
    fn space_f_and_escape_are_taken_only_when_they_mean_something() {
        let keys = Keys::default();
        let space = |down| press(49, NativeKey::Text(" ".into()), down);
        let f = |down| press(3, NativeKey::Text("f".into()), down);
        let escape = |down| press(53, NativeKey::Named(Key::Escape), down);
        // No song: they are the host's (a DAW's transport, the computer
        // keyboard's F).
        assert!(!keys.key(false, &space(true)) && !keys.key(false, &space(false)));
        assert!(!keys.key(false, &f(true)) && !keys.key(false, &escape(true)));
        keys.loaded.store(true, Ordering::Release);
        assert!(keys.key(false, &space(true)));
        assert!(
            keys.key(false, &space(true)),
            "a repeat is taken, not pressed again"
        );
        assert!(keys.key(false, &space(false)), "its release too");
        assert!(keys.key(false, &f(true)) && keys.key(false, &f(false)));
        assert!(!keys.key(false, &escape(true)), "the Stage is closed");
        keys.stage.store(true, Ordering::Release);
        assert!(keys.key(false, &escape(true)));
        assert_eq!(
            keys.take(),
            [Shortcut::PlayPause, Shortcut::Stage, Shortcut::Leave]
        );
        assert!(keys.take().is_empty());
        // Typing, or with Cmd: not ours.
        assert!(!keys.key(true, &press(9, NativeKey::Text(" ".into()), true)));
        let mut cmd_f = f(true);
        cmd_f.mods.cmd = true;
        assert!(!keys.key(false, &cmd_f));
    }

    #[test]
    fn the_editor_sees_the_transport_and_its_controls_reach_it() {
        let songs = Songs::default();
        let mut player = Player::default();
        let screen = Screen::default();
        assert!(player.song(&songs, None).is_none());
        songs.play(Song::read("01 Bach - Prelude.mid".into(), &song()).unwrap());
        let shown = player.song(&songs, screen.full()).unwrap();
        assert_eq!(shown.transport.title, "Prelude");
        assert_eq!(shown.transport.composer.as_deref(), Some("Bach"));
        assert_eq!(shown.transport.voices.len(), 1);
        for c in [
            Command::Pause,
            Command::Speed(0.5),
            Command::Transpose(-2),
            Command::Marks(Some(1.), Some(2.)),
            Command::Loop(true),
            Command::Hand(Hand::Left, true),
            Command::Metronome(true),
            Command::Volume(0.25),
            Command::CountIn(2),
            Command::FullScreen(true),
        ] {
            player.apply(&songs, c, &screen);
        }
        let t = player.song(&songs, screen.full()).unwrap().transport;
        assert!(t.paused && t.looping && t.left && !t.right && t.metronome);
        assert_eq!((t.speed, t.transpose, t.count_in), (0.5, -2, 2));
        assert_eq!((t.a, t.b, t.volume), (Some(1.), Some(2.), 0.25));
        assert!(player.moving(Instant::now()), "frames show what was asked");
        // One voice: the left hand is the keys below middle C.
        let s = songs.transport().settings();
        assert!(s.left && s.mutes(0, 59) && !s.mutes(0, 60));
        // The notes fall where they sound.
        let notes = player.song(&songs, None).unwrap().notes;
        assert_eq!(notes.keys(), Some((58, 65)));
        // The window hears the full screen asked for, once.
        assert_eq!(screen.full(), None);
        assert_eq!(screen.tick(Some(false)), Some(true));
        assert_eq!(screen.tick(Some(true)), None);
        assert_eq!(screen.full(), Some(true));
        // Two voices: the hands are the tracks.
        let two = file(
            1,
            480,
            &[
                track(&[(0, &[0x90, 72, 90]), (480, &[0x80, 72, 0])]),
                track(&[(0, &[0x91, 48, 90]), (480, &[0x81, 48, 0])]),
            ],
        );
        songs.play(Song::read("two.mid".into(), &two).unwrap());
        player.apply(&songs, Command::Hand(Hand::Left, true), &screen);
        assert_eq!(songs.transport().settings().mute, 0b10);
        let t = player.song(&songs, None).unwrap().transport;
        assert!(t.left && !t.right);
        player.apply(&songs, Command::Voice(1, false), &screen);
        assert_eq!(songs.transport().settings().mute, 0);
    }
}
