//! Foreground command composition. Arguments belong to the child; only the
//! driver's harness ID and process ownership enter TMT storage.

use crate::{
    binding_error::{binding_failure, endpoint_failure},
    invocation::OutputMode,
    output::Failure,
};
use std::{
    ffi::OsString,
    io,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    drivers::Registry,
    process::{
        UnixCommandRunner,
        interactive::InteractiveChild,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::{
        RuntimeCommand, RuntimeError, RuntimeRegistry, StateReconciliation,
        lifecycle::{NoLifecycle, RuntimeLifecycle},
    },
    setup::start_hook_installed,
    storage::{Storage, StorageError},
    tmux::{BindingSession, CallerEnvironment, PaneCosmetics, Tmux},
};
use tmt_core::{
    binding::{
        self, Binding, BindingRepository,
        session::{
            ObservedSessionKey, RememberedSession, RuntimeIncarnation, RuntimeLiveness,
            SessionPreferences, SessionTransition,
        },
    },
    driver::{ActionResult, HookEvent, HookObserver, observe_driver_hook},
    identity::IdentityReader,
    names::normalize_name,
    settings::PaneBadge,
};

struct Launch {
    command: RuntimeCommand,
    /// The exact remembered session this launch resumes, if any.
    resumed: Option<RememberedSession>,
}

/// `tmt resume <name>` and its `tmt run --resume <name>` alias.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Resume {
    /// Try a session marked stale once more.
    pub retry: bool,
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
        diagnostic("a lifecycle observer failed; the recorded command outcome is unchanged.");
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

/// A non-fatal problem while the command keeps running or has run.
fn diagnostic(message: &str) {
    warn(message, None);
}

/// The child shares stderr with `tmt run`, so each warning names `tmt` as
/// its source.
fn warn(message: &str, hint: Option<&str>) {
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    // A closed diagnostic stream must not unwind and drop a live owned child.
    let _ =
        tmt_cli_style::message::warning(&mut stderr, terminal, &format!("tmt: {message}"), hint);
}

#[derive(Clone, Copy)]
struct RunRequest<'a> {
    name: &'a str,
    command: &'a [OsString],
    resume: Option<Resume>,
    save: bool,
}

fn resume_failure(code: &'static str, message: String, name: &str) -> Failure {
    Failure::new(
        code,
        format!("{message} Start fresh with: tmt run {name}"),
        1,
    )
}

/// A resume never falls back to a fresh start: an unavailable or unsupported
/// session is reported, and a fresh start stays explicit (`tmt run <name>`).
fn select_command(
    registry: &mut RuntimeRegistry,
    name: &str,
    command: &[OsString],
    resume: Option<Resume>,
    preferences: &SessionPreferences,
) -> Result<Launch, Failure> {
    if let Some(command) = RuntimeCommand::verbatim(command) {
        return Ok(Launch {
            command,
            resumed: None,
        });
    }
    let Some(resume) = resume else {
        let command = preferences
            .preferred_harness
            .as_ref()
            .and_then(|harness| registry.relaunch(harness))
            .ok_or_else(|| {
                Failure::new(
                    "USAGE_ERROR",
                    "Specify a command, e.g. tmt run opus claude; no usable harness is remembered.",
                    1,
                )
            })?;
        return Ok(Launch {
            command,
            resumed: None,
        });
    };
    let Some(session) = &preferences.remembered else {
        return Err(resume_failure(
            "RESUME_UNAVAILABLE",
            format!("{name} has no remembered session to resume."),
            name,
        ));
    };
    let harness = session.harness.as_str();
    if session.stale_at_ms.is_some() && !resume.retry {
        return Err(resume_failure(
            "RESUME_UNAVAILABLE",
            format!(
                "{name}'s remembered {harness} session looked gone when it was last resumed. Forget it with: tmt resume --forget {name}; try it once more with: tmt resume --retry {name}."
            ),
            name,
        ));
    }
    match registry.resume(session) {
        ActionResult::Completed(command) => Ok(Launch {
            command,
            resumed: Some(session.clone()),
        }),
        ActionResult::Unsupported => Err(resume_failure(
            "RESUME_UNSUPPORTED",
            format!("The {harness} driver cannot resume {name}'s remembered session."),
            name,
        )),
        ActionResult::Failed(RuntimeError::InvalidSession) => Err(resume_failure(
            "RESUME_UNAVAILABLE",
            format!("{name}'s remembered {harness} session is not one its driver can resume."),
            name,
        )),
        ActionResult::Failed(error) => {
            Err(
                Failure::new("RUNTIME_ERROR", "Could not prepare runtime resume.", 1)
                    .caused_by(error),
            )
        }
    }
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

pub fn execute(
    name: &str,
    command: &[OsString],
    resume: Option<Resume>,
    save: bool,
) -> io::Result<u8> {
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
        &paths,
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
            "could not close launch state cleanly; inspect identity status before retrying.",
        );
    }
    pending
}

