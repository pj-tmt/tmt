//! The one verified stop of a bound member's agent, shared by `tmt stop` and by a
//! move (`resume --move`, a driver switch). The agent is stopped only when it is a
//! private main process proven by PID and start under a verified pane ownership,
//! with one TERM, a 3 s allowance, no KILL and no retry. A shared server, a nested
//! runtime, the pane process or unknown evidence refuses before anything is signalled.

use crate::{binding_error::endpoint_failure, output::Failure};
use std::time::{Duration, Instant};
use tmt_adapters::{
    host::{Host, OperationOptions},
    process::{
        UnixCommandRunner,
        runtime::{ProcessObservation, RuntimeStop, observe_runtime_process, request_runtime_stop},
    },
    runtime::lifecycle::RuntimeLifecycle,
    storage::{IdentityContextSnapshot, Storage, StorageError},
};
use tmt_core::binding::{
    Binding, BindingEvidence, BindingRecords, BindingRepository, evaluate_binding,
    session::{ObservedSessionKey, RememberedSession, RuntimeLiveness},
};

pub(crate) const STOP_ALLOWANCE: Duration = Duration::from_secs(3);

pub(crate) fn refusal(code: impl Into<String>, reason: impl std::fmt::Display) -> Failure {
    let code = code.into();
    let status = if matches!(code.as_str(), "RUNTIME_CONFLICT" | "IDENTITY_CONFLICT") {
        5
    } else {
        1
    };
    Failure::new(code, reason.to_string(), status)
}

pub(crate) fn stop_failed(error: impl std::fmt::Display) -> Failure {
    refusal(
        "PROCESS_ERROR",
        format!("{error}. Exit the old agent in its pane before retrying."),
    )
}

fn storage_failure(error: StorageError) -> Failure {
    Failure::new("IDENTITY_ERROR", "Could not access member state.", 1).caused_by(error)
}

/// What the caller's own recheck found, inside the stop's writer transaction.
pub(crate) enum Recheck {
    Holds,
    /// A record the caller captured changed.
    RecordsChanged,
    /// What the caller observed (a move's destination) changed.
    OwnershipChanged,
}

/// What was proven about the member's current attachment: its binding, the exact
/// recorded agent process and what a fresh observation of both showed.
pub(crate) struct Source<'a> {
    pub captured: &'a IdentityContextSnapshot,
    pub old: &'a Binding,
    pub key: &'a ObservedSessionKey,
    pub observed: ProcessObservation,
    pub evidence: BindingEvidence,
}

impl Source<'_> {
    pub(crate) fn agent_alive(&self) -> bool {
        self.observed.matches(&self.key.incarnation) == RuntimeLiveness::Alive
    }
}

/// Observes the binding and its recorded agent and refuses everything a stop could
/// not safely act on. An agent that already ended is proven too: nothing is signalled.
pub(crate) fn prove<'a>(
    host: &Host,
    captured: &'a IdentityContextSnapshot,
    old: &'a Binding,
    session: &RememberedSession,
    lifecycle: &dyn RuntimeLifecycle,
    deadline: Instant,
) -> Result<Source<'a>, Failure> {
    let probe = host
        .probe(
            &old.server,
            OperationOptions {
                deadline: Some(deadline),
                pane_ids: Some(std::slice::from_ref(&old.pane_id)),
            },
        )
        .map_err(endpoint_failure)?;
    let evidence = evaluate_binding(&captured.entry, &probe);
    if !matches!(
        evidence,
        BindingEvidence::Active(_) | BindingEvidence::EndpointLost
    ) {
        return Err(refusal(
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
            refusal(
                "RESUME_UNAVAILABLE",
                "The old binding has no exact recorded agent session.",
            )
        })?;
    let observed = observe_runtime_process(&UnixCommandRunner, key.incarnation.pid(), deadline)
        .map_err(|error| refusal("PROCESS_ERROR", error))?;
    match observed.matches(&key.incarnation) {
        RuntimeLiveness::Alive => {
            if key.incarnation.pid() == old.pane_pid {
                return Err(refusal(
                    "RUNTIME_CONFLICT",
                    "The agent is the pane process; stopping it would close the pane.",
                ));
            }
            if !matches!(evidence, BindingEvidence::Active(_)) {
                return Err(refusal(
                    "RUNTIME_CONFLICT",
                    "The old agent is alive but its pane ownership is no longer verified.",
                ));
            }
            if lifecycle
                .movable_process(&UnixCommandRunner, old, &captured.preferences, deadline)
                .as_ref()
                != Some(&key.incarnation)
            {
                return Err(refusal(
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
            return Err(refusal(
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
pub(crate) fn request<'r>(
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
) -> Result<RuntimeStop<'r, UnixCommandRunner>, Failure> {
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
                    return Ok(Some(Err(refusal("RUNTIME_CONFLICT", changed))));
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
                return Ok(Some(Err(refusal("RUNTIME_CONFLICT", changed))));
            }
            if source.agent_alive()
                && lifecycle
                    .movable_process(runner, old, &captured.preferences, stop_deadline)
                    .as_ref()
                    != Some(&source.key.incarnation)
            {
                return Ok(Some(Err(refusal(
                    "RUNTIME_CONFLICT",
                    "The old agent's exclusive foreground proof changed before the stop.",
                ))));
            }
            Ok(Some(
                request_runtime_stop(runner, &source.key.incarnation, stop_deadline)
                    .map_err(stop_failed),
            ))
        })
        .map_err(storage_failure)?
        .ok_or_else(|| {
            refusal(
                "IDENTITY_CONFLICT",
                "A binding or its session preferences changed before the stop.",
            )
        })?
}
