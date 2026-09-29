//! Pane IO only: callers own routing, endpoint verification and request state.

use super::valid_pane_id;
use super::{CommandRunner, OPERATION_TIMEOUT, Tmux, TmuxError, TmuxFailure, socket_args};
use std::{
    fmt,
    time::{Duration, Instant},
};
use tmt_core::limits::is_valid_capture_lines;

const SEND_MAX_OUTPUT: usize = 64 * 1024;
const CAPTURE_MAX_OUTPUT: usize = 4 * 1024 * 1024;

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
    cause: TmuxError,
    cleanup_error: Option<Box<TmuxError>>,
}

impl DeliveryError {
    fn new(stage: DeliveryStage, cause: TmuxError) -> Self {
        Self {
            stage,
            cause,
            cleanup_error: None,
        }
    }

    /// Refused before any pane input: the endpoint's host cannot take input
    /// yet (Herdr until #479 H4).
    pub(crate) fn unsupported() -> Self {
        Self::new(
            DeliveryStage::Prepare,
            TmuxError::evidence("This host cannot receive pane input yet"),
        )
    }

    pub fn uncertain(&self) -> bool {
        self.stage != DeliveryStage::Prepare
    }

    pub fn socket_permission_denied(&self) -> bool {
        !self.uncertain() && self.cause.socket_permission_denied()
    }

    pub fn cleanup_failed(&self) -> bool {
        self.cause.cleanup_failed()
            || self
                .cleanup_error
                .as_ref()
                .is_some_and(|error| error.cleanup_failed())
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

impl std::error::Error for DeliveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

fn validate_target(socket: &str, pane: &str) -> Result<(), TmuxError> {
    if socket.is_empty() || socket.contains('\0') || !valid_pane_id(pane) {
        return Err(TmuxError::evidence("Invalid explicit transport endpoint"));
    }
    Ok(())
}

fn protected_payload(message: &str) -> String {
    // Coding-agent shells can interpret ASCII ! as bash mode before input reaches the agent.
    let mut payload = message.replace('!', "\u{ff01}");
    if !payload.ends_with('\n') {
        payload.push('\n');
    }
    payload
}

impl<R: CommandRunner> Tmux<R> {
    fn transport_run(
        &self,
        socket: &str,
        args: Vec<String>,
        cap: usize,
    ) -> Result<String, TmuxError> {
        let mut scoped = socket_args(Some(socket));
        scoped.extend(args);
        self.run(
            "tmux",
            scoped,
            Instant::now() + OPERATION_TIMEOUT,
            cap,
            TmuxFailure::Command,
        )
    }

    /// Diagnostic terminal text, never a completion signal or a partial success.
    pub fn capture_on(&self, socket: &str, pane: &str, lines: u64) -> Result<String, TmuxError> {
        validate_target(socket, pane)?;
        if !is_valid_capture_lines(lines) {
            return Err(TmuxError::evidence("Invalid capture line count"));
        }
        self.transport_run(
            socket,
            vec![
                "capture-pane".into(),
                "-t".into(),
                pane.into(),
                "-p".into(),
                "-S".into(),
                format!("-{lines}"),
            ],
            CAPTURE_MAX_OUTPUT,
        )
    }

    /// Send once to the explicit endpoint; the caller owns the original prompt.
    pub fn send_on(
        &self,
        socket: &str,
        pane: &str,
        message: &str,
        enter_delay: Duration,
    ) -> Result<(), DeliveryError> {
        self.send_with_wait(socket, pane, message, enter_delay, std::thread::sleep)
    }

    fn send_with_wait(
        &self,
        socket: &str,
        pane: &str,
        message: &str,
        enter_delay: Duration,
        wait: impl FnOnce(Duration),
    ) -> Result<(), DeliveryError> {
        validate_target(socket, pane)
            .map_err(|cause| DeliveryError::new(DeliveryStage::Prepare, cause))?;
        if message.contains('\0') {
            return Err(DeliveryError::new(
                DeliveryStage::Prepare,
                TmuxError::evidence("Transport text cannot contain NUL"),
            ));
        }
        let payload = protected_payload(message);
        let buffer = format!("tmt-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        let run = |args| {
            self.transport_run(socket, args, SEND_MAX_OUTPUT)
                .map(|_| ())
        };
        let cleanup = || run(vec!["delete-buffer".into(), "-b".into(), buffer.clone()]);
        match run(vec![
            "set-buffer".into(),
            "-b".into(),
            buffer.clone(),
            "--".into(),
            payload.clone(),
        ]) {
            Ok(()) => {
                if let Err(cause) = run(vec![
                    "paste-buffer".into(),
                    "-b".into(),
                    buffer.clone(),
                    "-d".into(),
                    "-t".into(),
                    pane.into(),
                    "-p".into(),
                ]) {
                    let mut error = DeliveryError::new(DeliveryStage::Paste, cause);
                    error.cleanup_error = cleanup()
                        .err()
                        .filter(TmuxError::cleanup_failed)
                        .map(Box::new);
                    return Err(error);
                }
            }
            Err(cause) => {
                // Only set-buffer failure is safe to fall back from: no pane input was attempted.
                let cleanup_error = cleanup().err().filter(TmuxError::cleanup_failed);
                if cause.cleanup_failed() || cleanup_error.is_some() {
                    let mut error = DeliveryError::new(DeliveryStage::Prepare, cause);
                    error.cleanup_error = cleanup_error.map(Box::new);
                    return Err(error);
                }
                run(vec![
                    "send-keys".into(),
                    "-l".into(),
                    "-t".into(),
                    pane.into(),
                    "--".into(),
                    payload,
                ])
                .map_err(|cause| DeliveryError::new(DeliveryStage::Literal, cause))?;
            }
        }
        wait(enter_delay);
        run(vec![
            "send-keys".into(),
            "-t".into(),
            pane.into(),
            "Enter".into(),
        ])
        .map_err(|cause| DeliveryError::new(DeliveryStage::Submit, cause))
    }
}

#[cfg(test)]
mod tests;
