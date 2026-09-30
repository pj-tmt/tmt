//! Neutral executable discovery and one bounded, waited Unix byte capture.
//! Callers own command selection and response interpretation.

mod discovery;
mod process;

pub use discovery::{DiscoveryError, find_executable, invoking_tmt, is_executable};
use std::{ffi::OsString, fmt, io, path::Path, sync::atomic::AtomicBool, time::Instant};

#[derive(Debug)]
pub struct Request<'a> {
    pub program: &'a Path,
    pub args: &'a [OsString],
    pub input: &'a [u8],
    pub deadline: Instant,
    /// Independent bound for each captured stream; zero permits no output.
    pub max_stream_bytes: usize,
}

#[derive(Debug)]
pub struct ExitStatus {
    pub code: Option<u32>,
    pub signal: Option<i32>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == Some(0) && self.signal.is_none()
    }
}

#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    OpenPipes,
    Communicate,
    Wait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Spawn,
    Deadline,
    Interrupted,
    OutputLimit(Stream),
    Io(Phase),
}

#[derive(Debug)]
pub enum Cleanup {
    NotStarted,
    Confirmed,
    Unconfirmed(io::Error),
}

#[derive(Debug)]
pub struct InvokeError {
    pub kind: FailureKind,
    pub cause: Option<io::Error>,
    pub cleanup: Cleanup,
}

impl fmt::Display for InvokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invocation failed: {:?}", self.kind)?;
        if matches!(self.cleanup, Cleanup::Unconfirmed(_)) {
            write!(f, " (cleanup unconfirmed; outcome unknown)")?;
        }
        Ok(())
    }
}

impl std::error::Error for InvokeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|cause| cause as _)
    }
}

/// Capture one child in a fresh owned process group. Nonzero exits are data.
/// A started failure terminates the group with a one-second cleanup budget;
/// termination does not establish rollback and never causes an automatic retry.
pub fn invoke(request: Request<'_>, stop: Option<&AtomicBool>) -> Result<Output, InvokeError> {
    process::invoke(request, stop)
}

#[cfg(test)]
mod tests;
