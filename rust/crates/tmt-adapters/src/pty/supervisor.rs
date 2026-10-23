//! One member's supervisor: it owns the pseudo-terminal the program runs on and
//! serves it to attached clients over a private Unix socket.
//!
//! A single thread polls the listener, the terminal and every client, so no
//! state is shared. Output is read whether or not anyone is attached. While a
//! client lags far behind, reading pauses so the program slows to its pace; a
//! client that stops reading altogether is evicted, never buffered without bound.

use super::{lock_path, socket_path, validate};
use crate::{file_lock, private_file::ensure_private_directory};
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, FdFlag, Flock, OFlag, fcntl},
    poll::{PollFd, PollFlags, PollTimeout, poll},
    pty::{Winsize, openpty},
};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs::{self, File},
    io::{self, Read, Write},
    os::fd::AsFd,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
        process::ExitStatusExt,
    },
    path::PathBuf,
    process::{Child, Command, ExitStatus},
    time::{Duration, Instant},
};
use tmt_pty_protocol::{
    ClientMessage, Decoder, Eviction, Exit, Frame, MAX_OUTPUT, Refusal, ServerMessage, Size,
    VERSION,
};

/// How often the program's end is checked when nothing else wakes the loop.
const TICK: u16 = 200;
pub(super) const MAX_CLIENTS: usize = 16;
/// A client that has not completed the handshake by then is dropped.
const HELLO_DEADLINE: Duration = Duration::from_secs(5);
/// Queued output at which reading the program pauses until clients catch up.
const SOFT_BACKLOG: usize = 256 * 1024;
/// A client with output queued that has read none of it for this long is evicted.
const STALL: Duration = Duration::from_secs(3);
/// Keystrokes waiting for the program; clients are not read while above it.
const INPUT_HIGH_WATER: usize = 64 * 1024;
/// How long a closing client may take to read its last frame.
const CLOSING_GRACE: Duration = Duration::from_secs(1);
/// How long the program's last output and exit may take to reach clients.
const FLUSH_GRACE: Duration = Duration::from_millis(500);

/// Where and how to run one member.
pub struct Launch {
    /// The owner-only directory holding the member's socket and lock.
    pub directory: PathBuf,
    /// The member's id (see `pty::socket_path`); unique per run.
    pub id: String,
    /// The terminal's initial size.
    pub size: Size,
    /// The program and its arguments; the first word is looked up in `PATH`.
    /// The program inherits this process's environment and working directory.
    pub program: Vec<OsString>,
}

/// The socket and lock files of a running supervisor, removed when it ends. Ids
/// are never reused, so removing the lock cannot split a lock domain.
struct Files {
    socket: PathBuf,
    lock: PathBuf,
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_file(&self.lock);
    }
}

enum Fate {
    Leave,
    Evict(Eviction),
    Refuse(Refusal),
}

/// Encoded frames waiting for a client, with the progress into the first.
#[derive(Default)]
struct Outbox {
    frames: VecDeque<Vec<u8>>,
    offset: usize,
    queued: usize,
}

impl Outbox {
    fn push(&mut self, frame: Vec<u8>) {
        self.queued += frame.len();
        self.frames.push_back(frame);
    }

    fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Writes until the socket would block and returns how many bytes it took.
    fn flush(&mut self, mut stream: &UnixStream) -> io::Result<usize> {
        let mut written = 0;
        while let Some(frame) = self.frames.front() {
            match stream.write(&frame[self.offset..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => {
                    written += count;
                    self.offset += count;
                    self.queued -= count;
                    if self.offset == frame.len() {
                        self.frames.pop_front();
                        self.offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(written)
    }

    /// Drops every frame not yet started, so a final frame can follow cleanly.
    fn keep_only_started(&mut self) {
        let started = (self.offset > 0).then(|| self.frames.pop_front()).flatten();
        self.frames.clear();
        self.queued = 0;
        if let Some(frame) = started {
            self.queued = frame.len() - self.offset;
            self.frames.push_back(frame);
        }
    }
}

struct Client {
    id: u64,
    stream: UnixStream,
    decoder: Decoder,
    outbox: Outbox,
    welcomed: bool,
    /// Set by the client's own `Resize`; a client without one never moves the window.
    size: Option<Size>,
    since: Instant,
    /// The last time the client took output, or the outbox last went from empty.
    progress: Instant,
    fate: Option<Fate>,
    /// Set once a final frame is queued: the client is dropped when it is
    /// delivered or this deadline passes.
    closing_until: Option<Instant>,
}

impl Client {
    fn live(&self) -> bool {
        self.fate.is_none() && self.closing_until.is_none()
    }

    /// Far enough behind that the program should wait for it.
    fn congested(&self) -> bool {
        self.welcomed && self.live() && self.outbox.queued >= SOFT_BACKLOG
    }

    fn enqueue(&mut self, frame: Vec<u8>) {
        if self.outbox.is_empty() {
            self.progress = Instant::now();
        }
        self.outbox.push(frame);
    }

    fn queue(&mut self, message: &ServerMessage) {
        self.enqueue(message.encode());
    }

    fn close_with(&mut self, message: &ServerMessage, now: Instant) {
        self.outbox.keep_only_started();
        self.queue(message);
        self.closing_until = Some(now + CLOSING_GRACE);
    }
}

/// What one poll reported.
struct Ready {
    listener: bool,
    master_read: bool,
    master_write: bool,
    clients: Vec<PollFlags>,
}

#[derive(PartialEq, Eq)]
enum Pump {
    /// Nothing more to read right now.
    Idle,
    /// The budget ran out; more may be pending.
    More,
    /// The program closed its end of the terminal.
    Closed,
}

/// A started member: the lock is held, the socket is bound and the program runs.
pub struct Supervisor {
    listener: UnixListener,
    master: File,
    child: Child,
    size: Size,
    clients: Vec<Client>,
    next_client: u64,
    /// The client whose window the terminal follows.
    active: Option<u64>,
    input: VecDeque<u8>,
    owner: u32,
    // Drop order: the files go before the lock is released.
    _files: Files,
    _lock: Flock<File>,
}

impl Supervisor {
    /// Takes the member's lock, binds its socket and starts the program. Fails,
    /// leaving nothing behind, when the directory is not owner-only, another
    /// supervisor serves the id, or the program cannot start.
    pub fn start(launch: Launch) -> io::Result<Self> {
        let Launch {
            directory,
            id,
            size,
            program,
        } = launch;
        validate(&directory, &id)?;
        let Some((name, arguments)) = program.split_first() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a member needs a program to run",
            ));
        };
        ensure_private_directory(&directory)?;
        let lock = file_lock::exclusive(&lock_path(&directory, &id)).map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "another supervisor already serves this member",
                )
            } else {
                error
            }
        })?;
        let files = Files {
            socket: socket_path(&directory, &id),
            lock: lock_path(&directory, &id),
        };
        // Holding the lock, a socket file here is a dead supervisor's leftover.
        match fs::remove_file(&files.socket) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&files.socket)?;
        fs::set_permissions(&files.socket, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let terminal = openpty(Some(&winsize(size)), None)?;
        for end in [&terminal.master, &terminal.slave] {
            fcntl(end, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
        }
        fcntl(&terminal.master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
        let mut command = Command::new(name);
        command.args(arguments);
        let child = tmt_sys::spawn_on_terminal(command, terminal.slave)?;
        Ok(Self {
            listener,
            master: File::from(terminal.master),
            child,
            size,
            clients: Vec::new(),
            next_client: 0,
            active: None,
            input: VecDeque::new(),
            owner: nix::unistd::geteuid().as_raw(),
            _files: files,
            _lock: lock,
        })
    }

    /// The program's process id.
    pub fn program_pid(&self) -> u32 {
        self.child.id()
    }

    /// Serves until the program ends, then tells the attached clients and removes
    /// the socket. Returns an error only when the supervisor itself fails; the
    /// terminal then hangs up and the program receives SIGHUP.
    pub fn run(mut self) -> io::Result<()> {
        let mut master_open = true;
        let exit = loop {
            let ready = self.wait(master_open)?;
            if ready.listener {
                self.accept()?;
            }
            if master_open && ready.master_read {
                master_open = self.pump_output(16) != Pump::Closed;
            }
            if ready.master_write {
                self.pump_input();
            }
            for (index, flags) in ready.clients.into_iter().enumerate() {
                if !flags.is_empty() {
                    self.serve_client(index, flags);
                }
            }
            self.sweep();
            if let Some(status) = self.child.try_wait()? {
                break exit_of(status);
            }
        };
        if master_open {
            for _ in 0..256 {
                if self.pump_output(16) != Pump::More {
                    break;
                }
            }
        }
        self.broadcast(&ServerMessage::Exited(exit));
        self.flush_clients();
        Ok(())
    }

    fn wait(&self, master_open: bool) -> io::Result<Ready> {
        let mut set = vec![PollFd::new(self.listener.as_fd(), PollFlags::POLLIN)];
        let mut master_events = PollFlags::empty();
        if master_open && !self.congested() {
            master_events |= PollFlags::POLLIN;
        }
        if !self.input.is_empty() {
            master_events |= PollFlags::POLLOUT;
        }
        let master_slot = (!master_events.is_empty()).then(|| {
            set.push(PollFd::new(self.master.as_fd(), master_events));
            set.len() - 1
        });
        let first_client = set.len();
        for client in &self.clients {
            let mut events = PollFlags::empty();
            if client.live() && self.input.len() < INPUT_HIGH_WATER {
                events |= PollFlags::POLLIN;
            }
            if !client.outbox.is_empty() {
                events |= PollFlags::POLLOUT;
            }
            set.push(PollFd::new(client.stream.as_fd(), events));
        }
        match poll(&mut set, PollTimeout::from(TICK)) {
            Ok(_) | Err(Errno::EINTR) => {}
            Err(error) => return Err(error.into()),
        }
        let flags = |slot: usize| set[slot].revents().unwrap_or_else(PollFlags::empty);
        let master = master_slot.map_or_else(PollFlags::empty, flags);
        Ok(Ready {
            listener: flags(0).intersects(PollFlags::POLLIN),
            master_read: master.intersects(PollFlags::POLLIN | PollFlags::POLLHUP),
            master_write: master.contains(PollFlags::POLLOUT),
            clients: (first_client..set.len()).map(flags).collect(),
        })
    }

    fn accept(&mut self) -> io::Result<()> {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => self.admit(stream),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn admit(&mut self, stream: UnixStream) {
        // Refused connections wait to be told so; past this many, just drop them.
        if self.clients.len() >= 2 * MAX_CLIENTS || stream.set_nonblocking(true).is_err() {
            return;
        }
        let refusal = if tmt_sys::peer_uid(stream.as_fd()).ok() != Some(self.owner) {
            Some(Refusal::Owner)
        } else if self.clients.iter().filter(|client| client.live()).count() >= MAX_CLIENTS {
            Some(Refusal::Full)
        } else {
            None
        };
        self.next_client += 1;
        self.clients.push(Client {
            id: self.next_client,
            stream,
            decoder: Decoder::new(),
            outbox: Outbox::default(),
            welcomed: false,
            size: None,
            since: Instant::now(),
            progress: Instant::now(),
            fate: refusal.map(Fate::Refuse),
            closing_until: None,
        });
    }

    /// Reads what the program wrote and queues it for every attached client, in
    /// frames as full as the terminal's pending output allows.
    fn pump_output(&mut self, budget: usize) -> Pump {
        let mut buffer = [0u8; MAX_OUTPUT];
        for _ in 0..budget {
            if self.congested() {
                return Pump::Idle;
            }
            let mut filled = 0;
            let state = loop {
                match self.master.read(&mut buffer[filled..]) {
                    Ok(0) => break Pump::Closed,
                    Ok(count) => {
                        filled += count;
                        if filled == buffer.len() {
                            break Pump::More;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break Pump::Idle,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    // Linux reports a closed terminal as EIO.
                    Err(_) => break Pump::Closed,
                }
            };
            if filled > 0 {
                self.broadcast(&ServerMessage::Output(buffer[..filled].to_vec()));
            }
            if state != Pump::More {
                return state;
            }
        }
        Pump::More
    }

    fn pump_input(&mut self) {
        while !self.input.is_empty() {
            match self.master.write(self.input.as_slices().0) {
                Ok(0) => break,
                Ok(count) => {
                    self.input.drain(..count);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                // The program closed its terminal: nothing will read these.
                Err(_) => {
                    self.input.clear();
                    break;
                }
            }
        }
    }

    fn congested(&self) -> bool {
        self.clients.iter().any(Client::congested)
    }

    fn broadcast(&mut self, message: &ServerMessage) {
        let frame = message.encode();
        for client in self.clients.iter_mut().filter(|c| c.welcomed && c.live()) {
            client.enqueue(frame.clone());
        }
    }

    fn serve_client(&mut self, index: usize, flags: PollFlags) {
        let client = &mut self.clients[index];
        if flags.contains(PollFlags::POLLOUT) {
            match client.outbox.flush(&client.stream) {
                Ok(0) => {}
                Ok(_) => client.progress = Instant::now(),
                Err(_) => {
                    client.fate = Some(Fate::Leave);
                    return;
                }
            }
        }
        if !client.live() {
            if flags.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
                client.fate = Some(Fate::Leave);
            }
            return;
        }
        if !flags.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR) {
            return;
        }
        let mut buffer = [0u8; 8192];
        let count = match (&client.stream).read(&mut buffer) {
            Ok(0) => {
                client.fate = Some(Fate::Leave);
                return;
            }
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return,
            Err(_) => {
                client.fate = Some(Fate::Leave);
                return;
            }
        };
        let mut messages = Vec::new();
        if client.decoder.push(&buffer[..count]).is_err() {
            client.fate = Some(Fate::Leave);
            return;
        }
        loop {
            match client.decoder.frame::<ClientMessage>() {
                Ok(Some(message)) => messages.push(message),
                Ok(None) => break,
                Err(_) => {
                    // A frame that is not valid ends the connection; the earlier
                    // ones in the same read were valid and are still handled.
                    client.fate = Some(Fate::Leave);
                    break;
                }
            }
        }
        for message in messages {
            if self.clients[index].fate.is_some() {
                break;
            }
            self.handle(index, message);
        }
    }

    fn handle(&mut self, index: usize, message: ClientMessage) {
        let (id, welcomed) = (self.clients[index].id, self.clients[index].welcomed);
        match message {
            ClientMessage::Hello { version } if !welcomed => {
                let size = self.size;
                let client = &mut self.clients[index];
                if version == VERSION {
                    client.welcomed = true;
                    client.queue(&ServerMessage::Welcome {
                        version: VERSION,
                        size,
                    });
                } else {
                    client.fate = Some(Fate::Refuse(Refusal::Version));
                }
            }
            _ if !welcomed => self.clients[index].fate = Some(Fate::Refuse(Refusal::Handshake)),
            // A second hello is not part of the protocol.
            ClientMessage::Hello { .. } | ClientMessage::Detach => {
                self.clients[index].fate = Some(Fate::Leave);
            }
            ClientMessage::Input(data) => {
                // Typing from another window brings the terminal to that window.
                if self.active != Some(id)
                    && let Some(size) = self.clients[index].size
                {
                    self.active = Some(id);
                    self.resize(size);
                }
                self.input.extend(data);
            }
            ClientMessage::Resize(size) => {
                self.clients[index].size = Some(size);
                self.active = Some(id);
                self.resize(size);
            }
            ClientMessage::TakeSole => {
                for other in self.clients.iter_mut().filter(|c| c.id != id && c.live()) {
                    other.fate = Some(Fate::Evict(Eviction::TakenOver));
                }
                if let Some(size) = self.clients[index].size {
                    self.active = Some(id);
                    self.resize(size);
                }
            }
        }
    }

    fn resize(&mut self, size: Size) {
        if size == self.size {
            return;
        }
        // A terminal the program already closed cannot be resized; nothing to do.
        let _ = tmt_sys::set_window_size(self.master.as_fd(), size.cols, size.rows);
        self.size = size;
        self.broadcast(&ServerMessage::Resized(size));
    }

    /// Applies each client's pending end: leaving ones go, evicted and refused
    /// ones get their last frame, and silent or finished ones are dropped.
    fn sweep(&mut self) {
        let now = Instant::now();
        let mut kept = Vec::with_capacity(self.clients.len());
        for mut client in std::mem::take(&mut self.clients) {
            if client.live() && !client.welcomed && now - client.since > HELLO_DEADLINE {
                client.fate = Some(Fate::Refuse(Refusal::Handshake));
            } else if client.live() && !client.outbox.is_empty() && now - client.progress > STALL {
                client.fate = Some(Fate::Evict(Eviction::TooSlow));
            }
            match client.fate.take() {
                Some(Fate::Leave) => continue,
                Some(Fate::Evict(reason)) => {
                    client.close_with(&ServerMessage::Evicted(reason), now);
                }
                Some(Fate::Refuse(reason)) => {
                    client.close_with(&ServerMessage::Refused(reason), now);
                }
                None => {}
            }
            if client
                .closing_until
                .is_some_and(|until| client.outbox.is_empty() || now >= until)
            {
                continue;
            }
            kept.push(client);
        }
        self.clients = kept;
        if let Some(active) = self.active
            && !self.clients.iter().any(|c| c.id == active && c.live())
        {
            self.active = None;
        }
    }

    /// Gives the last frames, the program's exit among them, a short time to be read.
    fn flush_clients(&mut self) {
        let deadline = Instant::now() + FLUSH_GRACE;
        loop {
            self.clients
                .retain_mut(|c| c.outbox.flush(&c.stream).is_ok() && !c.outbox.is_empty());
            let remaining = deadline.saturating_duration_since(Instant::now());
            if self.clients.is_empty() || remaining.is_zero() {
                return;
            }
            let mut set: Vec<_> = self
                .clients
                .iter()
                .map(|c| PollFd::new(c.stream.as_fd(), PollFlags::POLLOUT))
                .collect();
            let wait = u16::try_from(remaining.as_millis()).unwrap_or(u16::MAX);
            let _ = poll(&mut set, PollTimeout::from(wait));
        }
    }
}

fn winsize(size: Size) -> Winsize {
    Winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

fn exit_of(status: ExitStatus) -> Exit {
    status
        .code()
        .map(Exit::Code)
        .unwrap_or_else(|| Exit::Signal(status.signal().unwrap_or(0)))
}
