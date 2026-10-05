use std::{fmt, io};

#[derive(Debug, PartialEq, Eq)]
pub struct RemoteError {
    pub code: String,
    pub message: String,
    /// The next step, when there is one. Human output prints it as its own `hint:` line.
    pub hint: Option<String>,
    /// Active session cap for a signed eviction refusal.
    pub limit: Option<usize>,
}
impl RemoteError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            limit: None,
        }
    }
    pub fn with_hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.into());
        self
    }
    /// The one-string `message` of JSON output, which has no hint field.
    pub fn json_message(&self) -> String {
        match &self.hint {
            Some(hint) => format!("{}: {hint}", self.message),
            None => self.message.clone(),
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
