//! Bounded newline framing on invocation-owned descriptors, without workers.

use super::{INPUT_LIMIT, OUTPUT_LIMIT, Session, ToolCall};
use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
    unistd::{isatty, read, write},
};
use serde_json::Value;
use std::{
    io::{self, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};

const IO_TIMEOUT: Duration = Duration::from_secs(5);

fn wait(fd: &impl AsFd, flags: PollFlags, deadline: Option<Instant>) -> io::Result<()> {
    loop {
        let timeout = match deadline {
            Some(deadline) => {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .filter(|d| !d.is_zero())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "MCP frame deadline exceeded")
                    })?;
                PollTimeout::try_from(remaining.as_millis().saturating_add(1))
                    .map_err(io::Error::other)?
            }
            None => PollTimeout::NONE,
        };
        let mut events = [PollFd::new(fd.as_fd(), flags)];
        match poll(&mut events, timeout) {
            Err(Errno::EINTR) => continue,
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "MCP frame deadline exceeded",
            ));
        }
        let ready = events[0]
            .revents()
            .ok_or_else(|| io::Error::other("Invalid MCP descriptor"))?;
        if ready.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
            return Err(io::Error::other("MCP descriptor failed"));
        }
        if ready.intersects(flags | PollFlags::POLLHUP) {
            return Ok(());
        }
    }
}

struct Reader<'a, F> {
    fd: &'a F,
    bytes: Vec<u8>,
    searched: usize,
    deadline: Option<Instant>,
}
impl<F: AsFd> Reader<'_, F> {
    fn next(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(end) = self.bytes[self.searched..]
                .iter()
                .position(|b| *b == b'\n')
                .map(|end| self.searched + end)
            {
                if end > INPUT_LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "MCP input exceeds its bound",
                    ));
                }
                let line: Vec<u8> = self.bytes.drain(..=end).collect();
                self.searched = 0;
                self.deadline = if self.bytes.is_empty() {
                    None
                } else {
                    self.deadline
                };
                return Ok(Some(line));
            }
            self.searched = self.bytes.len();
            if self.bytes.len() > INPUT_LIMIT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "MCP input exceeds its bound",
                ));
            }
            wait(self.fd, PollFlags::POLLIN, self.deadline)?;
            let mut chunk = [0; 8192];
            let count = match read(self.fd, &mut chunk) {
                Err(Errno::EINTR | Errno::EAGAIN) => continue,
                Err(error) => return Err(error.into()),
                Ok(count) => count,
            };
            if self.deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "MCP frame deadline exceeded",
                ));
            }
            if count == 0 {
                return if self.bytes.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Incomplete MCP frame",
                    ))
                };
            }
            if self.deadline.is_none() {
                self.deadline = Some(Instant::now() + IO_TIMEOUT);
            }
            self.bytes.extend_from_slice(&chunk[..count]);
        }
    }
}

struct Encoded {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Encoded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP output exceeds its bound",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode(value: &Value, limit: usize) -> io::Result<Vec<u8>> {
    let mut output = Encoded {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, value).map_err(io::Error::other)?;
    Ok(output.bytes)
}

fn publish(fd: &impl AsFd, value: &Value) -> io::Result<()> {
    let mut bytes = encode(value, OUTPUT_LIMIT - 1)?;
    bytes.push(b'\n');
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut offset = 0;
    while offset < bytes.len() {
        wait(fd, PollFlags::POLLOUT, Some(deadline))?;
        // One output writer; pipe writes stay below POSIX's minimum PIPE_BUF.
        let count = match write(fd, &bytes[offset..(offset + 512).min(bytes.len())]) {
            Err(Errno::EINTR | Errno::EAGAIN) => continue,
            Err(error) => return Err(error.into()),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "MCP output closed",
                ));
            }
            Ok(count) => count,
        };
        offset += count;
    }
    Ok(())
}

pub(super) fn serve(
    input: &impl AsFd,
    output: &impl AsFd,
    mut call: impl FnMut(ToolCall) -> Result<Value, Value>,
) -> io::Result<()> {
    if isatty(input).unwrap_or(false) || isatty(output).unwrap_or(false) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MCP requires redirected stdio",
        ));
    }
    let mut reader = Reader {
        fd: input,
        bytes: Vec::new(),
        searched: 0,
        deadline: None,
    };
    let mut session = Session::default();
    while let Some(line) = reader.next()? {
        if let Some(value) = session.receive(&line, &mut call) {
            publish(output, &value)?;
        }
    }
    Ok(())
}
