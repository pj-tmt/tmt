//! Bounded newline framing on invocation-owned descriptors, without workers.
use nix::{
    errno::Errno,
    poll::{PollFd, PollFlags, PollTimeout, poll},
    unistd::{isatty, read, write},
};
use std::{
    io,
    os::fd::AsFd,
    time::{Duration, Instant},
};
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Refuse terminals before a protocol handler can perform effects.
pub fn require_redirected(input: &impl AsFd, output: &impl AsFd) -> io::Result<()> {
    if isatty(input).unwrap_or(false) || isatty(output).unwrap_or(false) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Protocol requires redirected stdio",
        ));
    }
    Ok(())
}

fn wait(
    fd: &impl AsFd,
    flags: PollFlags,
    deadline: Option<Instant>,
    now: &impl Fn() -> Instant,
) -> io::Result<()> {
    loop {
        let timeout = match deadline {
            Some(deadline) => {
                let remaining = deadline
                    .checked_duration_since(now())
                    .filter(|d| !d.is_zero())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "stdio frame deadline exceeded")
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
        if deadline.is_some_and(|d| now() >= d) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "stdio frame deadline exceeded",
            ));
        }
        let ready = events[0]
            .revents()
            .ok_or_else(|| io::Error::other("Invalid stdio descriptor"))?;
        if ready.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
            return Err(io::Error::other("stdio descriptor failed"));
        }
        if ready.intersects(flags | PollFlags::POLLHUP) {
            return Ok(());
        }
    }
}

pub struct Reader<'a, F, C = fn() -> Instant> {
    fd: &'a F,
    maximum: usize,
    bytes: Vec<u8>,
    searched: usize,
    deadline: Option<Instant>,
    now: C,
}
impl<'a, F: AsFd> Reader<'a, F> {
    pub fn new(fd: &'a F, maximum: usize) -> Self {
        Self::with_clock(fd, maximum, Instant::now)
    }
}
impl<'a, F: AsFd, C: Fn() -> Instant> Reader<'a, F, C> {
    fn with_clock(fd: &'a F, maximum: usize, now: C) -> Self {
        Self {
            fd,
            maximum,
            bytes: Vec::new(),
            searched: 0,
            deadline: None,
            now,
        }
    }
    pub fn read_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(end) = self.bytes[self.searched..]
                .iter()
                .position(|b| *b == b'\n')
                .map(|end| self.searched + end)
            {
                if end > self.maximum {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "stdio input exceeds its bound",
                    ));
                }
                let line: Vec<u8> = self.bytes.drain(..=end).collect();
                self.searched = 0;
                // Buffered bytes of the next frame start their own interval
                // when reading resumes; handler time is not acquisition time.
                self.deadline = None;
                return Ok(Some(line));
            }
            self.searched = self.bytes.len();
            if self.bytes.len() > self.maximum {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stdio input exceeds its bound",
                ));
            }
            if self.deadline.is_none() && !self.bytes.is_empty() {
                self.deadline = Some((self.now)() + IO_TIMEOUT);
            }
            wait(self.fd, PollFlags::POLLIN, self.deadline, &self.now)?;
            let mut chunk = [0; 8192];
            let count = match read(self.fd, &mut chunk) {
                Err(Errno::EINTR | Errno::EAGAIN) => continue,
                Err(error) => return Err(error.into()),
                Ok(count) => count,
            };
            if self.deadline.is_some_and(|d| (self.now)() >= d) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stdio frame deadline exceeded",
                ));
            }
            if count == 0 {
                return if self.bytes.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Incomplete stdio frame",
                    ))
                };
            }
            self.bytes.extend_from_slice(&chunk[..count]);
        }
    }
}

pub fn publish(fd: &impl AsFd, body: &[u8], maximum: usize) -> io::Result<()> {
    if body.len() > maximum || body.contains(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid stdio output frame",
        ));
    }
    let mut bytes = body.to_vec();
    bytes.push(b'\n');
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut offset = 0;
    while offset < bytes.len() {
        wait(fd, PollFlags::POLLOUT, Some(deadline), &Instant::now)?;
        // One output writer; pipe writes stay below POSIX's minimum PIPE_BUF.
        let count = match write(fd, &bytes[offset..(offset + 512).min(bytes.len())]) {
            Err(Errno::EINTR | Errno::EAGAIN) => continue,
            Err(error) => return Err(error.into()),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "stdio output closed",
                ));
            }
            Ok(count) => count,
        };
        offset += count;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
