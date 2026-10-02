//! One call to a consented host driver (`contracts/driver-protocol-v1.md`).
//!
//! Before every call the executable must still be the approved one: owned by
//! the user, writable by no one else, and with the approved metadata
//! fingerprint (a stat, about 0.01 ms). Its content digest is checked once per
//! process, on first use: hashing a 20 MB driver takes about 9 ms, too much
//! for each of the dozen calls in one delivery (#570). A driver that fails
//! either check is not run until it is approved again.
//!
//! Each call runs under the operation's deadline and output bound, capped by
//! the caller's own deadline, with `TMT_DRIVER_CALL=1` in its environment. The
//! answer is decoded and checked against the driver's declared grammar before
//! anything else sees it.

use super::registry::DriverRecord;
use crate::{
    executable_trust,
    process::{CommandError, CommandRequest, CommandRunner},
};
use serde::Serialize;
use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};
use tmt_driver_protocol::{
    Answer, CALL_ENV, Capabilities, DecodeError, DriverError, Grammar, Op, PROTOCOL, Request,
    SUBCOMMAND, decode, decode_capabilities, decode_done,
};

const ENV: &str = "/usr/bin/env";

/// Executables whose digest this process has verified, by path and digest.
static VERIFIED: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());

#[derive(Debug)]
pub enum CallError {
    /// The executable changed since approval, or can't be read.
    Untrusted { driver: String, reason: String },
    /// The driver didn't finish in time, printed too much, or failed to run.
    Process { driver: String, error: CommandError },
    /// The driver's answer broke the protocol.
    Decode { driver: String, error: DecodeError },
}

impl fmt::Display for CallError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Untrusted { driver, reason } => write!(
                output,
                "Host driver {driver} is not trusted until it is approved again: {reason}"
            ),
            Self::Process { driver, error } => write!(output, "Host driver {driver}: {error}"),
            Self::Decode { driver, error } => write!(output, "Host driver {driver}: {error}"),
        }
    }
}

impl std::error::Error for CallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Process { error, .. } => Some(error),
            Self::Decode { error, .. } => Some(error),
            Self::Untrusted { .. } => None,
        }
    }
}

/// A consented driver, ready to call.
pub struct DriverProcess<R> {
    record: DriverRecord,
    grammar: Grammar,
    runner: R,
}

impl<R: CommandRunner> DriverProcess<R> {
    /// Checks the record's declaration and the executable, including its
    /// digest the first time this process opens it.
    pub fn open(record: DriverRecord, runner: R) -> Result<Self, CallError> {
        let untrusted = |reason: String| CallError::Untrusted {
            driver: record.name.clone(),
            reason,
        };
        let grammar = Grammar::from_capabilities(&record.capabilities)
            .map_err(|error| untrusted(format!("its recorded declaration is invalid: {error}")))?;
        if grammar.name() != record.name {
            return Err(untrusted(
                "its recorded name doesn't match its declaration".into(),
            ));
        }
        verify_digest_once(&record.path, &record.digest).map_err(untrusted)?;
        Ok(Self {
            record,
            grammar,
            runner,
        })
    }

    pub fn name(&self) -> &str {
        &self.record.name
    }

    pub fn grammar(&self) -> &Grammar {
        &self.grammar
    }

    /// Whether the driver declared `op`; any other op answers `unsupported`.
    pub fn supports(&self, op: Op) -> bool {
        self.record
            .capabilities
            .ops
            .iter()
            .any(|name| name == op.as_str())
    }

    /// Runs one operation: its checked answer, or the error the driver
    /// reported.
    pub fn call<T: Answer>(
        &self,
        body: impl Serialize,
        deadline: Instant,
    ) -> Result<Result<T, DriverError>, CallError> {
        let output = self.run(T::OP, body, deadline)?;
        decode(&self.grammar, &output).map_err(|error| self.decode_error(error))
    }

    /// Runs `publish`, `input` or `focus`, whose answer is `{}`.
    pub fn call_done(
        &self,
        op: Op,
        body: impl Serialize,
        deadline: Instant,
    ) -> Result<Result<(), DriverError>, CallError> {
        let output = self.run(op, body, deadline)?;
        decode_done(op, &output).map_err(|error| self.decode_error(error))
    }

    fn decode_error(&self, error: DecodeError) -> CallError {
        CallError::Decode {
            driver: self.record.name.clone(),
            error,
        }
    }

    fn run(&self, op: Op, body: impl Serialize, deadline: Instant) -> Result<Vec<u8>, CallError> {
        if !executable_trust::unchanged(&self.record.path, &self.record.fingerprint) {
            return Err(CallError::Untrusted {
                driver: self.record.name.clone(),
                reason: "the executable changed or is no longer private".into(),
            });
        }
        let now = Instant::now();
        let deadline = deadline.min(now + op.bounds().deadline);
        let request = serde_json::to_vec(&Request {
            deadline_ms: deadline.saturating_duration_since(now).as_millis() as u64,
            body,
        })
        .expect("a driver request serializes");
        invoke(&self.runner, &self.record.path, op, &request, deadline).map_err(|error| {
            CallError::Process {
                driver: self.record.name.clone(),
                error,
            }
        })
    }
}

/// A candidate driver's `capabilities`, before it is approved: nothing about
/// it is trusted yet, so this runs it once under the operation's bounds.
pub fn probe(
    runner: &impl CommandRunner,
    executable: &Path,
) -> Result<(Capabilities, Grammar), String> {
    let op = Op::Capabilities;
    let request = serde_json::to_vec(&Request {
        deadline_ms: op.bounds().deadline.as_millis() as u64,
        body: tmt_driver_protocol::Empty {},
    })
    .expect("a driver request serializes");
    let output = invoke(
        runner,
        executable,
        op,
        &request,
        Instant::now() + op.bounds().deadline,
    )
    .map_err(|error| format!("it did not answer capabilities: {error}"))?;
    decode_capabilities(&output).map_err(|error| error.to_string())
}

fn invoke(
    runner: &impl CommandRunner,
    executable: &Path,
    op: Op,
    request: &[u8],
    deadline: Instant,
) -> Result<Vec<u8>, CommandError> {
    // `/usr/bin/env` only adds the guard to core's environment. If
    // `CommandRequest` gains an environment field, set the guard there and
    // run the driver directly.
    let mut marker = OsString::from(CALL_ENV);
    marker.push("=1");
    let args = [
        marker,
        executable.as_os_str().to_owned(),
        SUBCOMMAND.into(),
        PROTOCOL.to_string().into(),
        op.as_str().into(),
    ];
    runner
        .execute(CommandRequest {
            program: OsStr::new(ENV),
            args: &args,
            input: request,
            deadline,
            max_output_bytes: op.bounds().max_output_bytes,
        })
        .map(|output| output.stdout)
}

fn verify_digest_once(path: &Path, digest: &str) -> Result<(), String> {
    let mut verified = VERIFIED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if verified
        .iter()
        .any(|(known, known_digest)| known == path && known_digest == digest)
    {
        return Ok(());
    }
    match executable_trust::digest(path) {
        Ok(actual) if actual == digest => {
            verified.push((path.to_owned(), digest.to_owned()));
            Ok(())
        }
        Ok(_) => Err("its content changed".into()),
        Err(error) => Err(format!("it can't be read: {error}")),
    }
}
