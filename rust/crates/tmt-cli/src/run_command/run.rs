//! Bound foreground launch and completion retain one owned child and storage lifetime.

use super::{
    RunRequest,
    channel::{HeldLease, admitted_key, claim_is_current, preflight, verify_claim},
    diagnostic, incarnation, observe, observe_admission,
    resume::{mark_resume_pending, select_command, settle_resume},
    storage_failure,
};
use crate::{
    binding_error::{binding_failure, endpoint_failure},
    output::Failure,
};
use std::time::{Duration, Instant};
use tmt_adapters::{
    config::ConfigPaths,
    drivers::Registry,
    host::{Host, PaneCosmetics},
    process::{
        UnixCommandRunner,
        interactive::InteractiveChild,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::{
        RuntimeRegistry, StateReconciliation,
        channel::{ChannelPlan, PaneAddress},
        lifecycle::{NoLifecycle, RuntimeLifecycle},
    },
    setup::start_hook_installed,
    storage::{Storage, StorageError},
};
use tmt_core::{
    binding::{
        self, Binding, BindingRepository,
        session::{ObservedSessionKey, RuntimeLiveness, SessionPreferences, SessionTransition},
    },
    driver::{HookEvent, HookObserver},
    endpoint::ProcessIncarnation,
    identity::IdentityReader,
    names::normalize_name,
    settings::PaneBadge,
};

pub(super) fn run_bound(
    storage: &mut Storage,
    paths: &ConfigPaths,
    host: &Host,
    pane: &str,
    request: RunRequest<'_>,
    badge: PaneBadge,
    observer: &mut impl HookObserver,
) -> Result<u8, Failure> {
    let RunRequest {
        name,
        command,
        resume,
        save,
        channel,
    } = request;
    let mut registry = RuntimeRegistry::first_party();
    // Only the resume path purges or reconciles, and it reports each change once.
    if resume.is_some() {
        let registered = registry.harnesses().collect::<Vec<_>>();
        for purged in storage
            .purge_unregistered_sessions(&registered)
            .map_err(storage_failure)?
        {
            diagnostic(&format!(
                "forgot {}'s remembered {} session; that driver is no longer registered.",
                purged.name, purged.harness
            ));
        }
    }
    let existing = storage
        .find_identity(&normalize_name(name))
        .map_err(storage_failure)?;
    // A bare registered executable is a convenient fresh launch only when no
    // identity holds that token. Explicit name + command keeps its old meaning.
    let auto_harness = if resume.is_none()
        && tmt_core::driver::ALL
            .iter()
            .any(|driver| driver.executables.contains(&name))
    {
        registry.claim(std::ffi::OsStr::new(name))
    } else {
        None
    };
    if auto_harness.is_some()
        && existing.is_some()
        && (command.is_empty() || command[0].as_encoded_bytes().starts_with(b"-"))
    {
        return Err(Failure::new(
            "RUN_NAME_COLLISION",
            format!(
                "'{name}' is both an identity and a registered command. Use `tmt run {name} {name}` to launch that identity, or `tmt run <new-name> {name}` for a new one."
            ),
            5,
        ));
    }
    let auto_named = auto_harness.is_some() && existing.is_none();
    let generated;
    let unnamed_command;
    let (name, command) = if auto_named {
        generated = tmt_core::identity::automatic_name(auto_harness.as_ref().unwrap().as_str());
        unnamed_command = std::iter::once(std::ffi::OsString::from(name))
            .chain(command.iter().cloned())
            .collect::<Vec<_>>();
        (generated.as_str(), unnamed_command.as_slice())
    } else {
        (name, command)
    };
    if command
        .first()
        .is_some_and(|word| word.as_encoded_bytes().starts_with(b"-"))
    {
        return Err(Failure::new(
            "USAGE_ERROR",
            "The command must not start with '-'. Put TMT options before the name.",
            1,
        ));
    }
    let preferences = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let Some(identity) = &existing else {
                return Ok(SessionPreferences::default());
            };
            let mut preferences = records.session_preferences(&identity.id)?;
            if resume.is_some()
                && let Some(StateReconciliation::DiscardedState { harness, version }) =
                    registry.reconcile(&mut preferences)
            {
                records.set_session_preferences(&identity.id, &preferences)?;
                diagnostic(&format!(
                    "discarded {name}'s unreadable {} resume details (version {version}); resuming with the provider's defaults.",
                    harness.as_str()
                ));
            }
            Ok(preferences)
        })
        .map_err(storage_failure)?;
    let launch = select_command(&mut registry, name, command, resume, &preferences)?;
    let claim = registry.claim(&launch.command.executable);
    let lifecycle = claim
        .as_ref()
        .and_then(|harness| registry.lifecycle(harness))
        .unwrap_or(&NoLifecycle);
    // Verified before any binding or spawn; a launch that cannot enroll fails.
    let channel = channel
        .then(|| preflight(&registry, claim.as_ref(), &launch.command.executable, paths))
        .transpose()?;
    host.resolve_servers(storage).map_err(endpoint_failure)?;
    let bound = if auto_named {
        binding::bind_auto_identity(storage, &mut host.session(), pane, name, save)
    } else {
        binding::bind_identity_with_creation(storage, &mut host.session(), pane, name, save)
    }
    .map_err(binding_failure)?;
    let binding = bound
        .presence
        .binding
        .as_ref()
        .expect("successful binding has endpoint");
    if command.is_empty()
        && existing
            .as_ref()
            .is_none_or(|identity| identity.id != binding.identity_id)
    {
        return Err(Failure::new(
            "IDENTITY_CONFLICT",
            "The identity changed while selecting its remembered command; no command was launched.",
            5,
        ));
    }
    let can_admit = if let Some(key) = &binding.session.key {
        let observation = observe_runtime_process(
            &UnixCommandRunner,
            key.incarnation.pid(),
            Instant::now() + Duration::from_secs(3),
        );
        let observation = observation.unwrap_or(ProcessObservation::Unknown);
        if matches!(&observation, ProcessObservation::Live(actual) | ProcessObservation::Stopped(actual) if actual == &key.incarnation)
        {
            return Err(Failure::new(
                "RUNTIME_CONFLICT",
                "An agent is still attached to this pane (possibly suspended). Resume it with fg or exit it first.",
                5,
            ));
        }
        if observation.matches(&key.incarnation) == RuntimeLiveness::Unknown {
            let fenced = storage
                .with_binding_transaction::<_, StorageError>(|records| {
                    let Some(current) = records
                        .entry_by_id(&binding.identity_id)?
                        .and_then(|entry| entry.binding)
                        .filter(|current| {
                            current.id == binding.id
                                && current.session.key.as_ref().map(|key| &key.incarnation)
                                    == Some(&key.incarnation)
                                && current.session.launch_owner == binding.session.launch_owner
                        })
                    else {
                        return Ok(false);
                    };
                    if current.session.state == tmt_core::binding::session::RuntimeState::Running {
                        let mut next = current.session.clone();
                        next.state = tmt_core::binding::session::RuntimeState::Unknown;
                        records.set_session_state(&binding.id, &current.session, &next)
                    } else {
                        Ok(true)
                    }
                })
                .map_err(storage_failure)?;
            if !fenced {
                return Err(Failure::new(
                    "RUNTIME_CONFLICT",
                    "The runtime attachment changed before launch; no command was launched.",
                    5,
                ));
            }
            diagnostic(
                "prior runtime evidence is unknown; automatic delivery will remain unavailable. After this command exits, run `tmt run <name>` again to establish runtime ownership.",
            );
            false
        } else {
            true
        }
    } else {
        true
    };
    if host
        .update_binding_cosmetics(
            binding,
            PaneCosmetics::Bound {
                identity: &bound.presence.identity,
                badge: badge == PaneBadge::On,
            },
        )
        .is_err()
    {
        diagnostic("identity bound, but its cosmetic badge could not be updated.");
    }
    // Mark the resumed session pending before the child can start, so its
    // first provider start (which clears the mark) cannot race ahead of it.
    let hooks_installed = match &launch.resumed {
        Some(session) => {
            mark_resume_pending(storage, &binding.identity_id, session)?;
            Registry::builtin()
                .find(session.harness.as_str())
                .is_some_and(start_hook_installed)
        }
        None => false,
    };
    let owner = incarnation(std::process::id());
    // Held for the child's whole lifetime. It is retired only for a failed spawn
    // (`never_spawned`) or a wait that returned (`settle_wait`); on every other path
    // out of here it is dropped and the driver's record stays. Declared before the
    // child, so an abandoned child is cleaned up first.
    let mut lease = HeldLease::default();
    if let Some((channel, directory)) = &channel {
        let unavailable = |message: &str| {
            Failure::new(
                "CHANNEL_UNAVAILABLE",
                format!("{message} No command was launched."),
                1,
            )
        };
        let owner = owner
            .as_ref()
            .ok_or_else(|| unavailable("Could not observe this launch's own process."))?;
        // The authority to enroll is this launch's own claim of the binding,
        // still current right now.
        verify_claim(storage, binding)?;
        let tmt = tmt_adapters::core_executable::selected().map_err(|error| {
            unavailable("Could not resolve this executable for the channel server.")
                .caused_by(error)
        })?;
        let working_directory = std::env::current_dir().map_err(|error| {
            unavailable("Could not read the launch directory.").caused_by(error)
        })?;
        lease.hold(
            channel
                .enroll(&ChannelPlan {
                    binding_id: &binding.id,
                    identity_id: &binding.identity_id,
                    pane: PaneAddress {
                        server: &binding.server,
                        pane_id: &binding.pane_id,
                        pane_pid: binding.pane_pid,
                    },
                    owner,
                    command: &launch.command,
                    working_directory: &working_directory,
                    tmt: &tmt,
                    directory,
                })
                .map_err(|error| unavailable(&error.to_string()))?,
        );
    }
    if auto_named {
        let mut stderr = tmt_cli_style::stream::stderr();
        use std::io::Write;
        let _ = writeln!(
            stderr,
            "tmt: {} name: {name}; name this agent with tmt this <name>",
            if save { "Saved" } else { "Temporary" }
        );
    }
    let planned = lease.command(&launch.command);
    let child =
        InteractiveChild::start_with(&planned.executable, &planned.args, lease.environment())
            .map_err(|error| {
                // No child exists, so nothing ran and the enrollment ends.
                lease.never_spawned();
                if auto_named && bound.created
                    && binding::retire_failed_auto_launch(storage, &mut host.session(), binding).is_err() {
                    diagnostic("could not retire the failed launch's temporary identity; inspect it before retrying.");
                }
                Failure::new("LAUNCH_FAILED", "Could not start the requested command.", 1)
                    .caused_by(error)
            })?;
    let child_evidence = child
        .observe_runtime(Instant::now() + Duration::from_secs(3))
        .unwrap_or(ProcessObservation::Unknown);
    let child_incarnation = match &child_evidence {
        ProcessObservation::Live(value) | ProcessObservation::UnreapedZombie(value) => {
            Some(value.clone())
        }
        _ => None,
    };
    let already_exited = matches!(child_evidence, ProcessObservation::UnreapedZombie(_));
    // The driver records the exact foreground before admission; if it cannot, the
    // enrollment stays unconfirmed (never "ended") and the child keeps running.
    if let Some(note) = lease.foreground_started(child_incarnation.as_ref()) {
        diagnostic(&note);
    }
    let admitted = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let Some(current) = records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding)
                .filter(|current| current.id == binding.id)
            else {
                return Ok(None);
            };
            if !claim_is_current(&current, binding)
                && !current
                    .session
                    .key
                    .as_ref()
                    .is_some_and(|key| Some(&key.incarnation) == child_incarnation.as_ref())
            {
                return Ok(None);
            }
            // A confirmed spawn remembers the pure driver claim even when
            // the process exits before live admission. It never writes a
            // session (hooks own that), but a session another driver
            // remembered is dropped: one identity has one current runtime.
            if let Some(harness) = &claim {
                let mut preferences = records.session_preferences(&binding.identity_id)?;
                preferences.launched(harness);
                records.set_session_preferences(&binding.identity_id, &preferences)?;
            }
            let (true, Some(owner), Some(child)) = (can_admit, &owner, &child_incarnation) else {
                return Ok(None);
            };
            // A hook may have attached its provider session before this probe.
            // Exact resume coordinates came from the driver-owned remembered
            // record, a channel launch's from its lease: never from user argv or
            // a hook pane. A session conflicting with hook evidence is not admitted.
            let Some(key) = admitted_key(
                current.session.key.as_ref(),
                child,
                launch
                    .resumed
                    .as_ref()
                    .filter(|session| Some(&session.harness) == claim.as_ref())
                    .map(|session| session.provider_session.clone()),
                lease.provider_session(),
            ) else {
                return Ok(None);
            };
            let next = if already_exited {
                lifecycle.client_exit(&current.session, key, owner.clone(), &preferences)
            } else {
                current.session.admit_launched(
                    key,
                    owner.clone(),
                    if launch.resumed.is_some() {
                        SessionTransition::Resumed
                    } else {
                        SessionTransition::Started
                    },
                    RuntimeLiveness::Alive,
                    RuntimeLiveness::Alive,
                )
            };
            let Some(next) = next else {
                return Ok(None);
            };
            if !records.set_session_state(&binding.id, &current.session, &next)? {
                return Ok(None);
            }
            Ok(next.key.map(|key| (key, next.state)))
        })
        .unwrap_or(None);
    if admitted.is_none() {
        diagnostic(
            "command started, but runtime ownership could not be recorded; automatic delivery is not established.",
        );
    }
    if storage.close().is_err() {
        diagnostic(
            "could not close launch state before waiting; the command will not be restarted.",
        );
    }
    crate::pane_badge::refresh(
        paths,
        host,
        binding,
        Instant::now() + Duration::from_secs(1),
    );
    // Only committed observations reach hooks, with SQLite closed before any
    // subscriber runs. A fast exit emits both events without persisting Running.
    observe_admission(
        observer,
        &binding.id,
        admitted.as_ref().map(|(key, _)| key),
        launch.resumed.is_some(),
        admitted
            .as_ref()
            .is_some_and(|(_, state)| *state == tmt_core::binding::session::RuntimeState::Ended),
    );
    let waited = child.wait(|_| {
        diagnostic(
            "signal observation degraded; waiting for the original command without restarting it.",
        )
    });
    // The enrollment ends with the foreground, and only when its end is confirmed:
    // a wait that failed proves nothing, so that path leaves the enrollment as it is.
    let status = lease.settle_wait(waited).map_err(|error| {
        Failure::new(
            "PROCESS_ERROR",
            "Could not finish observing the requested command.",
            1,
        )
        .caused_by(error)
    })?;
    if let Some(session) = &launch.resumed {
        settle_resume(
            paths,
            name,
            &binding.identity_id,
            session,
            status,
            hooks_installed,
        );
    }
    if admitted.is_some()
        && !already_exited
        && let (Some(owner), Some(child)) = (&owner, &child_incarnation)
    {
        let recorded = Storage::open(&paths.database).and_then(|mut storage| {
            let recorded = finish(&mut storage, binding, owner, child, lifecycle);
            if storage.close().is_err() {
                diagnostic("could not close final state cleanly; the recorded exit is unchanged.");
            }
            recorded
        });
        match recorded {
            Ok(Finished::Written(key)) => observe(
                observer,
                &HookEvent::SessionEnded {
                    binding_id: &binding.id,
                    key: &key,
                },
            ),
            Ok(Finished::AlreadyEnded) => {}
            Ok(Finished::Disconnected) => {}
            Ok(Finished::Replaced) | Err(_) => diagnostic(
                "command exited, but its final state could not be stored; no command was retried.",
            ),
        }
        crate::pane_badge::refresh(
            paths,
            host,
            binding,
            Instant::now() + Duration::from_secs(1),
        );
    }
    Ok(u8::try_from(status).unwrap_or(1))
}

