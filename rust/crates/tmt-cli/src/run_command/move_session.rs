//! Explicit move coordinates one verified stop and the existing binding owner.
//! No writer transaction spans the stop wait or the new foreground process.

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    output::Failure,
    verified_stop::{self, Recheck, refusal, stop_failed},
};
use std::time::{Duration, Instant};
use tmt_adapters::{
    config::ConfigPaths,
    host::{Host, OperationOptions},
    process::{
        UnixCommandRunner,
        runtime::{observe_runtime_process, observe_start},
    },
    runtime::lifecycle::RuntimeLifecycle,
    storage::{IdentityContextSnapshot, Storage},
};
use tmt_core::{
    binding::{
        self, BindingEndpoint, BindingTargetEvidence, BoundIdentity, GuardedBindingEndpoint,
        session::SessionPreferences,
    },
    identity::Identity,
};

pub(super) fn launch_failure(
    mut error: Failure,
    name: &str,
    effect: super::LaunchEffect,
) -> Failure {
    let command = match effect {
        super::LaunchEffect::NotLaunched => {
            error.message.push_str(" No new agent was launched.");
            format!("tmt resume --move -- {}", crate::output::shell_word(name))
        }
        super::LaunchEffect::OldStopped => {
            error
                .message
                .push_str(" The old agent was stopped. No new agent was launched.");
            return error.suggestion(format!(
                "To resume in this pane, run: tmt resume -- {}",
                crate::output::shell_word(name)
            ));
        }
        super::LaunchEffect::OldEnded => {
            error
                .message
                .push_str(" The old agent has ended. No new agent was launched.");
            format!("tmt resume --move -- {}", crate::output::shell_word(name))
        }
        super::LaunchEffect::BoundHere => {
            error
                .message
                .push_str(" The identity is now bound here. No new agent was launched.");
            format!("tmt resume -- {}", crate::output::shell_word(name))
        }
        super::LaunchEffect::Launched => {
            error
                .message
                .push_str(" A new agent was launched; inspect its pane before trying again.");
            format!("tmt resume -- {}", crate::output::shell_word(name))
        }
    };
    error.suggestion(format!("After resolving this refusal, retry: {command}"))
}

fn context(paths: &ConfigPaths, identity: &Identity) -> Result<IdentityContextSnapshot, Failure> {
    Storage::context_by_identity(&paths.database, &identity.id, 0)
        .map_err(super::storage_failure)?
        .filter(|stored| stored.entry.identity == *identity)
        .ok_or_else(|| refusal("IDENTITY_CONFLICT", "The identity changed before moving."))
}

type Handoff<'a> = &'a mut dyn FnMut(&tmt_core::binding::Binding) -> Result<(), Failure>;

pub(super) struct MoveRequest<'a> {
    pub identity: &'a Identity,
    pub preferences: &'a SessionPreferences,
    pub lifecycle: &'a dyn RuntimeLifecycle,
    pub switching: bool,
    pub handoff: Option<Handoff<'a>>,
}

