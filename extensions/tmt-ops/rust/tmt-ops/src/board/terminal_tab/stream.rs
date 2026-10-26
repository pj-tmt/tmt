//! One Terminal tab's connection to a member's supervisor, on a thread of its own. The board never
//! reads or writes the socket: it gives the thread [`Command`]s and receives [`StreamEvent`]s through the
//! sink it handed over, so painting and key handling do no I/O.
//!
//! The order is the contract's: `Hello`, `Welcome`, one `Snapshot`, then live `Output` applied on
//! top of the snapshot. Frames before `SnapshotStart` are superseded by it. A view never sends
//! `TakeSole`, never sends `Resize` on its own, and never sends or interprets Core's controls.
use super::discovery::Address;
use std::{
    io::{ErrorKind, Read, Write},
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, TryRecvError, channel},
    },
    thread,
    time::{Duration, Instant},
};
use tmt_pty_protocol::{
    ClientMessage, Decoder, Eviction, Exit, Frame, MAX_INPUT, Refusal, ServerMessage, Size, VERSION,
};
use tmt_tui::components::terminal::{Frame as Screenful, Screen};

/// What the board hears from the thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum StreamEvent {
    /// A connection attempt began: the first, or a retry after the stream was lost.
    Connecting { attempt: u32 },
    /// The screen to show now. The first follows the snapshot; later ones follow live output,
    /// at most one every [`Timing::publish`].
    Screen(Arc<Screenful>),
    /// The program ended. The stream is over and does not come back.
    Exited(Exit),
    /// Another client took sole use of the terminal. It does not come back by itself.
    Evicted(Eviction),
    /// The supervisor would not attach this viewer.
    Refused(Refusal),
    /// The stream could not be had again after the allowed tries, for this reason.
    Lost(String),
}

/// What the board tells the thread.
#[derive(Debug)]
enum Command {
    Input(Vec<u8>),
    Resize(Size),
    /// Typing ended: the window is no longer this viewer's to keep.
    StopTyping,
}

/// How long the thread waits, polls and backs off.
#[derive(Clone, Copy, Debug)]
pub(in crate::board) struct Timing {
    /// How long one read waits, which is also how soon a stop or a command is noticed.
    pub poll: Duration,
    /// The least time between two screens sent to the board.
    pub publish: Duration,
    /// How long `Welcome` may take.
    pub handshake: Duration,
    /// The first wait before a retry; it doubles up to [`Timing::backoff_cap`].
    pub backoff: Duration,
    pub backoff_cap: Duration,
    /// Failed tries in a row before the stream is called lost.
    pub attempts: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(20),
            publish: Duration::from_millis(33),
            handshake: Duration::from_secs(5),
            backoff: Duration::from_millis(500),
            backoff_cap: Duration::from_secs(5),
            attempts: 12,
        }
    }
}

/// The board's end of one connection. Dropping it detaches and ends the thread.
pub(in crate::board) struct Stream {
    commands: Sender<Command>,
    stop: Arc<AtomicBool>,
}

impl Stream {
    pub fn open(address: Address, sink: impl Fn(StreamEvent) + Send + 'static) -> Self {
        Self::open_with(address, Timing::default(), sink)
    }

    pub fn open_with(
        address: Address,
        timing: Timing,
        sink: impl Fn(StreamEvent) + Send + 'static,
    ) -> Self {
        let (commands, received) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let owned = Arc::clone(&stop);
        thread::Builder::new()
            .name("tmt-ops-terminal".into())
            .spawn(move || run(&address, timing, &sink, &received, &owned))
            .expect("a thread for the terminal stream");
        Self { commands, stop }
    }

    /// Keystrokes or a paste, in order, in frames the protocol allows. Input sent while the
    /// stream is not connected is dropped, never queued for later.
    pub fn input(&self, bytes: &[u8]) {
        for chunk in input_chunks(bytes) {
            let _ = self.commands.send(Command::Input(chunk.to_vec()));
        }
    }

    /// The window this viewer types in. A viewer that only watches never calls it.
    pub fn resize(&self, size: Size) {
        let _ = self.commands.send(Command::Resize(size));
    }

