//! Stop and remove one identity: the operations behind `tmt stop`, `tmt rm`,
//! `identity.stop` and `identity.remove`. Callers keep only selection and output.

use super::{Recheck, STOP_ALLOWANCE, StopRefusal, agent_ended, prove, request, stop_failed};
use crate::{
    config::ConfigPaths,
    host::{Host, HostError, Kept, PaneClose, external},
    process::UnixCommandRunner,
    runtime::RuntimeRegistry,
    storage::{IdentityContextSnapshot, Storage, StorageError},
};
use std::time::Instant;
use tmt_core::{
    binding::{
        BindingEndpoint, BindingEntry, BindingError, BindingEvidence, BindingRepository,
        RemoveOutcome, remove_identity, remove_identity_by_id,
    },
    host::HostKind,
    identity::Identity,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOutcome {
    /// The agent was running and exited after the one TERM.
    Stopped,
    /// The recorded agent process had already ended; nothing was signalled.
    AlreadyEnded,
    /// The member has no binding, so no agent is running anywhere tmt knows of.
    NotRunning,
}

/// What closing the member's pane did, once its agent was proven not running.
#[derive(Debug, PartialEq, Eq)]
pub enum PaneOutcome {
    /// tmt opened the pane and it was closed.
    Closed,
    /// The pane was already gone, so there was nothing to close.
    Gone,
    /// The pane stayed open. The agent is still proven stopped.
    Kept(PaneKept),
}

#[derive(Debug, PartialEq, Eq)]
pub enum PaneKept {
    /// The host declined: tmt did not open the pane, or cannot tell or close it.
    Declined(Kept),
    /// The host failed to close it.
    Failed(String),
}

impl PaneKept {
    /// Core's one sentence for why `name`'s pane stayed open.
    pub fn reason(&self, name: &str) -> String {
        let why = match self {
            Self::Declined(Kept::NotOpenedByTmt) => "was left open: TMT did not open that pane",
            Self::Declined(Kept::Unknown) => {
                "was left open: its host cannot say whether TMT opened that pane"
            }
            Self::Declined(Kept::Unsupported) => "was left open: its host cannot close a pane",
            Self::Failed(error) => {
                return format!(
                    "{name}'s agent was stopped, but its pane could not be closed: {error}. Close the pane yourself if you no longer need it."
                );
            }
        };
        format!(
            "{name}'s agent was stopped, but its pane {why}. Close the pane yourself if you no longer need it."
        )
    }
}

#[derive(Debug)]
pub struct Stopped {
    pub outcome: StopOutcome,
    /// Present only when the pane was asked to be closed and the member was bound.
    pub pane: Option<PaneOutcome>,
}

fn context(
    paths: &ConfigPaths,
    identity: &Identity,
) -> Result<IdentityContextSnapshot, StopRefusal> {
    Storage::context_by_identity(&paths.database, &identity.id, 0)
        .map_err(StopRefusal::storage)?
        .filter(|stored| stored.entry.identity == *identity)
        .ok_or_else(|| {
            StopRefusal::new(
                "IDENTITY_CONFLICT",
                "The identity changed before it was stopped.",
            )
        })
}

/// Stops `identity`'s agent with the verified stop. The binding and the
/// remembered session stay, so `tmt resume` works afterwards. With `close`, the
/// pane is closed only after the agent is proven not running, and only if tmt
/// opened it; a kept pane is still a stop.
pub fn stop_identity(
    storage: &mut Storage,
    paths: &ConfigPaths,
    identity: &Identity,
    close: bool,
) -> Result<Stopped, StopRefusal> {
    let captured = context(paths, identity)?;
    let Some(old) = captured.entry.binding.as_ref() else {
        return Ok(Stopped {
            outcome: StopOutcome::NotRunning,
            pane: None,
        });
    };
    let session = captured.preferences.remembered.as_ref().ok_or_else(|| {
        StopRefusal::new(
            "RESUME_UNAVAILABLE",
            "The member has no recorded agent session, so its process cannot be proven; no agent was stopped.",
        )
    })?;
    let registry = RuntimeRegistry::first_party();
    let lifecycle = registry.lifecycle(&session.harness).ok_or_else(|| {
        StopRefusal::new(
            "RESUME_UNSUPPORTED",
            format!(
                "The {} driver is not registered; no agent was stopped.",
                session.harness.as_str()
            ),
        )
    })?;
    let host = Host::for_server(&old.server);
    let source = prove(
        &host,
        &captured,
        old,
        session,
        lifecycle,
        Instant::now() + STOP_ALLOWANCE,
    )?;
    let outcome = if source.agent_alive() {
        let runner = UnixCommandRunner;
        request(
            storage,
            &host,
            &runner,
            &source,
            lifecycle,
            "The member's ownership changed before the stop.",
            |_, _| Ok(Recheck::Holds),
        )?
        .wait()
        .map_err(stop_failed)?;
        StopOutcome::Stopped
    } else {
        StopOutcome::AlreadyEnded
    };
    // Stopped on purpose, so the default built-in restore no longer brings it back.
    // Advisory: the stop already happened and keeps its result.
    if old.server.host == HostKind::pty() {
        let _ = crate::workspace::forget_builtin_member(paths, &identity.id);
    }
    // The pane is closed only after the stop succeeded or the agent had already
    // ended, on a fresh host session so the stop's wait does not eat its budget.
    let pane = if !close {
        None
    } else if matches!(source.evidence, BindingEvidence::EndpointLost) {
        Some(PaneOutcome::Gone)
    } else {
        Some(match host.session().close_opened_pane(&captured.entry) {
            Ok(PaneClose::Closed) => PaneOutcome::Closed,
            Ok(PaneClose::AlreadyGone) => PaneOutcome::Gone,
            Ok(PaneClose::Kept(why)) => PaneOutcome::Kept(PaneKept::Declined(why)),
            Err(error) => PaneOutcome::Kept(PaneKept::Failed(error.to_string())),
        })
    };
    Ok(Stopped { outcome, pane })
}

fn guard(
    entry: &BindingEntry,
    evidence: &BindingEvidence,
) -> Result<(), tmt_core::binding::AgentRunning> {
    agent_ended(entry, evidence, Instant::now() + STOP_ALLOWANCE)
}

/// `tmt rm`: removes the identity `selector` names through `endpoint` (the
/// caller's host session), refusing while its bound agent may still be running.
pub fn remove_by_name<O: BindingEndpoint>(
    storage: &mut Storage,
    endpoint: &mut O,
    selector: &str,
    force: bool,
) -> Result<BindingEntry, BindingError<StorageError, O::Error>> {
    remove_identity(storage, endpoint, selector, force, guard)
}

/// `identity.remove`: removes one exact UUID while its name is `expected_name`.
/// Its binding's own host observes it.
pub fn remove_by_id(
    storage: &mut Storage,
    identity_id: &str,
    expected_name: &str,
) -> Result<RemoveOutcome, BindingError<StorageError, HostError>> {
    let server = storage
        .with_binding_transaction::<_, StorageError>(|records| records.entry_by_id(identity_id))
        .map_err(BindingError::Repository)?
        .and_then(|entry| entry.binding)
        .map(|binding| binding.server);
    let host = match &server {
        Some(server) => Host::for_server(server),
        None => Host::for_host(HostKind::Tmux, UnixCommandRunner, external::approved()),
    };
    let mut endpoint = host.session();
    remove_identity_by_id(
        storage,
        &mut endpoint,
        identity_id,
        expected_name,
        |entry, evidence| {
            // The host was chosen from an earlier read; a binding that moved to
            // another server since was not observed on its own host.
            if entry.binding.as_ref().map(|binding| &binding.server) != server.as_ref()
                && entry.binding.is_some()
            {
                return Err(tmt_core::binding::AgentRunning::HostUnobserved);
            }
            guard(entry, evidence)
        },
    )
}