fn run_bound(
    storage: &mut Storage,
    paths: &ConfigPaths,
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
                "prior runtime evidence is unknown; automatic delivery will remain unavailable. After this command exits, run `tmt run` again to establish runtime ownership.",
            );
            false
        } else {
            true
        }
    } else {
        true
    };
    if tmux
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
                        .as_ref()
                        .filter(|session| Some(&session.harness) == claim.as_ref())
                        .map(|session| session.provider_session.clone()),
                });
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
        tmux,
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
    let status = child.wait(|_| diagnostic("signal observation degraded; waiting for the original command without restarting it."))
        .map_err(|error| Failure::new("PROCESS_ERROR", "Could not finish observing the requested command.", 1).caused_by(error))?;
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
            tmux,
            binding,
            Instant::now() + Duration::from_secs(1),
        );
    }
    Ok(u8::try_from(status).unwrap_or(1))
}

fn mark_resume_pending(
    storage: &mut Storage,
    identity_id: &str,
    session: &RememberedSession,
) -> Result<(), Failure> {
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let mut preferences = records.session_preferences(identity_id)?;
            if let Some(remembered) = preferences.remembered.as_mut().filter(|remembered| {
                remembered.harness == session.harness
                    && remembered.provider_session == session.provider_session
            }) {
                remembered.resume_pending_at_ms =
                    Some(tmt_adapters::request_runtime::wall_time_ms());
                records.set_session_preferences(identity_id, &preferences)?;
            }
            Ok(())
        })
        .map_err(storage_failure)
}

/// A non-zero exit that no signal caused (128 + n), from a provider whose
/// TMT start hook was installed at launch, so its start would have been seen.
fn failure_is_trustworthy(status: u32, hooks_installed: bool) -> bool {
    status != 0 && status <= 128 && hooks_installed
}

