//! Pane IO only: callers own routing, endpoint verification and request state.

use super::valid_pane_id;
use super::{CommandRunner, OPERATION_TIMEOUT, Tmux, TmuxError, TmuxFailure, socket_args};
use crate::host::{DeliveryError, DeliveryStage};
use std::time::{Duration, Instant};
use tmt_core::limits::is_valid_capture_lines;

const SEND_MAX_OUTPUT: usize = 64 * 1024;
const CAPTURE_MAX_OUTPUT: usize = 4 * 1024 * 1024;

fn validate_target(socket: &str, pane: &str) -> Result<(), TmuxError> {
    if socket.is_empty() || socket.contains('\0') || !valid_pane_id(pane) {
        return Err(TmuxError::evidence("Invalid explicit transport endpoint"));
    }
    Ok(())
}

/// The caller has already applied core's pane input policy
/// (`tmt_core::driver::pane_input_text`); the paste only ends in a newline.
fn pasted_payload(message: &str) -> String {
    let mut payload = message.to_owned();
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
        let payload = pasted_payload(message);
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
                    return Err(DeliveryError::new(DeliveryStage::Paste, cause)
                        .with_cleanup_failed(
                            cleanup().err().is_some_and(|error| error.cleanup_failed()),
                        ));
                }
            }
            Err(cause) => {
                // Only set-buffer failure is safe to fall back from: no pane input was attempted.
                let cleanup_failed = cleanup().err().is_some_and(|error| error.cleanup_failed());
                if cause.cleanup_failed() || cleanup_failed {
                    return Err(DeliveryError::new(DeliveryStage::Prepare, cause)
                        .with_cleanup_failed(cleanup_failed));
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
