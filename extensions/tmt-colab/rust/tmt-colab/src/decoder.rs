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
/// Creation-time routing preference; current Remote admission remains authoritative.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreationRecipient {
    pub machine_id: String,
    pub agent_id: String,
}
impl CreationRecipient {
    pub fn valid(&self) -> bool {
        tmt_colab_model::values::generated_id(&self.machine_id).is_ok()
            && tmt_colab_model::values::core_id(&self.agent_id).is_ok()
    }
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
    pub attachments: Option<&'a tmt_colab_model::attachment::DocumentAttachments>,
    pub source: &'a [u8],
    pub title: &'a str,
    pub publisher_agent: Option<&'a str>,
    pub source_digest: [u8; 32],
    pub creation_recipient: Option<&'a CreationRecipient>,
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
/// Plaintext causal preparation only; publication remains the caller's responsibility.
#[derive(Debug, PartialEq, Eq)]
pub enum ContentBatch {
    Noop,
    Updates(Vec<Vec<u8>>),
}
pub struct PreparedContent {
    pub batch: ContentBatch,
    pub projection: Value,
    pub memory_limit: MemoryLimit,
    pub child_pid: u32,
}
#[derive(Clone, Copy)]
enum ChildCommand {
    ContentDecode,
    OwnDecode,
    ContentMerge,
    OwnMerge,
    OwnEdit,
    BaselineProduce,
    BaselinePage,
    BaselineVerify,
    PrepareContent,
}
impl ChildCommand {
    fn decode(namespace: Namespace, edit: bool, merge_only: bool) -> Self {
        match (namespace, edit, merge_only) {
            (Namespace::Own, true, _) => Self::OwnEdit,
            (Namespace::Content, _, true) => Self::ContentMerge,
            (Namespace::Own, false, true) => Self::OwnMerge,
            (Namespace::Content, _, false) => Self::ContentDecode,
            (Namespace::Own, false, false) => Self::OwnDecode,
        }
    }
    fn phase(&self) -> &'static str {
        match self {
            Self::ContentDecode => "content.decode",
            Self::OwnDecode => "own.decode",
            Self::ContentMerge => "content.merge",
            Self::OwnMerge => "own.merge",
            Self::OwnEdit => "own.edit",
            Self::BaselineProduce => "baseline.produce",
            Self::BaselinePage => "baseline.page",
            Self::BaselineVerify => "baseline.verify",
            Self::PrepareContent => "content.prepare-content",
        }
    }
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
        records: Option<&[OwnRecord]>,
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
        let before_wire = Instant::now();
        let wire = WireBatch {
            version: 1,
            namespace: batch.namespace,
            records: records.map(<[OwnRecord]>::to_vec),
            baseline: EncodedBytes(batch.baseline),
            updates: batch.updates.iter().map(|v| EncodedBytes(v)).collect(),
            merge_only,
        };
        let after_wire = Instant::now();
        let input = SerializedInput::serialize(&wire)?;
        let after_json = Instant::now();
        if input.bytes.len() > STREAM_BYTES {
            return Err(DecodeFault::InvalidInput);
        }
        let (input, hash) = input.finish();
        let after_hash = Instant::now();
        let output = self.invoke(
            &input,
            ChildCommand::decode(batch.namespace, records.is_some(), merge_only),
            stop,
            deadline,
            Some(ParentTiming::from_samples([
                before_wire,
                after_wire,
                after_json,
                after_hash,
            ])),
        )?;
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
        let wrong_edit = records.is_some_and(|records| {
            records.iter().any(|record| {
                reply
                    .projection
                    .get(&record.root)
                    .and_then(|root| root.get(&record.key))
                    != Some(&record.value)
            })
        });
        if wrong_edit {
            return Err(DecodeFault::InvalidOutput);
        }
        // A prepared edit returns one update; a read returns the merged tail.
        let merged_limit = if records.is_some() {
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
            Some(records),
            false,
            stop,
            Instant::now() + self.config.deadline,
        )
    }
    /// Prepare bounded deltas against the exact admitted content projection. The
    /// parent checks correlation and projection; only the child parses Yjs.
    pub fn prepare_content_batch(
        &mut self,
        batch: UpdateBatch<'_>,
        expected_base: &Value,
        edit: ContentEdit<'_>,
        stop: Option<&AtomicBool>,
    ) -> Result<PreparedContent, DecodeFault> {
        if self.blocked {
            return Err(DecodeFault::CleanupBlocked);
        }
        let deadline = Instant::now() + self.config.deadline;
        if batch.namespace != Namespace::Content
            || edit.source.len() > BASELINE_BYTES
            || edit
                .publisher_agent
                .is_some_and(|v| !valid_publisher_agent(v))
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
        validate_projection(Namespace::Content, expected_base)
            .map_err(|_| DecodeFault::InvalidInput)?;
        let expected = edited_projection(expected_base, edit);
        let wire = WireContentPreparation {
            version: 1,
            baseline: URL_SAFE_NO_PAD.encode(batch.baseline),
            updates: batch
                .updates
                .iter()
                .map(|v| URL_SAFE_NO_PAD.encode(v))
                .collect(),
            expected_base,
            source: edit.source,
            publisher_agent: edit.publisher_agent,
        };
        let (input, hash) = SerializedInput::serialize(&wire)?.finish();
        let output = self.invoke(&input, ChildCommand::PrepareContent, stop, deadline, None)?;
        if !output.status.success() {
            return Err(DecodeFault::Rejected);
        }
        let reply: WirePreparedContent =
            serde_json::from_slice(&output.stdout).map_err(|_| DecodeFault::InvalidOutput)?;
        admit_prepared_content(reply, &hash, &expected, expected == *expected_base)
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
        if let Some(list) = view.attachments {
            tmt_colab_model::attachment::validate_attachment_list(
                list.as_slice(),
                tmt_colab_model::attachment::DOCUMENT_ATTACHMENTS,
                None,
            )
            .map_err(|_| DecodeFault::InvalidInput)?;
        }
        if view.creation_recipient.is_some_and(|v| !v.valid()) {
            return Err(DecodeFault::InvalidInput);
        }
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
        let command = match &action {
            BaselineAction::Produce { chunk_bytes: None } => ChildCommand::BaselineProduce,
            BaselineAction::Produce {
                chunk_bytes: Some(_),
            } => ChildCommand::BaselinePage,
            BaselineAction::Verify { .. } => ChildCommand::BaselineVerify,
        };
        let input = serde_json::to_vec(&WireBaseline {
            attachments: view.attachments.cloned(),
            version: 1,
            source: if matches!(action, BaselineAction::Produce { .. }) {
                URL_SAFE_NO_PAD.encode(view.source)
            } else {
                String::new()
            },
            title: view.title.into(),
            publisher_agent: view.publisher_agent.map(str::to_owned),
            creation_recipient: view.creation_recipient.cloned(),
            source_digest: URL_SAFE_NO_PAD.encode(view.source_digest),
            action,
        })
        .map_err(|_| DecodeFault::InvalidInput)?;
        let output = self.invoke(&input, command, stop, deadline, None)?;
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
        command: ChildCommand,
        stop: Option<&AtomicBool>,
        deadline: Instant,
        parent_timing: Option<ParentTiming>,
    ) -> Result<tmt_invoke::Output, DecodeFault> {
        if input.len() > STREAM_BYTES {
            return Err(DecodeFault::InvalidInput);
        }
        let mut args = vec!["__decoder".into()];
        match command {
            ChildCommand::ContentDecode
            | ChildCommand::OwnDecode
            | ChildCommand::ContentMerge
            | ChildCommand::OwnMerge
            | ChildCommand::OwnEdit => {}
            ChildCommand::BaselineProduce
            | ChildCommand::BaselinePage
            | ChildCommand::BaselineVerify => args.push("baseline".into()),
            ChildCommand::PrepareContent => args.push("prepare-content".into()),
        }
        let started = Instant::now();
        let remaining = deadline.saturating_duration_since(started);
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
            invocation_failure(
                &mut self.blocked,
                e,
                InvocationObservation {
                    command,
                    input_bytes: input.len(),
                    remaining,
                    elapsed: started.elapsed(),
                    parent_timing,
                },
                tmt_cli_style::stream::stderr,
            )
        })
    }
}
#[derive(Clone, Copy)]
struct ParentTiming {
    wire: Duration,
    json: Duration,
    hash: Duration,
}
impl ParentTiming {
    fn from_samples([before_wire, after_wire, after_json, after_hash]: [Instant; 4]) -> Self {
        Self {
            wire: after_wire.duration_since(before_wire),
            json: after_json.duration_since(after_wire),
            hash: after_hash.duration_since(after_json),
        }
    }
}
struct InvocationObservation {
    command: ChildCommand,
    input_bytes: usize,
    remaining: Duration,
    elapsed: Duration,
    parent_timing: Option<ParentTiming>,
}
fn invocation_failure<W: std::io::Write>(
    blocked: &mut bool,
    error: tmt_invoke::InvokeError,
    observation: InvocationObservation,
    writer: impl FnOnce() -> W,
) -> DecodeFault {
    *blocked = cleanup_blocks(&error.cleanup);
    write_invocation_failure(&mut writer(), &error, observation);
    DecodeFault::Invoke(error)
}
fn write_invocation_failure(
    writer: &mut impl std::io::Write,
    error: &tmt_invoke::InvokeError,
    observation: InvocationObservation,
) {
    use std::io::{Cursor, Write};
    use tmt_invoke::{FailureKind, Phase, Stream};
    let kind = match error.kind {
        FailureKind::Spawn => "spawn",
        FailureKind::Deadline => "deadline",
        FailureKind::Interrupted => "interrupted",
        FailureKind::OutputLimit(Stream::Stdout) => "stdout-limit",
        FailureKind::OutputLimit(Stream::Stderr) => "stderr-limit",
        FailureKind::Io(Phase::OpenPipes) => "io-open-pipes",
        FailureKind::Io(Phase::Communicate) => "io-communicate",
        FailureKind::Io(Phase::Wait) => "io-wait",
    };
    let cleanup = match &error.cleanup {
        Cleanup::NotStarted => "not-started",
        Cleanup::Confirmed => "confirmed",
        Cleanup::CallerOwned => "caller-owned",
        Cleanup::Unconfirmed(_) => "unconfirmed",
    };
    let mut bytes = [0; 512];
    let mut record = Cursor::new(bytes.as_mut_slice());
    // Static labels and numbers only. A record/stream write failure is supplementary.
    if (|| -> std::io::Result<()> {
        write!(
            record,
            "colab decoder failure phase={} input_bytes={} remaining_ns={} invocation_ns={} kind={} cleanup={}",
            observation.command.phase(),
            observation.input_bytes,
            observation.remaining.as_nanos(),
            observation.elapsed.as_nanos(),
            kind,
            cleanup
        )?;
        if let Some(parent) = observation.parent_timing {
            write!(
                record,
                " wire_ns={} json_ns={} hash_ns={}",
                parent.wire.as_nanos(),
                parent.json.as_nanos(),
                parent.hash.as_nanos()
            )?;
        }
        writeln!(record)
    })()
    .is_ok()
    {
        let length = record.position() as usize;
        if let Some(record) = bytes.get(..length) {
            let _ = writer.write_all(record);
        }
    }
}
fn cleanup_blocks(cleanup: &Cleanup) -> bool {
    !matches!(cleanup, Cleanup::NotStarted | Cleanup::Confirmed)
}
struct SerializedInput {
    bytes: Vec<u8>,
    hash: Sha256,
}
impl SerializedInput {
    fn serialize(value: &(impl Serialize + ?Sized)) -> Result<Self, DecodeFault> {
        let mut input = Self {
            bytes: Vec::with_capacity(128),
            hash: Sha256::new(),
        };
        serde_json::to_writer(&mut input, value).map_err(|_| DecodeFault::InvalidInput)?;
        Ok(input)
    }
    fn finish(self) -> (Vec<u8>, String) {
        (self.bytes, URL_SAFE_NO_PAD.encode(self.hash.finalize()))
    }
}
impl std::io::Write for SerializedInput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        self.hash.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct EncodedBytes<'a>(&'a [u8]);
impl Serialize for EncodedBytes<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&base64::display::Base64Display::new(
            self.0,
            &URL_SAFE_NO_PAD,
        ))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct BorrowedWireText<'a>(#[serde(borrow)] std::borrow::Cow<'a, str>);
