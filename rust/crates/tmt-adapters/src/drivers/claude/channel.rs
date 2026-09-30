//! Claude Code Channels (#329): the launch enrollment, the endpoint record and
//! the delivery classification. The wire facts come from the 2.1.285 spike;
//! `contracts/claude-channel-v1.md` owns the behavior and the mapping below.

use crate::{
    process::{
        CommandRequest, CommandRunner, UnixCommandRunner,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::{
        RuntimeError,
        channel::{ChannelError, ChannelFault, ChannelPlan, RuntimeChannel, ServeRequest},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, BufRead, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{BindingEntry, session::RuntimeLiveness},
    driver::{ActionResult, DeliveryAcceptance, SendFailure},
    endpoint::ProcessIncarnation,
    exact_text::MAX_EXCHANGE_TEXT_BYTES,
};

mod server;

/// The only provider build with recorded channel evidence. Widening it needs
/// new evidence and a reviewed change here (see the contract).
pub const SUPPORTED_VERSIONS: &[&str] = &["2.1.285 (Claude Code)"];
/// The MCP server name Claude sees; it is the `server:<name>` channel entry.
pub const SERVER_NAME: &str = "tmt";
pub const MCP_CONFIG_FLAG: &str = "--mcp-config";
pub const CHANNEL_FLAG: &str = "--dangerously-load-development-channels";
pub const CAPABILITY: &str = "claude/channel";
pub const NOTIFICATION_METHOD: &str = "notifications/claude/channel";
/// The protocol version the spike negotiated.
pub const PROTOCOL_VERSION: &str = "2025-11-25";
pub const RECORD_VERSION: u8 = 1;

/// Payload bound: one exchange text plus the generated request framing.
pub(super) const CONTENT_LIMIT: usize = MAX_EXCHANGE_TEXT_BYTES + 64 * 1024;
const RECORD_LIMIT: u64 = 4096;
const REPLY_LIMIT: u64 = 4096;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const VERSION_DEADLINE: Duration = Duration::from_secs(5);
const VERSION_OUTPUT: usize = 4096;
/// How long `send` waits for an opted-in session's channel to become ready
/// before reporting it not ready.
const READINESS_WAIT: Duration = Duration::from_secs(3);
const READINESS_POLL: Duration = Duration::from_millis(50);
const OWNER_PROBE: Duration = Duration::from_secs(1);
/// macOS `sockaddr_un` holds 104 bytes; stay below it (and Linux's 108).
const SOCKET_PATH_LIMIT: usize = 100;

/// A process identity as stored in the record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Process {
    pub pid: u64,
    pub start: String,
}

impl Process {
    pub(super) fn of(incarnation: &ProcessIncarnation) -> Self {
        Self {
            pid: incarnation.pid(),
            start: incarnation.start_identity().to_owned(),
        }
    }

    fn incarnation(&self) -> Option<ProcessIncarnation> {
        ProcessIncarnation::new(self.pid, &self.start).ok()
    }
}

/// The durable enrollment of one launch, written by `tmt run --channel` before
/// the provider starts (`claude: None`, "opted in, channel not ready") and
/// completed by the channel server after the MCP handshake. It is retained per
/// binding and generation, and grants nothing: `send` trusts none of it until it
/// matches the stored binding's launch owner and runtime observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Record {
    pub version: u8,
    pub binding_id: String,
    pub generation: String,
    pub launch_owner: Process,
    pub claude: Option<Process>,
}

/// One ingress connection carries exactly one frame.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Frame {
    pub version: u8,
    pub generation: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    #[serde(default)]
    pub written: bool,
    #[serde(default)]
    pub refused: Option<String>,
}

pub(super) fn record_path(directory: &Path, binding_id: &str) -> PathBuf {
    directory.join(format!("{binding_id}.json"))
}

pub(super) fn socket_path(directory: &Path, binding_id: &str) -> PathBuf {
    directory.join(format!("{binding_id}.sock"))
}

pub(super) fn socket_fits(directory: &Path, binding_id: &str) -> bool {
    socket_path(directory, binding_id).as_os_str().len() <= SOCKET_PATH_LIMIT
}

pub(super) fn write_record(directory: &Path, record: &Record) -> io::Result<()> {
    crate::private_file::replace(
        &record_path(directory, &record.binding_id),
        &serde_json::to_vec(record).map_err(io::Error::other)?,
    )
}

pub struct ClaudeChannel;

