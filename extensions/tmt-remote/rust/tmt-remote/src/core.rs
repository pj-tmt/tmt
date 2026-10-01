//! One-shot public subprocesses; no core dependencies, storage or PATH fallback.
use crate::error::RemoteError;
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use tmt_invoke::{Cleanup, DiscoveryError, FailureKind, InvokeError, Phase, Request};

pub const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

pub struct CoreClient {
    executable: PathBuf,
}
impl CoreClient {
    pub fn discover() -> Result<Self, RemoteError> {
        let executable = tmt_invoke::invoking_tmt().map_err(|error| match error {
            DiscoveryError::Missing => {
                failure("Run through tmt remote; TMT_EXECUTABLE is required.")
            }
            DiscoveryError::NotAbsolute(_) => failure("TMT_EXECUTABLE must be absolute."),
        })?;
        Ok(Self { executable })
    }
    pub fn at(executable: PathBuf) -> Result<Self, RemoteError> {
        if !executable.is_absolute() {
            return Err(failure("TMT_EXECUTABLE must be absolute."));
        }
        Ok(Self { executable })
    }
    pub fn capabilities(&self, stop: &AtomicBool) -> Result<Value, RemoteError> {
        self.call(
            &["api"],
            json!({"version":1,"operation":"capabilities","input":{}})
                .to_string()
                .as_bytes(),
            stop,
            Duration::from_secs(15),
            OUTPUT_LIMIT,
        )
    }
    pub fn agents(&self, stop: &AtomicBool) -> Result<Value, RemoteError> {
        self.call(
            &["list", "--json"],
            &[],
            stop,
            Duration::from_secs(15),
            OUTPUT_LIMIT,
        )
    }
    fn call(
        &self,
        argv: &[&str],
        input: &[u8],
        stop: &AtomicBool,
        timeout: Duration,
        limit: usize,
    ) -> Result<Value, RemoteError> {
        let args: Vec<OsString> = argv.iter().map(OsString::from).collect();
        let output = tmt_invoke::invoke(
            Request {
                program: &self.executable,
                args: &args,
                input,
                deadline: Instant::now() + timeout,
                max_stream_bytes: limit,
            },
            Some(stop),
        )
        .map_err(invocation_error)?;
        let success = output.status.success();
        let bytes = output.stdout;
        let document: Value =
            serde_json::from_slice(&bytes).map_err(|_| failure("Core returned invalid JSON."))?;
        if success {
            return Ok(document);
        }
        match (
            document["error"]["code"].as_str(),
            document["error"]["message"].as_str(),
        ) {
            (Some(code), Some(message)) => Err(RemoteError::new(code, message)),
            _ => Err(failure("Core failed without a structured error.")),
        }
    }
}
fn failure(message: &str) -> RemoteError {
    RemoteError::new("REMOTE_CORE_UNAVAILABLE", message)
}
fn invocation_error(error: InvokeError) -> RemoteError {
    if matches!(error.cleanup, Cleanup::Unconfirmed(_)) {
        return failure("Core cleanup could not be confirmed; outcome is unknown.");
    }
    match error.kind {
        FailureKind::Spawn => failure("Could not start the supplied tmt executable."),
        FailureKind::Interrupted => failure(if matches!(error.cleanup, Cleanup::NotStarted) {
            "Core observation interrupted."
        } else {
            "Core observation interrupted; outcome is unknown."
        }),
        FailureKind::Deadline => failure("Core timed out; outcome is unknown."),
        FailureKind::OutputLimit(_) | FailureKind::Io(Phase::Communicate) => {
            failure("Core output could not be read within its bound.")
        }
        FailureKind::Io(Phase::OpenPipes | Phase::Wait) => {
            RemoteError::new("REMOTE_IO", "Remote I/O failed; the door is closed.")
        }
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
