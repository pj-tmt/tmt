use std::{fmt, io};

#[derive(Debug, PartialEq, Eq)]
pub struct RemoteError {
    pub code: String,
    pub message: String,
}
impl RemoteError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}
impl std::error::Error for RemoteError {}
impl From<io::Error> for RemoteError {
    fn from(_: io::Error) -> Self {
        Self::new("REMOTE_IO", "Remote I/O failed; the door is closed.")
    }
}
