//! The blocking client of a supervisor: what delivery, capture and attach build on.

use super::{socket_path, validate};
use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use std::{
    io::{self, Read, Write},
    os::{
        fd::{AsFd, AsRawFd},
        unix::net::UnixStream,
    },
    path::Path,
    time::{Duration, Instant},
};
use tmt_pty_protocol::{
    ClientMessage, Control, ControlReply, Decoder, Frame, MAX_INPUT, ProtocolError, ServerMessage,
    Size, VERSION,
};

const HANDSHAKE: Duration = Duration::from_secs(5);

/// An attached client. Dropping it detaches; the program keeps running.
pub struct Connection {
    stream: UnixStream,
    decoder: Decoder,
    size: Size,
}

fn invalid(error: ProtocolError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

impl Connection {
    /// Connects and completes the handshake. A refusal, a closed socket and a
    /// wrong first frame are errors; none of them reaches the program.
    pub fn connect(directory: &Path, id: &str) -> io::Result<Self> {
        Self::connect_until(directory, id, Instant::now() + HANDSHAKE)
    }

    pub(crate) fn connect_until(directory: &Path, id: &str, deadline: Instant) -> io::Result<Self> {
        validate(directory, id)?;
        if !crate::private_file::directory_is_private(directory) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the socket directory is not private",
            ));
        }
        use std::os::unix::fs::FileTypeExt;
        if !std::fs::symlink_metadata(socket_path(directory, id))?
            .file_type()
            .is_socket()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the member path is not a socket",
            ));
        }
        let stream = connect_socket(&socket_path(directory, id), deadline)?;
        if tmt_sys::peer_uid(stream.as_fd())? != nix::unistd::geteuid().as_raw() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the supervisor belongs to a different user",
            ));
        }
        stream.set_write_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        Self::handshake_until(stream, deadline)
    }

    #[cfg(test)]
    pub(super) fn handshake(stream: UnixStream) -> io::Result<Self> {
        Self::handshake_until(stream, Instant::now() + HANDSHAKE)
    }

    fn handshake_until(stream: UnixStream, deadline: Instant) -> io::Result<Self> {

        let mut connection = Self {
            stream,
            decoder: Decoder::new(),
            size: Size::new(1, 1).map_err(invalid)?,
        };
        // A supervisor that refuses answers and closes without reading, so the
        // Hello can find the socket closed; the refusal it left is still there.
        let sent = connection.send(&ClientMessage::Hello { version: VERSION });
        let answer = connection.receive(deadline.saturating_duration_since(Instant::now()));
        if let Err(error) = sent
            && !matches!(answer, Ok(Some(ServerMessage::Refused(_))))
        {
            return Err(error);
        }
        match answer? {

            Some(ServerMessage::Welcome { version, size }) if version == VERSION => {
                connection.size = size;
                Ok(connection)
            }
            Some(ServerMessage::Refused(reason)) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the supervisor refused the connection: {reason:?}"),
            )),
            Some(other) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected first frame {other:?}"),
            )),
            None => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the supervisor did not answer the handshake",
            )),
        }
    }

    /// The window size in force when the handshake completed.
    pub fn size(&self) -> Size {
        self.size
    }

    pub fn send(&mut self, message: &ClientMessage) -> io::Result<()> {
        self.stream.write_all(&message.encode())
    }

    /// Keystrokes or pasted bytes for the program, in frames of at most [`MAX_INPUT`].
    pub fn input(&mut self, bytes: &[u8]) -> io::Result<()> {
        bytes
            .chunks(MAX_INPUT)
            .try_for_each(|chunk| self.send(&ClientMessage::Input(chunk.to_vec())))
    }

    /// Runs one Core control, draining live output until its reply. These clients
    /// never resize. Drop the connection promptly after the operation so it cannot
    /// become a slow output consumer; dropping its socket detaches even on error.
    pub fn control(&mut self, request: Control, timeout: Duration) -> io::Result<ControlReply> {
        let mut payload = Vec::new();
        request.write_payload(&mut payload);
        Control::read(request.kind(), &payload).map_err(invalid)?;
        let deadline = Instant::now() + timeout;
        self.stream
            .set_write_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        self.send(&ClientMessage::Control(request))?;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receive(remaining)? {
                Some(ServerMessage::Control(answer)) => return Ok(answer),
                Some(ServerMessage::Output(_) | ServerMessage::Resized(_)) => {}
                Some(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the supervisor ended the control connection",
                    ));
                }
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the supervisor did not answer the control",
                    ));
                }
            }
        }
    }

    /// Waits once for managed launch admission while draining output. Failure
    /// includes a bounded output tail; it never resends or stops the member.
    pub fn await_launch(&mut self, timeout: Duration) -> io::Result<String> {
        let deadline = Instant::now() + timeout;
        self.send(&ClientMessage::Control(Control::AwaitLaunch))?;
        let mut output = Vec::new();
        loop {
            match self.receive(deadline.saturating_duration_since(Instant::now()))? {
                Some(ServerMessage::Control(ControlReply::Launched(marker))) => return Ok(marker),
                Some(ServerMessage::Output(bytes)) => {
                    output.extend(bytes);
                    if output.len() > 8192 {
                        output.drain(..output.len() - 8192);
                    }
                }
                Some(ServerMessage::Resized(_)) => {}
                other => {
                    return Err(io::Error::other(format!(
                        "managed launch not confirmed ({other:?}); output: {}",
                        String::from_utf8_lossy(&output)
                    )));
                }
            }
        }
    }

    /// The next frame, or `None` when none arrived within `timeout`. A closed
    /// connection is `UnexpectedEof`: the supervisor ended or dropped this client.
    pub fn receive(&mut self, timeout: Duration) -> io::Result<Option<ServerMessage>> {
        let deadline = Instant::now() + timeout;
        let mut buffer = [0u8; 32 * 1024];
        loop {
            if let Some(message) = self.decoder.frame::<ServerMessage>().map_err(invalid)? {
                return Ok(Some(message));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            // Poll rather than set a read timeout: a socket its peer has closed
            // refuses the option on macOS, yet still holds data to read.
            let mut events = [PollFd::new(self.stream.as_fd(), PollFlags::POLLIN)];
            let timeout = PollTimeout::try_from(remaining).unwrap_or(PollTimeout::MAX);
            match poll(&mut events, timeout) {
                Ok(0) => return Ok(None),
                Ok(_) => {}
                Err(Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
            }
            match self.stream.read(&mut buffer) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(count) => self.decoder.push(&buffer[..count]).map_err(invalid)?,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}

/// Nonblocking connect keeps a stopped or saturated supervisor within the
/// caller's original budget, including the listener backlog and handshake.
fn connect_socket(path: &Path, deadline: Instant) -> io::Result<UnixStream> {
    use nix::sys::socket::{
        AddressFamily, SockFlag, SockType, UnixAddr, connect, getsockopt, socket,
        sockopt::SocketError,
    };
    let fd = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )?;
    use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
    fcntl(&fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
    fcntl(&fd, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
    let address = UnixAddr::new(path)?;
    match connect(fd.as_raw_fd(), &address) {
        Ok(()) => {}
        Err(Errno::EINPROGRESS | Errno::EAGAIN) => loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let mut events = [PollFd::new(fd.as_fd(), PollFlags::POLLOUT)];
            match poll(
                &mut events,
                PollTimeout::try_from(remaining).unwrap_or(PollTimeout::MAX),
            ) {
                Err(Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
                Ok(0) => return Err(io::ErrorKind::TimedOut.into()),
                Ok(_) => {}
            }
            let error = getsockopt(&fd, SocketError)?;
            if error != 0 {
                return Err(io::Error::from_raw_os_error(error));
            }
            break;
        },
        Err(error) => return Err(error.into()),
    }
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false)?;
    Ok(stream)
}
