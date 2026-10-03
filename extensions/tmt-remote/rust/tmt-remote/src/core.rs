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
            crate::limits::CORE_CALL,
            OUTPUT_LIMIT,
        )
    }
    /// Absolute core data root; remote state and extension sockets live beneath it.
    pub fn storage_root(&self, stop: &AtomicBool) -> Result<PathBuf, RemoteError> {
        let reply = self.call(
            &["api"],
            json!({"version":1,"operation":"storage.root","input":{}})
                .to_string()
                .as_bytes(),
            stop,
            crate::limits::CORE_CALL,
            64 * 1024,
        )?;
        reply["dataRoot"]
            .as_str()
            .map(PathBuf::from)
            .filter(|root| root.is_absolute())
            .ok_or_else(|| failure("Core storage.root did not report an absolute dataRoot."))
    }
    pub fn agents(&self, stop: &AtomicBool) -> Result<Value, RemoteError> {
        self.call(
            &["list", "--json"],
            &[],
            stop,
            crate::limits::CORE_CALL,
            OUTPUT_LIMIT,
        )
    }
    pub(crate) fn check(
        &self,
        agent: &str,
        lines: Option<u64>,
        stop: &AtomicBool,
    ) -> Result<Value, RemoteError> {
        // UUID authority stays at admission; the public check command takes a name.
        // Identity inventory excludes retired rows and both reads share one budget.
        let deadline = Instant::now() + crate::limits::CORE_CALL;
        let directory = self.call_until(
            &["identity", "list", "--json"],
            &[],
            stop,
            deadline,
            OUTPUT_LIMIT,
        )?;
        let rows = directory["identities"]
            .as_array()
            .ok_or_else(|| failure("Core identity list returned invalid JSON."))?;
        let mut matching = rows.iter().filter(|row| row["id"] == agent);
        let row = matching
            .next()
            .filter(|_| matching.next().is_none())
            .ok_or_else(|| failure("Check agent is absent or ambiguous."))?;
        let name = row["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| failure("Check agent has no current name."))?;
        let canonical = row["canonicalName"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| failure("Check agent has no canonical name."))?;
        if rows
            .iter()
            .filter(|row| row["canonicalName"] == canonical)
            .count()
            != 1
        {
            return Err(failure("Check agent name is ambiguous."));
        }
        let count = lines.map(|value| value.to_string());
        let mut argv = vec!["check", name, "--json"];
        if let Some(count) = count.as_deref() {
            argv.extend(["--lines", count]);
        }
        self.call_until(&argv, &[], stop, deadline, OUTPUT_LIMIT)
    }
    pub(crate) fn api(&self, input: &[u8], stop: &AtomicBool) -> Result<Value, RemoteError> {
        self.call(
            &["api"],
            input,
            stop,
            crate::limits::CORE_CALL,
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
        self.call_until(argv, input, stop, Instant::now() + timeout, limit)
    }
    fn call_until(
        &self,
        argv: &[&str],
        input: &[u8],
        stop: &AtomicBool,
        deadline: Instant,
        limit: usize,
    ) -> Result<Value, RemoteError> {
        let args: Vec<OsString> = argv.iter().map(OsString::from).collect();
        let output = tmt_invoke::invoke(
            Request {
                program: &self.executable,
                args: &args,
                input,
                deadline,
                max_stream_bytes: limit,
                launch: Default::default(),
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
        return RemoteError::new(
            "REMOTE_CORE_UNCERTAIN",
            "Core cleanup could not be confirmed; outcome is unknown.",
        );
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
