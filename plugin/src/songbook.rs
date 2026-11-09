//! The Songs tab's side on the control thread: the player's song folders,
//! favourites, recent songs and touch-ups ([`SongPrefs`], kept in
//! `songs.json` beside the library prefs), the folders read on a thread of
//! their own into what each MIDI file holds ([`About`]), and the song last
//! played as it was read. [`crate::Tick`] carries out what the tab asks
//! and plays songs through [`Songbook::play`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use kontra_library::{SongPrefs, Touch};
use kontra_ui::app::{self, SongAction};

use crate::folder::{Dialog, Pick};
use crate::song::transform::{self, About};
use crate::song::{Songs, Touched};

/// Where v3 keeps its settings, beside the library prefs.
const FOLDER: &str = "kontra-v3";

/// The folder a new player is offered, under their home: the test pieces
/// this fork is played with.
const SUGGESTED: &str = "Downloads/KONTRA test pieces";

/// The most songs listed, the deepest folder read, the largest file read.
const MOST: usize = 20_000;
const DEPTH: usize = 8;
const LARGEST: u64 = 16 << 20;

/// What the control threads share of the songbook: it, opened on the
/// first tick, and the "Add a folder of MIDI files" picker, which the
/// editor opens without waiting on the tick.
pub struct Shared {
    pub book: Mutex<Option<Songbook>>,
    pub dialog: Dialog,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            book: Mutex::new(None),
            dialog: Dialog::new(Pick::Songs),
        }
    }
}

/// A song found: its file, the folder it is in and where inside, and what
/// it holds (or why it cannot be read).
#[derive(Clone, Debug)]
struct Found {
    path: PathBuf,
    folder: Option<usize>,
    dir: String,
    about: Result<About, String>,
}

/// What a file held when last read, by its size and time.
type Seen = HashMap<PathBuf, (u64, Option<SystemTime>, Result<About, String>)>;

pub struct Songbook {
    prefs: SongPrefs,
    /// Where the prefs are kept; `None` keeps nothing on disk.
    path: Option<PathBuf>,
    found: Vec<Found>,
    seen: Arc<Seen>,
    scan: Option<mpsc::Receiver<(Vec<Found>, Seen)>>,
    /// Read again once the read under way ends: the folders changed.
    again: bool,
    /// The song last played, and how it was read.
    played: Option<(PathBuf, String, Touched)>,
    /// Moves whenever what the tab shows changes.
    pub generation: u64,
}

/// The player's songbook, its prefs in the user's settings, offered the
/// suggested folder; tests keep nothing on disk and are offered nothing.
pub fn open() -> Songbook {
    if cfg!(test) {
        return Songbook::open(None, None);
    }
    let config = dirs::config_dir().map(|c| c.join(FOLDER).join("songs.json"));
    let suggested = dirs::home_dir().map(|h| h.join(SUGGESTED));
    Songbook::open(config, suggested.as_deref())
}

impl Songbook {
    /// The prefs at `path` (none kept when `None`), offered `suggested`
    /// once, with the folders being read.
    pub fn open(path: Option<PathBuf>, suggested: Option<&Path>) -> Self {
        let mut prefs = path.as_deref().map(SongPrefs::load).unwrap_or_default();
        let mut book = Self {
            prefs: SongPrefs::default(),
            path,
            found: Vec::new(),
            seen: Arc::default(),
            scan: None,
            again: false,
            played: None,
            generation: 1,
        };
        if let Some(folder) = suggested
            && !prefs.suggested
        {
            prefs.suggest(folder);
            book.prefs = prefs;
            book.save();
        } else {
            book.prefs = prefs;
        }
        book.rescan();
        book
    }

    fn save(&self) {
        if let Some(path) = &self.path {
            let _kept = self.prefs.save(path);
        }
    }

    fn changed(&mut self) {
        self.generation += 1;
        self.save();
    }

    /// Reads the folders (and the songs played or starred from elsewhere)
    /// again on a thread; files unchanged since they were last read are
    /// not read again. A read under way finishes first.
    pub fn rescan(&mut self) {
        if self.scan.is_some() {
            self.again = true;
            return;
        }
        let folders = self.prefs.folders.clone();
        let loose: Vec<PathBuf> = (self.prefs.recent.iter())
            .chain(&self.prefs.starred)
            .filter(|p| !folders.iter().any(|f| p.starts_with(f)))
            .cloned()
            .collect();
        let seen = Arc::clone(&self.seen);
        let (send, receive) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("kontra-songs".into())
            .spawn(move || {
                let _gone = send.send(scan(&folders, &loose, &seen));
            });
        if spawned.is_ok() {
            self.scan = Some(receive);
            self.generation += 1;
        }
    }

