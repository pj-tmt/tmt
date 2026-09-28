//! Complete exact-text acquisition before request storage is opened.
//! Explicit file input is not a filesystem confinement boundary.

use nix::{
    errno::Errno,
    fcntl::{FcntlArg, OFlag, fcntl},
    poll::{PollFd, PollFlags, PollTimeout, poll},
    sys::stat::{SFlag, fstat},
    unistd::{isatty, read},
};
use std::{
    error::Error,
    fmt, io,
    os::fd::AsFd,
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::exact_text::{ExactTextError, MAX_EXCHANGE_TEXT_BYTES, validate_exact_text};

const STDIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseInputFailure {
    Invalid,
    TooLarge,
    Timeout,
    File,
    Restore,
}

impl ResponseInputFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid | Self::Restore => "RESPONSE_INPUT_INVALID",
            Self::TooLarge => "RESPONSE_INPUT_TOO_LARGE",
            Self::Timeout => "RESPONSE_INPUT_TIMEOUT",
            Self::File => "RESPONSE_FILE_ERROR",
        }
    }
}

impl fmt::Display for ResponseInputFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "Response input must be complete valid UTF-8 and cannot be a TTY.",
            Self::TooLarge => "Response body must not exceed 1048576 UTF-8 bytes.",
            Self::Timeout => "Timed out while reading response input.",
            Self::File => "Could not read response input file.",
            Self::Restore => "Could not restore response input descriptor flags.",
        })
    }
}
#[derive(Debug)]
pub struct ResponseInputError {
    pub kind: ResponseInputFailure,
    pub cleanup_error: Option<io::Error>,
    cause: Option<io::Error>,
}

impl ResponseInputError {
    fn io(kind: ResponseInputFailure, cause: impl Into<io::Error>) -> Self {
        Self {
            kind,
            cause: Some(cause.into()),
            cleanup_error: None,
        }
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
}

impl From<ResponseInputFailure> for ResponseInputError {
    fn from(kind: ResponseInputFailure) -> Self {
        Self {
            kind,
            cause: None,
            cleanup_error: None,
        }
    }
}

impl fmt::Display for ResponseInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)?;
        if self.cleanup_error.is_some() {
            f.write_str(" Input cleanup also failed.")?;
        }
        Ok(())
    }
}

impl Error for ResponseInputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as &dyn Error)
    }
}

fn decode(bytes: &[u8]) -> Result<String, ResponseInputError> {
    // Acquisition rejects overflow before decoding, including invalid trailing
    // bytes. The shared core validator owns exact UTF-8 and body semantics.
    if bytes.len() > MAX_EXCHANGE_TEXT_BYTES {
        return Err(ResponseInputFailure::TooLarge.into());
    }
    validate_exact_text(bytes)
        .map(str::to_owned)
        .map_err(|error| match error {
            ExactTextError::InvalidUtf8 => ResponseInputFailure::Invalid.into(),
            ExactTextError::TooLarge => ResponseInputFailure::TooLarge.into(),
        })
}

pub fn read_file(path: &Path) -> Result<String, ResponseInputError> {
    let bytes =
        crate::bounded_file::read(path, MAX_EXCHANGE_TEXT_BYTES).map_err(|error| match error {
            crate::bounded_file::FileReadError::TooLarge => ResponseInputFailure::TooLarge.into(),
            crate::bounded_file::FileReadError::Io(cause) => {
                ResponseInputError::io(ResponseInputFailure::File, cause)
            }
        })?;
    decode(&bytes)
}

pub fn read_stdin() -> Result<String, ResponseInputError> {
    read_stream(&io::stdin(), STDIN_TIMEOUT)
}

/// Reuse descriptor restoration and bounded EOF acquisition for wire envelopes.
/// The caller owns wire validation and maps errors without exposing payloads.
pub fn read_stdin_bounded(timeout: Duration, maximum: usize) -> Result<String, ResponseInputError> {
    read_stream_bounded(&io::stdin(), timeout, maximum)
}

// fcntl operates on the inherited open-file description. Restore its exact
// previous flags before reporting success or failure; never leave a shell pipe
// nonblocking. The caller must not concurrently read this invocation's stdin.
struct StreamFlags<'a, F: AsFd> {
    stream: &'a F,
    previous: OFlag,
    restored: bool,
}

