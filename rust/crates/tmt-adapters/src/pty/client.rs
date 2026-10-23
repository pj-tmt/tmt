//! The blocking client of a supervisor: what delivery, capture and attach build on.

use super::{socket_path, validate};
use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use std::{
    io::{self, Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    path::Path,
    time::{Duration, Instant},
};
use tmt_pty_protocol::{
    ClientMessage, Decoder, Frame, MAX_INPUT, ProtocolError, ServerMessage, Size, VERSION,
};

const HANDSHAKE: Duration = Duration::from_secs(5);
const WRITE_DEADLINE: Duration = Duration::from_secs(5);

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
        validate(directory, id)?;
        let stream = UnixStream::connect(socket_path(directory, id))?;
        stream.set_write_timeout(Some(WRITE_DEADLINE))?;
        let mut connection = Self {
            stream,
            decoder: Decoder::new(),
            size: Size::new(1, 1).map_err(invalid)?,
        };
        connection.send(&ClientMessage::Hello { version: VERSION })?;
        match connection.receive(HANDSHAKE)? {
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