    /// Takes a finished read; true when what the tab shows changed.
    pub fn poll(&mut self) -> bool {
        let Some(done) = self.scan.as_ref().and_then(|r| r.try_recv().ok()) else {
            return false;
        };
        self.scan = None;
        (self.found, self.seen) = (done.0, Arc::new(done.1));
        self.generation += 1;
        if std::mem::take(&mut self.again) {
            self.rescan();
        }
        true
    }

    /// Reads the song at `path` with the touch-ups asked for and plays it
    /// on `songs`: first among the recent. Err says why it cannot play.
    pub fn play(&mut self, songs: &Songs, path: &Path) -> Result<(), String> {
        self.play_at(songs, path, None)
    }

    /// [`Songbook::play`] from `at` seconds into the song: the one playing,
    /// read again with other touch-ups, plays on from where it was.
    pub fn play_at(&mut self, songs: &Songs, path: &Path, at: Option<f64>) -> Result<(), String> {
        let touched = songs.open_at(path, &self.prefs.touch, at)?;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        self.prefs.played(path);
        if !self.found.iter().any(|f| f.path == path) {
            self.found.push(Found {
                path: path.to_path_buf(),
                folder: None,
                dir: String::new(),
                about: std::fs::read(path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| transform::about(&b)),
            });
        }
        self.played = Some((path.to_path_buf(), name, touched));
        self.changed();
        Ok(())
    }

    /// Carries out what the tab asks; the song to play again when the
    /// touch-ups change while one plays. Playing a song and picking a
    /// folder are the caller's ([`crate::Request::Song`], the picker).
    pub fn act(&mut self, action: SongAction, songs: &Songs) -> Option<PathBuf> {
        match action {
            SongAction::Star(path, on) => {
                self.prefs.star(&path, on);
                self.changed();
            }
            SongAction::AddFolder(Some(path)) => self.add_folder(path),
            SongAction::RemoveFolder(path) => {
                if self.prefs.remove_folder(&path) {
                    self.changed();
                    self.rescan();
                }
            }
            SongAction::Touch(t) => {
                let touch = Touch {
                    pedal: t.pedal,
                    humanise: t.humanise,
                    roll: t.roll,
                    velocity: t.velocity,
                };
                if touch != self.prefs.touch {
                    self.prefs.touch = touch;
                    self.changed();
                    // Heard at once: the song playing plays again so.
                    if songs.status().is_some() {
                        return self.played.as_ref().map(|p| p.0.clone());
                    }
                }
            }
            SongAction::Rescan => self.rescan(),
            SongAction::Play(_) | SongAction::AddFolder(None) => {}
        }
        None
    }

    /// Lists `folder` and reads it.
    pub fn add_folder(&mut self, folder: PathBuf) {
        if self.prefs.add_folder(&folder) {
            self.changed();
            self.rescan();
        }
    }

    /// What the Songs tab shows.
    pub fn view(&self) -> app::Songbook {
        let touch = |t: Touch| app::Touch {
            pedal: t.pedal,
            humanise: t.humanise,
            roll: t.roll,
            velocity: t.velocity,
        };
        let needs = |about: &About| {
            let a = &about.analysis;
            app::Touch {
                pedal: a.unpedalled(),
                humanise: a.quantised(),
                roll: a.quantised(),
                velocity: a.flat(),
            }
        };
        let songs: Vec<app::SongFile> = (self.found.iter())
            .map(|f| {
                let name = f.path.file_stem().map_or_else(
                    || f.path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let (length, tracks, wants, problem) = match &f.about {
                    Ok(a) => (a.length, a.analysis.tracks, needs(a), None),
                    Err(e) => (0.0, 0, touch(Touch::NONE), Some(e.clone())),
                };
                app::SongFile {
                    path: f.path.clone(),
                    name,
                    folder: f.folder,
                    dir: f.dir.clone(),
                    length,
                    tracks,
                    needs: wants,
                    starred: self.prefs.starred(&f.path),
                    problem,
                }
            })
            .collect();
        let at = |path: &PathBuf| self.found.iter().position(|f| &f.path == path);
        app::Songbook {
            folders: (self.prefs.folders.iter())
                .map(|path| app::SongFolder {
                    name: path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    ),
                    path: path.clone(),
                })
                .collect(),
            songs,
            recent: self.prefs.recent.iter().filter_map(at).collect(),
            touch: touch(self.prefs.touch),
            playing: self.played.as_ref().map(|(path, name, t)| app::Played {
                path: path.clone(),
                name: name.clone(),
                needs: app::Touch {
                    pedal: t.analysis.unpedalled(),
                    humanise: t.analysis.quantised(),
                    roll: t.analysis.quantised(),
                    velocity: t.analysis.flat(),
                },
                applied: touch(t.applied),
                ringing: t.ringing.clone(),
            }),
            scanning: self.scan.is_some(),
        }
    }
}

