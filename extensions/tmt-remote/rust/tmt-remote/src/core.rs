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

#[derive(Clone)]
pub struct CoreClient {
    executable: PathBuf,
}
/// Atomic UUID/name observation through Core's public command boundary.
pub trait CloudIdentityQuery: Send + Sync {
    fn identity_show(
        &self,
        id: &str,
        expected_name: &str,
        stop: &AtomicBool,
    ) -> Result<(), RemoteError>;
}
/// Only a verified, route-bound inbox final reaches the ordinary receipt-bound reply.
pub trait CloudFinalDelivery: Send + Sync {
    fn deliver_final(
        &self,
        reply: &crate::cloud_command::Reply,
        response: &str,
        stop: &AtomicBool,
    ) -> Result<(), RemoteError>;
}
impl CloudFinalDelivery for CoreClient {
    fn deliver_final(
        &self,
        reply: &crate::cloud_command::Reply,
        response: &str,
        stop: &AtomicBool,
    ) -> Result<(), RemoteError> {
        let answer = self.call(
            &[
                "reply",
                &reply.request_id,
                "--receipt",
                &reply.receipt,
                "--stdin",
                "--json",
            ],
            response.as_bytes(),
            stop,
            crate::limits::CORE_CALL,
            4096,
        )?;
        if answer["status"] != "submitted"
            || answer["requestId"] != reply.request_id
            || answer["bodyBytes"].as_u64() != Some(response.len() as u64)
        {
            return Err(failure("Core reply returned invalid completion evidence."));
        }
        Ok(())
    }
}
impl CloudIdentityQuery for CoreClient {
    fn identity_show(
        &self,
        id: &str,
        expected_name: &str,
        stop: &AtomicBool,
    ) -> Result<(), RemoteError> {
        self.cloud_identity_show(id, expected_name, stop)
    }
}
impl CoreClient {
    /// No UUID inventory followed by a name-addressed capture is a substitute.
    pub fn cloud_identity_show(
        &self,
        id: &str,
        expected_name: &str,
        stop: &AtomicBool,
    ) -> Result<(), RemoteError> {
        self.identity_by_id_until(
            id,
            Some(expected_name),
            stop,
            Instant::now() + crate::limits::CORE_CALL,
        )
        .map(|_| ())
    }

