//! What the player keeps between sessions about their songs (MIDI files):
//! the folders the Songs tab lists, favourites, what played lately and the
//! touch-ups a rough file gets. Kept in `songs.json` beside the library
//! prefs, written whole and renamed over the old file as they are, so a
//! build that does not know songs never drops them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many recent songs are kept.
const RECENT: usize = 32;

/// The player's song preferences. Fields this version does not know are
/// dropped on save.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SongPrefs {
    /// Folders of MIDI files, in the order they were added.
    pub folders: Vec<PathBuf>,
    /// Starred songs, by file.
    pub starred: Vec<PathBuf>,
    /// Songs played lately, newest first.
    pub recent: Vec<PathBuf>,
    pub touch: Touch,
    /// Whether the suggested folder has been offered: once added, a folder
    /// the player removes stays removed.
    pub suggested: bool,
    /// Transcribe a recording as it is, voice and all, rather than its
    /// piano alone (which is the default).
    pub whole_mix: bool,
}

/// The touch-ups a song gets when it is read, each only where the file
/// needs it: a pedal for a file with none, a human hand for one quantised
/// to a grid, a velocity curve for one played at one velocity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Touch {
    /// A sustain pedal for a file that never presses one.
    pub pedal: bool,
    /// Chords spread by a few milliseconds and velocities varied, for a
    /// file whose notes sit on a grid.
    pub humanise: bool,
    /// Big chords rolled from the bottom up, slightly; with `humanise`.
    pub roll: bool,
    /// A velocity curve for a file whose notes share one velocity.
    pub velocity: bool,
}

impl Touch {
    /// No touch-ups: the file as written.
    pub const NONE: Self = Self {
        pedal: false,
        humanise: false,
        roll: false,
        velocity: false,
    };
}

/// A pedal and a human hand where a file lacks them; rolled chords and a
/// velocity curve only when asked for.
impl Default for Touch {
    fn default() -> Self {
        Self {
            pedal: true,
            humanise: true,
            ..Self::NONE
        }
    }
}

impl SongPrefs {
    /// The prefs at `path`, or the defaults when it holds none.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Writes them to `path` whole, then renames them over the old file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)
    }

    /// Adds a folder once; true when it is new.
    pub fn add_folder(&mut self, folder: &Path) -> bool {
        if self.folders.iter().any(|f| f == folder) {
            return false;
        }
        self.folders.push(folder.to_path_buf());
        true
    }

    /// Stops listing `folder`; true when it was listed.
    pub fn remove_folder(&mut self, folder: &Path) -> bool {
        let before = self.folders.len();
        self.folders.retain(|f| f != folder);
        self.folders.len() != before
    }

    /// Stars or unstars `song`.
    pub fn star(&mut self, song: &Path, on: bool) {
        self.starred.retain(|s| s != song);
        if on {
            self.starred.push(song.to_path_buf());
        }
    }

    #[must_use]
    pub fn starred(&self, song: &Path) -> bool {
        self.starred.iter().any(|s| s == song)
    }

    /// Puts `song` first among the recent.
    pub fn played(&mut self, song: &Path) {
        self.recent.retain(|s| s != song);
        self.recent.insert(0, song.to_path_buf());
        self.recent.truncate(RECENT);
    }

    /// The first time it is offered, lists `folder` if it exists: a
    /// starting folder for a new player. True when it was added.
    pub fn suggest(&mut self, folder: &Path) -> bool {
        if std::mem::replace(&mut self.suggested, true) || !folder.is_dir() {
            return false;
        }
        self.add_folder(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kontra-songs-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn songs_keep_their_folders_stars_recent_and_touch_ups() {
        let dir = scratch("prefs");
        let path = dir.join("kontra-v3").join("songs.json");
        assert_eq!(SongPrefs::load(&path), SongPrefs::default());
        assert_eq!(SongPrefs::default().touch, Touch::default());
        assert!(Touch::default().pedal && Touch::default().humanise);
        assert!(!Touch::default().roll && !Touch::default().velocity);

        let mut p = SongPrefs::default();
        assert!(p.add_folder(Path::new("/songs/a")));
        assert!(!p.add_folder(Path::new("/songs/a")), "once");
        p.add_folder(Path::new("/songs/b"));
        p.star(Path::new("/songs/a/x.mid"), true);
        p.played(Path::new("/songs/a/x.mid"));
        p.played(Path::new("/songs/b/y.mid"));
        p.played(Path::new("/songs/a/x.mid"));
        p.touch.roll = true;
        p.save(&path).unwrap();

        let back = SongPrefs::load(&path);
        assert_eq!(back, p);
        assert_eq!(
            back.recent,
            [Path::new("/songs/a/x.mid"), Path::new("/songs/b/y.mid")]
        );
        assert!(back.starred(Path::new("/songs/a/x.mid")));
        assert!(back.touch.roll);

        let mut gone = back;
        assert!(gone.remove_folder(Path::new("/songs/a")));
        assert!(!gone.remove_folder(Path::new("/songs/a")));
        gone.star(Path::new("/songs/a/x.mid"), false);
        assert!(!gone.starred(Path::new("/songs/a/x.mid")));
        assert_eq!(gone.folders, [Path::new("/songs/b")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_missing_fields_takes_their_defaults() {
        let p: SongPrefs =
            serde_json::from_str(r#"{"folders": ["/m"], "touch": {"roll": true}}"#).unwrap();
        assert_eq!(p.folders, [Path::new("/m")]);
        assert!(p.touch.pedal && p.touch.humanise && p.touch.roll);
    }

    #[test]
    fn the_suggested_folder_is_offered_once() {
        let dir = scratch("suggest");
        let mut p = SongPrefs::default();
        assert!(!p.suggest(&dir.join("missing")), "only if it exists");
        assert!(p.suggested);
        let mut fresh = SongPrefs::default();
        assert!(fresh.suggest(&dir));
        assert_eq!(fresh.folders, std::slice::from_ref(&dir));
        fresh.remove_folder(&dir);
        assert!(!fresh.suggest(&dir), "removed stays removed");
        assert!(fresh.folders.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