    /// Typing ended. The window is forgotten, so a reconnect from here on sends no `Resize`
    /// and a viewer that only watches never takes the size back from another client; nothing
    /// is resized back now either.
    pub fn stop_typing(&self) {
        let _ = self.commands.send(Command::StopTyping);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Pieces of at most [`MAX_INPUT`] bytes that never cut a UTF-8 character in two.
fn input_chunks(bytes: &[u8]) -> Vec<&[u8]> {
    let mut chunks = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let mut end = rest.len().min(MAX_INPUT);
        // Back up to the start of a character when the cut would split one.
        while end < rest.len() && end > 0 && (rest[end] & 0b1100_0000) == 0b1000_0000 {
            end -= 1;
        }
        if end == 0 {
            end = rest.len().min(MAX_INPUT);
        }
        let (chunk, tail) = rest.split_at(end);
        chunks.push(chunk);
        rest = tail;
    }
    chunks
}

/// How one connection ended.
enum End {
    /// The board dropped the stream.
    Stopped,
    /// Over for good: say so and finish.
    Over(StreamEvent),
    /// Lost: try again. `live` says whether the screen had been shown, which restarts the count.
    Lost { reason: String, live: bool },
}

fn run(
    address: &Address,
    timing: Timing,
    sink: &impl Fn(StreamEvent),
    commands: &Receiver<Command>,
    stop: &AtomicBool,
) {
    let mut attempt = 0;
    let mut failures = 0;
    let mut window: Option<Size> = None;
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        attempt += 1;
        sink(StreamEvent::Connecting { attempt });
        match session(address, timing, sink, commands, stop, &mut window) {
            End::Stopped => return,
            End::Over(event) => {
                sink(event);
                return;
            }
            End::Lost { reason, live } => {
                failures = if live { 1 } else { failures + 1 };
                if failures > timing.attempts {
                    sink(StreamEvent::Lost(reason));
                    return;
                }
                let wait = timing
                    .backoff
                    .saturating_mul(1 << (failures - 1).min(10))
                    .min(timing.backoff_cap);
                if !sleep(wait, timing.poll, stop) {
                    return;
                }
            }
        }
    }
}

/// Waits `wait` in polls; false when the stream was dropped meanwhile.
fn sleep(wait: Duration, poll: Duration, stop: &AtomicBool) -> bool {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        thread::sleep(poll.min(until.saturating_duration_since(Instant::now())));
    }
    !stop.load(Ordering::Relaxed)
}

/// Where a connection is between `Hello` and live output.
enum Phase {
    /// `Hello` is sent; the first frame back must be `Welcome`.
    Handshake(Instant),
    /// `Snapshot` is asked for. Output that comes first is superseded by it.
    Asked,
    /// The snapshot is arriving.
    Receiving(Vec<u8>),
    Live(Box<Screen>),
}

/// The most a snapshot may hold: far more than any screen the protocol sizes allow.
const SNAPSHOT_LIMIT: usize = 8 * 1024 * 1024;

