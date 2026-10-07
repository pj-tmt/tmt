//! Deadline-bounded reads and writes over a nonblocking Unix stream. Every wait is a
//! `poll(2)` capped at a short slice, so a stop flag or a shutdown from another thread
//! is noticed promptly, and no read timeout is set on the socket (macOS refuses one
//! once the peer has closed even though buffered bytes remain readable).
use super::{Fault, Idle, Refusal, Stage};
use crate::limits::PREFIX_BYTES;
use nix::poll::{PollFd, PollFlags, poll};
use std::{
    io::{self, Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

/// The longest a single wait blocks before the caller's stop flag is looked at again.
const SLICE: Duration = Duration::from_millis(50);

/// Wait for the stream to be ready for `flags`, for at most `limit`. `true` means ready
/// (a hangup or error counts: the next read or write reports it).
fn ready(stream: &UnixStream, flags: PollFlags, limit: Duration) -> Result<bool, Fault> {
    let millis = limit.as_millis().min(u128::from(u16::MAX)) as u16;
    loop {
        let mut events = [PollFd::new(stream.as_fd(), flags)];
        match poll(&mut events, millis) {
            Ok(count) => return Ok(count > 0),
            Err(nix::errno::Errno::EINTR) => {}
            Err(error) => return Err(Fault::Io(io::Error::from(error).kind())),
        }
    }
}

fn remaining(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
}

#[derive(Debug)]
pub(super) struct Reader {
    stream: UnixStream,
    /// Bytes read past the upgrade head, consumed before the stream.
    first_bytes: Vec<u8>,
    #[cfg(test)]
    pub(super) prefix_seen: Option<std::sync::mpsc::Sender<Instant>>,
}
impl Reader {
    pub(super) fn new(stream: UnixStream, first_bytes: Vec<u8>) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            first_bytes,
            #[cfg(test)]
            prefix_seen: None,
        })
    }

    /// Another handle to the same socket, for shutting it down from elsewhere.
    pub(super) fn duplicate_stream(&self) -> io::Result<UnixStream> {
        self.stream.try_clone()
    }

    /// Bytes read past the upgrade head, to be consumed before the stream.
    pub(super) fn keep(&mut self, bytes: Vec<u8>) {
        self.first_bytes = bytes;
    }

    /// Fill `out` completely by `deadline`; EOF before that is `Truncated`.
    fn fill(&mut self, out: &mut [u8], deadline: Instant, stage: Stage) -> Result<(), Fault> {
        let mut filled = 0;
        let carried = self.first_bytes.len().min(out.len());
        out[..carried].copy_from_slice(&self.first_bytes[..carried]);
        self.first_bytes.drain(..carried);
        filled += carried;
        while filled < out.len() {
            match self.stream.read(&mut out[filled..]) {
                Ok(0) => return Err(Fault::Truncated),
                Ok(count) => filled += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let left = remaining(deadline).ok_or(Fault::Timeout(stage))?;
                    ready(&self.stream, PollFlags::POLLIN, left.min(SLICE))?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(Fault::Io(error.kind())),
            }
        }
        Ok(())
    }

    /// Wait for the first byte of a frame. The wait ends on a byte, EOF, the stop flag
    /// or the caller's own deadline; the byte, once seen, is not consumed twice.
    fn first_prefix_byte(&mut self, idle: Idle<'_>) -> Result<u8, Fault> {
        if let Some(&byte) = self.first_bytes.first() {
            self.first_bytes.remove(0);
            return Ok(byte);
        }
        let mut byte = [0u8; 1];
        loop {
            if idle.stop.load(Ordering::Acquire) {
                return Err(Fault::Closed);
            }
            match self.stream.read(&mut byte) {
                Ok(0) => return Err(Fault::Closed),
                Ok(_) => return Ok(byte[0]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let wait = match idle.until {
                        None => SLICE,
                        Some(until) => remaining(until)
                            .ok_or(Fault::Timeout(Stage::Idle))?
                            .min(SLICE),
                    };
                    ready(&self.stream, PollFlags::POLLIN, wait)?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(Fault::Io(error.kind())),
            }
        }
    }

    /// One frame body. The announced length is checked before anything is allocated
    /// from it, and `budget` runs from the first prefix byte without renewal.
    pub(super) fn frame(
        &mut self,
        idle: Idle<'_>,
        budget: Duration,
    ) -> Result<(Vec<u8>, Instant), Fault> {
        let first = self.first_prefix_byte(idle)?;
        let first_prefix = Instant::now();
        #[cfg(test)]
        if let Some(seen) = &self.prefix_seen {
            let _ = seen.send(first_prefix);
        }
        let deadline = first_prefix + budget;
        let mut prefix = [0u8; PREFIX_BYTES];
        prefix[0] = first;
        self.fill(&mut prefix[1..], deadline, Stage::Frame)?;
        let length = crate::decode_length(prefix).map_err(Fault::Frame)?;
        let mut body = vec![0u8; length];
        self.fill(&mut body, deadline, Stage::Frame)?;
        Ok((body, first_prefix))
    }

    /// Read a request or reply head: bytes up to and including the blank line, at most
    /// `limit` of them. What arrived after the blank line is returned separately.
    pub(super) fn head(
        &mut self,
        limit: usize,
        deadline: Instant,
    ) -> Result<(Vec<u8>, Vec<u8>), Fault> {
        let mut bytes = std::mem::take(&mut self.first_bytes);
        let mut chunk = [0u8; 1024];
        loop {
            if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let rest = bytes.split_off(at + 4);
                return if bytes.len() > limit {
                    Err(Fault::Handshake(Refusal::Head))
                } else {
                    Ok((bytes, rest))
                };
            }
            if bytes.len() >= limit {
                return Err(Fault::Handshake(Refusal::Head));
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(Fault::Truncated),
                Ok(count) => bytes.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let left = remaining(deadline).ok_or(Fault::Timeout(Stage::Head))?;
                    ready(&self.stream, PollFlags::POLLIN, left.min(SLICE))?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(Fault::Io(error.kind())),
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct Writer {
    stream: UnixStream,
}
impl Writer {
    pub(super) fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self { stream })
    }

    pub(super) fn send_by(
        &mut self,
        mut bytes: &[u8],
        deadline: Instant,
        stage: Stage,
    ) -> Result<(), Fault> {
        while !bytes.is_empty() {
            remaining(deadline).ok_or(Fault::Timeout(stage))?;
            match self.stream.write(bytes) {
                Ok(0) => return Err(Fault::Io(io::ErrorKind::WriteZero)),
                Ok(count) => bytes = &bytes[count..],
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let left = remaining(deadline).ok_or(Fault::Timeout(stage))?;
                    ready(&self.stream, PollFlags::POLLOUT, left.min(SLICE))?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(Fault::Io(error.kind())),
            }
        }
        Ok(())
    }
}
