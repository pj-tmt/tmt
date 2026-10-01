//! Pane input outcomes, the same for every host: whether any text may have
//! reached the pane, and the stage that failed.

use std::{error::Error, fmt};

/// What a host's own input error says about itself.
pub(crate) trait DeliveryCause: Error + Send + Sync + 'static {
    /// The host's socket refused this user before any pane input.
    fn socket_permission_denied(&self) -> bool {
        false
    }

    fn cleanup_failed(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStage {
    Prepare,
    Paste,
    Literal,
    Submit,
}

impl DeliveryStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Paste => "paste",
            Self::Literal => "literal",
            Self::Submit => "submit",
        }
    }
}

#[derive(Debug)]
pub struct DeliveryError {
    pub stage: DeliveryStage,
    cause: Box<dyn DeliveryCause>,
    /// Cleanup after the failed step (a tmux paste buffer) also failed.
    cleanup_failed: bool,
}

impl DeliveryError {
    pub(crate) fn new(stage: DeliveryStage, cause: impl DeliveryCause) -> Self {
        Self {
            stage,
            cause: Box::new(cause),
            cleanup_failed: false,
        }
    }

    /// Refused before any pane input: the endpoint's host cannot take input.
    pub(crate) fn unsupported() -> Self {
        Self::new(DeliveryStage::Prepare, Unsupported)
    }

    pub(crate) fn with_cleanup_failed(mut self, failed: bool) -> Self {
        self.cleanup_failed = failed;
        self
    }

    pub fn uncertain(&self) -> bool {
        self.stage != DeliveryStage::Prepare
    }

    pub fn socket_permission_denied(&self) -> bool {
        !self.uncertain() && self.cause.socket_permission_denied()
    }

    #[cfg(test)]
    pub(crate) fn cause<T: Error + 'static>(&self) -> &T {
        let cause: &(dyn Error + 'static) = self.cause.as_ref();
        cause.downcast_ref().expect("the host's own cause")
    }

    /// Only the cleanup after the failed step, not the step's own cause.
    #[cfg(test)]
    pub(crate) fn cleanup_step_failed(&self) -> bool {
        self.cleanup_failed
    }

    pub fn cleanup_failed(&self) -> bool {
        self.cleanup_failed || self.cause.cleanup_failed()
    }
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.uncertain() {
            write!(
                output,
                "Message delivery is uncertain during {}.",
                self.stage.as_str()
            )?;
        } else {
            write!(output, "Message preparation failed before pane input.")?;
        }
        if self.cleanup_failed() {
            write!(output, " Subprocess cleanup also failed.")?;
        }
        Ok(())
    }
}

impl Error for DeliveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

#[derive(Debug)]
struct Unsupported;

impl fmt::Display for Unsupported {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("This host cannot receive pane input yet.")
    }
}

impl Error for Unsupported {}

impl DeliveryCause for Unsupported {
    fn cleanup_failed(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_without_input_refuses_before_any_pane_input() {
        let error = DeliveryError::unsupported();
        assert!(!error.uncertain());
        assert!(!error.cleanup_failed());
        assert!(!error.socket_permission_denied());
        assert_eq!(error.stage.as_str(), "prepare");
        assert_eq!(
            error.to_string(),
            "Message preparation failed before pane input."
        );
        assert_eq!(
            error.source().unwrap().to_string(),
            "This host cannot receive pane input yet."
        );
    }
}
