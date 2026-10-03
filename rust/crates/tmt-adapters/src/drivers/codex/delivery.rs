//! Terminal one-shot native delivery after exact enrollment evidence.
use super::{
    queue::{MESSAGE_LIMIT, QueueOutcome, QueueRequest},
    record::{Foreground, Record, Store},
    transport::{Client, Endpoint, TransportError},
};
use crate::{
    process::{UnixCommandRunner, runtime::observe_runtime_process},
    runtime::{RuntimeError, channel::ChannelFault},
};
use std::{
    io::Read,
    net::{Ipv4Addr, SocketAddrV4},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{
        BindingEntry,
        session::{BindingSessionState, ProviderSessionId, RuntimeLiveness, RuntimeState},
    },
    driver::{ActionResult, DeliveryAcceptance, SendFailure},
    endpoint::ProcessIncarnation,
};

const PREPARATION_BUDGET: Duration = Duration::from_secs(3);
const DELIVERY_BUDGET: Duration = Duration::from_secs(3);
pub(super) const MAXIMUM_SEND_DURATION: Duration =
    PREPARATION_BUDGET.saturating_add(DELIVERY_BUDGET);

type Sent = ActionResult<DeliveryAcceptance, SendFailure<RuntimeError>>;
fn denied(fault: ChannelFault) -> Sent {
    ActionResult::Failed(SendFailure::Denied(RuntimeError::Channel(fault)))
}
fn uncertain() -> Sent {
    ActionResult::Failed(SendFailure::Uncertain(RuntimeError::Channel(
        ChannelFault::Uncertain,
    )))
}

pub fn enrolled(directory: &Path, binding: &str) -> Result<bool, ChannelFault> {
    Store::at(directory)
        .read(binding)
        .map(|value| value.is_some())
        .map_err(|_| ChannelFault::InvalidRecord)
}

pub fn send(directory: Option<&Path>, entry: &BindingEntry, message: &str) -> Sent {
    send_with_runner(directory, entry, message, &UnixCommandRunner)
}

fn send_with_runner(
    directory: Option<&Path>,
    entry: &BindingEntry,
    message: &str,
    runner: &impl crate::process::CommandRunner,
) -> Sent {
    let Some(binding) = &entry.binding else {
        return denied(ChannelFault::Unverifiable);
    };
    let Some(directory) = directory else {
        return denied(ChannelFault::Unverifiable);
    };
    let store = Store::at(directory);
    let record = match store.read(&binding.id) {
        Ok(Some(record)) => record,
        Ok(None) => return ActionResult::Unsupported,
        Err(_) => return denied(ChannelFault::InvalidRecord),
    };
    if record.attribution.as_ref().is_none_or(|attribution| {
        attribution.identity_id != binding.identity_id
            || !attribution.matches(&binding.server, &binding.pane_id, binding.pane_pid)
    }) {
        return denied(ChannelFault::Mismatch);
    }
    let deadline = Instant::now() + PREPARATION_BUDGET;
    let observe = |process: &ProcessIncarnation| {
        observe_runtime_process(runner, process.pid(), deadline)
            .map(|value| value.matches(process))
            .unwrap_or(RuntimeLiveness::Unknown)
    };
    match applicable(&record, &binding.session, &observe) {
        Ok(false) => return ActionResult::Unsupported,
        Ok(true) => {}
        Err(fault) => return denied(fault),
    }
    if message.len() > MESSAGE_LIMIT {
        return denied(ChannelFault::TooLarge);
    }
    let ready = record
        .ready
        .as_ref()
        .expect("applicable verified readiness");
    let token = match capability(&store, &record) {
        Ok(token) => token,
        Err(fault) => return denied(fault),
    };
    let endpoint = match Endpoint::new(SocketAddrV4::new(Ipv4Addr::LOCALHOST, ready.port), token) {
        Ok(value) => value,
        Err(_) => return denied(ChannelFault::InvalidRecord),
    };
    let request = match ProviderSessionId::new(&ready.thread)
        .ok()
        .and_then(|thread| QueueRequest::new(thread, message).ok())
    {
        Some(value) => value,
        None => return denied(ChannelFault::InvalidRecord),
    };
    let client = match Client::connect(&endpoint, deadline) {
        Ok(client) => client,
        Err(TransportError::Unreachable) => return denied(ChannelFault::Unreachable),
        Err(TransportError::Uncertain) => return uncertain(),
        Err(_) => return denied(ChannelFault::NotReady),
    };
    // Qualification is not a delivery write. Recheck enrollment and process
    // coordinates immediately before consuming the sole queue attempt.
    if !matches!(store.read(&binding.id), Ok(Some(current)) if current == record) {
        return denied(ChannelFault::Mismatch);
    }
    match applicable(&record, &binding.session, &observe) {
        Ok(true) => {}
        Ok(false) => return denied(ChannelFault::Mismatch),
        Err(fault) => return denied(fault),
    }
    if Instant::now() >= deadline {
        return denied(ChannelFault::NotReady);
    }
    // Process qualification owns the preparation budget. Only the verified
    // sole delivery attempt gets a new absolute write/receipt budget.
    match client.queue(&request, Instant::now() + DELIVERY_BUDGET) {
        QueueOutcome::Accepted { .. } => ActionResult::Completed(DeliveryAcceptance::Queued),
        QueueOutcome::Refused { .. } => denied(ChannelFault::Refused),
        QueueOutcome::Uncertain => uncertain(),
    }
}