/// A resumed provider that exited unconfirmed marks its session stale only
/// when the failure is trustworthy: a non-zero exit that no signal caused
/// (128 + n), with the provider's TMT start hook installed at launch, so a
/// start would have been observed. The child's status stays authoritative.
fn settle_resume(
    paths: &ConfigPaths,
    name: &str,
    identity_id: &str,
    session: &RememberedSession,
    status: u32,
    hooks_installed: bool,
) {
    let trustworthy = failure_is_trustworthy(status, hooks_installed);
    let settled = Storage::open(&paths.database).and_then(|mut storage| {
        let stale = storage.with_binding_transaction::<_, StorageError>(|records| {
            let mut preferences = records.session_preferences(identity_id)?;
            let before = preferences.clone();
            let stale = preferences.settle_resume(
                &session.provider_session,
                trustworthy,
                tmt_adapters::request_runtime::wall_time_ms(),
            );
            if preferences != before {
                records.set_session_preferences(identity_id, &preferences)?;
            }
            Ok(stale)
        });
        storage.close().and(stale)
    });
    match settled {
        Ok(true) => warn(
            &format!(
                "{} exited before confirming {name}'s resumed session; marked it stale.",
                session.harness.as_str()
            ),
            Some(&format!(
                "forget it with tmt resume --forget {name}, or try once more with tmt resume --retry {name}"
            )),
        ),
        Ok(false) => {}
        Err(_) => {
            diagnostic("could not record the resume outcome; the recorded exit is unchanged.")
        }
    }
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
    use tmt_adapters::drivers::{
        claude::MODE_DEFAULT as CLAUDE_MODE_DEFAULT, codex::MODE_EMBEDDED as CODEX_MODE_EMBEDDED,
    };
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
                state: None,
                stale_at_ms: None,
                resume_pending_at_ms: None,
            }),
        }
    }

    const SESSION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const RESUME: Option<Resume> = Some(Resume { retry: false });

    fn refused(result: Result<Launch, Failure>) -> (String, String) {
        let error = result.err().expect("resume must not launch");
        (error.code.to_string(), error.message.to_string())
    }

    #[test]
    fn launch_selection_has_no_implicit_resume_or_argument_replay() {
        let mut registry = RuntimeRegistry::first_party();
        let preferences = preferences("claude", CLAUDE_MODE_DEFAULT, SESSION);
        let explicit = vec![
            OsString::from("/bin/sh"),
            "-c".into(),
            "printf secret".into(),
        ];
        let launch = select_command(&mut registry, "Ann", &explicit, None, &preferences).unwrap();
        assert_eq!(launch.command, RuntimeCommand::verbatim(&explicit).unwrap());
        assert!(launch.resumed.is_none());
        let launch = select_command(&mut registry, "Ann", &[], None, &preferences).unwrap();
        assert_eq!(launch.command.executable, "claude");
        assert!(launch.command.args.is_empty());
        assert!(launch.resumed.is_none());
        assert!(
            select_command(
                &mut registry,
                "Ann",
                &[],
                None,
                &SessionPreferences::default()
            )
            .is_err()
        );
    }

    #[test]
    fn resume_launches_only_the_exact_session_and_never_starts_fresh() {
        let mut registry = RuntimeRegistry::first_party();
        let mut preferences = preferences("codex", CODEX_MODE_EMBEDDED, SESSION);
        let launch = select_command(&mut registry, "Ann", &[], RESUME, &preferences).unwrap();
        assert_eq!(launch.resumed.as_ref(), preferences.remembered.as_ref());
        assert_eq!(
            launch.command.args,
            ["resume", SESSION, "--no-daemon"].map(OsString::from)
        );
        preferences.remembered.as_mut().unwrap().mode = RuntimeMode::new("unsupported").unwrap();
        let (code, message) = refused(select_command(
            &mut registry,
            "Ann",
            &[],
            RESUME,
            &preferences,
        ));
        assert_eq!(code, "RESUME_UNSUPPORTED");
        assert!(
            message.ends_with("Start fresh with: tmt run Ann"),
            "{message}"
        );
        preferences.remembered.as_mut().unwrap().mode =
            RuntimeMode::new(CODEX_MODE_EMBEDDED).unwrap();
        preferences.remembered.as_mut().unwrap().provider_session =
            ProviderSessionId::new("not-a-uuid").unwrap();
        assert_eq!(
            refused(select_command(
                &mut registry,
                "Ann",
                &[],
                RESUME,
                &preferences
            ))
            .0,
            "RESUME_UNAVAILABLE"
        );
        preferences.remembered = None;
        let (code, message) = refused(select_command(
            &mut registry,
            "Ann",
            &[],
            RESUME,
            &preferences,
        ));
        assert_eq!(code, "RESUME_UNAVAILABLE");
        assert_eq!(
            message,
            "Ann has no remembered session to resume. Start fresh with: tmt run Ann"
        );
    }

    #[test]
    fn a_stale_session_is_refused_until_the_user_retries() {
        let mut registry = RuntimeRegistry::first_party();
        let mut preferences = preferences("claude", CLAUDE_MODE_DEFAULT, SESSION);
        preferences.remembered.as_mut().unwrap().stale_at_ms = Some(1);
        let (code, message) = refused(select_command(
            &mut registry,
            "Ann",
            &[],
            RESUME,
            &preferences,
        ));
        assert_eq!(code, "RESUME_UNAVAILABLE");
        assert!(message.contains("tmt resume --forget Ann"), "{message}");
        assert!(message.contains("tmt resume --retry Ann"), "{message}");
        let launch = select_command(
            &mut registry,
            "Ann",
            &[],
            Some(Resume { retry: true }),
            &preferences,
        )
        .unwrap();
        assert_eq!(
            launch.command.args,
            ["--resume", SESSION].map(OsString::from)
        );
    }

    #[test]
    fn an_unregistered_session_driver_is_reported_not_replaced() {
        let mut registry = RuntimeRegistry::first_party();
        let mut preferences = preferences("missing-driver", "default", "provider-session");
        preferences.preferred_harness = Some(HarnessId::new("claude").unwrap());
        assert_eq!(
            refused(select_command(
                &mut registry,
                "Ann",
                &[],
                RESUME,
                &preferences
            ))
            .0,
            "RESUME_UNSUPPORTED"
        );
    }

    #[test]
    fn only_an_unsignalled_failure_with_hooks_installed_can_mark_a_session_stale() {
        assert!(failure_is_trustworthy(1, true));
        assert!(!failure_is_trustworthy(0, true), "a clean exit");
        assert!(!failure_is_trustworthy(130, true), "Ctrl-C (128 + SIGINT)");
        assert!(!failure_is_trustworthy(143, true), "SIGTERM");
        assert!(!failure_is_trustworthy(1, false), "hooks absent at launch");
    }
}