pub(super) fn move_binding(
    storage: &mut Storage,
    host: &Host,
    paths: &ConfigPaths,
    pane: &str,
    request: MoveRequest<'_>,
    effect: &mut super::LaunchEffect,
) -> Result<Option<BoundIdentity>, Failure> {
    let MoveRequest {
        identity,
        preferences,
        lifecycle,
        switching,
        handoff,
    } = request;
    let session = preferences.remembered.as_ref().ok_or_else(|| {
        refusal(
            "RESUME_UNAVAILABLE",
            "There is no remembered session to move.",
        )
    })?;
    let captured = context(paths, identity)?;
    if captured.preferences != *preferences {
        return Err(refusal(
            "IDENTITY_CONFLICT",
            "The session preferences changed before moving.",
        ));
    }
    let Some(old) = captured.entry.binding.as_ref() else {
        return Ok(None);
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    let snapshot = host
        .snapshot(OperationOptions {
            deadline: Some(deadline),
            pane_ids: Some(&[pane.into()]),
        })
        .map_err(endpoint_failure)?;
    let destination = snapshot
        .panes
        .iter()
        .find(|candidate| candidate.id == pane)
        .ok_or_else(|| refusal("RUNTIME_CONFLICT", "The destination pane is unavailable."))?;
    if old.server.host == snapshot.server.host
        && old.server.socket_path == snapshot.server.socket_path
        && old.pane_id == pane
    {
        if switching {
            if let Some(key) = &old.session.key {
                let observed =
                    observe_runtime_process(&UnixCommandRunner, key.incarnation.pid(), deadline)
                        .map_err(|error| refusal("PROCESS_ERROR", error))?;
                super::resume::check_process(&identity.name, pane, &key.incarnation, &observed)?;
            }
            return Ok(None);
        }
        return Err(refusal(
            "RUNTIME_CONFLICT",
            "The identity is already bound to this pane; use ordinary resume here.",
        ));
    }
    if destination.marker.is_some() {
        return Err(refusal(
            "RUNTIME_CONFLICT",
            "The destination pane has an ownership marker.",
        ));
    }
    let mut target = BindingTargetEvidence {
        server: snapshot.server.clone(),
        pane_id: pane.into(),
        pane_pid: destination.pane_pid,
        pane_incarnation: Some(
            observe_start(&UnixCommandRunner, destination.pane_pid, deadline)
                .map_err(|error| refusal("PROCESS_ERROR", error))?
                .ok_or_else(|| {
                    refusal(
                        "RUNTIME_CONFLICT",
                        "The destination process could not be verified.",
                    )
                })?,
        ),
        discovery: None,
    };
    if target.server.host == tmt_core::host::HostKind::Tmux {
        // Ordinary tmux read snapshots intentionally omit pane starts. Reuse the
        // guarded binding session to retain and re-observe the native fence;
        // never weaken target_pane to accept a missing incarnation.
        let mut endpoint = host.session();
        endpoint.begin_coordination();
        let fence = endpoint
            .discovery_target(pane)
            .map_err(endpoint_failure)?
            .filter(|fence| {
                fence.host == target.server.host
                    && fence.socket == target.server.socket_path
                    && fence.server.pid() == target.server.server_pid
                    && fence.server_start_time == target.server.server_start_time
                    && fence
                        .server_id
                        .as_ref()
                        .is_none_or(|id| id == &target.server.server_id)
                    && fence.process.pid() == target.pane_pid
                    && Some(fence.process.start_identity()) == target.pane_incarnation.as_deref()
                    && fence.marker.is_none()
            })
            .ok_or_else(|| {
                refusal(
                    "RUNTIME_CONFLICT",
                    "The destination's native ownership could not be pinned.",
                )
            })?;
        target.discovery = Some(fence);
    }
    let source = verified_stop::prove(host, &captured, old, session, lifecycle, deadline)?;
    if source.agent_alive()
        && let Some(handoff) = handoff
    {
        handoff(old)?;
        // The request may admit prompt/Stop observations. Recapture and re-prove
        // every endpoint and preference after it, without ever asking twice.
        let refreshed = context(paths, identity)?;
        let same = refreshed.entry.binding.as_ref().is_some_and(|current| {
            current.id == old.id
                && current.session.key == old.session.key
                && current.session.launch_owner == old.session.launch_owner
        });
        if !same {
            return Err(refusal(
                "IDENTITY_CONFLICT",
                "The old attachment changed during handoff; no stop was requested.",
            ));
        }
        return move_binding(
            storage,
            host,
            paths,
            pane,
            MoveRequest {
                identity,
                preferences: &refreshed.preferences,
                lifecycle,
                switching,
                handoff: None,
            },
            effect,
        );
    }
    let runner = UnixCommandRunner;
    // Recheck both endpoints and all captured records under the same writer
    // that serializes replacement bindings. Only the TERM request runs here.
    let requested = verified_stop::request(
        storage,
        host,
        &runner,
        &source,
        lifecycle,
        "The source or destination ownership changed before the stop.",
        |records, stop_deadline| {
            if records
                .entry_by_pane(target.server.host, pane, &target.server.server_id)?
                .is_some()
            {
                return Ok(Recheck::RecordsChanged);
            }
            let target_probe = host.observe_snapshot(OperationOptions {
                deadline: Some(stop_deadline),
                pane_ids: Some(&[pane.into()]),
            });
            let target_matches = target_probe.as_ref().is_ok_and(|snapshot| {
                snapshot.server == target.server
                    && snapshot.panes.iter().any(|candidate| {
                        candidate.id == pane
                            && candidate.pane_pid == target.pane_pid
                            && candidate.marker.is_none()
                    })
            });
            let target_start_matches = observe_start(&runner, target.pane_pid, stop_deadline)
                .is_ok_and(|start| start == target.pane_incarnation);
            Ok(if target_matches && target_start_matches {
                Recheck::Holds
            } else {
                Recheck::OwnershipChanged
            })
        },
    )?;
    requested.wait().map_err(stop_failed)?;
    *effect = super::LaunchEffect::OldStopped;
    let refreshed = context(paths, identity)?;
    let same_attachment = refreshed.entry.binding.as_ref().is_some_and(|binding| {
        binding.id == old.id
            && binding.server == old.server
            && binding.pane_id == old.pane_id
            && binding.pane_pid == old.pane_pid
            && binding.pane_incarnation == old.pane_incarnation
            && binding.session.key == old.session.key
            && binding.session.launch_owner == old.session.launch_owner
    });
    let same_session = refreshed
        .preferences
        .remembered
        .as_ref()
        .is_some_and(|remembered| {
            remembered.harness == session.harness
                && remembered.mode == session.mode
                && remembered.provider_session == session.provider_session
        });
    let same_choices = refreshed.preferences.channel == captured.preferences.channel
        && refreshed.preferences.preferred_harness == captured.preferences.preferred_harness
        && refreshed
            .preferences
            .remembered
            .as_ref()
            .and_then(|remembered| remembered.state.as_ref())
            .and_then(|state| lifecycle.state_model(state))
            == session
                .state
                .as_ref()
                .and_then(|state| lifecycle.state_model(state));
    if !same_attachment || !same_session || !same_choices {
        return Err(refusal(
            "IDENTITY_CONFLICT",
            "The old binding or remembered session changed after the stop.",
        ));
    }
    let mut endpoint = host.session();
    if let Some(fence) = &target.discovery {
        endpoint = endpoint.with_discovery_target(fence.clone());
    }
    binding::move_identity(
        storage,
        &mut endpoint,
        &refreshed.entry,
        &refreshed.preferences,
        &source.key.incarnation,
        &target,
    )
    .map(Some)
    .map_err(binding_failure)
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core("tmt resume --move -- {}", &[""], &[("{}", "worker")]),
    crate::cli_style_tests::HintSpec::core("tmt resume -- {}", &[""], &[("{}", "worker")]),
    crate::cli_style_tests::HintSpec::core(
        "To resume in this pane, run: tmt resume -- {}",
        &[""],
        &[("{}", "worker")],
    ),
];
