//! Stateless job calendars. No store, policy, process or dispatch ownership.
mod schedule;
pub use schedule::{Schedule, ScheduleInput};

#[derive(Debug)]
pub struct Error {
    pub message: String,
}
impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
