//! A song's falling notes as a video: each frame drawn headless by vello's
//! CPU renderer ([`kontra_ui::export::Frames`]) on a few threads at once,
//! piped in order to ffmpeg as raw RGBA, encoded as H.264 in an MP4, with
//! the song's exported WAV as its sound when there is one.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;

use kontra_ui::export::{Frames, range};
use kontra_ui::waterfall::Notes;

use super::Progress;

/// Where ffmpeg may be: Homebrew's (an app opened from the Finder does not
/// get the shell's PATH), then the PATH's.
const FFMPEG: [&str; 4] = [
    "/opt/homebrew/bin/ffmpeg",
    "/usr/local/bin/ffmpeg",
    "/usr/bin/ffmpeg",
    "ffmpeg",
];

/// ffmpeg, if it is installed.
pub fn ffmpeg() -> Option<PathBuf> {
    FFMPEG.iter().map(PathBuf::from).find(|p| {
        Command::new(p)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// What a video is made of.
pub struct Video<'a> {
    pub notes: &'a Notes,
    pub name: &'a str,
    /// Seconds it runs.
    pub seconds: f64,
    /// Its height in pixels (1080 or 2160) and frames a second.
    pub height: u32,
    pub fps: u32,
    /// Its sound.
    pub audio: Option<&'a Path>,
}

impl Video<'_> {
    fn size(&self) -> (u16, u16) {
        let h = self.height.clamp(144, 4320);
        let w = (u64::from(h) * 16 / 9) & !1;
        (
            u16::try_from(w).unwrap_or(3840),
            u16::try_from(h & !1).unwrap_or(2160),
        )
    }

    fn frames(&self) -> usize {
        (self.seconds.max(0.0) * f64::from(self.fps.max(1))).ceil() as usize
    }

    /// ffmpeg's arguments to encode it into `out` from raw frames on stdin.
    fn args(&self, out: &Path) -> Vec<String> {
        let (w, h) = self.size();
        let mut a: Vec<String> = [
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{w}x{h}"),
            "-r",
            &self.fps.to_string(),
            "-i",
            "-",
        ]
        .map(String::from)
        .to_vec();
        if let Some(audio) = self.audio {
            a.extend(["-i".into(), audio.display().to_string()]);
            a.extend(
                ["-map", "0:v", "-map", "1:a", "-c:a", "aac", "-b:a", "320k"].map(String::from),
            );
        }
        a.extend(
            [
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                "-shortest",
            ]
            .map(String::from),
        );
        a.push(out.display().to_string());
        a
    }
}

/// Threads drawing frames: most of the machine, leaving the audio thread
/// and ffmpeg room; fewer for 4K, whose frames are 33 MB each in flight.
fn threads(height: u32) -> usize {
    let cores = std::thread::available_parallelism().map_or(2, usize::from);
    let most = if height > 1080 { 4 } else { 8 };
    cores.saturating_sub(2).clamp(1, most)
}

/// Draws `video`'s frames on several threads, handing each to `sink` in
/// order. Stops at the first error, or when cancelled.
fn draw(
    video: &Video<'_>,
    progress: &Progress,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let total = video.frames();
    let (w, h) = video.size();
    let range = range(video.notes);
    let n = threads(video.height).min(total.max(1));
    let fps = f64::from(video.fps.max(1));
    std::thread::scope(|s| {
        // A channel a thread, a frame in it at most: thread `t` draws
        // frames t, t + n, t + 2n... and they are taken in turn.
        let mut lanes = Vec::with_capacity(n);
        for t in 0..n {
            let (tx, rx) = mpsc::sync_channel::<Result<Vec<u8>, String>>(1);
            lanes.push(rx);
            let notes = video.notes;
            let name = video.name;
            s.spawn(move || {
                let mut frames = Frames::new(w, h);
                for k in (t..total).step_by(n) {
                    if progress.cancelled() {
                        return;
                    }
                    let at = k as f64 / fps;
                    if tx.send(frames.draw(notes, at, range, name)).is_err() {
                        return;
                    }
                }
            });
        }
        let result = (|| {
            for k in 0..total {
                if progress.cancelled() {
                    return Err("cancelled".to_owned());
                }
                let lane = lanes.get(k % n).ok_or("no drawing thread")?;
                let frame = lane
                    .recv()
                    .map_err(|_| "a drawing thread stopped".to_owned())??;
                sink(&frame)?;
                progress.set((k + 1) as f64 / total.max(1) as f64);
            }
            Ok(())
        })();
        // Dropping the channels ends any thread still drawing.
        drop(lanes);
        result
    })
}

/// Encodes `video` into `out` with the ffmpeg at `ffmpeg`.
///
/// # Errors
/// When ffmpeg fails or cannot start, a frame cannot be drawn, or the
/// export is cancelled (ffmpeg is stopped and `out` removed).
pub fn encode(
    ffmpeg: &Path,
    video: &Video<'_>,
    out: &Path,
    progress: &Progress,
) -> Result<(), String> {
    let log = out.with_extension("ffmpeg.log");
    let errors = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let mut child = Command::new(ffmpeg)
        .args(video.args(out))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(errors)
        .spawn()
        .map_err(|e| format!("ffmpeg did not start: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("ffmpeg took no input")?;
    let drawn = draw(video, progress, &mut |frame| {
        stdin
            .write_all(frame)
            .map_err(|e| format!("ffmpeg stopped taking frames: {e}"))
    });
    drop(stdin);
    if drawn.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string());
    let said = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    let result = match (drawn, status) {
        (Err(e), _) => Err(e),
        (Ok(()), Err(e)) => Err(e),
        (Ok(()), Ok(s)) if !s.success() => Err(format!("ffmpeg failed: {}", said.trim())),
        (Ok(()), Ok(_)) => Ok(()),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use kontra_ui::waterfall::Note;

    fn notes() -> Notes {
        Notes::new(
            (0..8u8)
                .map(|n| Note {
                    start: f64::from(n) * 0.25,
                    end: f64::from(n) * 0.25 + 0.5,
                    key: 60 + n,
                    velocity: 90,
                    voice: n % 2,
                })
                .collect(),
        )
    }

    #[test]
    fn frames_arrive_in_order_whatever_thread_drew_them() {
        let notes = notes();
        let video = Video {
            notes: &notes,
            name: "order.mid",
            seconds: 1.0,
            height: 144,
            fps: 12,
            audio: None,
        };
        assert_eq!(video.size(), (256, 144));
        assert_eq!(video.frames(), 12);
        let mut got = Vec::new();
        let progress = Progress::default();
        draw(&video, &progress, &mut |f| {
            got.push(f.to_vec());
            Ok(())
        })
        .unwrap();
        assert_eq!(got.len(), 12);
        // Each is the frame drawn alone at its moment.
        let mut one = Frames::new(256, 144);
        let r = range(&notes);
        for (k, f) in got.iter().enumerate() {
            assert!(
                *f == one.draw(&notes, k as f64 / 12.0, r, "order.mid").unwrap(),
                "frame {k}"
            );
        }
        assert!((progress.done() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_cancelled_video_stops_drawing() {
        let notes = notes();
        let video = Video {
            notes: &notes,
            name: "x.mid",
            seconds: 30.0,
            height: 144,
            fps: 30,
            audio: None,
        };
        let progress = Progress::default();
        let mut seen = 0;
        let r = draw(&video, &progress, &mut |_| {
            seen += 1;
            if seen == 5 {
                progress.cancel();
            }
            Ok(())
        });
        assert_eq!(r, Err("cancelled".into()));
        assert_eq!(seen, 5);
    }

    /// The real thing, when ffmpeg is installed: an MP4 of the right size,
    /// length and rate, with the WAV as its sound.
    #[test]
    fn ffmpeg_encodes_an_mp4_with_its_sound() {
        let Some(ffmpeg) = ffmpeg() else {
            eprintln!("skipped: no ffmpeg");
            return;
        };
        let dir = std::env::temp_dir().join(format!("kontra-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("a.wav");
        let tone: Vec<[f32; 2]> = (0..48_000)
            .map(|n| {
                let x = (std::f64::consts::TAU * 440.0 * f64::from(n) / 48_000.0).sin() as f32;
                [x * 0.2; 2]
            })
            .collect();
        super::super::wav::write(&wav, 48_000, &tone).unwrap();
        let notes = notes();
        let video = Video {
            notes: &notes,
            name: "a.mid",
            seconds: 1.0,
            height: 144,
            fps: 30,
            audio: Some(&wav),
        };
        let out = dir.join("a.mp4");
        encode(&ffmpeg, &video, &out, &Progress::default()).unwrap();
        let probe = ffmpeg.with_file_name("ffprobe");
        if let Ok(o) = Command::new(&probe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,width,height,r_frame_rate,nb_frames",
            ])
            .args(["-of", "csv=p=0"])
            .arg(&out)
            .output()
        {
            let said = String::from_utf8_lossy(&o.stdout);
            assert!(said.contains("256,144,30/1"), "{said}");
            assert!(said.contains("audio"), "{said}");
        }
        assert!(std::fs::metadata(&out).unwrap().len() > 1000);
        assert!(!out.with_extension("ffmpeg.log").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