enum Finished {
    Written(ObservedSessionKey),
    AlreadyEnded,
    Disconnected,
    Replaced,
}

fn finish(
    storage: &mut Storage,
    binding: &Binding,
    owner: &ProcessIncarnation,
    child: &ProcessIncarnation,
    lifecycle: &dyn RuntimeLifecycle,
) -> Result<Finished, StorageError> {
    storage.with_binding_transaction(|records| {
        let Some(current) = records
            .entry_by_id(&binding.identity_id)?
            .and_then(|entry| entry.binding)
            .filter(|current| {
                current.id == binding.id && current.session.launch_owner.as_ref() == Some(owner)
            })
        else {
            return Ok(Finished::Replaced);
        };
        if let Some(next) = lifecycle.disconnected(
            &current.session,
            &records.session_preferences(&binding.identity_id)?,
        ) {
            return records
                .set_session_state(&binding.id, &current.session, &next)
                .map(|written| {
                    if written {
                        Finished::Disconnected
                    } else {
                        Finished::Replaced
                    }
                });
        }
        let Some(key) = current
            .session
            .key
            .as_ref()
            .filter(|key| &key.incarnation == child)
        else {
            return Ok(Finished::Replaced);
        };
        if let Some(next) = current
            .session
            .transition(key, SessionTransition::Ended, None)
        {
            return records
                .set_session_state(&binding.id, &current.session, &next)
                .map(|written| {
                    if written {
                        Finished::Written(key.clone())
                    } else {
                        Finished::Replaced
                    }
                });
        }
        Ok(
            if current.session.state == tmt_core::binding::session::RuntimeState::Ended {
                Finished::AlreadyEnded
            } else {
                Finished::Replaced
            },
        )
    })
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "'{name}' is both an identity and a registered command. Use `tmt run {name} {name}` to launch that identity, or `tmt run <new-name> {name}` for a new one.",
        &["`", "`"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "prior runtime evidence is unknown; automatic delivery will remain unavailable. After this command exits, run `tmt run <name>` again to establish runtime ownership.",
        &["`"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt: {} name: {name}; name this agent with tmt this <name>",
        &[""],
        &[],
    ),
];
