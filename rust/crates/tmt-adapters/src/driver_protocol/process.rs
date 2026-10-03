//! One call to a consented host or runtime driver (`contracts/driver-protocol-v1.md`).
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

use super::{Declaration, registry::DriverRecord};
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
    time::{Duration, Instant},
};
use tmt_driver_protocol::{
    Answer, CALL_ENV, DecodeError, DriverError, Grammar, LocationsRequest, LocationsResponse, Op,
    PROTOCOL, Request, RuntimeDeclaration, SUBCOMMAND, decode, decode_capabilities, decode_done,
    decode_runtime_capabilities,
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
                "Driver {driver} is not trusted until it is approved again: {reason}"
            ),
            Self::Process { driver, error } => write!(output, "Driver {driver}: {error}"),
            Self::Decode { driver, error } => write!(output, "Driver {driver}: {error}"),
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
pub struct DriverProcess<R, D = Grammar> {
    record: DriverRecord,
    grammar: D,
    runner: R,
}

impl<R: CommandRunner> DriverProcess<R> {
    pub fn open(record: DriverRecord, runner: R) -> Result<Self, CallError> {
        let grammar = record
            .capabilities
            .host()
            .ok_or_else(|| untrusted(&record, "not a host driver"))
            .and_then(|value| {
                Grammar::from_capabilities(value).map_err(|error| untrusted(&record, error))
            })?;
        Self::verified(record, grammar, runner)
    }
}

impl<R: CommandRunner> DriverProcess<R, RuntimeDeclaration> {
    pub fn open_runtime(record: DriverRecord, runner: R) -> Result<Self, CallError> {
        let Declaration::Runtime(value) = &record.capabilities else {
            return Err(untrusted(&record, "not a runtime driver"));
        };
        let declaration =
            RuntimeDeclaration::new(value).map_err(|error| untrusted(&record, error))?;
        Self::verified(record, declaration, runner)
    }

    /// Approval and setup share this admission boundary before using write targets.
    pub fn locations(
        &self,
        mut request: LocationsRequest,
        deadline: Instant,
    ) -> Result<LocationsResponse, CallError> {
        request
            .env
            .retain(|name, _| self.grammar.env().contains(name));
        let answer = self
            .call::<LocationsResponse>(&request, deadline)?
            .map_err(|error| untrusted(&self.record, error.message))?;
        if !answer.within(&request.home) {
            return Err(untrusted(
                &self.record,
                "skills must lie inside configDirs or this home's shared skills root",
            ));
        }
        Ok(answer)
    }
}

fn untrusted(record: &DriverRecord, reason: impl fmt::Display) -> CallError {
    CallError::Untrusted {
        driver: record.name.clone(),
        reason: reason.to_string(),
    }
}

impl<R: CommandRunner, D> DriverProcess<R, D> {
    fn verified(record: DriverRecord, grammar: D, runner: R) -> Result<Self, CallError> {
        if record.capabilities.name() != record.name {
            return Err(untrusted(
                &record,
                "its recorded name doesn't match its declaration",
            ));
        }
        if !executable_trust::unchanged(&record.path, &record.fingerprint) {
            return Err(untrusted(
                &record,
                "the executable changed or is no longer private",
            ));
        }
        verify_digest_once(&record.path, &record.digest)
            .map_err(|error| untrusted(&record, error))?;
        Ok(Self {
            record,
            grammar,
            runner,
        })
    }

    pub fn name(&self) -> &str {
        &self.record.name
    }

    pub fn grammar(&self) -> &D {
        &self.grammar
    }

    /// Whether the driver declared `op`; any other op answers `unsupported`.
    pub fn supports(&self, op: Op) -> bool {
        self.record
            .capabilities
            .ops()
            .iter()
            .any(|name| name == op.as_str())
    }

    /// Runs one operation: its checked answer, or the error the driver
    /// reported.
    pub fn call<T: Answer<Declaration = D>>(
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

/// How long approval waits for `capabilities`. The driver is still told the
/// protocol's deadline; this only keeps a busy machine (a cold start, a
/// loaded scheduler) from refusing a working driver at a one-off, consented
/// step. Calls at run time keep the protocol's bounds.
const APPROVAL_WAIT: Duration = Duration::from_secs(10);

/// A candidate driver's `capabilities`, before it is approved: nothing about
/// it is trusted yet, so this runs it once, under the operation's output
/// bound and [`APPROVAL_WAIT`].
pub fn probe(runner: &impl CommandRunner, executable: &Path) -> Result<Declaration, String> {
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
        Instant::now() + APPROVAL_WAIT.max(op.bounds().deadline),
    )
    .map_err(|error| format!("it did not answer capabilities: {error}"))?;
    // Peek only to select the strict, bounded decoder; never accept the peeked value.
    let value: serde_json::Value =
        serde_json::from_slice(&output).map_err(|error| error.to_string())?;
    match value
        .pointer("/ok/kind")
        .and_then(serde_json::Value::as_str)
    {
        Some("runtime") => {
            decode_runtime_capabilities(&output).map(|(value, _)| Declaration::Runtime(value))
        }
        _ => decode_capabilities(&output).map(|(value, _)| Declaration::Host(value)),
    }
    .map_err(|error| error.to_string())
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
