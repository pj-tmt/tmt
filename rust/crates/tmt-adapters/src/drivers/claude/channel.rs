//! Claude Code Channels (#329): the endpoint record and the delivery
//! classification. The wire facts come from the 2.1.285 spike;
//! `contracts/claude-channel-v1.md` owns the behavior and the mapping below.

use crate::{
    process::{UnixCommandRunner, runtime::observe_runtime_process},
    runtime::{RuntimeError, channel::ChannelFault},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
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

const RECORD_VERSION: u8 = 1;
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

/// A process identity as stored in the record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Process {
    pub pid: u64,
    pub start: String,
}

impl Process {
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

fn record_path(directory: &Path, binding_id: &str) -> PathBuf {
    directory.join(format!("{binding_id}.json"))
}

fn socket_path(directory: &Path, binding_id: &str) -> PathBuf {
    directory.join(format!("{binding_id}.sock"))
}

fn directory_is_private(directory: &Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    fs::symlink_metadata(directory).is_ok_and(|metadata| {
        metadata.is_dir()
            && metadata.uid() == nix::unistd::geteuid().as_raw()
            && metadata.permissions().mode() & 0o077 == 0
    })
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
            // The launch ended. That alone is not evidence that the session never
            // opted in, nor that a different launch is current.
            RuntimeLiveness::Gone => return denied(ChannelFault::Stale),
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
    Record {
        claude: None,
        ..reread.clone()
    } == Record {
        claude: None,
        ..verified.clone()
    }
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
    let Ok(line) = read_line_until(stream, deadline) else {
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
fn remaining(deadline: Instant) -> io::Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        Err(io::ErrorKind::TimedOut.into())
    } else {
        Ok(left)
    }
}

fn write_until(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
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

/// One reply line, at most `REPLY_LIMIT` bytes, read before the deadline. The end
/// of the stream also ends the line, so a reply cut short is judged as it is.
fn read_line_until(stream: &mut UnixStream, deadline: Instant) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut chunk = [0u8; 256];
    while (line.len() as u64) < REPLY_LIMIT {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        let room = chunk.len().min((REPLY_LIMIT as usize) - line.len());
        match stream.read(&mut chunk[..room]) {
            Ok(0) => break,
            Ok(count) => {
                line.extend_from_slice(&chunk[..count]);
                if let Some(end) = line.iter().position(|byte| *byte == b'\n') {
                    line.truncate(end);
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
