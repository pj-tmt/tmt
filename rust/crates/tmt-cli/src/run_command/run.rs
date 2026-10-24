//! Bound foreground launch and completion retain one owned child and storage lifetime.

use super::{
    LaunchEffect, RunRequest, agent_name,
    channel::{
        HeldLease, admitted_key, claim_is_current, enrollment_failure, paste_notice, preflight,
        verify_claim, verify_plain_fallback,
    },
    diagnostic, incarnation, observe, observe_admission,
    resume::{ResumeExit, mark_resume_pending, select_command, settle_resume},
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
    driver::HookEvent,
    endpoint::ProcessIncarnation,
    identity::IdentityReader,
    names::normalize_name,
    settings::{PaneBadge, Settings},
};

pub(super) fn run_bound(
    storage: &mut Storage,
    paths: &ConfigPaths,
    host: &Host,
    pane: &str,
    request: RunRequest<'_>,
    settings: &Settings,
    effect: &mut LaunchEffect,
) -> Result<u8, Failure> {
    let observer = &mut super::EmptyObserver;
    let RunRequest {
        name,
        options,
        command,
        resume,
        save,
        channel,
    } = request;
    let switching = resume.and_then(|resume| resume.driver);
    let moving = resume.is_some_and(|resume| resume.move_running || resume.driver.is_some());
    let mut registry = RuntimeRegistry::first_party();
    let existing = storage
        .find_identity(&normalize_name(name))
        .map_err(storage_failure)?;
    if switching.is_some() && existing.is_none() {
        return Err(Failure::new(
            "NAME_NOT_FOUND",
            "The identity to switch does not exist; no agent was stopped or launched.",
            3,
        ));
    }
    if resume.is_some()
        && !moving
        && let Some(identity) = &existing
    {
        super::resume::preflight_attached(storage, &identity.id)?;
    }
    // Only the resume path purges or reconciles, and it reports each change once.
    if resume.is_some() && !moving {
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
            if resume.is_some() && !moving
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
    let selected_preferences = super::switch_driver::selection(&preferences, switching)?;
    let mut launch = select_command(
        &mut registry,
        RunRequest {
            name,
            command,
            ..request
        },
        &selected_preferences,
    )?;
    let preset = existing
        .as_ref()
        .map(|identity| match launch.resumed.as_ref() {
            Some(session) => tmt_adapters::runtime::launch_preset::read_for_session(
                storage,
                &identity.id,
                session,
            ),
            None => Ok(None),
        })
        .transpose()
        .map_err(storage_failure)?
        .flatten();
    let replay = launch
        .resumed
        .as_ref()
        .and_then(|session| preset.as_ref().filter(|preset| preset.matches(session)));
    let environment = replay
        .map(|preset| preset.environment(|key| std::env::var_os(key).is_some()))
        .unwrap_or_default();
    if let Some(session) = &launch.resumed {
        let lifecycle = registry.lifecycle(&session.harness).ok_or_else(|| {
            Failure::new(
                "RESUME_UNSUPPORTED",
                "The session driver is unavailable.",
                1,
            )
        })?;
        let mut settings = replay.map(|p| p.settings()).unwrap_or_default();
        // The resume model a starting hook reported wins over the launch
        // model; the last observed model (`resume.observedModel`) is not replayed.
        if let Some(model) = session
            .state
            .as_ref()
            .and_then(|state| lifecycle.state_model(state))
        {
            settings.model = Some(model);
        }
        if options.model.is_some() {
            settings.model = options.model.clone();
        }
        if options.effort.is_some() {
            settings.effort = options.effort.clone();
        }
        if !lifecycle.resume_settings(&mut launch.command, &settings) {
            return Err(Failure::new(
                "RESUME_SETTINGS_INVALID",
                "The driver cannot replay the selected launch settings.",
                1,
            ));
        }
        if let Some(preset) = replay {
            launch.command.executable = preset.executable.clone().into();
        } else {
            diagnostic(
                "no launch preset is associated with this session; using its registered driver command.",
            );
        }
    }
    // A resumed wrapper is associated through the admitted session, never its name.
    let claim = launch
        .resumed
        .as_ref()
        .map(|session| session.harness.clone())
        .or_else(|| registry.claim(&launch.command.executable));
    let lifecycle = claim
        .as_ref()
        .and_then(|harness| registry.lifecycle(harness))
        .unwrap_or(&NoLifecycle);
    if switching.is_some()
        && launch.resumed.is_none()
        && !lifecycle.resume_settings(&mut launch.command, options)
    {
        return Err(Failure::new(
            "RESUME_SETTINGS_INVALID",
            "The driver cannot use the selected launch settings; no new agent was launched.",
            1,
        ));
    }
    let before_handoff = launch.command.clone();
    if switching.is_some()
        && !lifecycle.initial_message(&mut launch.command, "TMT driver-switch handoff")
    {
        return Err(Failure::new(
            "RESUME_UNSUPPORTED",
            "The selected driver cannot accept a handoff as first input; no new agent was launched.",
            1,
        ));
    }
    // Policy is selected once; the driver only advertises its Default choice.
    let remembered_channel = launch
        .resumed
        .as_ref()
        .filter(|session| selected_preferences.preferred_harness.as_ref() == Some(&session.harness))
        .and(selected_preferences.channel);
    let mode = if settings.experimental_channel {
        super::channel::resume_mode(channel, launch.resumed.is_some(), remembered_channel)
    } else {
        crate::invocation::ChannelMode::Disabled
    };
    let remember_channel =
        launch.resumed.is_none() || channel != crate::invocation::ChannelMode::Default;
    let mut channel =
        preflight(&registry, mode, claim.as_ref(), &launch.command, paths).map_err(|error| {
            if let Some(driver) = switching
                && channel == crate::invocation::ChannelMode::Default
                && mode == crate::invocation::ChannelMode::Required
            {
                error.suggestion(super::switch_driver::channel_retry(request, driver))
            } else {
                channel_preflight_failure(error, channel, mode, name)
            }
        })?;
    if let Some(driver) = switching
        && channel.channel.is_some()
    {
        return Err(super::switch_driver::channel_refusal(request, driver));
    }
    host.resolve_servers(storage).map_err(endpoint_failure)?;
    let mut handoff_request = |binding: &Binding| {
        super::switch_driver::request(
            existing.as_ref().expect("existing switch identity"),
            binding,
            paths,
            settings,
        )
    };
    let moved = if moving {
        super::move_session::move_binding(
            storage,
            host,
            paths,
            pane,
            super::move_session::MoveRequest {
                identity: existing
                    .as_ref()
                    .expect("a selected resume has an existing identity"),
                preferences: &preferences,
                lifecycle: preferences
                    .remembered
                    .as_ref()
                    .and_then(|session| registry.lifecycle(&session.harness))
                    .unwrap_or(&NoLifecycle),
                switching: switching.is_some(),
                handoff: if switching.is_some() {
                    Some(&mut handoff_request)
                } else {
                    None
                },
            },
            effect,
        )?
    } else {
        None
    };
    if switching.is_some() {
        *effect = if moved.is_some() {
            LaunchEffect::BoundHere
        } else {
            LaunchEffect::OldEnded
        };
        let identity = existing
            .as_ref()
            .expect("an explicit switch has an identity");
        let handoff = super::switch_driver::context(storage, paths, identity)?;
        launch.command = before_handoff;
        if !lifecycle.initial_message(&mut launch.command, &handoff) {
            return Err(Failure::new(
                "RESUME_UNSUPPORTED",
                "The driver refused the handoff; no new agent was launched.",
                1,
            ));
        }
        if let Some(session) = preferences.remembered.as_ref()
            && let Some(preset) = tmt_adapters::runtime::launch_preset::read_for_session(
                storage,
                &identity.id,
                session,
            )
            .map_err(storage_failure)?
        {
            tmt_adapters::runtime::launch_preset::retain(storage, &identity.id, &preset)
                .map_err(storage_failure)?;
        }
    }
    let bound = if let Some(bound) = moved {
        Ok(bound)
    } else if auto_named {
        binding::bind_auto_identity(storage, &mut host.session(), pane, name, save)
    } else {
        binding::bind_identity_with_creation(storage, &mut host.session(), pane, name, save)
    }
    .map_err(binding_failure)?;
    if moving {
        *effect = LaunchEffect::BoundHere;
    }
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
        if launch.resumed.is_some() {
            super::resume::check_process(name, &binding.pane_id, &key.incarnation, &observation)?;
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
            super::warn(
                "automatic delivery is off for this run: TMT can't confirm who owns this pane; retry after this command exits",
                Some(&format!("tmt run {}", crate::output::shell_word(name))),
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
                badge: settings.pane_badge == PaneBadge::On,
                border_hint: true,
            },
        )
        .is_err()
    {
        diagnostic("identity bound, but its cosmetic badge could not be updated.");
    }
    if let Some(driver) = switching {
        super::switch_driver::activate(storage, binding, &selected_preferences, driver)?;
    }
    let mut hooks_installed = match &launch.resumed {
        Some(session) => Registry::builtin()
            .find(session.harness.as_str())
            .is_some_and(start_hook_installed),
        None => false,
    };
    let owner = incarnation(std::process::id());
    let hook_command = match prepare_launch_hooks(
        lifecycle,
        &launch.command,
        binding,
        owner.as_ref(),
    ) {
        Ok(command) => command,
        Err(error) => {
            let reason = error.to_string().replace(['\r', '\n'], " ");
            diagnostic(&format!(
                "session-only Digest hooks unavailable for this launch: {reason}; continuing with the original command."
            ));
            None
        }
    };
    // Both composed and original commands are real launch attempts. Mark the
    // exact resume pending before either can start.
    if let Some(session) = &launch.resumed {
        mark_resume_pending(storage, &binding.identity_id, session)?;
    }
    hooks_installed |= hook_command.is_some();
    let launch_command = hook_command.as_ref().unwrap_or(&launch.command);
    // Held for the child's whole lifetime. It is retired only for a failed spawn
    // (`never_spawned`) or a wait that returned (`settle_wait`); on every other path
    // out of here it is dropped and the driver's record stays. Declared before the
    // child, so an abandoned child is cleaned up first.
    let mut lease = HeldLease::default();
    if let Some(port) = channel.channel {
        // Loss of binding authority is terminal in every mode.
        verify_claim(storage, binding)?;
        let enrolled = (|| {
            let owner = owner.as_ref().ok_or_else(|| {
                Failure::new(
                    "CHANNEL_UNAVAILABLE",
                    "Could not observe this launch's own process.",
                    1,
                )
            })?;
            let tmt = tmt_adapters::core_executable::selected().map_err(|error| {
                Failure::new(
                    "CHANNEL_UNAVAILABLE",
                    "Could not resolve this executable for the channel server.",
                    1,
                )
                .caused_by(error)
            })?;
            let working_directory = std::env::current_dir().map_err(|error| {
                Failure::new(
                    "CHANNEL_UNAVAILABLE",
                    "Could not read the launch directory.",
                    1,
                )
                .caused_by(error)
            })?;
            port.enroll(&ChannelPlan {
                binding_id: &binding.id,
                identity_id: &binding.identity_id,
                pane: PaneAddress {
                    server: &binding.server,
                    pane_id: &binding.pane_id,
                    pane_pid: binding.pane_pid,
                },
                owner,
                command: launch_command,
                resume_session: launch
                    .resumed
                    .as_ref()
                    .map(|session| &session.provider_session),
                working_directory: &working_directory,
                tmt: &tmt,
                directory: &channel.directory,
            })
            .map_err(enrollment_failure)
        })();
        match enrolled {
            Ok(enrolled) => lease.hold(enrolled),
            Err(error) if mode == crate::invocation::ChannelMode::Default => {
                channel.notice = Some(error.message.to_string());
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(reason) = &channel.notice {
        verify_plain_fallback(&registry, storage, binding, &channel.directory)?;
        diagnostic(&paste_notice(name, reason));
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
    let mut planned = lease.command(launch_command).clone();
    lifecycle.prepare_launch(&mut planned, &paths.global_dir, launch.resumed.is_some());
    tmt_adapters::workspace::refresh_server(paths, host, &binding.server);
    let child_environment = environment
        .iter()
        .cloned()
        .chain(lease.environment().iter().cloned())
        .chain(std::iter::once(("TMT_AGENT".into(), "1".into())))
        .collect::<Vec<_>>();
    let captured_preset = if switching.is_some() && launch.resumed.is_some() {
        replay.zip(owner.as_ref()).map(|(preset, owner)| {
            super::switch_driver::rebound_preset(
                preset,
                binding,
                owner,
                launch.resumed.as_ref().unwrap(),
                lifecycle,
                options,
            )
        })
    } else {
        (launch.resumed.is_none())
            .then(|| {
                owner.as_ref().and_then(|owner| {
                    tmt_adapters::runtime::launch_preset::LaunchPreset::capture(
                        &launch.command,
                        &registry,
                        &binding.id,
                        owner,
                        |key| std::env::var(key).ok(),
                    )
                })
            })
            .flatten()
    };
    let child =
        InteractiveChild::start_with(&planned.executable, &planned.args, &child_environment)
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
    *effect = LaunchEffect::Launched;
    if let Some(preset) = &captured_preset {
        if !tmt_adapters::runtime::launch_preset::remember(storage, &binding.identity_id, preset)
            .unwrap_or(false)
        {
            diagnostic("could not record the bounded launch preset; this command keeps running.");
        }
    } else if launch.resumed.is_none() || switching.is_some() {
        // A fresh or explicit switch launch without a record must not reuse an older launcher.
        if tmt_adapters::runtime::launch_preset::clear(storage, &binding.identity_id).is_err() {
            diagnostic("could not clear the previous launch preset; this command keeps running.");
        }
    }
    let child_evidence = child
        .observe_runtime(Instant::now() + Duration::from_secs(3))
        .unwrap_or(ProcessObservation::Unknown);
    let child_incarnation = match &child_evidence {
        ProcessObservation::Live(value)
        | ProcessObservation::Stopped(value)
        | ProcessObservation::UnreapedZombie(value) => Some(value.clone()),
        _ => None,
    };
    let already_exited = matches!(child_evidence, ProcessObservation::UnreapedZombie(_));
    let already_stopped = matches!(child_evidence, ProcessObservation::Stopped(_));
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
            // session (hooks own that); another driver's record becomes
            // inactive while this identity still has one current runtime.
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
            } else if already_stopped {
                // Keep exact ownership for duplicate refusal, without treating
                // a stopped child as live evidence or authorizing delivery.
                current.session.record_stopped_launch(
                    key,
                    owner.clone(),
                    if launch.resumed.is_some() {
                        SessionTransition::Resumed
                    } else {
                        SessionTransition::Started
                    },
                )
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
            if remember_channel && claim.is_some() {
                let mut preferences = records.session_preferences(&binding.identity_id)?;
                preferences.channel = Some(lease.enrolled());
                records.set_session_preferences(&binding.identity_id, &preferences)?;
            }
            Ok(next.key.map(|key| (key, next.state)))
        })
        .unwrap_or(None);
    if admitted.is_none() {
        diagnostic(
            "command started, but runtime ownership could not be recorded; automatic delivery is not established.",
        );
    }
    // A start hook can race the foreground admission. Associate its already
    // admitted session only when this exact child/owner still holds the binding.
    if let (Some(owner), Some((key, _))) = (&owner, &admitted)
        && let Ok(preferences) = storage.session_preferences(&binding.identity_id)
        && let Some(session) = preferences
            .remembered
            .as_ref()
            .filter(|session| key.provider_session.as_ref() == Some(&session.provider_session))
        && tmt_adapters::runtime::launch_preset::associate(
            storage,
            &binding.identity_id,
            &binding.id,
            owner,
            session,
            &registry,
        )
        .is_err()
    {
        diagnostic("could not associate this launch preset with its admitted session.");
    }
    let (closed, signal_result) = child.with_deferred_suspend(|| storage.close());
    let storage_closed = closed.is_ok();
    if signal_result.is_err() {
        diagnostic(
            "could not preserve terminal suspension during launch-state close; the command will not be restarted.",
        );
    }
    if !storage_closed {
        diagnostic(
            "could not close launch state before waiting; the command will not be restarted.",
        );
    }
    if let Some((key, tmt_core::binding::session::RuntimeState::Running)) = &admitted
        && storage_closed
        && !already_exited
        && child_incarnation.as_ref() == Some(&key.incarnation)
        && key.provider_session.as_ref() == lease.provider_session()
        && let Some(note) = lease.foreground_admitted(&key.incarnation)
    {
        diagnostic(&note);
    }
    crate::pane_badge::refresh(
        paths,
        host,
        binding,
        Instant::now() + Duration::from_secs(1),
    );
    if let Err(error) = host.notify_launch_ready(binding) {
        diagnostic(&format!(
            "the agent was launched, but its background readiness receipt failed: {error}"
        ));
    }
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
    // Only after this exact child was admitted, and outside the wait tick.
    let naming = match (&admitted, &owner, &child_incarnation) {
        (
            Some((key, tmt_core::binding::session::RuntimeState::Running)),
            Some(owner),
            Some(child),
        ) if storage_closed && !already_exited && key.incarnation == *child => {
            agent_name::Naming::start(
                agent_name::Launch {
                    database: paths.database.clone(),
                    identity_id: binding.identity_id.clone(),
                    binding_id: binding.id.clone(),
                    owner: owner.clone(),
                    child: child.clone(),
                },
                binding,
            )
        }
        _ => None,
    };
    let on_degraded = |_: &std::io::Error| {
        diagnostic(
            "signal observation degraded; waiting for the original command without restarting it.",
        )
    };
    let waited = if storage_closed
        && admitted.is_some()
        && !already_exited
        && let Some(owner) = owner.as_ref()
        && claim.is_some()
    {
        child.wait_with_ticks(
            Duration::from_millis(tmt_adapters::runtime::sampling::CADENCE_MS),
            || crate::consumption_sample_command::tick(&binding.identity_id, &binding.id, owner),
            on_degraded,
        )
    } else {
        child.wait(on_degraded)
    };
    // The child has ended and given the terminal back, so a cleanup failure
    // can be said; any other naming outcome is silent.
    if let Some(naming) = naming
        && naming
            .finish(agent_name::FINISH_WITHIN)
            .is_some_and(|outcome| outcome.cleanup_failed)
    {
        diagnostic(
            "could not clean up the process that named this agent; it may still be running.",
        );
    }
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
            ResumeExit {
                binding,
                owner: owner.as_ref(),
                child: child_incarnation.as_ref(),
            },
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
            Ok(Finished::Replaced) => diagnostic(
                "command exited, but its final state was not stored because this launch no longer owns the identity's binding; no command was retried.",
            ),
            // A newer tmt migrated the database while this older binary ran,
            // so this binary must not write it; the binding is unchanged.
            Err(error) if error.is_newer_schema() => super::warn(
                "command exited, but a newer tmt upgraded identity storage while it ran, so this exit was not recorded; no command was retried.",
                Some("to continue this pane's remembered session, use the current tmt: tmt resume"),
            ),
            Err(error) => diagnostic(&format!(
                "command exited, but its final state could not be stored: {error}; no command was retried."
            )),
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
        if current.session.state == tmt_core::binding::session::RuntimeState::Ended {
            return Ok(Finished::AlreadyEnded);
        }
        if let Some(next) = current
            .session
            .record_launched_exit(key.clone(), owner.clone())
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
        Ok(Finished::Replaced)
    })
}

/// Inherited channel policy stays required, but the user can explicitly override it.
pub(super) fn channel_preflight_failure(
    error: Failure,
    requested: crate::invocation::ChannelMode,
    effective: crate::invocation::ChannelMode,
    name: &str,
) -> Failure {
    use crate::invocation::ChannelMode;
    if requested == ChannelMode::Default && effective == ChannelMode::Required {
        let name = crate::output::shell_word(name);
        error.suggestion(format!(
            "This session was launched with a message channel; resume without it: tmt resume --no-channel -- {name}"
        ))
    } else {
        error
    }
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "This session was launched with a message channel; resume without it: tmt resume --no-channel -- {name}",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "'{name}' is both an identity and a registered command. Use `tmt run {name} {name}` to launch that identity, or `tmt run <new-name> {name}` for a new one.",
        &["`", "`"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core("tmt run {}", &[""], &[]),
    crate::cli_style_tests::HintSpec::core(
        "to continue this pane's remembered session, use the current tmt: tmt resume",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::skipped(
        "command exited, but a newer tmt upgraded identity storage while it ran, so this exit was not recorded; no command was retried.",
        "Executable reference in prose, not a command suggestion.",
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt: {} name: {name}; name this agent with tmt this <name>",
        &[""],
        &[],
    ),
];

/// Provider-owned session settings are composed before enrollment or spawn.
fn prepare_launch_hooks(
    lifecycle: &dyn RuntimeLifecycle,
    command: &tmt_adapters::runtime::RuntimeCommand,
    binding: &Binding,
    owner: Option<&ProcessIncarnation>,
) -> std::io::Result<Option<tmt_adapters::runtime::RuntimeCommand>> {
    let Some(owner) = owner else {
        return Ok(None);
    };
    let environment = tmt_adapters::skill_installation::ProviderEnvironment::capture()?;
    let tmt = tmt_adapters::core_executable::selected()?;
    let launch = tmt_adapters::runtime::hook_protocol::HookLaunch {
        identity_id: binding.identity_id.clone(),
        binding_id: binding.id.clone(),
        owner_pid: owner.pid(),
        owner_start: owner.start_identity().to_owned(),
    };
    lifecycle.prepare_launch_hooks(&tmt_adapters::runtime::hook_protocol::LaunchHooks {
        command,
        launch: &launch,
        tmt: &tmt,
        environment: &environment,
    })
}
