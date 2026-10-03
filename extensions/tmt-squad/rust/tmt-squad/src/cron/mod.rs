//! Time-based Squad jobs, independent of core storage and execution results.
mod schedule;
mod store;

pub use schedule::{Schedule, ScheduleInput};
pub use store::{Job, Jobs, Pause, Store};

/// Expected input/persistence failures, mapped to CLI output by the caller.
#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
