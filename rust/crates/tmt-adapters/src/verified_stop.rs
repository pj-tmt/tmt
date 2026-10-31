//! The one verified stop of a bound member's agent, shared by `tmt stop`, by a
//! move (`resume --move`, a driver switch) and by the extension API. The agent is
//! stopped only when it is a private main process proven by PID and start under a
//! verified pane ownership, with one TERM, a 3 s allowance, no KILL and no retry.
//! A shared server, a nested runtime, the pane process or unknown evidence refuses
//! before anything is signalled. The remove guard beside it never deletes an
//! identity whose agent is alive or not proven ended.

mod identity;

pub use identity::{
    PaneKept, PaneOutcome, StopOutcome, Stopped, remove_by_id, remove_by_name, stop_identity,
};

use crate::{
    host::{Host, HostError, OperationOptions},
    process::{
        UnixCommandRunner,
        runtime::{
            ProcessObservation, RuntimeStop, StopRuntimeError, observe_runtime_process,
            request_runtime_stop,
        },
    },
    runtime::lifecycle::RuntimeLifecycle,
    storage::{IdentityContextSnapshot, Storage, StorageError},
};
use std::time::{Duration, Instant};
use tmt_core::binding::{
    AgentRunning, Binding, BindingEntry, BindingEvidence, BindingRecords, BindingRepository,
    evaluate_binding,
    session::{ObservedSessionKey, RememberedSession, RuntimeLiveness},
};

pub const STOP_ALLOWANCE: Duration = Duration::from_secs(3);

/// Why a verified stop did not prove the agent stopped. Every refusal means the
/// agent may still be running.
#[derive(Debug)]
pub struct StopRefusal {
    pub code: &'static str,
    pub reason: String,
    /// A TERM was, or may have been, sent.
    pub signalled: bool,
    pub cause: Option<Box<StopCause>>,
}

#[derive(Debug)]
pub enum StopCause {
    Host(HostError),
    Storage(StorageError),
}

impl StopRefusal {
    pub fn new(code: &'static str, reason: impl std::fmt::Display) -> Self {
        Self {
            code,
            reason: reason.to_string(),
            signalled: false,
            cause: None,
        }
    }

    fn host(error: HostError) -> Self {
        let code = if error.socket_permission_denied() {
            "TMUX_PERMISSION_DENIED"
        } else {
            "RECONCILIATION_FAILED"
        };
        Self {
            code,
            reason: error.to_string(),
            signalled: false,
            cause: Some(Box::new(StopCause::Host(error))),
        }
    }

    fn storage(error: StorageError) -> Self {
        Self {
            code: "IDENTITY_ERROR",
            reason: "Could not access member state.".into(),
            signalled: false,
            cause: Some(Box::new(StopCause::Storage(error))),
        }
    }

    fn process(error: StopRuntimeError, signalled: bool) -> Self {
        Self {
            signalled,
            ..Self::new(
                "PROCESS_ERROR",
                format!("{error}. Exit the old agent in its pane before retrying."),
            )
        }
    }
}

/// The TERM was sent, but the agent was not seen to exit.
pub fn stop_failed(error: StopRuntimeError) -> StopRefusal {
    StopRefusal::process(error, true)
}

/// Requesting the TERM failed; only a failed `kill` command may have sent it.
fn request_failed(error: StopRuntimeError) -> StopRefusal {
    let signalled = matches!(error, StopRuntimeError::Command(_));
    StopRefusal::process(error, signalled)
}

/// What the caller's own recheck found, inside the stop's writer transaction.
pub enum Recheck {
    Holds,
    /// A record the caller captured changed.
    RecordsChanged,
    /// What the caller observed (a move's destination) changed.
    OwnershipChanged,
}

/// What was proven about the member's current attachment: its binding, the exact
/// recorded agent process and what a fresh observation of both showed.
pub struct Source<'a> {
    pub captured: &'a IdentityContextSnapshot,
    pub old: &'a Binding,
    pub key: &'a ObservedSessionKey,
    pub observed: ProcessObservation,
    pub evidence: BindingEvidence,
}

impl Source<'_> {
    pub fn agent_alive(&self) -> bool {
        self.observed.matches(&self.key.incarnation) == RuntimeLiveness::Alive
    }
}