fn session(
    address: &Address,
    timing: Timing,
    sink: &impl Fn(StreamEvent),
    commands: &Receiver<Command>,
    stop: &AtomicBool,
    window: &mut Option<Size>,
) -> End {
    let lost = |reason: String| End::Lost {
        reason,
        live: false,
    };
    let mut socket = match UnixStream::connect(&address.socket) {
        Ok(socket) => socket,
        Err(error) => return lost(format!("could not reach the terminal: {error}")),
    };
    if socket.set_read_timeout(Some(timing.poll)).is_err()
        || socket.set_write_timeout(Some(timing.handshake)).is_err()
    {
        return lost("could not set up the terminal connection".into());
    }
    if let Err(error) = send(&mut socket, &ClientMessage::Hello { version: VERSION }) {
        return lost(error);
    }
    let mut decoder = Decoder::new();
    let mut phase = Phase::Handshake(Instant::now() + timing.handshake);
    let mut shown = false;
    let mut dirty = false;
    let mut published = Instant::now();
    let mut buffer = [0; 16 * 1024];
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = send(&mut socket, &ClientMessage::Detach);
            return End::Stopped;
        }
        let connected = !matches!(phase, Phase::Handshake(_));
        loop {
            match commands.try_recv() {
                Ok(Command::Input(bytes)) if connected => {
                    if let Err(error) = send(&mut socket, &ClientMessage::Input(bytes)) {
                        return End::Lost {
                            reason: error,
                            live: shown,
                        };
                    }
                }
                Ok(Command::Input(_)) => {}
                Ok(Command::Resize(size)) => {
                    *window = Some(size);
                    if connected && let Err(error) = send(&mut socket, &ClientMessage::Resize(size))
                    {
                        return End::Lost {
                            reason: error,
                            live: shown,
                        };
                    }
                }
                Ok(Command::StopTyping) => *window = None,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        match socket.read(&mut buffer) {
            Ok(0) => {
                return End::Lost {
                    reason: "the terminal closed".into(),
                    live: shown,
                };
            }
            Ok(read) => {
                if decoder.push(&buffer[..read]).is_err() {
                    return End::Lost {
                        reason: "the terminal sent a frame the protocol does not allow".into(),
                        live: shown,
                    };
                }
                loop {
                    let message = match decoder.frame::<ServerMessage>() {
                        Ok(Some(message)) => message,
                        Ok(None) => break,
                        Err(_) => {
                            return End::Lost {
                                reason: "the terminal sent a frame the protocol does not allow"
                                    .into(),
                                live: shown,
                            };
                        }
                    };
                    match apply(message, &mut phase, &mut socket, *window) {
                        Step::Continue => {}
                        Step::Changed { immediate } => {
                            shown |= matches!(phase, Phase::Live(_));
                            dirty = true;
                            if immediate {
                                publish(&phase, sink, &mut dirty, &mut published);
                            }
                        }
                        Step::End(end) => {
                            publish(&phase, sink, &mut dirty, &mut published);
                            return match end {
                                End::Lost { reason, .. } => End::Lost {
                                    reason,
                                    live: shown,
                                },
                                other => other,
                            };
                        }
                    }
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => {
                return End::Lost {
                    reason: format!("the terminal could not be read: {error}"),
                    live: shown,
                };
            }
        }
        if let Phase::Handshake(deadline) = phase
            && Instant::now() > deadline
        {
            return lost("the terminal did not welcome the connection".into());
        }
        if dirty && published.elapsed() >= timing.publish {
            publish(&phase, sink, &mut dirty, &mut published);
        }
    }
}

fn publish(phase: &Phase, sink: &impl Fn(StreamEvent), dirty: &mut bool, published: &mut Instant) {
    if let Phase::Live(screen) = phase {
        sink(StreamEvent::Screen(Arc::new(screen.frame())));
        *dirty = false;
        *published = Instant::now();
    }
}

enum Step {
    Continue,
    /// The screen changed; `immediate` publishes it now rather than at the next tick.
    Changed {
        immediate: bool,
    },
    End(End),
}

fn apply(
    message: ServerMessage,
    phase: &mut Phase,
    socket: &mut UnixStream,
    window: Option<Size>,
) -> Step {
    let ended = |reason: &str| {
        Step::End(End::Lost {
            reason: reason.into(),
            live: false,
        })
    };
    match message {
        ServerMessage::Welcome { version, .. } => {
            if !matches!(phase, Phase::Handshake(_)) {
                return ended("the terminal welcomed the connection twice");
            }
            if version != VERSION {
                return Step::End(End::Over(StreamEvent::Refused(Refusal::Version)));
            }
            *phase = Phase::Asked;
            // While the viewer is typing, the window it typed in is still its window.
            let sent = window
                .map_or(Ok(()), |size| send(socket, &ClientMessage::Resize(size)))
                .and_then(|()| send(socket, &ClientMessage::Snapshot));
            match sent {
                Ok(()) => Step::Continue,
                Err(error) => Step::End(End::Lost {
                    reason: error,
                    live: false,
                }),
            }
        }
        ServerMessage::Refused(refusal) => Step::End(End::Over(StreamEvent::Refused(refusal))),
        // Core's controls share the socket but are not part of this stream.
        ServerMessage::Control(_) => Step::Continue,
        _ if matches!(phase, Phase::Handshake(_)) => ended("the terminal did not welcome first"),
        ServerMessage::Output(bytes) => {
            if let Phase::Live(screen) = phase {
                screen.feed(&bytes);
                Step::Changed { immediate: false }
            } else {
                // Before the snapshot starts, or inside one, output is superseded by it.
                Step::Continue
            }
        }
        ServerMessage::SnapshotStart => {
            *phase = Phase::Receiving(Vec::new());
            Step::Continue
        }
        ServerMessage::SnapshotData(bytes) => {
            if let Phase::Receiving(held) = phase {
                if held.len() + bytes.len() > SNAPSHOT_LIMIT {
                    return ended("the terminal sent a snapshot that is too large");
                }
                held.extend_from_slice(&bytes);
            }
            Step::Continue
        }
        ServerMessage::SnapshotEnd(size) => {
            if let Phase::Receiving(held) = phase {
                let screen = Screen::from_snapshot(size.cols, size.rows, held);
                *phase = Phase::Live(Box::new(screen));
                Step::Changed { immediate: true }
            } else {
                Step::Continue
            }
        }
        ServerMessage::Resized(size) => {
            if let Phase::Live(screen) = phase {
                screen.resize(size.cols, size.rows);
                Step::Changed { immediate: true }
            } else {
                Step::Continue
            }
        }
        ServerMessage::Exited(exit) => Step::End(End::Over(StreamEvent::Exited(exit))),
        ServerMessage::Evicted(Eviction::TakenOver) => {
            Step::End(End::Over(StreamEvent::Evicted(Eviction::TakenOver)))
        }
        ServerMessage::Evicted(Eviction::TooSlow) => ended("the terminal could not keep up"),
    }
}

fn send(socket: &mut UnixStream, message: &ClientMessage) -> Result<(), String> {
    socket
        .write_all(&message.encode())
        .map_err(|error| format!("the terminal could not be written: {error}"))
}

#[cfg(test)]
mod tests;