    fn identity_by_id_until(
        &self,
        id: &str,
        expected_name: Option<&str>,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<crate::cloud_command::PeerIdentity, RemoteError> {
        // Equals-form values and the separator keep names/UUIDs out of option syntax.
        let expected = expected_name.map(|name| format!("--expect-name={name}"));
        let mut argv = vec!["identity", "show", "--json"];
        if let Some(expected) = expected.as_deref() {
            argv.push(expected);
        }
        argv.extend(["--", id]);
        let answer = self
            .call_until(&argv, &[], stop, deadline, OUTPUT_LIMIT)
            .map_err(agent_identity_error)?;
        let identity = &answer["identity"];
        let name = identity["name"].as_str().filter(|name| {
            !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
        });
        // Core owns normalization and the expected-name comparison. Its pinned query
        // reports presence; ordinary UUID resolution need not report it.
        if identity["id"].as_str() != Some(id)
            || !crate::canonical::is_core_id(id)
            || name.is_none()
            || identity["retired"] == true
            || (expected_name.is_some()
                && !matches!(
                    identity["presence"].as_str(),
                    Some("active" | "offline" | "unknown")
                ))
        {
            return Err(failure(
                "Core identity show returned invalid identity evidence.",
            ));
        }
        Ok(crate::cloud_command::PeerIdentity {
            id: id.to_owned(),
            name: name.unwrap().to_owned(),
        })
    }

    /// Core alone resolves its name syntax and excludes retired identities. The returned
    /// UUID/name pair is frozen before dispatch and checked by the UUID-pinned seam at effect.
    pub(crate) fn cloud_target(
        &self,
        name: &str,
        stop: &AtomicBool,
    ) -> Result<crate::cloud_command::PeerIdentity, RemoteError> {
        let answer = self
            .call(
                &["identity", "show", "--json", "--", name],
                &[],
                stop,
                crate::limits::CORE_CALL,
                OUTPUT_LIMIT,
            )
            .map_err(|error| match error.code.as_str() {
                "NAME_NOT_FOUND" | "INVALID_NAME" => RemoteError::new(
                    "REMOTE_AGENT_NOT_FOUND",
                    "The recipient name was not found.",
                ),
                "NAME_AMBIGUOUS" => {
                    RemoteError::new("REMOTE_AGENT_AMBIGUOUS", "The recipient name is ambiguous.")
                }
                _ => error,
            })?;
        let identity = &answer["identity"];
        // `identity show` also accepts UUID selectors. A name-only target must not
        // accidentally select a different display name through that shortcut.
        if crate::canonical::is_core_id(name) && identity["name"] != name {
            return Err(RemoteError::new(
                "REMOTE_AGENT_NOT_FOUND",
                "The recipient name was not found.",
            ));
        }
        let id = identity["id"]
            .as_str()
            .filter(|id| crate::canonical::is_core_id(id));
        let name = identity["name"].as_str().filter(|name| {
            !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
        });
        match (id, name) {
            (Some(id), Some(name)) if identity["retired"] != true => {
                Ok(crate::cloud_command::PeerIdentity {
                    id: id.into(),
                    name: name.into(),
                })
            }
            _ => Err(failure(
                "Core identity show returned invalid identity evidence.",
            )),
        }
    }

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
    /// Installed extension discovery uses this same supplied public executable.
    pub fn executable(&self) -> &std::path::Path {
        &self.executable
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
        self.storage_root_until(stop, Instant::now() + crate::limits::CORE_CALL)
    }
    /// Read-only root discovery clipped to the caller's absolute operation bound.
    pub fn storage_root_until(
        &self,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<PathBuf, RemoteError> {
        let reply = self.call_until(
            &["api"],
            json!({"version":1,"operation":"storage.root","input":{}})
                .to_string()
                .as_bytes(),
            stop,
            deadline.min(Instant::now() + crate::limits::CORE_CALL),
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
        self.check_until(
            agent,
            lines,
            stop,
            Instant::now() + crate::limits::CORE_CALL,
        )
    }

    fn check_until(
        &self,
        agent: &str,
        lines: Option<u64>,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<Value, RemoteError> {
        // Core selects this admitted UUID's binding directly; names grant no authority.
        if !crate::canonical::is_core_id(agent) {
            return Err(failure("Agent capture requires a canonical identity UUID."));
        }
        let count = lines.map(|value| value.to_string());
        let mut argv = vec!["check", "--capture-only", "--json"];
        if let Some(count) = count.as_deref() {
            argv.extend(["--lines", count]);
        }
        argv.extend(["--", agent]);
        self.call_until(&argv, &[], stop, deadline, OUTPUT_LIMIT)
            .map_err(agent_identity_error)
    }
    pub(crate) fn api(&self, input: &[u8], stop: &AtomicBool) -> Result<Value, RemoteError> {
        self.api_with_limit(input, stop, OUTPUT_LIMIT)
    }
    pub(crate) fn api_with_limit(
        &self,
        input: &[u8],
        stop: &AtomicBool,
        limit: usize,
    ) -> Result<Value, RemoteError> {
        self.call(&["api"], input, stop, crate::limits::CORE_CALL, limit)
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
fn agent_identity_error(error: RemoteError) -> RemoteError {
    match error.code.as_str() {
        "IDENTITY_CHANGED" => {
            RemoteError::new("REMOTE_AGENT_STALE", "The recipient identity changed.")
        }
        "NAME_NOT_FOUND" => RemoteError::new(
            "REMOTE_AGENT_NOT_FOUND",
            "The recipient identity was not found.",
        ),
        _ => error,
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
pub(crate) mod tests;
