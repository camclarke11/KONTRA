//! The editor's "Add folder" and "Play MIDI": the system's folder or file
//! picker, off the editor's thread, so no frame waits on it. [`crate::Tick`]
//! takes what was picked: it scans a folder, plays a MIDI file.
//!
//! On Linux the picker is the desktop portal (v2's `mui-native-dialog`):
//! dropping [`Dialog`] at plugin teardown cancels an open picker and joins
//! its worker, so no thread is left running in an unloaded binary.
//!
//! Elsewhere it is rfd's, which has no cancel, so it is modal to the host
//! instead: on macOS rfd runs it on the host's main thread; on Windows it is
//! owned by the host's window the editor opened in, which stays disabled
//! while it is open. Either way the host cannot unload the plugin under it.
//!
// ponytail: a Windows host that unloads the plugin on its own (not from its
// window) while the picker is open is the ceiling: the worker is left to
// finish. Post WM_CLOSE to the picker's thread at teardown if one shows.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use crate::lock;

/// What a [`Dialog`] picks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pick {
    /// A folder of libraries, for the browser.
    #[default]
    Folder,
    /// A Standard MIDI File to play ([`crate::song`]).
    Song,
}

impl Pick {
    fn title(self) -> &'static str {
        match self {
            Self::Folder => "A folder of libraries",
            Self::Song => "A MIDI file to play",
        }
    }
}

/// The extensions a MIDI file picker offers.
const SONG_EXTENSIONS: [&str; 2] = ["mid", "midi"];

/// A picker while it is open.
#[derive(Default)]
pub struct Dialog {
    pick: Pick,
    #[cfg(target_os = "linux")]
    service: mui_native_dialog::DialogService,
    #[cfg(target_os = "linux")]
    job: Mutex<Option<mui_native_dialog::DialogJob>>,
    #[cfg(not(target_os = "linux"))]
    job: Mutex<Option<std::thread::JoinHandle<Option<PathBuf>>>>,
    /// The host's Win32 window the editor is open in; 0 when none.
    owner: AtomicIsize,
}

impl Dialog {
    /// A picker for `pick`.
    pub fn new(pick: Pick) -> Self {
        Self {
            pick,
            ..Self::default()
        }
    }

    /// The host's Win32 window the editor opened in, or `None` as it closes.
    pub fn set_owner(&self, hwnd: Option<isize>) {
        self.owner.store(hwnd.unwrap_or(0), Ordering::Release);
    }

    /// Opens the picker at `from`, unless one is open.
    pub fn ask(&self, from: Option<PathBuf>) {
        let mut job = lock(&self.job);
        if job.is_some() {
            return;
        }
        let from = from.or_else(dirs::home_dir);
        *job = self.open(from);
    }

    #[cfg(target_os = "linux")]
    fn open(&self, from: Option<PathBuf>) -> Option<mui_native_dialog::DialogJob> {
        use mui_native_dialog::{DialogFilter, DialogKind, DialogRequest};
        let (kind, filters) = match self.pick {
            Pick::Folder => (DialogKind::PickFolder { multiple: false }, Vec::new()),
            Pick::Song => (
                DialogKind::OpenFile { multiple: false },
                vec![DialogFilter {
                    name: "MIDI file".into(),
                    extensions: SONG_EXTENSIONS.map(String::from).to_vec(),
                }],
            ),
        };
        let request = DialogRequest {
            parent: None,
            title: self.pick.title().into(),
            kind,
            directory: from,
            filters,
        };
        self.service.spawn(request).ok()
    }

    #[cfg(not(target_os = "linux"))]
    fn open(&self, from: Option<PathBuf>) -> Option<std::thread::JoinHandle<Option<PathBuf>>> {
        let owner = self.owner.load(Ordering::Acquire);
        let pick = self.pick;
        std::thread::Builder::new()
            .name("kontra-folder-dialog".into())
            .spawn(move || {
                let mut d = rfd::FileDialog::new().set_title(pick.title());
                if let Some(from) = from {
                    d = d.set_directory(from);
                }
                if cfg!(windows) && owner != 0 {
                    d = d.set_parent(&moose::mui::ParentWindow::win32(owner));
                }
                match pick {
                    Pick::Folder => d.pick_folder(),
                    Pick::Song => d.add_filter("MIDI file", &SONG_EXTENSIONS).pick_file(),
                }
            })
            .ok()
    }

    /// What was picked, once the picker closed.
    #[cfg(target_os = "linux")]
    pub fn picked(&self) -> Option<PathBuf> {
        let mut job = lock(&self.job);
        // Finished first: the worker sends its answer before it finishes.
        let finished = job.as_ref()?.is_finished();
        let result = job.as_ref()?.try_result();
        if result.is_none() && !finished {
            return None;
        }
        job.take();
        result?.ok()??.into_iter().next()
    }

    /// What was picked, once the picker closed.
    #[cfg(not(target_os = "linux"))]
    pub fn picked(&self) -> Option<PathBuf> {
        let mut job = lock(&self.job);
        if !job.as_ref()?.is_finished() {
            return None;
        }
        job.take()?.join().ok().flatten()
    }
}
