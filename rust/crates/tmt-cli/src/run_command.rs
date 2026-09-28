//! Foreground command composition. Arguments belong to the child; only the
//! driver's harness ID and process ownership enter TMT storage.

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    invocation::OutputMode,
    output::Failure,
};
use std::{
    ffi::OsString,
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    process::{
        UnixCommandRunner,
        interactive::InteractiveChild,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::{
        RuntimeCommand, RuntimeError, RuntimeRegistry,
        lifecycle::{NoLifecycle, RuntimeLifecycle},
    },
    storage::{Storage, StorageError},
    tmux::{BindingSession, CallerEnvironment, Tmux},
};
use tmt_core::{
    binding::{
        self, Binding, BindingRepository,
        session::{
            ObservedSessionKey, RuntimeIncarnation, RuntimeLiveness, SessionPreferences,
            SessionTransition,
        },
    },
    driver::{ActionResult, HookEvent, HookObserver, observe_driver_hook},
    identity::IdentityReader,
    names::normalize_name,
    settings::PaneBadge,
};

struct Launch {
    command: RuntimeCommand,
    resumed: bool,
    notice: bool,
}

// The extension envelope (#324/#328) will supply registered observers here.
// Foreground launch has no subscription discovery or executable event bus.
struct EmptyObserver;

impl HookObserver for EmptyObserver {
    type Error = std::convert::Infallible;
    fn observe(&mut self, _: &HookEvent<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn observe(observer: &mut impl HookObserver, event: &HookEvent<'_>) {
    if observe_driver_hook(observer, event).is_some() {
        diagnostic("tmt: a lifecycle observer failed; the recorded command outcome is unchanged.");
    }
}

fn observe_admission(
    observer: &mut impl HookObserver,
    binding_id: &str,
    key: Option<&ObservedSessionKey>,
    resumed: bool,
    already_exited: bool,
) {
    let Some(key) = key else {
        return;
    };
    observe(
        observer,
        &HookEvent::SessionStarted {
            binding_id,
            key,
            transition: if resumed {
                SessionTransition::Resumed
            } else {
                SessionTransition::Started
            },
        },
    );
    if already_exited {
        observe(observer, &HookEvent::SessionEnded { binding_id, key });
    }
}

fn diagnostic(message: &str) {
    // A closed diagnostic stream must not unwind and drop a live owned child.
    let _ = writeln!(io::stderr().lock(), "{message}");
}

#[derive(Clone, Copy)]
struct RunRequest<'a> {
    name: &'a str,
    command: &'a [OsString],
    resume: bool,
    save: bool,
}

fn select_command(
    registry: &mut RuntimeRegistry,
    command: &[OsString],
    resume: bool,
    preferences: &SessionPreferences,
) -> Result<Launch, Failure> {
    if let Some(command) = RuntimeCommand::verbatim(command) {
        return Ok(Launch {
            command,
            resumed: false,
            notice: false,
        });
    }
    if resume && let Some(session) = &preferences.remembered {
        match registry.resume(session) {
            ActionResult::Completed(command) => {
                return Ok(Launch {
                    command,
                    resumed: true,
                    notice: false,
                });
            }
            ActionResult::Unsupported | ActionResult::Failed(RuntimeError::InvalidSession) => {}
            ActionResult::Failed(error) => {
                return Err(
                    Failure::new("RUNTIME_ERROR", "Could not prepare runtime resume.", 1)
                        .caused_by(error),
                );
            }
        }
    }
    // Prefer a bare launch of the session's own driver. If that registration
    // is unavailable, the independently remembered preferred harness may still
    // be launchable. Never guess an executable from an unknown driver ID.
    let session_fallback = resume
        .then(|| {
            preferences
                .remembered
                .as_ref()
                .and_then(|session| registry.relaunch(&session.harness))
        })
        .flatten();
    let command = session_fallback
        .or_else(|| {
            preferences
                .preferred_harness
                .as_ref()
                .and_then(|harness| registry.relaunch(harness))
        })
        .ok_or_else(|| {
            Failure::new(
                "USAGE_ERROR",
                "Specify a command, e.g. tmt run opus claude; no usable harness is remembered.",
                1,
            )
        })?;
    Ok(Launch {
        command,
        resumed: false,
        notice: resume,
    })
}

fn storage_failure(error: StorageError) -> Failure {
    Failure::new(
        "IDENTITY_ERROR",
        "Could not access launch identity state.",
        1,
    )
    .caused_by(error)
}

fn incarnation(pid: u32) -> Option<RuntimeIncarnation> {
    match observe_runtime_process(
        &UnixCommandRunner,
        u64::from(pid),
        Instant::now() + Duration::from_secs(3),
    ) {
        Ok(ProcessObservation::Live(value)) => Some(value),
        _ => None,
    }
}

pub fn execute(name: &str, command: &[OsString], resume: bool, save: bool) -> io::Result<u8> {
    match run(RunRequest {
        name,
        command,
        resume,
        save,
    }) {
        Ok(status) => Ok(status),
        Err(error) => error.publish(OutputMode::default()),
    }
}

fn run(request: RunRequest<'_>) -> Result<u8, Failure> {
    crate::caller_context::require_independent_host()?;
    let tmux = Tmux::default();
    let pane = tmux
        .caller_pane(&CallerEnvironment::current())
        .map_err(endpoint_failure)?
        .ok_or_else(|| {
            Failure::new(
                "PANE_NOT_FOUND",
                "Not running inside a resolvable tmux pane.",
                3,
            )
        })?;
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let badge = ConfigFiles {
        paths: paths.clone(),
    }
    .load()
    .map_err(Failure::from)?
    .settings
    .pane_badge;
    let mut storage = Storage::open(&paths.database).map_err(storage_failure)?;
    let pending = run_bound(
        &mut storage,
        &paths.database,
        &tmux,
        &pane,
        request,
        badge,
        &mut EmptyObserver,
    );
    // The child's status remains authoritative after it has actually run.
    // Closing local state cannot turn a successful child into a different exit.
    if storage.close().is_err() {
        diagnostic(
            "tmt: could not close launch state cleanly; inspect identity status before retrying.",
        );
    }
    pending
}

fn run_bound(
    storage: &mut Storage,
    database: &std::path::Path,
    tmux: &Tmux,
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
    } = request;
    let existing = storage
        .find_identity(&normalize_name(name))
        .map_err(storage_failure)?;
    let preferences = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            existing
                .as_ref()
                .map(|identity| records.session_preferences(&identity.id))
                .unwrap_or_else(|| Ok(SessionPreferences::default()))
        })
        .map_err(storage_failure)?;
    let mut registry = RuntimeRegistry::first_party();
    let launch = select_command(&mut registry, command, resume, &preferences)?;
    let claim = registry.claim(&launch.command.executable);
    let lifecycle = claim
        .as_ref()
        .and_then(|harness| registry.lifecycle(harness))
        .unwrap_or(&NoLifecycle);
    let bound = binding::bind_identity_with_creation(
        storage,
        &mut BindingSession::new(tmux),
        pane,
        name,
        save,
    )
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
                "tmt: prior runtime evidence is unknown; automatic delivery will remain unavailable. After this command exits, run `tmt run` again to establish runtime ownership.",
            );
            false
        } else {
            true
        }
    } else {
        true
    };
    if tmux
        .update_binding_badge(
            binding,
            (badge == PaneBadge::On).then_some(bound.presence.identity.name.as_str()),
        )
        .is_err()
    {
        diagnostic("tmt: identity bound, but its cosmetic badge could not be updated.");
    }
    if launch.notice {
        diagnostic(
            "tmt: no supported exact session is available; starting the remembered harness without arguments.",
        );
    }
    let owner = incarnation(std::process::id());
    let child = InteractiveChild::start(&launch.command.executable, &launch.command.args).map_err(
        |error| {
            Failure::new("LAUNCH_FAILED", "Could not start the requested command.", 1)
                .caused_by(error)
        },
    )?;
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
    let admitted = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let Some(current) = records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding)
                .filter(|current| current.id == binding.id)
            else {
                return Ok(None);
            };
            if current.session != binding.session
                && !(current.session.state == tmt_core::binding::session::RuntimeState::Unknown
                    && current.session.key == binding.session.key
                    && current.session.launch_owner == binding.session.launch_owner)
                && !current
                    .session
                    .key
                    .as_ref()
                    .is_some_and(|key| Some(&key.incarnation) == child_incarnation.as_ref())
            {
                return Ok(None);
            }
            // A confirmed spawn remembers the pure driver claim even when
            // the process exits before live admission. It never replaces
            // the independently hook-owned remembered session.
            if let Some(harness) = &claim {
                let mut preferences = records.session_preferences(&binding.identity_id)?;
                preferences.preferred_harness = Some(harness.clone());
                records.set_session_preferences(&binding.identity_id, &preferences)?;
            }
            let (true, Some(owner), Some(child)) = (can_admit, &owner, &child_incarnation) else {
                return Ok(None);
            };
            // A hook may have attached its provider session before this probe.
            let key = current
                .session
                .key
                .as_ref()
                .filter(|key| &key.incarnation == child)
                .cloned()
                .unwrap_or(ObservedSessionKey {
                    incarnation: child.clone(),
                    // Exact resume coordinates came from the driver-owned
                    // remembered record, never from user argv or a hook pane.
                    provider_session: launch
                        .resumed
                        .then_some(preferences.remembered.as_ref())
                        .flatten()
                        .filter(|session| Some(&session.harness) == claim.as_ref())
                        .map(|session| session.provider_session.clone()),
                });
            let next = if already_exited {
                lifecycle.client_exit(&current.session, key, owner.clone(), &preferences)
            } else {
                current.session.admit_launched(
                    key,
                    owner.clone(),
                    if launch.resumed {
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
            "tmt: command started, but runtime ownership could not be recorded; automatic delivery is not established.",
        );
    }
    if storage.close().is_err() {
        diagnostic(
            "tmt: could not close launch state before waiting; the command will not be restarted.",
        );
    }
    // Only committed observations reach hooks, with SQLite closed before any
    // subscriber runs. A fast exit emits both events without persisting Running.
    observe_admission(
        observer,
        &binding.id,
        admitted.as_ref().map(|(key, _)| key),
        launch.resumed,
        admitted
            .as_ref()
            .is_some_and(|(_, state)| *state == tmt_core::binding::session::RuntimeState::Ended),
    );
    let status = child.wait(|_| diagnostic("tmt: signal observation degraded; waiting for the original command without restarting it."))
        .map_err(|error| Failure::new("PROCESS_ERROR", "Could not finish observing the requested command.", 1).caused_by(error))?;
    if admitted.is_some()
        && !already_exited
        && let (Some(owner), Some(child)) = (&owner, &child_incarnation)
    {
        let recorded = Storage::open(database).and_then(|mut storage| {
            let recorded = finish(&mut storage, binding, owner, child, lifecycle);
            if storage.close().is_err() {
                diagnostic(
                    "tmt: could not close final state cleanly; the recorded exit is unchanged.",
                );
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
                "tmt: command exited, but its final state could not be stored; no command was retried.",
            ),
        }
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
    owner: &RuntimeIncarnation,
    child: &RuntimeIncarnation,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_adapters::runtime::{CLAUDE_MODE_DEFAULT, CODEX_MODE_EMBEDDED};
    use tmt_core::binding::session::{
        HarnessId, ProviderSessionId, RememberedSession, RuntimeMode,
    };

    #[derive(Default)]
    struct RecordingObserver {
        events: Vec<(String, ObservedSessionKey, SessionTransition)>,
        fail: bool,
    }

    impl HookObserver for RecordingObserver {
        type Error = &'static str;
        fn observe(&mut self, event: &HookEvent<'_>) -> Result<(), Self::Error> {
            let (binding_id, key, transition) = match event {
                HookEvent::SessionStarted {
                    binding_id,
                    key,
                    transition,
                } => (*binding_id, *key, *transition),
                HookEvent::SessionEnded { binding_id, key } => {
                    (*binding_id, *key, SessionTransition::Ended)
                }
                _ => panic!("unexpected foreground event"),
            };
            self.events
                .push((binding_id.into(), key.clone(), transition));
            if self.fail {
                Err("fixture observer failure")
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn only_recorded_admission_emits_and_fast_exit_keeps_event_order_on_observer_failure() {
        let key = ObservedSessionKey {
            incarnation: RuntimeIncarnation::new(17, "fixture-child-start").unwrap(),
            provider_session: Some(ProviderSessionId::new("provider-key").unwrap()),
        };
        let mut observer = RecordingObserver {
            fail: true,
            ..Default::default()
        };
        observe_admission(&mut observer, "binding", None, false, false);
        assert!(observer.events.is_empty());
        observe_admission(&mut observer, "binding", Some(&key), false, true);
        assert_eq!(
            observer.events,
            vec![
                ("binding".into(), key.clone(), SessionTransition::Started),
                ("binding".into(), key.clone(), SessionTransition::Ended),
            ]
        );
        observer.events.clear();
        observe_admission(&mut observer, "binding", Some(&key), true, false);
        assert_eq!(
            observer.events,
            vec![("binding".into(), key, SessionTransition::Resumed)]
        );
    }

    fn preferences(harness: &str, mode: &str, session: &str) -> SessionPreferences {
        SessionPreferences {
            preferred_harness: Some(HarnessId::new(harness).unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new(harness).unwrap(),
                mode: RuntimeMode::new(mode).unwrap(),
                provider_session: ProviderSessionId::new(session).unwrap(),
            }),
        }
    }

    #[test]
    fn launch_selection_has_no_implicit_resume_or_argument_replay() {
        let mut registry = RuntimeRegistry::first_party();
        let preferences = preferences(
            "claude",
            CLAUDE_MODE_DEFAULT,
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        );
        let explicit = vec![
            OsString::from("/bin/sh"),
            "-c".into(),
            "printf secret".into(),
        ];
        let launch = select_command(&mut registry, &explicit, false, &preferences).unwrap();
        assert_eq!(launch.command, RuntimeCommand::verbatim(&explicit).unwrap());
        assert!(!launch.resumed && !launch.notice);
        let launch = select_command(&mut registry, &[], false, &preferences).unwrap();
        assert_eq!(launch.command.executable, "claude");
        assert!(launch.command.args.is_empty());
        assert!(!launch.resumed && !launch.notice);
        assert!(select_command(&mut registry, &[], false, &SessionPreferences::default()).is_err());
    }

    #[test]
    fn resume_selection_falls_back_only_for_known_prelaunch_conditions() {
        let mut registry = RuntimeRegistry::first_party();
        let mut preferences = preferences(
            "codex",
            CODEX_MODE_EMBEDDED,
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        );
        let launch = select_command(&mut registry, &[], true, &preferences).unwrap();
        assert!(launch.resumed && !launch.notice);
        assert_eq!(
            launch.command.args,
            [
                "resume",
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                "--no-daemon"
            ]
            .map(OsString::from)
        );
        for (mode, session) in [
            ("unsupported", "valid-opaque-id"),
            (CODEX_MODE_EMBEDDED, "invalid-uuid"),
        ] {
            preferences.remembered.as_mut().unwrap().mode = RuntimeMode::new(mode).unwrap();
            preferences.remembered.as_mut().unwrap().provider_session =
                ProviderSessionId::new(session).unwrap();
            let launch = select_command(&mut registry, &[], true, &preferences).unwrap();
            assert!(!launch.resumed && launch.notice);
            assert_eq!(launch.command.executable, "codex");
            assert!(launch.command.args.is_empty());
        }
        preferences.remembered = None;
        let launch = select_command(&mut registry, &[], true, &preferences).unwrap();
        assert!(!launch.resumed && launch.notice);
        assert!(launch.command.args.is_empty());
        preferences.preferred_harness = Some(HarnessId::new("uninstalled").unwrap());
        assert!(select_command(&mut registry, &[], true, &preferences).is_err());
    }

    #[test]
    fn unavailable_session_driver_can_fall_back_to_a_registered_preference_only() {
        let mut registry = RuntimeRegistry::first_party();
        let mut preferences = preferences("missing-driver", "default", "provider-session");
        preferences.preferred_harness = Some(HarnessId::new("claude").unwrap());
        let launch = select_command(&mut registry, &[], true, &preferences).unwrap();
        assert_eq!(launch.command.executable, "claude");
        assert!(launch.command.args.is_empty());
        assert!(launch.notice && !launch.resumed);
        preferences.preferred_harness = Some(HarnessId::new("also-missing").unwrap());
        assert!(select_command(&mut registry, &[], true, &preferences).is_err());
    }
}