impl<'a, F: AsFd> StreamFlags<'a, F> {
    fn acquire(stream: &'a F) -> Result<Self, ResponseInputError> {
        let previous = OFlag::from_bits_retain(
            fcntl(stream, FcntlArg::F_GETFL)
                .map_err(|cause| ResponseInputError::io(ResponseInputFailure::Invalid, cause))?,
        );
        fcntl(stream, FcntlArg::F_SETFL(previous | OFlag::O_NONBLOCK))
            .map_err(|cause| ResponseInputError::io(ResponseInputFailure::Invalid, cause))?;
        Ok(Self {
            stream,
            previous,
            restored: false,
        })
    }

    fn restore(&mut self) -> Result<(), ResponseInputError> {
        fcntl(self.stream, FcntlArg::F_SETFL(self.previous))
            .map_err(|cause| ResponseInputError::io(ResponseInputFailure::Restore, cause))?;
        self.restored = true;
        Ok(())
    }
}

impl<F: AsFd> Drop for StreamFlags<'_, F> {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.restore();
        }
    }
}

fn read_stream(stream: &impl AsFd, timeout: Duration) -> Result<String, ResponseInputError> {
    read_stream_bounded(stream, timeout, MAX_EXCHANGE_TEXT_BYTES)
}

fn read_stream_bounded(
    stream: &impl AsFd,
    timeout: Duration,
    maximum: usize,
) -> Result<String, ResponseInputError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(ResponseInputFailure::Invalid)?;
    let terminal = match isatty(stream) {
        Ok(terminal) => terminal,
        // Darwin returns EOPNOTSUPP for a socket-backed child stdin (including
        // Node's pipe launcher). Confirm the descriptor kind instead of treating
        // arbitrary terminal-probe failures as usable input.
        Err(Errno::EOPNOTSUPP) => {
            let metadata = fstat(stream)
                .map_err(|cause| ResponseInputError::io(ResponseInputFailure::Invalid, cause))?;
            if SFlag::from_bits_truncate(metadata.st_mode) & SFlag::S_IFMT != SFlag::S_IFSOCK {
                return Err(ResponseInputFailure::Invalid.into());
            }
            false
        }
        Err(cause) => return Err(ResponseInputError::io(ResponseInputFailure::Invalid, cause)),
    };
    if timeout.is_zero() || terminal || maximum == 0 || maximum.checked_add(1).is_none() {
        return Err(ResponseInputFailure::Invalid.into());
    }
    let mut flags = StreamFlags::acquire(stream)?;
    let pending = read_until_eof(stream, deadline, maximum);
    let restored = flags.restore();
    match pending {
        Err(mut primary) => {
            primary.cleanup_error = restored.err().and_then(|error| error.cause);
            Err(primary)
        }
        Ok(body) => restored.map(|()| body),
    }
}

fn read_until_eof(
    stream: &impl AsFd,
    deadline: Instant,
    maximum: usize,
) -> Result<String, ResponseInputError> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|time| !time.is_zero())
            .ok_or(ResponseInputFailure::Timeout)?;
        // Round up to the next millisecond to avoid busy-polling the final
        // fraction. The monotonic check after every poll/read decides expiry.
        let timeout =
            PollTimeout::try_from(remaining.as_millis().saturating_add(1)).map_err(|cause| {
                ResponseInputError::io(ResponseInputFailure::Invalid, io::Error::other(cause))
            })?;
        let mut events = [PollFd::new(stream.as_fd(), PollFlags::POLLIN)];
        match poll(&mut events, timeout) {
            Err(Errno::EINTR) => continue,
            Err(cause) => return Err(ResponseInputError::io(ResponseInputFailure::Invalid, cause)),
            Ok(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(ResponseInputFailure::Timeout.into());
        }
        let ready = events[0].revents().ok_or(ResponseInputFailure::Invalid)?;
        if ready.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
            return Err(ResponseInputFailure::Invalid.into());
        }
        if !ready.intersects(PollFlags::POLLIN | PollFlags::POLLHUP) {
            continue;
        }
        let capacity = chunk.len().min(maximum + 1 - bytes.len());
        let count = match read(stream, &mut chunk[..capacity]) {
            Ok(count) => count,
            Err(Errno::EINTR | Errno::EAGAIN) => continue,
            Err(cause) => return Err(ResponseInputError::io(ResponseInputFailure::Invalid, cause)),
        };
        if Instant::now() >= deadline {
            return Err(ResponseInputFailure::Timeout.into());
        }
        if count == 0 {
            // The caller owns the wire limit; an escaped JSON envelope can be
            // larger than its separately validated canonical message body.
            return String::from_utf8(bytes).map_err(|_| ResponseInputFailure::Invalid.into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > maximum {
            return Err(ResponseInputFailure::TooLarge.into());
        }
    }
}

#[cfg(test)]
mod tests;