fn applicable(
    record: &Record,
    current: &BindingSessionState,
    observe: &impl Fn(&ProcessIncarnation) -> RuntimeLiveness,
) -> Result<bool, ChannelFault> {
    let owner = record
        .launch_owner
        .incarnation()
        .ok_or(ChannelFault::InvalidRecord)?;
    let live_owner = observe(&owner);
    if current.launch_owner.as_ref() != Some(&owner) {
        // A live pending enrollment precedes admission/preferences.launched.
        // The earlier binding owner must not make that new opt-in disappear.
        return if record.ended(observe)
            && current
                .launch_owner
                .as_ref()
                .is_some_and(|next| observe(next) == RuntimeLiveness::Alive)
        {
            Ok(false)
        } else {
            Err(ChannelFault::Mismatch)
        };
    }
    match live_owner {
        RuntimeLiveness::Alive => {}
        RuntimeLiveness::Gone => return Err(ChannelFault::Stale),
        RuntimeLiveness::Unknown => return Err(ChannelFault::Unverifiable),
    }
    if current.state != RuntimeState::Running {
        return Err(ChannelFault::NotReady);
    }
    let ready = record.ready.as_ref().ok_or(ChannelFault::NotReady)?;
    let key = current.key.as_ref().ok_or(ChannelFault::NotReady)?;
    if key.provider_session.as_ref().map(ProviderSessionId::as_str) != Some(ready.thread.as_str()) {
        return Err(ChannelFault::Mismatch);
    }
    let server = ready
        .server
        .incarnation()
        .ok_or(ChannelFault::InvalidRecord)?;
    let Foreground::Known(foreground) = &record.foreground else {
        return Err(ChannelFault::Unverifiable);
    };
    if foreground.incarnation().as_ref() != Some(&key.incarnation) {
        return Err(ChannelFault::Mismatch);
    }
    if key.incarnation == server {
        return Err(ChannelFault::Mismatch);
    }
    if observe(&key.incarnation) != RuntimeLiveness::Alive
        || observe(&server) != RuntimeLiveness::Alive
    {
        return Err(ChannelFault::Unverifiable);
    }
    Ok(true)
}

pub(super) fn capability(store: &Store, record: &Record) -> Result<String, ChannelFault> {
    let read = || -> std::io::Result<String> {
        let directory = store.generation_directory(record)?;
        let metadata = std::fs::symlink_metadata(&directory)?;
        if !metadata.is_dir()
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(directory.join("capability"))?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.len() > 512
        {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let mut token = String::new();
        file.take(513).read_to_string(&mut token)?;
        if token.len() > 512 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        Ok(token)
    };
    read().map_err(|_| ChannelFault::InvalidRecord)
}

#[cfg(test)]
mod tests;