impl RuntimeChannel for ClaudeChannel {
    fn preflight(
        &self,
        executable: &OsStr,
        directory: &Path,
        deadline: Instant,
    ) -> Result<(), ChannelError> {
        // A binding ID is a 36-character UUID; check the longest path up front.
        if !directory.is_absolute() || !socket_fits(directory, &"0".repeat(36)) {
            return Err(ChannelError::PathTooLong);
        }
        let deadline = deadline.min(Instant::now() + VERSION_DEADLINE);
        let output = UnixCommandRunner
            .execute(CommandRequest {
                program: executable,
                args: &["--version".into()],
                input: &[],
                deadline,
                max_output_bytes: VERSION_OUTPUT,
            })
            .map_err(|_| ChannelError::ProviderUnavailable)?;
        let found = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if SUPPORTED_VERSIONS.contains(&found.as_str()) {
            Ok(())
        } else {
            Err(ChannelError::ProviderVersion { found })
        }
    }

    fn enroll(&self, plan: &ChannelPlan<'_>) -> Result<Vec<OsString>, ChannelError> {
        // A fresh generation per launch tells a relaunched server from a stale one.
        let generation = uuid::Uuid::new_v4().to_string();
        ensure_private_directory(plan.directory).map_err(|_| ChannelError::Enrollment)?;
        write_record(
            plan.directory,
            &Record {
                version: RECORD_VERSION,
                binding_id: plan.binding_id.to_owned(),
                generation: generation.clone(),
                launch_owner: Process::of(plan.owner),
                claude: None,
            },
        )
        .map_err(|_| ChannelError::Enrollment)?;
        let config = serde_json::json!({
            "mcpServers": {
                SERVER_NAME: {
                    "command": plan.tmt,
                    "args": [
                        "__channel-server",
                        super::NAME,
                        plan.binding_id,
                        generation,
                        plan.directory,
                    ],
                }
            }
        });
        Ok(vec![
            MCP_CONFIG_FLAG.into(),
            config.to_string().into(),
            CHANNEL_FLAG.into(),
            format!("server:{SERVER_NAME}").into(),
        ])
    }

    fn withdraw(&self, directory: &Path, binding_id: &str) {
        let _ = fs::remove_file(record_path(directory, binding_id));
        let _ = fs::remove_file(socket_path(directory, binding_id));
    }

    fn serve(
        &self,
        request: &ServeRequest<'_>,
        input: Box<dyn BufRead + Send>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        server::serve(request, input, output)
    }
}

fn directory_is_private(directory: &Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    fs::symlink_metadata(directory).is_ok_and(|metadata| {
        metadata.is_dir()
            && metadata.uid() == nix::unistd::geteuid().as_raw()
            && metadata.permissions().mode() & 0o077 == 0
    })
}

/// The endpoint directory is created owner-only and must stay so: a record or
/// socket another user could write is no evidence.
pub(super) fn ensure_private_directory(directory: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the channel directory must be an owner-only directory",
        ));
    }
    Ok(())
}

type Sent = ActionResult<DeliveryAcceptance, SendFailure<RuntimeError>>;

fn denied(fault: ChannelFault) -> Sent {
    ActionResult::Failed(SendFailure::Denied(RuntimeError::Channel(fault)))
}

fn uncertain() -> Sent {
    ActionResult::Failed(SendFailure::Uncertain(RuntimeError::Channel(
        ChannelFault::Uncertain,
    )))
}

/// Delivery through an opted-in session's channel, classified per the contract.
/// Only a session that never opted in (no record) leaves this driver for paste.
/// Every opted-in outcome is terminal: this driver never returns `NotSent`.
pub(super) fn send(directory: &Path, entry: &BindingEntry, message: &str) -> Sent {
    send_within(directory, entry, message, READINESS_WAIT)
}

fn send_within(directory: &Path, entry: &BindingEntry, message: &str, wait: Duration) -> Sent {
    let Some(binding) = &entry.binding else {
        return ActionResult::Unsupported;
    };
    let record = match read_record(directory, &binding.id) {
        Ok(Some(record)) => record,
        // Enrollment is written before the provider starts, so no record means
        // this session never opted in.
        Ok(None) => return ActionResult::Unsupported,
        Err(fault) => return denied(fault),
    };
    if !directory_is_private(directory) {
        return denied(ChannelFault::InvalidRecord);
    }
    if record.version != RECORD_VERSION || record.binding_id != binding.id {
        return denied(ChannelFault::Mismatch);
    }
    // The enrollment belongs to one `tmt run`. Once that launch owner is
    // conclusively gone, the enrollment is over and the session is not opted in.
    let Some(owner) = record.launch_owner.incarnation() else {
        return denied(ChannelFault::InvalidRecord);
    };
    match observe_runtime_process(
        &UnixCommandRunner,
        owner.pid(),
        Instant::now() + OWNER_PROBE,
    ) {
        Ok(observed) => match observed.matches(&owner) {
            RuntimeLiveness::Alive => {}
            // A launch that ended leaves an enrollment, not evidence that the
            // session never opted in: only a new launch supersedes it.
            RuntimeLiveness::Gone => return denied(ChannelFault::Stale),
            RuntimeLiveness::Unknown => return denied(ChannelFault::Unverifiable),
        },
        Err(_) => return denied(ChannelFault::Unverifiable),
    }
    if binding.session.launch_owner.as_ref() != Some(&owner) {
        return denied(ChannelFault::Mismatch);
    }
    let record = match await_ready(directory, &binding.id, record, wait) {
        Ok(record) => record,
        Err(result) => return result,
    };
    let claude = record.claude.as_ref().and_then(Process::incarnation);
    let observed = binding.session.key.as_ref().map(|key| &key.incarnation);
    if claude.is_none() || claude.as_ref() != observed {
        return denied(ChannelFault::Mismatch);
    }
    if message.len() > CONTENT_LIMIT {
        return denied(ChannelFault::TooLarge);
    }
    let mut stream = match UnixStream::connect(socket_path(directory, &binding.id)) {
        Ok(stream) => stream,
        // No byte moved, but the session is opted in: a missing or refusing
        // endpoint is reported, never turned into paste.
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return denied(ChannelFault::Unreachable);
        }
        Err(_) => return denied(ChannelFault::Refused),
    };
    exchange(&mut stream, &record.generation, message)
}