impl std::ops::Deref for BorrowedWireText<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBatch<B = String> {
    version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    records: Option<Vec<OwnRecord>>,
    namespace: Namespace,
    baseline: B,
    updates: Vec<B>,
    /// Merge the updates and return them, without projecting the document: a device's own
    /// stream may depend on structs another device wrote, so alone it is not a complete page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    merge_only: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResult<B = String> {
    version: u8,
    namespace: Namespace,
    input_hash: String,
    merged: B,
    projection: Value,
    memory_limit: MemoryLimit,
    pid: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireContentPreparation<S = String, V = Value, B = String> {
    version: u8,
    baseline: B,
    updates: Vec<B>,
    expected_base: V,
    source: S,
    publisher_agent: Option<S>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum WireContentBatch<B = String> {
    Noop,
    Updates { updates: Vec<B> },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePreparedContent<B = String> {
    version: u8,
    input_hash: String,
    batch: WireContentBatch<B>,
    projection: Value,
    memory_limit: MemoryLimit,
    pid: u32,
}
fn edited_projection(base: &Value, edit: ContentEdit<'_>) -> Value {
    let mut expected = serde_json::json!({
        "html": edit.source,
        "meta": base["meta"].clone(),
    });
    let meta = expected["meta"]
        .as_object_mut()
        .expect("validated content metadata");
    if let Some(agent) = edit.publisher_agent {
        meta.insert("publisherAgent".into(), Value::String(agent.into()));
    } else {
        meta.remove("publisherAgent");
    }
    expected
}
fn admit_prepared_content(
    reply: WirePreparedContent,
    input_hash: &str,
    expected: &Value,
    noop: bool,
) -> Result<PreparedContent, DecodeFault> {
    if reply.version != 1
        || reply.input_hash != input_hash
        || reply.pid == 0
        || reply.memory_limit != memory_limit()
        || reply.projection != *expected
        || serde_json::to_vec(&reply.projection)
            .map_err(|_| DecodeFault::InvalidOutput)?
            .len()
            > STATE_BYTES
    {
        return Err(DecodeFault::InvalidOutput);
    }
    validate_projection(Namespace::Content, &reply.projection)?;
    let batch = match reply.batch {
        WireContentBatch::Noop if noop => ContentBatch::Noop,
        WireContentBatch::Updates { updates } if !noop => {
            if updates.is_empty() || updates.len() > WRITE_TAIL_UPDATES {
                return Err(DecodeFault::InvalidOutput);
            }
            let updates = updates
                .iter()
                .map(|v| binary(v, UPDATE_BYTES).map_err(|_| DecodeFault::InvalidOutput))
                .collect::<Result<Vec<_>, _>>()?;
            if updates.iter().any(Vec::is_empty)
                || updates.iter().map(Vec::len).sum::<usize>() > WRITE_TAIL_BYTES
            {
                return Err(DecodeFault::InvalidOutput);
            }
            ContentBatch::Updates(updates)
        }
        _ => return Err(DecodeFault::InvalidOutput),
    };
    Ok(PreparedContent {
        batch,
        projection: reply.projection,
        memory_limit: reply.memory_limit,
        child_pid: reply.pid,
    })
}
// A full baseline is not a 256 KiB stream update. Allow source, title and
// bounded update-v1 framing, within the decoder's stream cap.
pub const BASELINE_TITLE_BYTES: usize = 256 * 1024;
pub const BASELINE_UPDATE_BYTES: usize = STATE_BYTES + BASELINE_TITLE_BYTES + 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBaseline {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attachments: Option<tmt_colab_model::attachment::DocumentAttachments>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creation_recipient: Option<CreationRecipient>,
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
    // The pinned URL_SAFE_NO_PAD engine rejects padding and nonzero trailing bits.
    if bytes.len() > limit {
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
                "creationRecipient" => {
                    !serde_json::from_value::<CreationRecipient>(v.clone()).is_ok_and(|v| v.valid())
                }
                "attachments" => !serde_json::from_value::<
                    tmt_colab_model::attachment::DocumentAttachments,
                >(v.clone())
                .is_ok_and(|list| {
                    tmt_colab_model::attachment::validate_attachment_list(
                        list.as_slice(),
                        tmt_colab_model::attachment::DOCUMENT_ATTACHMENTS,
                        None,
                    )
                    .is_ok()
                }),
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
