//! Complete exact-text acquisition before request storage is opened.
//! Explicit file input is not a filesystem confinement boundary.

use nix::{
    errno::Errno,
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
}

impl ResponseInputFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "RESPONSE_INPUT_INVALID",
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
        })
    }
}
#[derive(Debug)]
pub struct ResponseInputError {
    pub kind: ResponseInputFailure,
    cause: Option<io::Error>,
}

impl ResponseInputError {
    fn io(kind: ResponseInputFailure, cause: impl Into<io::Error>) -> Self {
        Self {
            kind,
            cause: Some(cause.into()),
        }
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
}

impl From<ResponseInputFailure> for ResponseInputError {
    fn from(kind: ResponseInputFailure) -> Self {
        Self { kind, cause: None }
    }
}

impl fmt::Display for ResponseInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
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

/// Reuse bounded EOF acquisition for wire envelopes.
/// The caller owns wire validation and maps errors without exposing payloads.
pub fn read_stdin_bounded(timeout: Duration, maximum: usize) -> Result<String, ResponseInputError> {
    read_stream_bounded(&io::stdin(), timeout, maximum)
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
    // This invocation exclusively owns stdin. Poll before each read; never
    // change the inherited open-file description, even during cancellation.
    read_until_eof(stream, deadline, maximum)
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
