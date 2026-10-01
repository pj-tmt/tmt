//! Claude Code Channels (#329): the endpoint record and the delivery
//! classification. The wire facts come from the 2.1.285 spike;
//! `contracts/claude-channel-v1.md` owns the behavior and the mapping below.

use crate::{
    process::{
        CommandRequest, CommandRunner, UnixCommandRunner,
        ps::query_ps,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::{
        RuntimeCommand, RuntimeError,
        channel::{
            ChannelEnrollment, ChannelError, ChannelFault, ChannelPlan, EvidenceError, PaneAddress,
            PaneEvidence, RuntimeChannel, ServeRequest,
        },
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
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

/// The oldest provider build with recorded channel evidence. A build is accepted
/// when it is this one or newer within the same major line; whether a newer build
/// really speaks the channel is decided by the handshake itself (a session that
/// never becomes ready is terminal and never pasted to). Lowering the minimum or
/// accepting another major line needs new evidence and a reviewed change here.
pub const MINIMUM_VERSION: &str = "2.1.285";
/// The builds on which the channel was exercised against the real provider; any
/// other accepted build launches with an advisory that names it.
pub const TESTED_VERSIONS: &[&str] = &["2.1.285"];
/// Every `--version` line of the product ends with this.
const PRODUCT_SUFFIX: &str = " (Claude Code)";
/// The MCP server name Claude sees; it is the `server:<name>` channel entry.
pub const SERVER_NAME: &str = "tmt";
pub const MCP_CONFIG_FLAG: &str = "--mcp-config";
pub const CHANNEL_FLAG: &str = "--dangerously-load-development-channels";
pub const CAPABILITY: &str = "claude/channel";
pub const NOTIFICATION_METHOD: &str = "notifications/claude/channel";
/// The protocol version the spike negotiated.
pub const PROTOCOL_VERSION: &str = "2025-11-25";
pub(super) const RECORD_VERSION: u8 = 1;
/// Payload bound: one exchange text plus the generated request framing.
pub(super) const CONTENT_LIMIT: usize = MAX_EXCHANGE_TEXT_BYTES + 64 * 1024;
const RECORD_LIMIT: u64 = 4096;
const REPLY_LIMIT: u64 = 4096;
/// The whole frame exchange, write and reply together.
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(2);
/// How long `send` waits for an opted-in session's channel to become ready
/// before reporting it not ready.
const READINESS_WAIT: Duration = Duration::from_secs(3);
const READINESS_POLL: Duration = Duration::from_millis(50);
const OWNER_PROBE: Duration = Duration::from_secs(1);
const VERSION_DEADLINE: Duration = Duration::from_secs(5);
const VERSION_OUTPUT: usize = 4096;
/// macOS `sockaddr_un` holds 104 bytes; stay below it (and Linux's 108).
const SOCKET_PATH_LIMIT: usize = 100;
/// One lock file serializes every mutation of a record or socket in the channel
/// directory (see "Enrollment ownership and serialization" in the contract).
const LOCK_FILE: &str = ".lock";
const LOCK_WAIT: Duration = Duration::from_secs(2);
const LOCK_POLL: Duration = Duration::from_millis(10);

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

/// The durable enrollment of one launch (`claude: None` is "opted in, channel
/// not ready"; the channel server sets it after the MCP handshake). It is
/// retained per binding and generation, and grants nothing: `send` trusts none
/// of it until it matches the stored binding's launch owner and runtime
/// observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Record {
    pub version: u8,
    pub binding_id: String,
    /// The identity the launch ran as, persisted before spawn. Absent only in a
    /// record written without attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_id: Option<String>,
    pub generation: String,
    pub launch_owner: Process,
    /// The pane the launch runs in, persisted before spawn from the binding the
    /// launcher holds, so the record can be matched to the pane after the binding
    /// is gone. A record without it cannot be attributed to any pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PaneRecord>,
    /// The foreground child the launcher spawned and observed (`foreground_started`).
    /// Absent until then, which is "unknown", never "ended".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<Process>,
    pub claude: Option<Process>,
}

/// A pane as an enrollment names it: the full server incarnation, the pane ID on
/// it and the pane's process, so a reused pane ID on another server never matches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct PaneRecord {
    pub host: String,
    pub server_id: String,
    pub socket_path: String,
    pub server_pid: u64,
    pub server_start_time: String,
    pub pane_id: String,
    pub pane_pid: u64,
}

impl PaneRecord {
    /// `None` when the address cannot identify a pane later.
    fn of(pane: &PaneAddress<'_>) -> Option<Self> {
        let server = pane.server;
        let complete = !server.server_id.is_empty()
            && !server.socket_path.is_empty()
            && server.server_pid > 0
            && !server.server_start_time.is_empty()
            && !pane.pane_id.is_empty()
            && pane.pane_pid > 0;
        complete.then(|| Self {
            host: server.host.as_str().to_owned(),
            server_id: server.server_id.clone(),
            socket_path: server.socket_path.clone(),
            server_pid: server.server_pid,
            server_start_time: server.server_start_time.clone(),
            pane_id: pane.pane_id.to_owned(),
            pane_pid: pane.pane_pid,
        })
    }

    fn is(&self, pane: &PaneAddress<'_>) -> bool {
        Self::of(pane).as_ref() == Some(self)
    }
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

fn record_path(directory: &Path, binding_id: &str) -> PathBuf {
    directory.join(format!("{binding_id}.json"))
}

fn socket_path(directory: &Path, binding_id: &str) -> PathBuf {
    directory.join(format!("{binding_id}.sock"))
}

fn socket_fits(directory: &Path, binding_id: &str) -> bool {
    socket_path(directory, binding_id).as_os_str().len() <= SOCKET_PATH_LIMIT
}

fn write_record(directory: &Path, record: &Record) -> io::Result<()> {
    crate::private_file::replace(
        &record_path(directory, &record.binding_id),
        &serde_json::to_vec(record).map_err(io::Error::other)?,
    )
}

/// Runs `action` while holding the channel directory lock, retrying a busy lock
/// for a bounded time. A lock that cannot be taken is an error and the action
/// does not run: every caller fails closed.
fn locked<T>(directory: &Path, action: impl FnOnce() -> T) -> io::Result<T> {
    locked_within(directory, LOCK_WAIT, action)
}

fn locked_within<T>(directory: &Path, wait: Duration, action: impl FnOnce() -> T) -> io::Result<T> {
    let deadline = Instant::now() + wait;
    loop {
        match crate::file_lock::exclusive(&directory.join(LOCK_FILE)) {
            Ok(guard) => {
                let value = action();
                drop(guard);
                return Ok(value);
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(LOCK_POLL);
            }
            Err(error) => return Err(error),
        }
    }
}

pub struct ClaudeChannel;

impl RuntimeChannel for ClaudeChannel {
    fn preflight(
        &self,
        executable: &OsStr,
        directory: &Path,
        deadline: Instant,
    ) -> Result<Option<String>, ChannelError> {
        check_provider(&UnixCommandRunner, executable, directory, deadline)
    }

    fn enroll(&self, plan: &ChannelPlan<'_>) -> Result<Box<dyn ChannelEnrollment>, ChannelError> {
        // A command line that already names a development channel cannot be
        // planned around: ours would be ambiguous with it.
        if plan
            .command
            .args
            .iter()
            .any(|argument| argument == CHANNEL_FLAG)
        {
            return Err(ChannelError::UnsupportedArguments(
                "it already passes --dangerously-load-development-channels",
            ));
        }
        if !socket_fits(plan.directory, plan.binding_id) {
            return Err(ChannelError::PathTooLong);
        }
        let Some(pane) = PaneRecord::of(&plan.pane) else {
            return Err(ChannelError::Unattributed);
        };
        ensure_private_directory(plan.directory).map_err(|_| ChannelError::Enrollment)?;
        let owner = Process::of(plan.owner);
        // A fresh generation per launch tells a relaunched server from a stale one.
        let generation = uuid::Uuid::new_v4().to_string();
        locked(plan.directory, || {
            // The newest launch of a binding replaces an earlier enrollment only
            // when that one is positively over (see `may_take_over`), or it is this
            // very launch. Anything alive, different or unverifiable stays
            // untouched and refuses this launch.
            match read_record(plan.directory, plan.binding_id) {
                Ok(None) => {}
                Ok(Some(old)) if old.launch_owner == owner => {}
                Ok(Some(old)) if may_take_over(&old, &pane) => {}
                Ok(Some(_)) | Err(_) => return Err(ChannelError::Occupied),
            }
            prune_ended(plan.directory, plan.binding_id);
            write_record(
                plan.directory,
                &Record {
                    version: RECORD_VERSION,
                    binding_id: plan.binding_id.to_owned(),
                    identity_id: Some(plan.identity_id.to_owned()),
                    generation: generation.clone(),
                    launch_owner: owner.clone(),
                    pane: Some(pane.clone()),
                    foreground: None,
                    claude: None,
                },
            )
            .map_err(|_| ChannelError::Enrollment)
        })
        .map_err(|_| ChannelError::Enrollment)??;
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
        let mut command = plan.command.clone();
        command.args.extend([
            MCP_CONFIG_FLAG.into(),
            config.to_string().into(),
            CHANNEL_FLAG.into(),
            format!("server:{SERVER_NAME}").into(),
        ]);
        Ok(Box::new(Lease {
            command,
            directory: plan.directory.to_owned(),
            binding_id: plan.binding_id.to_owned(),
            generation,
            owner,
        }))
    }

    fn enrolled(&self, directory: &Path, binding_id: &str) -> Result<bool, ChannelFault> {
        read_record(directory, binding_id).map(|record| record.is_some())
    }

    fn enrolled_in_pane(
        &self,
        directory: &Path,
        pane: &PaneAddress<'_>,
        binding_id: Option<&str>,
        deadline: Instant,
    ) -> Result<PaneEvidence, EvidenceError> {
        pane_enrolled(&UnixCommandRunner, directory, pane, binding_id, deadline)
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

/// How a provider build stands against the recorded evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStatus {
    /// The channel was exercised on exactly this build.
    Tested,
    /// Accepted by the range rule only; its handshake is the compatibility check.
    Untested,
}

/// Parses a whole `--version` line as `<major>.<minor>.<patch> (Claude Code)` with
/// canonical decimal numbers; anything else is not a version line.
fn parse_build(line: &str) -> Option<([u32; 3], &str)> {
    let number = line.strip_suffix(PRODUCT_SUFFIX)?;
    let mut parts = number.split('.');
    let mut parsed = [0; 3];
    for slot in &mut parsed {
        let part = parts.next()?;
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some((parsed, number))
}

/// The status of a provider's `--version` line, or `None` when it is not a
/// version line, is older than [`MINIMUM_VERSION`] or is another major line.
pub fn build_status(line: &str) -> Option<BuildStatus> {
    let (minimum, _) = parse_build(&format!("{MINIMUM_VERSION}{PRODUCT_SUFFIX}"))
        .expect("the minimum is a canonical version");
    let (build, number) = parse_build(line)?;
    if build[0] != minimum[0] || build < minimum {
        return None;
    }
    Some(if TESTED_VERSIONS.contains(&number) {
        BuildStatus::Tested
    } else {
        BuildStatus::Untested
    })
}

/// The pre-launch check, single-shot: the directory must fit a socket path, and
/// the provider's bounded `--version` must be a build the range rule accepts. An
/// accepted build that was never tested comes back with an advisory naming it.
fn check_provider(
    runner: &dyn CommandRunner,
    executable: &OsStr,
    directory: &Path,
    deadline: Instant,
) -> Result<Option<String>, ChannelError> {
    // A binding ID is a 36-character UUID; check the longest path up front.
    if !directory.is_absolute() || !socket_fits(directory, &"0".repeat(36)) {
        return Err(ChannelError::PathTooLong);
    }
    let deadline = deadline.min(Instant::now() + VERSION_DEADLINE);
    let output = runner
        .execute(CommandRequest {
            program: executable,
            args: &["--version".into()],
            input: &[],
            deadline,
            max_output_bytes: VERSION_OUTPUT,
        })
        .map_err(|_| ChannelError::ProviderUnavailable)?;
    let found = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    match build_status(&found) {
        Some(BuildStatus::Tested) => Ok(None),
        Some(BuildStatus::Untested) => Ok(Some(format!(
            "Claude Code {} has not been tested with message channels (tested: {}). Channel delivery depends on its handshake completing; if it never does, requests to this session fail as not ready and nothing is pasted.",
            found.strip_suffix(PRODUCT_SUFFIX).unwrap_or(&found),
            TESTED_VERSIONS.join(", ")
        ))),
        None => Err(ChannelError::ProviderVersion { found }),
    }
}

/// One launch's enrollment: it knows exactly which record it wrote, and only
/// ever removes that one.
struct Lease {
    command: RuntimeCommand,
    directory: PathBuf,
    binding_id: String,
    generation: String,
    owner: Process,
}

impl ChannelEnrollment for Lease {
    fn command(&self) -> &RuntimeCommand {
        &self.command
    }

    fn environment(&self) -> &[(OsString, OsString)] {
        &[]
    }

    /// Records the spawned foreground in this launch's enrollment, under the
    /// directory lock and only while the record still carries exactly this
    /// launch's generation and owner.
    fn foreground_started(&mut self, foreground: &ProcessIncarnation) -> Result<(), ChannelError> {
        let process = Process::of(foreground);
        locked(&self.directory, || {
            match read_record(&self.directory, &self.binding_id) {
                Ok(Some(mut record))
                    if record.generation == self.generation
                        && record.launch_owner == self.owner =>
                {
                    record.foreground = Some(process);
                    write_record(&self.directory, &record).map_err(|_| ChannelError::Enrollment)
                }
                _ => Err(ChannelError::Enrollment),
            }
        })
        .map_err(|_| ChannelError::Enrollment)?
    }

    /// Removes the record and socket only if the record still carries exactly
    /// this launch's generation and owner, under the directory lock, and no
    /// process it names as the provider can still be running. A replacement
    /// enrollment, or a record this launch never wrote, is left alone, and so is
    /// everything when the lock cannot be taken. The launcher calls this only for
    /// a launch that never spawned or whose child was confirmed reaped.
    fn withdraw(self: Box<Self>) {
        let _ = locked(&self.directory, || {
            let ours = matches!(
                read_record(&self.directory, &self.binding_id),
                Ok(Some(record)) if record.generation == self.generation
                    && record.launch_owner == self.owner
                    && record.claude.as_ref().is_none_or(is_gone)
            );
            if ours {
                let _ = fs::remove_file(record_path(&self.directory, &self.binding_id));
                let _ = fs::remove_file(socket_path(&self.directory, &self.binding_id));
            }
        });
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
fn ensure_private_directory(directory: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    if directory_is_private(directory) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the channel directory must be an owner-only directory",
        ))
    }
}

/// The Claude process the channel server belongs to: its parent, observed live.
fn parent_incarnation(deadline: Instant) -> Option<ProcessIncarnation> {
    let parent = u64::try_from(nix::unistd::getppid().as_raw()).ok()?;
    match observe_runtime_process(&UnixCommandRunner, parent, deadline) {
        Ok(ProcessObservation::Live(incarnation)) => Some(incarnation),
        _ => None,
    }
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
/// `directory` is the channel directory, or `None` when TMT's configuration could
/// not be discovered. Only a session that never opted in, or whose enrollment
/// belongs to a different launch that is positively proven current, leaves this
/// driver for the baseline transport. Every other outcome is terminal: this
/// driver never returns `NotSent`.
pub(super) fn send(directory: Option<&Path>, entry: &BindingEntry, message: &str) -> Sent {
    send_within(directory, entry, message, READINESS_WAIT)
}

fn send_within(
    directory: Option<&Path>,
    entry: &BindingEntry,
    message: &str,
    wait: Duration,
) -> Sent {
    let Some(binding) = &entry.binding else {
        return ActionResult::Unsupported;
    };
    // Without the configuration there is no way to tell whether this session
    // opted in, so it can neither be sent to nor pasted to.
    let Some(directory) = directory else {
        return denied(ChannelFault::Unverifiable);
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
    let Some(owner) = record.launch_owner.incarnation() else {
        return denied(ChannelFault::InvalidRecord);
    };
    // An enrollment applies only to the exact launch that created it.
    match binding.session.launch_owner.as_ref() {
        Some(current) if *current == owner => match liveness(&owner) {
            RuntimeLiveness::Alive => {}
            RuntimeLiveness::Gone => {
                // A plain relaunch outside `tmt run` leaves the binding's launch
                // owner at the old one. If the runtime now observed for the binding
                // is positively alive and is not the Claude this enrollment names,
                // the enrollment is stale for it: baseline, record untouched. That
                // needs a valid recorded Claude to differ from: a record that never
                // named one (the launch owner died before the handshake) cannot
                // tell the original child, which may still run, from a new one.
                let recorded = record.claude.as_ref().and_then(Process::incarnation);
                let running = binding.session.key.as_ref().map(|key| &key.incarnation);
                if let (Some(recorded), Some(running)) = (&recorded, running)
                    && running != recorded
                    && liveness(running) == RuntimeLiveness::Alive
                {
                    return ActionResult::Unsupported;
                }
                // Otherwise the launch ended. That alone is not evidence that the
                // session never opted in, nor that a different launch is current.
                return denied(ChannelFault::Stale);
            }
            RuntimeLiveness::Unknown => return denied(ChannelFault::Unverifiable),
        },
        // A different launch, positively proven current (its owner is observed
        // live with a matching start identity), makes the old record
        // non-applicable: baseline delivery, record untouched.
        Some(current) if liveness(current) == RuntimeLiveness::Alive => {
            return ActionResult::Unsupported;
        }
        // A different launch that cannot be proven current, or none recorded.
        _ => return denied(ChannelFault::Mismatch),
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

/// A whole-system `ps` snapshot of pid and parent only (used to prune).
const PS_SNAPSHOT_LIMIT: usize = 4 * 1024 * 1024;
/// Other enrollments one `enroll` looks at when it prunes ended launches.
const PRUNE_EXAMINED: usize = 64;

/// Whether a recorded process is conclusively gone (exact incarnation, not a
/// reused pid); anything unverifiable is not.
fn is_gone(process: &Process) -> bool {
    process
        .incarnation()
        .is_some_and(|incarnation| liveness(&incarnation) == RuntimeLiveness::Gone)
}

/// A new launch of a binding replaces the earlier enrollment only when nothing the
/// old record names can still be running: its owner and every recorded process
/// (foreground, provider) are conclusively gone. When nothing but the owner was
/// ever recorded (the launcher died before it published the foreground) nothing
/// proves where the foreground went, and only an explicit relaunch in the very
/// pane the record names may replace it. A live or unverifiable process, or such a
/// record for another or an unnamed pane, refuses the launch.
fn may_take_over(old: &Record, pane: &PaneRecord) -> bool {
    if !is_gone(&old.launch_owner) {
        return false;
    }
    let recorded: Vec<&Process> = old.foreground.iter().chain(old.claude.iter()).collect();
    if recorded.is_empty() {
        return old.pane.as_ref() == Some(pane);
    }
    recorded.into_iter().all(is_gone)
}

/// Quotes a path for a shell command that is printed for the user to run.
fn shell_quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

/// The recovery a user runs after verifying that nothing of the launch is left.
fn recovery(directory: &Path, binding_id: &str) -> String {
    format!(
        "rm -- {} {}",
        shell_quoted(&record_path(directory, binding_id)),
        shell_quoted(&socket_path(directory, binding_id)),
    )
}

/// Whether an enrollment attributed to `pane` is live, or unconfirmed because its
/// foreground was never recorded. It reads this driver's records, matches each to
/// the pane through the address it persisted at enroll (never through a stored
/// binding, which observation may have deleted) and observes only that record's
/// own exact incarnations: the launch owner, the foreground, the provider. A
/// record that recorded a foreground or the provider and whose processes are all
/// gone has ended and is not evidence; one that recorded neither stays unknown
/// (terminal for this pane) even when its launcher is gone, because nothing proves
/// where the agent it may have started went. A record
/// that cannot be attributed (unreadable, older, naming no pane) never blocks an
/// unrelated pane and is reported in `skipped`, unless its file is named for
/// `binding_id`, the binding being delivered to, which makes it that binding's
/// own invalid evidence. Evidence about this pane that cannot be told is an error.
/// The whole work is bounded by the one `deadline` (the directory read, every
/// record read and each observation); there is no record-count cap, a leftover
/// costs one small read, and with no records the cost is one directory read.
fn pane_enrolled<R: CommandRunner>(
    runner: &R,
    directory: &Path,
    pane: &PaneAddress<'_>,
    binding_id: Option<&str>,
    deadline: Instant,
) -> Result<PaneEvidence, EvidenceError> {
    let mut evidence = PaneEvidence::default();
    let unverifiable = || EvidenceError::at(ChannelFault::Unverifiable, directory);
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(evidence),
        Err(_) => return Err(unverifiable()),
    };
    for entry in entries {
        if Instant::now() >= deadline {
            return Err(unverifiable());
        }
        let entry = entry.map_err(|_| unverifiable())?;
        let name = entry.file_name();
        // This driver's namespace is exactly `<binding-id>.json`; sockets, the lock
        // and every other driver's files are never opened.
        let Some(stem) = name.to_str().and_then(owned_record_stem) else {
            continue;
        };
        let path = entry.path();
        let record = match read_record(directory, stem) {
            Ok(Some(record)) if record.version == RECORD_VERSION && record.binding_id == stem => {
                record
            }
            // Withdrawn between the listing and the read.
            Ok(None) => continue,
            Ok(Some(_)) | Err(_) => {
                if binding_id == Some(stem) {
                    return Err(EvidenceError::at(ChannelFault::InvalidRecord, &path)
                        .with_detail(format!(
                            "The enrollment record of this pane's own binding is unreadable. After confirming that the session it belonged to is gone, remove it with: {}.",
                            recovery(directory, stem)
                        )));
                }
                evidence.skipped.push(path);
                continue;
            }
        };
        let Some(recorded) = &record.pane else {
            evidence.skipped.push(path);
            continue;
        };
        if !recorded.is(pane) {
            continue;
        }
        let processes = [
            Some(&record.launch_owner),
            record.foreground.as_ref(),
            record.claude.as_ref(),
        ];
        for process in processes.into_iter().flatten() {
            match observe_runtime_process(runner, process.pid, deadline) {
                Ok(ProcessObservation::Live(seen) | ProcessObservation::Stopped(seen)) => {
                    if Process::of(&seen) == *process {
                        evidence.enrolled = true;
                        return Ok(evidence);
                    }
                }
                Ok(ProcessObservation::Gone) => {}
                _ => {
                    return Err(
                        EvidenceError::at(ChannelFault::Unverifiable, &path).with_detail(format!(
                            "Process {} of this pane's enrollment could not be observed. Retry, or after confirming that it is gone remove the enrollment with: {}.",
                            process.pid,
                            recovery(directory, stem)
                        )),
                    );
                }
            }
        }
        if record.foreground.is_none() && record.claude.is_none() {
            // The launcher died before it recorded the foreground: nothing proves
            // that the agent it may have started is gone.
            return Err(
                EvidenceError::at(ChannelFault::Unverifiable, &path).with_detail(format!(
                    "A `tmt run --channel` launch in pane {} on the tmux server at {} ended before it recorded its agent process, so that agent cannot be proven gone. After confirming that no agent started by launch owner {} (started {}) still runs in that pane, remove its enrollment with: {}. Nothing is pasted until then.",
                    recorded.pane_id,
                    recorded.socket_path,
                    record.launch_owner.pid,
                    record.launch_owner.start,
                    recovery(directory, stem)
                )),
            );
        }
    }
    Ok(evidence)
}

/// This driver's enrollment records: exactly `<binding-id>.json`.
fn owned_record_stem(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".json")?;
    (stem.len() == 36 && uuid::Uuid::parse_str(stem).is_ok()).then_some(stem)
}

/// Every process and its parent from one bounded snapshot.
fn process_parents<R: CommandRunner>(
    runner: &R,
    deadline: Instant,
) -> Result<HashMap<u64, u64>, ChannelFault> {
    let output = query_ps(
        runner,
        &["-A".into(), "-o".into(), "pid=,ppid=".into()],
        deadline,
        PS_SNAPSHOT_LIMIT,
    )
    .map_err(|_| ChannelFault::Unverifiable)?;
    let text = std::str::from_utf8(&output.stdout).map_err(|_| ChannelFault::Unverifiable)?;
    let mut parents = HashMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let (Some(pid), Some(parent), None) = (fields.next(), fields.next(), fields.next()) else {
            return Err(ChannelFault::Unverifiable);
        };
        let (Ok(pid), Ok(parent)) = (pid.parse::<u64>(), parent.parse::<u64>()) else {
            return Err(ChannelFault::Unverifiable);
        };
        if parents.insert(pid, parent).is_some() {
            return Err(ChannelFault::Unverifiable);
        }
    }
    Ok(parents)
}

/// Removes the enrollment (record and socket) of launches that are over in every
/// way the record can show: it recorded a foreground or a provider, and the launch
/// owner and every recorded process are absent from one process snapshot. A record
/// that never recorded either (the unconfirmed case), has a live or unobservable
/// process, is unreadable, or is not this driver's is left alone, as is the binding
/// being enrolled. It runs under the directory lock, looks at a bounded number of
/// records, and never blocks an enrollment.
fn prune_ended(directory: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut candidates = Vec::new();
    for entry in entries.flatten().take(PRUNE_EXAMINED * 4) {
        let name = entry.file_name();
        let Some(binding_id) = name.to_str().and_then(owned_record_stem) else {
            continue;
        };
        if binding_id == keep {
            continue;
        }
        if let Ok(Some(record)) = read_record(directory, binding_id)
            && record.version == RECORD_VERSION
            && record.binding_id == binding_id
            && (record.foreground.is_some() || record.claude.is_some())
        {
            candidates.push(record);
        }
        if candidates.len() == PRUNE_EXAMINED {
            break;
        }
    }
    if candidates.is_empty() {
        return;
    }
    let Ok(alive) = process_parents(&UnixCommandRunner, Instant::now() + OWNER_PROBE) else {
        return;
    };
    for record in candidates {
        let absent = |process: &Process| !alive.contains_key(&process.pid);
        if absent(&record.launch_owner)
            && record.foreground.as_ref().is_none_or(absent)
            && record.claude.as_ref().is_none_or(absent)
        {
            let _ = fs::remove_file(record_path(directory, &record.binding_id));
            let _ = fs::remove_file(socket_path(directory, &record.binding_id));
        }
    }
}

/// Whether the process is the recorded incarnation, observed within a bound.
fn liveness(process: &ProcessIncarnation) -> RuntimeLiveness {
    match observe_runtime_process(
        &UnixCommandRunner,
        process.pid(),
        Instant::now() + OWNER_PROBE,
    ) {
        Ok(observed) => observed.matches(process),
        Err(_) => RuntimeLiveness::Unknown,
    }
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
    let verified = record.clone();
    while record.claude.is_none() {
        if Instant::now() >= deadline {
            return Err(denied(ChannelFault::NotReady));
        }
        std::thread::sleep(READINESS_POLL.min(deadline.saturating_duration_since(Instant::now())));
        record = match read_record(directory, binding_id) {
            Ok(Some(next)) if same_enrollment(&verified, &next) => next,
            // Replaced, changed, removed or unreadable while waiting: not the
            // enrollment that was verified, and no evidence for a fallback.
            Ok(_) => return Err(denied(ChannelFault::Mismatch)),
            Err(fault) => return Err(denied(fault)),
        };
    }
    Ok(record)
}

/// Whether a reread is the enrollment that was verified: everything but the
/// readiness (`claude`), which the server completes, is the enrollment's identity.
fn same_enrollment(verified: &Record, reread: &Record) -> bool {
    let settled = |record: &Record| Record {
        claude: None,
        foreground: None,
        ..record.clone()
    };
    settled(reread) == settled(verified)
}

/// Everything after the connection exists is uncertain unless the endpoint
/// answers: a completed write has no provider receipt, and a failure may still
/// have reached Claude. One deadline bounds the whole exchange, enforced at each
/// underlying read and write, so a slow or trickling endpoint cannot hold the
/// caller beyond it.
fn exchange(stream: &mut UnixStream, generation: &str, message: &str) -> Sent {
    exchange_within(stream, generation, message, EXCHANGE_DEADLINE)
}

fn exchange_within(
    stream: &mut UnixStream,
    generation: &str,
    message: &str,
    limit: Duration,
) -> Sent {
    let deadline = Instant::now() + limit;
    let frame = Frame {
        version: RECORD_VERSION,
        generation: generation.to_owned(),
        content: message.to_owned(),
    };
    let Ok(mut bytes) = serde_json::to_vec(&frame) else {
        return uncertain();
    };
    bytes.push(b'\n');
    if write_until(stream, &bytes, deadline).is_err() {
        return uncertain();
    }
    let Ok(line) = read_line_until(stream, deadline, REPLY_LIMIT) else {
        return uncertain();
    };
    match serde_json::from_slice::<Reply>(&line) {
        // Only an unambiguous answer decides: a reply that claims both a write
        // and a refusal, or neither, is not one.
        Ok(Reply {
            written: true,
            refused: None,
        }) => ActionResult::Completed(DeliveryAcceptance::Unacknowledged),
        // The endpoint answered that it did not hand the frame to Claude.
        Ok(Reply {
            written: false,
            refused: Some(_),
        }) => denied(ChannelFault::Refused),
        _ => uncertain(),
    }
}

/// The time left, or `TimedOut` once the deadline has passed. A zero timeout is
/// not a valid socket timeout, so an expired deadline never reaches the socket.
pub(super) fn remaining(deadline: Instant) -> io::Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(left)
    }
}

pub(super) fn write_until(
    stream: &mut UnixStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// One line (newline included), at most `limit` bytes, read before the deadline.
/// The end of the stream also ends the line, so one cut short is judged as it is.
pub(super) fn read_line_until(
    stream: &mut UnixStream,
    deadline: Instant,
    limit: u64,
) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut chunk = [0u8; 8192];
    while (line.len() as u64) < limit {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        let room = chunk.len().min((limit as usize) - line.len());
        match stream.read(&mut chunk[..room]) {
            Ok(0) => break,
            Ok(count) => {
                line.extend_from_slice(&chunk[..count]);
                if let Some(end) = line.iter().position(|byte| *byte == b'\n') {
                    line.truncate(end + 1);
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(line)
}

/// `Ok(None)` means no record exists (not opted in); an unreadable or invalid
/// one is an error, never a reason to assume the session did not opt in.
fn read_record(directory: &Path, binding_id: &str) -> Result<Option<Record>, ChannelFault> {
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

#[cfg(test)]
mod tests;
