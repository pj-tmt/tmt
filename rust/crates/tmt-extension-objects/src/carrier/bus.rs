//! The owned driver of one opened channel: one reader thread that keeps reading while
//! consumers wait, a bounded inbound queue, serialized bounded writes, and a close that
//! joins the thread and releases the socket. No frame ever spawns a thread.
//!
//! A frame is checked against direction and the correlation ledger before it is
//! queued or written. A refusal of something this end tried to send is returned and
//! nothing is sent; a refusal of something received, a framing violation, a stalled or
//! failed write or the end of the stream is final: the first such fault is kept, the
//! socket is shut down and every later call reports it.
use super::{
    Budget, Budgets, Caps, Fault, Idle, Link, Role, Stage, bounded, ledger::Ledger, prepared,
    read_checked,
};
use crate::{Frame, Uuid4};
use std::{
    collections::VecDeque,
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Queued inbound frames and their JSON bytes (the contract's eight frames and
/// 524,288 bytes plus prefixes).
const QUEUE_FRAMES: usize = 8;
const QUEUE_BYTES: usize = 8 * 65_536;
/// How often a waiting thread looks at the stop flag and the clock.
const SLICE: Duration = Duration::from_millis(50);

#[derive(Debug, Default)]
struct Inbox {
    frames: VecDeque<(Frame, usize)>,
    bytes: usize,
    /// The first final fault, kept for every later call.
    fault: Option<Fault>,
}

#[derive(Debug)]
struct Shared {
    role: Role,
    generation: Uuid4,
    budgets: Budgets,
    stop: AtomicBool,
    inbox: Mutex<Inbox>,
    changed: Condvar,
    ledger: Mutex<Ledger>,
    writer: Mutex<bounded::Writer>,
    /// A duplicate handle used only to shut the socket down without taking a lock.
    socket: UnixStream,
}
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
impl Shared {
    /// Keep the first fault, end the channel and wake every waiter.
    fn fail(&self, fault: Fault) {
        locked(&self.inbox).fault.get_or_insert(fault);
        self.stop.store(true, Ordering::Release);
        let _ = self.socket.shutdown(Shutdown::Both);
        self.changed.notify_all();
    }
}

/// An opened channel with its reader running. Share it between threads by reference.
#[derive(Debug)]
pub struct Bus {
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
}

impl Bus {
    /// Start driving `link`. `limits` bound the outstanding entries of this channel,
    /// `budget` those shared with the installation's other channels.
    pub fn start(
        link: Link,
        budgets: Budgets,
        limits: Caps,
        budget: Option<Budget>,
    ) -> Result<Self, Fault> {
        let Link {
            generation,
            role,
            reader,
            writer,
        } = link;
        let socket = reader
            .duplicate_stream()
            .map_err(|error| Fault::Io(error.kind()))?;
        let shared = Arc::new(Shared {
            role,
            generation,
            budgets,
            stop: AtomicBool::new(false),
            inbox: Mutex::new(Inbox::default()),
            changed: Condvar::new(),
            ledger: Mutex::new(Ledger::new(role, limits, budget)),
            writer: Mutex::new(writer),
            socket,
        });
        let driver = Arc::clone(&shared);
        let reader = thread::Builder::new()
            .name("tmt-object-channel".into())
            .spawn(move || drive(&driver, reader))
            .map_err(|error| Fault::Io(error.kind()))?;
        Ok(Self {
            shared,
            reader: Some(reader),
        })
    }

    pub fn role(&self) -> Role {
        self.shared.role
    }
    pub fn generation(&self) -> Uuid4 {
        self.shared.generation
    }
    /// The fault that ended the channel, if one has.
    pub fn fault(&self) -> Option<Fault> {
        locked(&self.shared.inbox).fault
    }

    /// Send one frame within the write bound. A frame this end may not send, or whose
    /// identifiers the ledger refuses, comes back as `Fault::Correlation` with nothing
    /// written and the channel still usable; a failed or stalled write ends it.
    pub fn send(&self, frame: &Frame) -> Result<(), Fault> {
        if let Some(fault) = self.fault() {
            return Err(fault);
        }
        let bytes = prepared(self.shared.generation, frame)?;
        // The ledger and the wire see frames in one order: the writer is held across both.
        let mut writer = locked(&self.shared.writer);
        locked(&self.shared.ledger)
            .apply(frame, true)
            .map_err(Fault::Correlation)?;
        match writer.send(&bytes, self.shared.budgets.write) {
            Ok(()) => Ok(()),
            Err(fault) => {
                drop(writer);
                self.shared.fail(fault);
                Err(self.fault().unwrap_or(fault))
            }
        }
    }

    /// The next frame, waiting until `until` (`None` waits for a frame or the end of the
    /// channel). Frames received before a fault are delivered before it is reported.
    pub fn recv(&self, until: Option<Instant>) -> Result<Frame, Fault> {
        let mut inbox = locked(&self.shared.inbox);
        loop {
            if let Some((frame, bytes)) = inbox.frames.pop_front() {
                inbox.bytes -= bytes;
                self.shared.changed.notify_all();
                return Ok(frame);
            }
            if let Some(fault) = inbox.fault {
                return Err(fault);
            }
            let wait = match until {
                None => SLICE,
                Some(limit) => limit
                    .checked_duration_since(Instant::now())
                    .filter(|left| !left.is_zero())
                    .ok_or(Fault::Timeout(Stage::Idle))?
                    .min(SLICE),
            };
            inbox = self
                .shared
                .changed
                .wait_timeout(inbox, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// End the channel: shut the socket down, wake every waiter, join the reader.
    pub fn close(mut self) {
        self.end();
    }
    fn end(&mut self) {
        self.shared.fail(Fault::Closed);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }

    #[cfg(test)]
    fn watch(&self) -> std::sync::Weak<Shared> {
        Arc::downgrade(&self.shared)
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        self.end();
    }
}

/// The reader thread: read, check, queue, repeat, until a fault or a stop.
fn drive(shared: &Shared, mut reader: bounded::Reader) {
    let idle = Idle {
        stop: &shared.stop,
        until: None,
    };
    loop {
        let (frame, bytes) =
            match read_checked(&mut reader, shared.generation, idle, &shared.budgets) {
                Ok(read) => read,
                Err(fault) => return shared.fail(fault),
            };
        if let Err(reason) = locked(&shared.ledger).apply(&frame, false) {
            return shared.fail(Fault::Correlation(reason));
        }
        let mut inbox = locked(&shared.inbox);
        while !inbox.frames.is_empty()
            && (inbox.frames.len() >= QUEUE_FRAMES || inbox.bytes + bytes > QUEUE_BYTES)
        {
            if shared.stop.load(Ordering::Acquire) {
                return;
            }
            // Reading pauses while the consumer is behind; the peer's own write bound
            // ends a peer that outruns it.
            inbox = shared
                .changed
                .wait_timeout(inbox, SLICE)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        inbox.bytes += bytes;
        inbox.frames.push_back((frame, bytes));
        drop(inbox);
        shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests;