/// Whether `path` names a MIDI file.
pub fn is_song(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("mid" | "midi"))
}

/// Whether a dropped `folder` is one of songs rather than of libraries: it
/// holds MIDI files (a few levels down) and no instrument or library file.
pub fn holds_songs(folder: &Path) -> bool {
    const LIBRARY: [&str; 9] = [
        "nki", "nkm", "nkb", "nicnt", "nkx", "nkc", "nkr", "ufs", "uvip",
    ];
    let mut songs = false;
    let mut budget = 4000usize;
    let mut stack = vec![(folder.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            budget = budget.saturating_sub(1);
            if budget == 0 {
                return songs;
            }
            let path = entry.path();
            let kind = entry.file_type();
            if kind.as_ref().is_ok_and(std::fs::FileType::is_dir) {
                if depth < 3 && !hidden(&path) {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if ext.as_deref().is_some_and(|e| LIBRARY.contains(&e)) {
                return false;
            }
            songs |= is_song(&path);
        }
    }
    songs
}

fn hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// The MIDI files in `folders` (by folder, then naturally by path inside
/// it) and the `loose` ones, each read unless `seen` holds it unchanged;
/// and what was read, to keep.
fn scan(folders: &[PathBuf], loose: &[PathBuf], seen: &Seen) -> (Vec<Found>, Seen) {
    let mut files: Vec<(PathBuf, Option<usize>, String)> = Vec::new();
    for (n, folder) in folders.iter().enumerate() {
        let mut here = Vec::new();
        let mut stack = vec![(folder.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if hidden(&path) || here.len() + files.len() >= MOST {
                    continue;
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() && depth < DEPTH => stack.push((path, depth + 1)),
                    Ok(t) if !t.is_dir() && is_song(&path) => here.push(path),
                    _ => {}
                }
            }
        }
        // The folder's own songs first, then each folder inside it.
        let key = |p: &PathBuf| {
            let inside = p.strip_prefix(folder).unwrap_or(p);
            let dir = inside
                .parent()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = inside
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (
                !dir.is_empty(),
                kontra_library::natural(&dir),
                kontra_library::natural(&name),
            )
        };
        here.sort_by_cached_key(key);
        for path in here {
            let dir = (path.parent())
                .and_then(|d| d.strip_prefix(folder).ok())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            files.push((path, Some(n), dir));
        }
    }
    for path in loose {
        if !files.iter().any(|f| &f.0 == path) && path.is_file() {
            files.push((path.clone(), None, String::new()));
        }
    }
    let mut kept = Seen::new();
    let found = files
        .into_iter()
        .map(|(path, folder, dir)| {
            let meta = std::fs::metadata(&path).ok();
            let size = meta.as_ref().map_or(0, std::fs::Metadata::len);
            let time = meta.as_ref().and_then(|m| m.modified().ok());
            let about = match seen.get(&path) {
                Some((s, t, about)) if *s == size && *t == time => about.clone(),
                _ if size > LARGEST => Err("it is too large for a MIDI file".into()),
                _ => std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| transform::about(&b)),
            };
            kept.insert(path.clone(), (size, time, about.clone()));
            Found {
                path,
                folder,
                dir,
                about,
            }
        })
        .collect();
    (found, kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::tests::song;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("kontra-songbook-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn settle(book: &mut Songbook) {
        for _ in 0..5000 {
            book.poll();
            if book.scan.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("the folders were never read");
    }

    #[test]
    fn folders_list_their_songs_with_what_they_hold() {
        let s = Scratch::new("folders");
        let songs = s.0.join("Songs");
        std::fs::create_dir_all(songs.join("Etudes")).unwrap();
        std::fs::create_dir_all(songs.join("0 First")).unwrap();
        std::fs::write(songs.join("0 First").join("z.mid"), song()).unwrap();
        std::fs::create_dir_all(songs.join(".hidden")).unwrap();
        std::fs::write(songs.join("b 10.mid"), song()).unwrap();
        std::fs::write(songs.join("b 9.MID"), song()).unwrap();
        std::fs::write(songs.join("Etudes").join("a.midi"), song()).unwrap();
        std::fs::write(songs.join(".hidden").join("x.mid"), song()).unwrap();
        std::fs::write(songs.join("broken.mid"), b"RIFF").unwrap();
        std::fs::write(songs.join("notes.txt"), b"").unwrap();
        let prefs = s.0.join("kontra-v3").join("songs.json");
        let mut book = Songbook::open(Some(prefs.clone()), Some(&songs));
        settle(&mut book);
        let v = book.view();
        assert_eq!(v.folders.len(), 1, "the suggested folder");
        assert_eq!(v.folders[0].name, "Songs");
        let listed: Vec<(&str, &str)> = v
            .songs
            .iter()
            .map(|s| (s.dir.as_str(), s.name.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("", "b 9"),
                ("", "b 10"),
                ("", "broken"),
                ("0 First", "z"),
                ("Etudes", "a")
            ],
            "its own songs, then its folders', naturally; hidden folders skipped"
        );
        let b9 = &v.songs[0];
        assert!((b9.length - 3.0).abs() < 1e-9 && b9.tracks == 1);
        assert!(!b9.needs.pedal, "it presses the pedal");
        assert!(v.songs[2].problem.is_some());
        assert!(SongPrefs::load(&prefs).suggested, "offered once, and kept");

        // A folder removed stays gone, even though it is the suggestion.
        let path = v.folders[0].path.clone();
        let songs_now = Songs::default();
        book.act(SongAction::RemoveFolder(path), &songs_now);
        settle(&mut book);
        assert!(book.view().songs.is_empty());
        let again = Songbook::open(Some(prefs), Some(&songs));
        assert!(again.view().folders.is_empty());
    }

    #[test]
    fn a_song_played_is_recent_and_stars_and_touch_ups_are_kept() {
        let s = Scratch::new("played");
        let file = s.0.join("loose.mid");
        std::fs::write(&file, song()).unwrap();
        let prefs = s.0.join("songs.json");
        let mut book = Songbook::open(Some(prefs.clone()), None);
        settle(&mut book);
        let songs = Songs::default();
        let before = book.generation;
        book.play(&songs, &file).unwrap();
        assert!(book.generation > before);
        assert!(book.play(&songs, &s.0.join("missing.mid")).is_err());
        let v = book.view();
        assert_eq!(v.recent, [0], "a loose song is listed for the recent");
        assert_eq!(v.songs[0].folder, None);
        let played = v.playing.unwrap();
        assert_eq!(played.name, "loose.mid");
        assert_eq!(
            played.applied,
            app::Touch {
                pedal: false,
                humanise: false,
                roll: false,
                velocity: false
            }
        );
        assert_eq!(songs.status().map(|s| s.0), Some("loose.mid".into()));

        book.act(SongAction::Star(file.clone(), true), &songs);
        // A new touch-up while it plays: play it again.
        let t = app::Touch {
            roll: true,
            ..app::Touch::default()
        };
        assert_eq!(book.act(SongAction::Touch(t), &songs), Some(file.clone()));
        assert_eq!(book.act(SongAction::Touch(t), &songs), None, "no change");
        songs.stop();
        let off = app::Touch { pedal: false, ..t };
        assert_eq!(
            book.act(SongAction::Touch(off), &songs),
            None,
            "nothing plays"
        );

        let back = Songbook::open(Some(prefs), None);
        assert!(back.prefs.starred(&file));
        assert_eq!(back.prefs.recent, [file]);
        assert_eq!(
            back.prefs.touch,
            Touch {
                pedal: false,
                humanise: true,
                roll: true,
                velocity: false
            }
        );
    }

    #[test]
    fn a_dropped_folder_of_midi_files_is_songs_and_one_with_instruments_is_not() {
        let s = Scratch::new("drop");
        let songs = s.0.join("songs");
        std::fs::create_dir_all(songs.join("deep")).unwrap();
        std::fs::write(songs.join("deep").join("a.mid"), song()).unwrap();
        assert!(holds_songs(&songs));
        let library = s.0.join("library");
        std::fs::create_dir_all(library.join("Instruments")).unwrap();
        std::fs::write(library.join("groove.mid"), song()).unwrap();
        std::fs::write(library.join("Instruments").join("Piano.nki"), b"").unwrap();
        assert!(!holds_songs(&library));
        assert!(!holds_songs(&s.0.join("nothing")));
        assert!(is_song(Path::new("/a/B.MIDI")) && !is_song(Path::new("/a/b.wav")));
    }
}
