//! MIDI out: what KONTRA plays (a song's notes and pedal, and what the
//! player plays into it) sent on to other apps through a virtual MIDI port
//! named "KONTRA", for an instrument KONTRA does not host (Splice
//! INSTRUMENT, GarageBand, Logic) to play along. Off until the player turns
//! it on ("MIDI Out"), as other apps may listen to every port there is.
//!
//! The audio thread stamps each message with when it falls in its block
//! ([`MidiOut::process`]); a thread of its own owns the port and sends each
//! as it falls due, so a run within a block keeps its timing. Turned off,
//! or on Panic, every note and the pedal are let go first.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_queue::ArrayQueue;
use kontra_host::rack::Event;

use crate::lock;

/// The port other apps see.
pub const NAME: &str = "KONTRA";

/// Messages waiting to be sent, at most; past it the oldest go.
const QUEUE: usize = 4096;

/// A channel message and when it is due.
type Due = (Instant, [u8; 3]);

/// Every note and the pedal let go, on every channel.
fn silence() -> impl Iterator<Item = [u8; 3]> {
    (0u8..16).flat_map(|c| [[0xB0 | c, 64, 0], [0xB0 | c, 123, 0]])
}

/// The thread that owns the port, and how to stop it.
struct Sender {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

/// MIDI out, shared by the editor, the tick and the audio thread.
pub struct MidiOut {
    on: AtomicBool,
    queue: Arc<ArrayQueue<Due>>,
    sender: Mutex<Option<Sender>>,
}

impl Default for MidiOut {
    fn default() -> Self {
        Self {
            on: AtomicBool::new(false),
            queue: Arc::new(ArrayQueue::new(QUEUE)),
            sender: Mutex::default(),
        }
    }
}

impl MidiOut {
    /// Whether it is on.
    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// Turns it on (opening the port) or off (letting go of every note,
    /// then closing it).
    ///
    /// # Errors
    /// When the port cannot be made (or this system has no virtual ports).
    pub fn set(&self, on: bool) -> Result<(), String> {
        let mut sender = lock(&self.sender);
        if !on {
            self.on.store(false, Ordering::Relaxed);
            if let Some(s) = sender.take() {
                s.stop.store(true, Ordering::Relaxed);
                let _ = s.thread.join();
            }
            return Ok(());
        }
        if sender.is_none() {
            *sender = Some(open(Arc::clone(&self.queue))?);
        }
        self.on.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Panic: every note and the pedal let go in the apps listening.
    pub fn panic(&self) {
        if self.is_on() {
            let now = Instant::now();
            for m in silence() {
                self.queue.force_push((now, m));
            }
        }
    }

    /// On the audio thread: `events` (frame in the block, event) sent on,
    /// each due `at` its frame from now at `rate`. Nothing is read or
    /// stamped while it is off.
    pub fn process(&self, rate: f64, events: &[(u32, Event)]) {
        if !self.is_on() || events.is_empty() {
            return;
        }
        let now = Instant::now();
        for (at, e) in events {
            if let Some((_, m)) = crate::record::message(e) {
                let due = now + Duration::from_secs_f64(f64::from(*at) / rate.max(1.0));
                self.queue.force_push((due, m));
            }
        }
    }

    /// The editor's keyboard played `e`: sent on now.
    pub fn keyboard(&self, e: &Event) {
        if self.is_on()
            && let Some((_, m)) = crate::record::message(e)
        {
            self.queue.force_push((Instant::now(), m));
        }
    }
}

impl Drop for MidiOut {
    fn drop(&mut self) {
        let _ = self.set(false);
    }
}

/// The bytes `m` takes on the wire: one data byte for a program change or
/// channel pressure, two for the rest.
fn wire(m: &[u8; 3]) -> &[u8] {
    let len = if matches!(m[0] & 0xF0, 0xC0 | 0xD0) {
        2
    } else {
        3
    };
    m.get(..len).unwrap_or(m)
}

/// Opens the port and the thread that sends what `queue` holds as it
/// falls due.
#[cfg(target_os = "macos")]
fn open(queue: Arc<ArrayQueue<Due>>) -> Result<Sender, String> {
    use midir::os::unix::VirtualOutput;
    let out = midir::MidiOutput::new(NAME).map_err(|e| e.to_string())?;
    let mut port = out.create_virtual(NAME).map_err(|e| e.to_string())?;
    // Anything queued before it opened is stale.
    while queue.pop().is_some() {}
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("kontra-midi-out".into())
        .spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Some((due, m)) = queue.pop() else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                let wait = due.saturating_duration_since(Instant::now());
                if !wait.is_zero() {
                    std::thread::sleep(wait.min(Duration::from_millis(250)));
                }
                let _ = port.send(wire(&m));
            }
            for m in silence() {
                let _ = port.send(wire(&m));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Sender { stop, thread })
}

#[cfg(not(target_os = "macos"))]
fn open(_queue: Arc<ArrayQueue<Due>>) -> Result<Sender, String> {
    Err("MIDI out needs macOS's virtual MIDI ports".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kontra_host::rack::{HostNote, HostPattern};

    fn on(key: u8) -> Event {
        Event::NoteOn {
            note: HostNote {
                port: 254,
                channel: 0,
                key,
                id: -1,
                clap: false,
            },
            velocity: 0.5,
            tune: 0.0,
        }
    }

    fn off(key: u8) -> Event {
        Event::NoteOff(HostPattern {
            port: 254,
            channel: 0,
            key: i32::from(key),
            id: -1,
            clap: false,
        })
    }

    #[test]
    fn off_it_takes_nothing_and_on_it_stamps_each_message_by_its_frame() {
        let out = MidiOut::default();
        out.process(48_000.0, &[(0, on(60))]);
        assert!(out.queue.is_empty(), "off: nothing taken");
        // On without a port: the queue alone, as the audio thread fills it.
        out.on.store(true, Ordering::Relaxed);
        let before = Instant::now();
        out.process(
            48_000.0,
            &[
                (0, on(60)),
                (24_000, off(60)),
                (100, Event::midi1(0xB0, 64, 127)),
            ],
        );
        let (a, m) = out.queue.pop().unwrap();
        assert_eq!(m, [0x90, 60, 64]);
        let (b, m) = out.queue.pop().unwrap();
        assert_eq!(m, [0x80, 60, 0]);
        let gap = b.duration_since(a).as_secs_f64();
        assert!((gap - 0.5).abs() < 1e-3, "half a second later: {gap}");
        assert!(a >= before);
        assert_eq!(out.queue.pop().unwrap().1, [0xB0, 64, 127]);
        out.panic();
        assert_eq!(
            out.queue.len(),
            32,
            "the pedal and every note, each channel"
        );
        out.on.store(false, Ordering::Relaxed);
    }

    /// The port is there for other apps, and what is played reaches them
    /// in order; turned off, the port lets go of everything and goes. Only
    /// messages that sound nothing, should an app be listening.
    #[cfg(target_os = "macos")]
    #[test]
    fn another_app_hears_the_port() {
        use std::sync::mpsc;
        let out = MidiOut::default();
        out.set(true).unwrap();
        let input = midir::MidiInput::new("kontra-test").unwrap();
        let mut found = None;
        for _ in 0..50 {
            found = input
                .ports()
                .into_iter()
                .find(|p| input.port_name(p).is_ok_and(|n| n == NAME));
            if found.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let port = found.expect("the KONTRA port");
        let (tx, rx) = mpsc::channel();
        let _conn = input
            .connect(
                &port,
                "kontra-test-in",
                move |_, bytes, ()| {
                    let _ = tx.send(bytes.to_vec());
                },
                (),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        out.process(48_000.0, &[(0, off(60)), (480, Event::midi1(0xB0, 64, 0))]);
        let wait = Duration::from_secs(2);
        assert_eq!(rx.recv_timeout(wait).unwrap(), [0x80, 60, 0]);
        assert_eq!(rx.recv_timeout(wait).unwrap(), [0xB0, 64, 0]);
        out.set(false).unwrap();
        // Let go of on every channel as it closes.
        assert_eq!(rx.recv_timeout(wait).unwrap(), [0xB0, 64, 0]);
        assert_eq!(rx.recv_timeout(wait).unwrap(), [0xB0, 123, 0]);
    }

    #[test]
    fn a_program_change_is_two_bytes_on_the_wire() {
        assert_eq!(wire(&[0xC0, 5, 0]), [0xC0, 5]);
        assert_eq!(wire(&[0x90, 60, 100]), [0x90, 60, 100]);
    }
}