/// Observes the binding and its recorded agent and refuses everything a stop could
/// not safely act on. An agent that already ended is proven too: nothing is signalled.
pub fn prove<'a>(
    host: &Host,
    captured: &'a IdentityContextSnapshot,
    old: &'a Binding,
    session: &RememberedSession,
    lifecycle: &dyn RuntimeLifecycle,
    deadline: Instant,
) -> Result<Source<'a>, StopRefusal> {
    let probe = host
        .probe(
            &old.server,
            OperationOptions {
                deadline: Some(deadline),
                pane_ids: Some(std::slice::from_ref(&old.pane_id)),
            },
        )
        .map_err(StopRefusal::host)?;
    let evidence = evaluate_binding(&captured.entry, &probe);
    if !matches!(
        evidence,
        BindingEvidence::Active(_) | BindingEvidence::EndpointLost
    ) {
        return Err(StopRefusal::new(
            "RUNTIME_CONFLICT",
            "The old binding's ownership could not be verified.",
        ));
    }
    let key = old
        .session
        .key
        .as_ref()
        .filter(|key| key.provider_session.as_ref() == Some(&session.provider_session))
        .ok_or_else(|| {
            StopRefusal::new(
                "RESUME_UNAVAILABLE",
                "The old binding has no exact recorded agent session.",
            )
        })?;
    let observed = observe_runtime_process(&UnixCommandRunner, key.incarnation.pid(), deadline)
        .map_err(|error| StopRefusal::new("PROCESS_ERROR", error))?;
    match observed.matches(&key.incarnation) {
        RuntimeLiveness::Alive => {
            if key.incarnation.pid() == old.pane_pid {
                return Err(StopRefusal::new(
                    "RUNTIME_CONFLICT",
                    "The agent is the pane process; stopping it would close the pane.",
                ));
            }
            if !matches!(evidence, BindingEvidence::Active(_)) {
                return Err(StopRefusal::new(
                    "RUNTIME_CONFLICT",
                    "The old agent is alive but its pane ownership is no longer verified.",
                ));
            }
            if lifecycle
                .movable_process(&UnixCommandRunner, old, &captured.preferences, deadline)
                .as_ref()
                != Some(&key.incarnation)
            {
                return Err(StopRefusal::new(
                    "RESUME_UNSUPPORTED",
                    format!(
                        "The {} driver cannot prove a private main foreground for this {} runtime; shared/channel servers are not stopped.",
                        session.harness.as_str(),
                        session.mode.as_str()
                    ),
                ));
            }
        }
        RuntimeLiveness::Unknown => {
            return Err(StopRefusal::new(
                "RUNTIME_CONFLICT",
                "The old agent is suspended or its process could not be verified.",
            ));
        }
        RuntimeLiveness::Gone => (),
    }
    Ok(Source {
        captured,
        old,
        key,
        observed,
        evidence,
    })
}

/// Rechecks every captured record and the source under the writer that serializes
/// replacement bindings, then sends the one TERM. Waiting belongs outside the
/// writer, on the returned handle. `extra` rechecks what only the caller knows
/// (a move's destination) in the same transaction; `changed` is the message when
/// the ownership it or the source observed no longer matches.
pub fn request<'r>(
    storage: &mut Storage,
    host: &Host,
    runner: &'r UnixCommandRunner,
    source: &Source<'_>,
    lifecycle: &dyn RuntimeLifecycle,
    changed: &'static str,
    extra: impl FnOnce(
        &dyn BindingRecords<Error = StorageError>,
        Instant,
    ) -> Result<Recheck, StorageError>,
) -> Result<RuntimeStop<'r, UnixCommandRunner>, StopRefusal> {
    let (old, captured) = (source.old, source.captured);
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            if records.entry_by_id(&captured.entry.identity.id)?.as_ref() != Some(&captured.entry)
                || records.session_preferences(&captured.entry.identity.id)? != captured.preferences
            {
                return Ok(None);
            }
            let stop_deadline = Instant::now() + STOP_ALLOWANCE;
            match extra(records, stop_deadline)? {
                Recheck::Holds => (),
                Recheck::RecordsChanged => return Ok(None),
                Recheck::OwnershipChanged => {
                    return Ok(Some(Err(StopRefusal::new("RUNTIME_CONFLICT", changed))));
                }
            }
            let source_probe = host.probe(
                &old.server,
                OperationOptions {
                    deadline: Some(stop_deadline),
                    pane_ids: Some(std::slice::from_ref(&old.pane_id)),
                },
            );
            let source_matches = source_probe.as_ref().is_ok_and(|probe| {
                matches!(
                    (evaluate_binding(&captured.entry, probe), &source.evidence),
                    (BindingEvidence::Active(_), BindingEvidence::Active(_))
                        | (BindingEvidence::EndpointLost, BindingEvidence::EndpointLost)
                )
            });
            if !source_matches {
                return Ok(Some(Err(StopRefusal::new("RUNTIME_CONFLICT", changed))));
            }
            if source.agent_alive()
                && lifecycle
                    .movable_process(runner, old, &captured.preferences, stop_deadline)
                    .as_ref()
                    != Some(&source.key.incarnation)
            {
                return Ok(Some(Err(StopRefusal::new(
                    "RUNTIME_CONFLICT",
                    "The old agent's exclusive foreground proof changed before the stop.",
                ))));
            }
            Ok(Some(
                request_runtime_stop(runner, &source.key.incarnation, stop_deadline)
                    .map_err(request_failed),
            ))
        })
        .map_err(StopRefusal::storage)?
        .ok_or_else(|| {
            StopRefusal::new(
                "IDENTITY_CONFLICT",
                "A binding or its session preferences changed before the stop.",
            )
        })?
}

/// The remove guard: `Ok` when the identity has no binding, no recorded agent
/// process (a hand-bound pane removes as before), or its recorded agent process
/// is proven gone.
fn agent_ended(
    entry: &BindingEntry,
    evidence: &BindingEvidence,
    deadline: Instant,
) -> Result<(), AgentRunning> {
    let Some(binding) = &entry.binding else {
        return Ok(());
    };
    if *evidence == BindingEvidence::Unknown {
        return Err(AgentRunning::HostUnobserved);
    }
    let Some(key) = &binding.session.key else {
        return Ok(());
    };
    match observe_runtime_process(&UnixCommandRunner, key.incarnation.pid(), deadline)
        .map(|observed| observed.matches(&key.incarnation))
    {
        Ok(RuntimeLiveness::Gone) => Ok(()),
        Ok(RuntimeLiveness::Alive) => Err(AgentRunning::Alive),
        Ok(RuntimeLiveness::Unknown) | Err(_) => Err(AgentRunning::Unproven),
    }
}