/// A ready record (the server completed the MCP handshake), waiting a bounded
/// time for one that is still starting. Failing to become ready is terminal and
/// never a paste: a failed handshake cannot be told from a session sitting at
/// Claude's own trust or consent prompt, where pasted input could answer it.
/// Nothing was sent to the channel, and that is all `NotReady` claims.
fn await_ready(
    directory: &Path,
    binding_id: &str,
    mut record: Record,
    wait: Duration,
) -> Result<Record, Sent> {
    let deadline = Instant::now() + wait;
    let generation = record.generation.clone();
    while record.claude.is_none() {
        if Instant::now() >= deadline {
            return Err(denied(ChannelFault::NotReady));
        }
        std::thread::sleep(READINESS_POLL.min(deadline.saturating_duration_since(Instant::now())));
        record = match read_record(directory, binding_id) {
            Ok(Some(next)) if next.generation == generation => next,
            // Replaced, removed or unreadable while waiting: not the launch that
            // was verified, and no evidence for a fallback.
            Ok(_) => return Err(denied(ChannelFault::Mismatch)),
            Err(fault) => return Err(denied(fault)),
        };
    }
    Ok(record)
}

/// Everything after the connection exists is uncertain unless the endpoint
/// answers: a completed write has no provider receipt, and a failure may still
/// have reached Claude.
fn exchange(stream: &mut UnixStream, generation: &str, message: &str) -> Sent {
    let frame = Frame {
        version: RECORD_VERSION,
        generation: generation.to_owned(),
        content: message.to_owned(),
    };
    let Ok(mut bytes) = serde_json::to_vec(&frame) else {
        return uncertain();
    };
    bytes.push(b'\n');
    if stream.set_write_timeout(Some(IO_TIMEOUT)).is_err()
        || stream.set_read_timeout(Some(IO_TIMEOUT)).is_err()
        || stream.write_all(&bytes).is_err()
        || stream.flush().is_err()
    {
        return uncertain();
    }
    let mut line = Vec::new();
    let read = io::BufReader::new(stream.take(REPLY_LIMIT)).read_until(b'\n', &mut line);
    let Ok(_) = read else {
        return uncertain();
    };
    match serde_json::from_slice::<Reply>(&line) {
        Ok(Reply { written: true, .. }) => {
            ActionResult::Completed(DeliveryAcceptance::Unacknowledged)
        }
        // The endpoint answered that it did not hand the frame to Claude.
        Ok(Reply {
            refused: Some(_), ..
        }) => denied(ChannelFault::Refused),
        _ => uncertain(),
    }
}

/// `Ok(None)` means no record exists (not opted in); an unreadable or invalid
/// one is an error, never a reason to assume the session did not opt in.
pub(super) fn read_record(
    directory: &Path,
    binding_id: &str,
) -> Result<Option<Record>, ChannelFault> {
    let path = record_path(directory, binding_id);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ChannelFault::InvalidRecord),
    };
    if !metadata.file_type().is_file() {
        return Err(ChannelFault::InvalidRecord);
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| file.take(RECORD_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|_| ChannelFault::InvalidRecord)?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err(ChannelFault::InvalidRecord);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| ChannelFault::InvalidRecord)
}

/// The Claude process the channel server belongs to: its parent, observed live.
pub(super) fn parent_incarnation(deadline: Instant) -> Option<ProcessIncarnation> {
    let parent = u64::try_from(nix::unistd::getppid().as_raw()).ok()?;
    match observe_runtime_process(&UnixCommandRunner, parent, deadline) {
        Ok(ProcessObservation::Live(incarnation)) => Some(incarnation),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
