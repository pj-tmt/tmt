//! One caller-owned decoder per page. This parent never parses Yjs bytes.
//! Admission and atomic application remain with the caller; no server integration.
mod child;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use tmt_invoke::{Cleanup, EnvironmentPolicy, LaunchOptions, Request};

pub const DEADLINE: Duration = Duration::from_secs(2);
pub const UPDATE_BYTES: usize = 256 * 1024;
pub const BASELINE_BYTES: usize = 2 * 1024 * 1024;
pub const STREAM_BYTES: usize = 4 * 1024 * 1024;
pub const UPDATES: usize = 200;
pub const MEMORY_BYTES: u64 = 512 * 1024 * 1024;
pub use crate::store::Namespace;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Viewer,
    Commenter,
    Editor,
    Bridge,
}
pub struct Batch<'a> {
    pub namespace: Namespace,
    pub baseline: &'a [u8],
    pub updates: &'a [&'a [u8]],
}
#[derive(Debug)]
pub enum DecodeFault {
    InvalidInput,
    Denied,
    Rejected,
    InvalidOutput,
    CleanupBlocked,
    Invoke(tmt_invoke::InvokeError),
}
impl std::fmt::Display for DecodeFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Decoder failed: {self:?}")
    }
}
impl std::error::Error for DecodeFault {}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryLimit {
    #[serde(rename = "512 MiB address-space limit")]
    Enforced,
    #[serde(rename = "memory limit unavailable")]
    Unavailable,
}
fn memory_limit() -> MemoryLimit {
    if cfg!(target_os = "linux") {
        MemoryLimit::Enforced
    } else {
        MemoryLimit::Unavailable
    }
}
/// Untrusted decoded state: the caller must apply its typed operation/authority
/// policy before any transaction. `merged` contains ONLY the supplied updates,
/// preserving writer attribution and dependencies; the baseline is not folded in.
pub struct Decoded {
    pub merged: Vec<u8>,
    pub projection: Value,
    pub memory_limit: MemoryLimit,
    pub child_pid: u32,
}
pub struct Decoder {
    program: PathBuf,
    blocked: bool,
}
impl Decoder {
    pub fn new(program: PathBuf) -> Result<Self, DecodeFault> {
        if !program.is_absolute() {
            return Err(DecodeFault::InvalidInput);
        }
        Ok(Self {
            program,
            blocked: false,
        })
    }
    /// Exclusive mutable ownership prevents concurrent children through this page owner.
    /// Failed invocations can be reused only after invoke reports Confirmed cleanup.
    pub fn decode(
        &mut self,
        batch: Batch<'_>,
        role: Role,
        stop: Option<&AtomicBool>,
    ) -> Result<Decoded, DecodeFault> {
        self.decode_until(batch, role, stop, Instant::now() + DEADLINE)
    }
    fn decode_until(
        &mut self,
        batch: Batch<'_>,
        role: Role,
        stop: Option<&AtomicBool>,
        deadline: Instant,
    ) -> Result<Decoded, DecodeFault> {
        if self.blocked {
            return Err(DecodeFault::CleanupBlocked);
        }
        if role == Role::Viewer || (batch.namespace == Namespace::Content && role != Role::Editor) {
            return Err(DecodeFault::Denied);
        }
        if batch.baseline.len() > BASELINE_BYTES
            || batch.updates.len() > UPDATES
            || batch
                .updates
                .iter()
                .map(|v| v.len())
                .try_fold(0usize, usize::checked_add)
                .is_none_or(|n| n > UPDATE_BYTES)
        {
            return Err(DecodeFault::InvalidInput);
        }
        let wire = WireBatch {
            version: 1,
            namespace: batch.namespace,
            baseline: URL_SAFE_NO_PAD.encode(batch.baseline),
            updates: batch
                .updates
                .iter()
                .map(|v| URL_SAFE_NO_PAD.encode(v))
                .collect(),
        };
        let input = serde_json::to_vec(&wire).map_err(|_| DecodeFault::InvalidInput)?;
        if input.len() > STREAM_BYTES {
            return Err(DecodeFault::InvalidInput);
        }
        let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(&input));
        let args = ["__decoder".into()];
        let output = tmt_invoke::invoke(
            Request {
                program: &self.program,
                args: &args,
                input: &input,
                deadline,
                max_stream_bytes: STREAM_BYTES,
                launch: LaunchOptions {
                    environment: EnvironmentPolicy::ClearAllowlist(&[]),
                },
            },
            stop,
        )
        .map_err(|e| {
            self.blocked = !matches!(e.cleanup, Cleanup::Confirmed);
            DecodeFault::Invoke(e)
        })?;
        if !output.status.success() {
            return Err(DecodeFault::Rejected);
        }
        let reply: WireResult =
            serde_json::from_slice(&output.stdout).map_err(|_| DecodeFault::InvalidOutput)?;
        if reply.version != 1
            || reply.namespace != batch.namespace
            || reply.input_hash != hash
            || reply.pid == 0
            || reply.memory_limit != memory_limit()
        {
            return Err(DecodeFault::InvalidOutput);
        }
        validate_projection(batch.namespace, &reply.projection)?;
        let merged = binary(&reply.merged, UPDATE_BYTES).map_err(|_| DecodeFault::InvalidOutput)?;
        Ok(Decoded {
            merged,
            projection: reply.projection,
            memory_limit: reply.memory_limit,
            child_pid: reply.pid,
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBatch {
    version: u8,
    namespace: Namespace,
    baseline: String,
    updates: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResult {
    version: u8,
    namespace: Namespace,
    input_hash: String,
    merged: String,
    projection: Value,
    memory_limit: MemoryLimit,
    pid: u32,
}
fn binary(value: &str, limit: usize) -> Result<Vec<u8>, DecodeFault> {
    if value.len() > limit.div_ceil(3) * 4 {
        return Err(DecodeFault::InvalidInput);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| DecodeFault::InvalidInput)?;
    if bytes.len() > limit || URL_SAFE_NO_PAD.encode(&bytes) != value {
        return Err(DecodeFault::InvalidInput);
    }
    Ok(bytes)
}
fn validate_projection(namespace: Namespace, value: &Value) -> Result<(), DecodeFault> {
    let roots = value.as_object().ok_or(DecodeFault::InvalidOutput)?;
    match namespace {
        Namespace::Content => {
            if roots.len() != 2
                || roots
                    .get("html")
                    .and_then(Value::as_str)
                    .is_none_or(|v| v.len() > BASELINE_BYTES)
            {
                return Err(DecodeFault::InvalidOutput);
            }
            let meta = roots
                .get("meta")
                .and_then(Value::as_object)
                .ok_or(DecodeFault::InvalidOutput)?;
            if meta.len() > 1 || meta.iter().any(|(k, v)| k != "title" || !v.is_string()) {
                return Err(DecodeFault::InvalidOutput);
            }
        }
        Namespace::Own => {
            if roots.len() != 4
                || ["threads", "messages", "intents", "replies"]
                    .iter()
                    .any(|k| !roots.get(*k).is_some_and(Value::is_object))
                || roots["threads"].as_object().unwrap().len() > 1000
            {
                return Err(DecodeFault::InvalidOutput);
            }
            for message in roots["messages"].as_object().unwrap().values() {
                if message
                    .get("body")
                    .is_some_and(|v| v.as_str().is_none_or(|v| v.len() > 16 * 1024))
                {
                    return Err(DecodeFault::InvalidOutput);
                }
            }
        }
    }
    Ok(())
}
/// Private stdin/stdout protocol, called before CLI routing or any data-root access.
pub fn child_main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() != Some("__decoder") {
        return std::process::ExitCode::FAILURE;
    }
    child::run()
}
#[cfg(test)]
mod tests;
