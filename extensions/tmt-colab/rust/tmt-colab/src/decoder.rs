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
/// One update the write path may prepare or submit.
pub const UPDATE_BYTES: usize = 256 * 1024;
/// A page's tail since its last baseline that a write accepts. Browser reads have
/// wider limits; browser writes retain this bound (`fold.worker.ts`).
pub const WRITE_TAIL_UPDATES: usize = 200;
/// Bytes of updates since the last baseline that a write leaves behind: a 2 MiB source plus
/// as much again in edits. At 200 updates a 4 MiB block decodes in well under the 2 s deadline
/// (cost is about 0.6 ms per MiB of block per update), so the bound never rests on the deadline.
pub const WRITE_TAIL_BYTES: usize = 4 * 1024 * 1024;
/// A new page's source is published as updates of at most this many bytes of text each, which
/// every reader admits as ordinary updates.
pub const CREATE_CHUNK_BYTES: usize = 192 * 1024;
/// A new page's or baseline's source (write path).
pub const BASELINE_BYTES: usize = 2 * 1024 * 1024;
/// Owner decision (#1627): a page's whole state as the browser loads it, gzipped, is at most
/// this many bytes. Size errors name it; the raw caps below are the containment it implies.
pub const PAGE_BUDGET_GZIP_BYTES: usize = 5_000_000;
/// Raw bytes a read may decode for one page: its baseline plus every update since. Measured
/// (#1627): decoding peaks near 9 B of child resident memory per state byte, so 24 MiB stays
/// inside the 512 MiB limit while leaving room for a page near the gzipped budget.
pub const STATE_BYTES: usize = 24 * 1024 * 1024;
/// Updates since the last baseline that one read may fold.
pub const UPDATES: usize = 5_000;
/// One JSON frame to or from the isolated decoder: the worst case is a state of control
/// characters, which JSON escapes to six bytes each, plus base64 of the input and output.
pub const STREAM_BYTES: usize = STATE_BYTES * 6 + STATE_BYTES.div_ceil(3) * 4 * 2 + 1024;
pub const MEMORY_BYTES: u64 = 512 * 1024 * 1024;
pub use crate::store::Namespace;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Viewer,
    Commenter,
    Editor,
    Bridge,
}
pub struct UpdateBatch<'a> {
    pub namespace: Namespace,
    pub baseline: &'a [u8],
    pub updates: &'a [&'a [u8]],
}
/// One CLI source replacement, including its optional display-only caller label.
#[derive(Clone, Copy)]
pub struct ContentEdit<'a> {
    pub source: &'a str,
    pub publisher_agent: Option<&'a str>,
}
/// A bounded plain-data batch for one authenticated writer's own document.
/// The caller establishes writer/page/epoch scope; only the child touches Yjs.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnRecord {
    pub root: String,
    pub key: String,
    pub value: Value,
}
#[derive(Clone, Copy)]
enum Edit<'a> {
    Content(ContentEdit<'a>),
    Own(&'a [OwnRecord]),
}
fn validate_own_records(records: &[OwnRecord]) -> Result<(), DecodeFault> {
    let mut keys = std::collections::BTreeSet::new();
    if records.is_empty() || records.len() > crate::limits::OWN_RECORDS {
        return Err(DecodeFault::InvalidInput);
    }
    for record in records {
        if !matches!(
            record.root.as_str(),
            "threads" | "messages" | "intents" | "replies"
        ) || !keys.insert((&record.root, &record.key))
            || crate::threads::validate_record(&record.root, &record.key, &record.value).is_err()
            || crate::ask::validate_record(&record.root, &record.key, &record.value).is_err()
        {
            return Err(DecodeFault::InvalidInput);
        }
    }
    Ok(())
}
/// A publisher-asserted display label, never an identity or authorization claim.
pub fn valid_publisher_agent(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= crate::limits::PUBLISHER_AGENT_BYTES
        && !value.chars().any(char::is_control)
}
/// Exact current view supplied by the owner's authenticated fold. This seam
/// does not establish log, page or epoch authority.
pub struct BaselineInput<'a> {
    pub source: &'a [u8],
    pub title: &'a str,
    pub publisher_agent: Option<&'a str>,
    pub source_digest: [u8; 32],
}
/// One fresh struct identity to persist and distribute unchanged to every client.
/// The caller owns descriptor signing, encryption and atomic epoch admission.
pub struct Baseline {
    pub update: Vec<u8>,
    /// The same state as ordered updates of at most `chunk_bytes` of text each; empty unless
    /// asked for.
    pub chunks: Vec<Vec<u8>>,
    pub source_digest: [u8; 32],
    pub commitment: [u8; 32],
    pub memory_limit: MemoryLimit,
    pub child_pid: u32,
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
pub fn memory_limit() -> MemoryLimit {
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
/// Caller-owned invocation configuration. Production composition uses `new`;
/// tests can inject a larger deadline without changing caps or cleanup ownership.
#[derive(Clone, Debug)]
pub struct Config {
    pub program: PathBuf,
    pub deadline: Duration,
}
impl Config {
    pub fn new(program: PathBuf) -> Self {
        Self {
            program,
            deadline: DEADLINE,
        }
    }
    fn validate(&self) -> Result<(), DecodeFault> {
        if !self.program.is_absolute() {
            return Err(DecodeFault::InvalidInput);
        }
        validate_deadline(self.deadline)
    }
}
fn validate_deadline(deadline: Duration) -> Result<(), DecodeFault> {
    if deadline.is_zero() || Instant::now().checked_add(deadline).is_none() {
        return Err(DecodeFault::InvalidInput);
    }
    Ok(())
}
pub struct Decoder {
    config: Config,
    blocked: bool,
}
impl Decoder {
    pub fn new(program: PathBuf) -> Result<Self, DecodeFault> {
        Self::with_config(Config::new(program))
    }
    pub fn with_config(config: Config) -> Result<Self, DecodeFault> {
        config.validate()?;
        Ok(Self {
            config,
            blocked: false,
        })
    }
    /// Change the next invocation's budget without clearing the cleanup fence.
    /// Tests use this to retain tight timeout checks and generous reuse controls.
    pub fn set_deadline(&mut self, deadline: Duration) -> Result<(), DecodeFault> {
        validate_deadline(deadline)?;
        self.config.deadline = deadline;
        Ok(())
    }
    /// Exclusive mutable ownership prevents concurrent children through this page owner.
    /// Reuse requires no started child or confirmed cleanup; possible survivors block it.
    pub fn decode(
        &mut self,
        batch: UpdateBatch<'_>,
        role: Role,
        stop: Option<&AtomicBool>,
    ) -> Result<Decoded, DecodeFault> {
        self.decode_until(batch, role, stop, Instant::now() + self.config.deadline)
    }
    fn decode_until(
        &mut self,
        batch: UpdateBatch<'_>,
        role: Role,
        stop: Option<&AtomicBool>,
        deadline: Instant,
    ) -> Result<Decoded, DecodeFault> {
        self.decode_request(batch, role, None, false, stop, deadline)
    }
    /// Merge one device's own updates into a single update-v1 with `merge_updates_v1`, without
    /// building or checking the page: the compaction a device publishes as its checkpoint. Every
    /// update must still decode and apply.
    pub fn merge(
        &mut self,
        namespace: Namespace,
        updates: &[&[u8]],
        stop: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, DecodeFault> {
        let role = match namespace {
            Namespace::Content => Role::Editor,
            Namespace::Own => Role::Commenter,
        };
        let deadline = Instant::now() + self.config.deadline;
        let batch = UpdateBatch {
            namespace,
            baseline: &[],
            updates,
        };
        Ok(self
            .decode_request(batch, role, None, true, stop, deadline)?
            .merged)
    }
    fn decode_request(
        &mut self,
        batch: UpdateBatch<'_>,
        role: Role,
        edit: Option<Edit<'_>>,
        merge_only: bool,
        stop: Option<&AtomicBool>,
        deadline: Instant,
    ) -> Result<Decoded, DecodeFault> {
        if self.blocked {
            return Err(DecodeFault::CleanupBlocked);
        }
        if role == Role::Viewer || (batch.namespace == Namespace::Content && role != Role::Editor) {
            return Err(DecodeFault::Denied);
        }
        if batch.baseline.len() > STATE_BYTES
            || batch.updates.len() > UPDATES
            || batch
                .updates
                .iter()
                .map(|v| v.len())
                .try_fold(batch.baseline.len(), usize::checked_add)
                .is_none_or(|n| n > STATE_BYTES)
        {
            return Err(DecodeFault::InvalidInput);
        }
        let wire = WireBatch {
            version: 1,
            namespace: batch.namespace,
            source: match edit {
                Some(Edit::Content(v)) => Some(v.source.to_owned()),
                _ => None,
            },
            publisher_agent: match edit {
                Some(Edit::Content(v)) => v.publisher_agent.map(str::to_owned),
                _ => None,
            },
            records: match edit {
                Some(Edit::Own(v)) => Some(v.to_vec()),
                _ => None,
            },
            baseline: URL_SAFE_NO_PAD.encode(batch.baseline),
            updates: batch
                .updates
                .iter()
                .map(|v| URL_SAFE_NO_PAD.encode(v))
                .collect(),
            merge_only,
        };
        let input = serde_json::to_vec(&wire).map_err(|_| DecodeFault::InvalidInput)?;
        if input.len() > STREAM_BYTES {
            return Err(DecodeFault::InvalidInput);
        }
        let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(&input));
        let output = self.invoke(&input, false, stop, deadline)?;
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
        if merge_only {
            if !reply.projection.is_null() {
                return Err(DecodeFault::InvalidOutput);
            }
        } else {
            validate_projection(batch.namespace, &reply.projection)?;
        }
        let wrong_edit = match edit {
            Some(Edit::Content(value)) => {
                reply.projection["html"].as_str() != Some(value.source)
                    || reply.projection["meta"]["publisherAgent"].as_str() != value.publisher_agent
            }
            Some(Edit::Own(records)) => records.iter().any(|record| {
                reply
                    .projection
                    .get(&record.root)
                    .and_then(|root| root.get(&record.key))
                    != Some(&record.value)
            }),
            None => false,
        };
        if wrong_edit {
            return Err(DecodeFault::InvalidOutput);
        }
        // A prepared edit returns one update; a read returns the merged tail.
        let merged_limit = if edit.is_some() {
            UPDATE_BYTES
        } else {
            STATE_BYTES
        };
        let merged = binary(&reply.merged, merged_limit).map_err(|_| DecodeFault::InvalidOutput)?;
        Ok(Decoded {
            merged,
            projection: reply.projection,
            memory_limit: reply.memory_limit,
            child_pid: reply.pid,
        })
    }
    pub fn prepare(
        &mut self,
        batch: UpdateBatch<'_>,
        edit: ContentEdit<'_>,
        stop: Option<&AtomicBool>,
    ) -> Result<Decoded, DecodeFault> {
        if edit.source.len() > BASELINE_BYTES
            || batch.namespace != Namespace::Content
            || edit
                .publisher_agent
                .is_some_and(|v| !valid_publisher_agent(v))
        {
            return Err(DecodeFault::InvalidInput);
        }
        self.decode_request(
            batch,
            Role::Editor,
            Some(Edit::Content(edit)),
            false,
            stop,
            Instant::now() + self.config.deadline,
        )
    }
    /// Prepares one immutable own update without committing it to any stream.
    pub fn prepare_own(
        &mut self,
        batch: UpdateBatch<'_>,
        records: &[OwnRecord],
        stop: Option<&AtomicBool>,
    ) -> Result<Decoded, DecodeFault> {
        if batch.namespace != Namespace::Own {
            return Err(DecodeFault::InvalidInput);
        }
        validate_own_records(records)?;
        self.decode_request(
            batch,
            Role::Commenter,
            Some(Edit::Own(records)),
            false,
            stop,
            Instant::now() + self.config.deadline,
        )
    }
    /// Produces once from a fresh document and checks materialization in the child.
    pub fn produce_baseline(
        &mut self,
        view: BaselineInput<'_>,
        stop: Option<&AtomicBool>,
    ) -> Result<Baseline, DecodeFault> {
        self.baseline(view, BaselineAction::Produce { chunk_bytes: None }, stop)
    }
    /// Like `produce_baseline`, plus the state as ordered updates of at most `CREATE_CHUNK_BYTES`
    /// of text each, for a new page whose source is larger than one update.
    pub fn produce_page(
        &mut self,
        view: BaselineInput<'_>,
        stop: Option<&AtomicBool>,
    ) -> Result<Baseline, DecodeFault> {
        self.baseline(
            view,
            BaselineAction::Produce {
                chunk_bytes: Some(CREATE_CHUNK_BYTES),
            },
            stop,
        )
    }
    /// Checks a received baseline against the authenticated source/title and hashes.
    /// The child reconstructs source from the update and checks its digest,
    /// avoiding two source copies in the bounded request.
    pub fn verify_baseline(
        &mut self,
        view: BaselineInput<'_>,
        update: &[u8],
        commitment: [u8; 32],
        stop: Option<&AtomicBool>,
    ) -> Result<Baseline, DecodeFault> {
        if update.len() > BASELINE_UPDATE_BYTES {
            return Err(DecodeFault::InvalidInput);
        }
        self.baseline(
            view,
            BaselineAction::Verify {
                update: URL_SAFE_NO_PAD.encode(update),
                commitment: URL_SAFE_NO_PAD.encode(commitment),
            },
            stop,
        )
    }
    fn baseline(
        &mut self,
        view: BaselineInput<'_>,
        action: BaselineAction,
        stop: Option<&AtomicBool>,
    ) -> Result<Baseline, DecodeFault> {
        let deadline = Instant::now() + self.config.deadline;
        if self.blocked {
            return Err(DecodeFault::CleanupBlocked);
        }
        validate_view(view.source, view.title, &view.source_digest)?;
        if view
            .publisher_agent
            .is_some_and(|v| !valid_publisher_agent(v))
        {
            return Err(DecodeFault::InvalidInput);
        }
        let expected = match &action {
            BaselineAction::Produce { .. } => None,
            BaselineAction::Verify { update, commitment } => {
                Some((update.clone(), commitment.clone()))
            }
        };
        let input = serde_json::to_vec(&WireBaseline {
            version: 1,
            source: if matches!(action, BaselineAction::Produce { .. }) {
                URL_SAFE_NO_PAD.encode(view.source)
            } else {
                String::new()
            },
            title: view.title.into(),
            publisher_agent: view.publisher_agent.map(str::to_owned),
            source_digest: URL_SAFE_NO_PAD.encode(view.source_digest),
            action,
        })
        .map_err(|_| DecodeFault::InvalidInput)?;
        let output = self.invoke(&input, true, stop, deadline)?;
        if !output.status.success() {
            return Err(DecodeFault::Rejected);
        }
        let reply: WireBaselineResult =
            serde_json::from_slice(&output.stdout).map_err(|_| DecodeFault::InvalidOutput)?;
        if reply.version != 1
            || reply.input_hash != URL_SAFE_NO_PAD.encode(Sha256::digest(&input))
            || reply.pid == 0
            || reply.memory_limit != memory_limit()
            || reply.source_digest != URL_SAFE_NO_PAD.encode(view.source_digest)
        {
            return Err(DecodeFault::InvalidOutput);
        }
        if expected.is_some_and(|(update, commitment)| {
            reply.update != update || reply.commitment != commitment
        }) {
            return Err(DecodeFault::InvalidOutput);
        }
        let update =
            binary(&reply.update, BASELINE_UPDATE_BYTES).map_err(|_| DecodeFault::InvalidOutput)?;
        let commitment = baseline_commitment(view.source, &update)?;
        if reply.commitment != URL_SAFE_NO_PAD.encode(commitment) {
            return Err(DecodeFault::InvalidOutput);
        }
        let chunks = reply
            .chunks
            .iter()
            .map(|chunk| binary(chunk, UPDATE_BYTES).map_err(|_| DecodeFault::InvalidOutput))
            .collect::<Result<Vec<_>, _>>()?;
        if chunks.len() > WRITE_TAIL_UPDATES {
            return Err(DecodeFault::InvalidOutput);
        }
        Ok(Baseline {
            update,
            chunks,
            source_digest: view.source_digest,
            commitment,
            memory_limit: reply.memory_limit,
            child_pid: reply.pid,
        })
    }
    fn invoke(
        &mut self,
        input: &[u8],
        baseline: bool,
        stop: Option<&AtomicBool>,
        deadline: Instant,
    ) -> Result<tmt_invoke::Output, DecodeFault> {
        if input.len() > STREAM_BYTES {
            return Err(DecodeFault::InvalidInput);
        }
        let mut args = vec!["__decoder".into()];
        if baseline {
            args.push("baseline".into());
        }
        tmt_invoke::invoke(
            Request {
                program: &self.config.program,
                args: &args,
                input,
                deadline,
                max_stream_bytes: STREAM_BYTES,
                launch: LaunchOptions {
                    environment: EnvironmentPolicy::ClearAllowlist(&[]),
                    ..Default::default()
                },
            },
            stop,
        )
        .map_err(|e| {
            self.blocked = cleanup_blocks(&e.cleanup);
            DecodeFault::Invoke(e)
        })
    }
}
fn cleanup_blocks(cleanup: &Cleanup) -> bool {
    !matches!(cleanup, Cleanup::NotStarted | Cleanup::Confirmed)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBatch {
    version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publisher_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    records: Option<Vec<OwnRecord>>,
    namespace: Namespace,
    baseline: String,
    updates: Vec<String>,
    /// Merge the updates and return them, without projecting the document: a device's own
    /// stream may depend on structs another device wrote, so alone it is not a complete page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    merge_only: bool,
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
// A full baseline is not a 256 KiB stream update. Allow source, title and
// bounded update-v1 framing, within the decoder's stream cap.
pub const BASELINE_TITLE_BYTES: usize = 256 * 1024;
pub const BASELINE_UPDATE_BYTES: usize = STATE_BYTES + BASELINE_TITLE_BYTES + 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBaseline {
    version: u8,
    source: String,
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publisher_agent: Option<String>,
    source_digest: String,
    action: BaselineAction,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
enum BaselineAction {
    Produce {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chunk_bytes: Option<usize>,
    },
    Verify {
        update: String,
        commitment: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBaselineResult {
    version: u8,
    input_hash: String,
    update: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    chunks: Vec<String>,
    source_digest: String,
    commitment: String,
    memory_limit: MemoryLimit,
    pid: u32,
}
fn validate_view(source: &[u8], title: &str, digest: &[u8]) -> Result<(), DecodeFault> {
    if source.len() > BASELINE_BYTES
        || title.len() > BASELINE_TITLE_BYTES
        || std::str::from_utf8(source).is_err()
        || Sha256::digest(source).as_slice() != digest
    {
        return Err(DecodeFault::InvalidInput);
    }
    Ok(())
}
fn baseline_commitment(source: &[u8], update: &[u8]) -> Result<[u8; 32], DecodeFault> {
    let framed = tmt_colab_model::framing::frame(&[b"tmt-colab-baseline-v1", b"1", source, update])
        .map_err(|_| DecodeFault::InvalidInput)?;
    Ok(Sha256::digest(framed).into())
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
                    .is_none_or(|v| v.len() > STATE_BYTES)
            {
                return Err(DecodeFault::InvalidOutput);
            }
            let meta = roots
                .get("meta")
                .and_then(Value::as_object)
                .ok_or(DecodeFault::InvalidOutput)?;
            if meta.iter().any(|(k, v)| match k.as_str() {
                "title" => !v.is_string(),
                "publisherAgent" => v.as_str().is_none_or(|v| !valid_publisher_agent(v)),
                _ => true,
            }) {
                return Err(DecodeFault::InvalidOutput);
            }
        }
        Namespace::Own => {
            let Some(threads) = roots.get("threads").and_then(Value::as_object) else {
                return Err(DecodeFault::InvalidOutput);
            };
            let Some(messages) = roots.get("messages").and_then(Value::as_object) else {
                return Err(DecodeFault::InvalidOutput);
            };
            if roots.len() != 4
                || ["threads", "messages", "intents", "replies"]
                    .iter()
                    .any(|k| !roots.get(*k).is_some_and(Value::is_object))
                || threads.len() > 1000
            {
                return Err(DecodeFault::InvalidOutput);
            }
            for root in ["threads", "intents", "messages", "replies"] {
                for (key, value) in roots[root].as_object().ok_or(DecodeFault::InvalidOutput)? {
                    crate::threads::validate_record(root, key, value)
                        .map_err(|_| DecodeFault::InvalidOutput)?;
                    crate::ask::validate_record(root, key, value)
                        .map_err(|_| DecodeFault::InvalidOutput)?;
                }
            }
            for message in messages.values() {
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
