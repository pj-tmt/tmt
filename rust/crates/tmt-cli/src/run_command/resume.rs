//! Fresh command selection and exact resume settlement share the existing session policy.

use super::{Launch, RunRequest, diagnostic, storage_failure, warn};
use crate::output::Failure;
use tmt_adapters::{
    config::ConfigPaths,
    runtime::{RuntimeCommand, RuntimeError, RuntimeRegistry, launch_preset::LaunchPreset},
    skill_installation::ProviderEnvironment,
    storage::{Storage, StorageError},
};
use tmt_core::{
    binding::{
        Binding, BindingRepository,
        session::{RememberedSession, SessionPreferences},
    },
    driver::ActionResult,
    endpoint::ProcessIncarnation,
};

/// Capture the original attachment before binding reconciliation can discard its
/// process key, then release the writer before probing it. Binding and final
/// launch admission recheck current evidence after this read-only preflight.
pub(super) fn preflight_attached(storage: &mut Storage, identity_id: &str) -> Result<(), Failure> {
    preflight_attached_with(storage, identity_id, |expected| {
        tmt_adapters::process::runtime::observe_runtime_process(
            &tmt_adapters::process::UnixCommandRunner,
            expected.pid(),
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        )
        .unwrap_or(tmt_adapters::process::runtime::ProcessObservation::Unknown)
    })
}

fn preflight_attached_with(
    storage: &mut Storage,
    identity_id: &str,
    observe: impl FnOnce(&ProcessIncarnation) -> tmt_adapters::process::runtime::ProcessObservation,
) -> Result<(), Failure> {
    let captured = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let Some(entry) = records.entry_by_id(identity_id)? else {
                return Ok(Err(Failure::new(
                    "RUNTIME_CONFLICT",
                    "The identity changed before resume. No new agent was launched; check the old pane and retry.",
                    5,
                )));
            };
            let preferences = records.session_preferences(identity_id)?;
            let Some(binding) = entry.binding else { return Ok(Ok(None)); };
            let Some(session) = preferences.remembered else { return Ok(Ok(None)); };
            let Some(key) = binding.session.key.filter(|key| {
                key.provider_session.as_ref() == Some(&session.provider_session)
            }) else { return Ok(Ok(None)); };
            Ok(Ok(Some((entry.identity.name, binding.pane_id, key.incarnation))))
        })
        .map_err(storage_failure)??;
    let Some((name, pane, expected)) = captured else {
        return Ok(());
    };
    check_process(&name, &pane, &expected, &observe(&expected))
}

pub(super) fn check_process(
    name: &str,
    pane: &str,
    expected: &ProcessIncarnation,
    observation: &tmt_adapters::process::runtime::ProcessObservation,
) -> Result<(), Failure> {
    use tmt_adapters::process::runtime::ProcessObservation;
    if observation.matches(expected) == tmt_core::binding::session::RuntimeLiveness::Gone {
        return Ok(());
    }
    let reason = match observation {
        ProcessObservation::Live(_) | ProcessObservation::Stopped(_) => {
            format!("The recorded agent is still attached to pane {pane} (possibly suspended).")
        }
        _ => format!("TMT cannot confirm that the recorded agent in pane {pane} has ended."),
    };
    Err(Failure::new(
        "RUNTIME_CONFLICT",
        format!(
            "{reason} No new agent was launched. Check the old pane and exit the agent, or move it with: tmt resume --move -- {}",
            crate::output::shell_word(name)
        ),
        5,
    ))
}

fn resume_failure(code: &'static str, message: String, name: &str) -> Failure {
    Failure::new(
        code,
        format!("{message} Start fresh with: tmt run {name}"),
        1,
    )
}

/// A session the provider no longer keeps, or keeps where TMT may not read it,
/// is refused before launch. `Unknown` and `Present` resume as before (#4062).
fn unrecoverable(
    name: &str,
    harness: &str,
    presence: tmt_adapters::runtime::session_presence::SessionPresence,
) -> Option<Failure> {
    use tmt_adapters::runtime::session_presence::SessionPresence;
    let (reason, message) = match presence {
        SessionPresence::Missing => (
            "session_missing",
            format!(
                "{name}'s remembered {harness} session is gone or expired: {harness} no longer keeps its transcript. If it still exists elsewhere, try it once more with: tmt resume --retry {name}."
            ),
        ),
        SessionPresence::Unreadable => (
            "session_unreadable",
            format!(
                "{name}'s remembered {harness} session cannot be read: TMT may not open its transcript file. Check the file's owner and permissions, then try it once more with: tmt resume --retry {name}."
            ),
        ),
        SessionPresence::Present | SessionPresence::Unknown => return None,
    };
    Some(resume_failure("RESUME_UNAVAILABLE", message, name).with_reason(reason))
}

/// Only the driver's own executable is known to keep its sessions in the
/// caller's provider tree; a replayed wrapper may set its own config root.
fn launches_driver(session: &RememberedSession, preset: Option<&LaunchPreset>) -> bool {
    preset
        .filter(|preset| preset.matches(session))
        .is_none_or(|preset| {
            tmt_core::driver::descriptor::for_executable(&preset.executable)
                .is_some_and(|driver| driver.name == session.harness.as_str())
        })
}

/// A resume never falls back to a fresh start: an unavailable or unsupported
/// session is reported, and a fresh start stays explicit (`tmt run <name>`).
/// `preset` is the launch preset the resume would replay, if any.
pub(super) fn select_command(
    registry: &mut RuntimeRegistry,
    request: RunRequest<'_>,
    preferences: &SessionPreferences,
    preset: Option<&LaunchPreset>,
) -> Result<Launch, Failure> {
    select_command_in(
        registry,
        request,
        preferences,
        preset,
        provider_environment().as_ref(),
    )
}

/// Unit tests never read the host's provider trees; they pass their own.
#[cfg(test)]
fn provider_environment() -> Option<ProviderEnvironment> {
    None
}

/// The provider trees the session probe reads.
#[cfg(not(test))]
fn provider_environment() -> Option<ProviderEnvironment> {
    ProviderEnvironment::capture().ok()
}

fn select_command_in(
    registry: &mut RuntimeRegistry,
    request: RunRequest<'_>,
    preferences: &SessionPreferences,
    preset: Option<&LaunchPreset>,
    environment: Option<&ProviderEnvironment>,
) -> Result<Launch, Failure> {
    let RunRequest {
        name,
        command,
        resume,
        ..
    } = request;
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
    if let Some(driver) = resume.driver {
        let harness = tmt_core::binding::session::HarnessId::new(driver).map_err(|_| {
            Failure::new(
                "RESUME_UNSUPPORTED",
                "The selected driver name is invalid; no new agent was launched.",
                1,
            )
        })?;
        if registry.lifecycle(&harness).is_none() {
            return Err(Failure::new(
                "RESUME_UNSUPPORTED",
                "The selected driver cannot switch sessions; no new agent was launched.",
                1,
            ));
        }
        if preferences.remembered.is_none() {
            let command = registry.relaunch(&harness).ok_or_else(|| {
                Failure::new(
                    "RESUME_UNSUPPORTED",
                    "The selected driver has no foreground command; no new agent was launched.",
                    1,
                )
            })?;
            return Ok(Launch {
                command,
                resumed: None,
            });
        }
    }
    let Some(session) = &preferences.remembered else {
        return Err(resume_failure(
            "RESUME_UNAVAILABLE",
            format!("{name} has no remembered session to resume."),
            name,
        ));
    };
    let harness = session.harness.as_str();
    // `--retry` tries the exact session once more, whatever the probe says. A
    // move needs a running agent, whose transcript may not be written yet.
    if !resume.retry
        && !resume.move_running
        && launches_driver(session, preset)
        && let Some(environment) = environment
        && let Some(failure) = unrecoverable(
            name,
            harness,
            registry.session_presence(session, environment),
        )
    {
        return Err(failure);
    }
    if session.stale_at_ms.is_some() && !resume.retry {
        if let Some(driver) = resume.driver {
            let retry = super::switch_driver::retry(
                RunRequest {
                    resume: Some(super::Resume {
                        retry: true,
                        ..resume
                    }),
                    ..request
                },
                driver,
            );
            return Err(Failure::new(
                "RESUME_UNAVAILABLE",
                format!("{name}'s remembered {harness} session looked gone when it was last resumed. No new agent was launched."),
                1,
            ).suggestion(format!("To try the selected driver's session once more, run: {retry}"))
            .with_reason("session_stale"));
        }
        return Err(resume_failure(
            "RESUME_UNAVAILABLE",
            format!(
                "{name}'s remembered {harness} session looked gone when it was last resumed. Forget it with: tmt resume --forget {name}; try it once more with: tmt resume --retry {name}."
            ),
            name,
        )
        .with_reason("session_stale"));
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

pub(super) fn mark_resume_pending(
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
pub(super) fn failure_is_trustworthy(status: u32, hooks_installed: bool) -> bool {
    status != 0 && status <= 128 && hooks_installed
}

/// A resumed provider that exited unconfirmed marks its session stale only
/// when the failure is trustworthy: a non-zero exit that no signal caused
/// (128 + n), with the provider's TMT start hook installed at launch, so a
/// start would have been observed. The child's status stays authoritative.
pub(super) struct ResumeExit<'a> {
    pub binding: &'a Binding,
    pub owner: Option<&'a ProcessIncarnation>,
    pub child: Option<&'a ProcessIncarnation>,
}

pub(super) fn settle_resume(
    paths: &ConfigPaths,
    name: &str,
    exited: ResumeExit<'_>,
    session: &RememberedSession,
    status: u32,
    hooks_installed: bool,
) {
    let (Some(owner), Some(child)) = (exited.owner, exited.child) else {
        return;
    };
    let identity_id = &exited.binding.identity_id;
    let trustworthy = failure_is_trustworthy(status, hooks_installed);
    let settled = Storage::open(&paths.database).and_then(|mut storage| {
        let stale = storage.with_binding_transaction::<_, StorageError>(|records| {
            let ours = records
                .entry_by_id(identity_id)?
                .and_then(|entry| entry.binding)
                .is_some_and(|current| {
                    current.id == exited.binding.id
                        && current.session.launch_owner.as_ref() == Some(owner)
                        && current.session.key.as_ref().is_some_and(|key| {
                            key.incarnation == *child
                                && key.provider_session.as_ref() == Some(&session.provider_session)
                        })
                });
            if !ours {
                return Ok(false);
            }
            let mut preferences = records.session_preferences(identity_id)?;
            if preferences.remembered.as_ref().is_none_or(|current| {
                current.harness != session.harness
                    || current.mode != session.mode
                    || current.provider_session != session.provider_session
            }) {
                return Ok(false);
            }
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

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "{reason} No new agent was launched. Check the old pane and exit the agent, or move it with: tmt resume --move -- {}",
        &[""],
        &[("{}", "worker")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Specify a command, e.g. tmt run opus claude; no usable harness is remembered.",
        &[";"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "forget it with tmt resume --forget {name}, or try once more with tmt resume --retry {name}",
        &[", or", ""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{message} Start fresh with: tmt run {name}",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{name}'s remembered {harness} session is gone or expired: {harness} no longer keeps its transcript. If it still exists elsewhere, try it once more with: tmt resume --retry {name}.",
        &["."],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{name}'s remembered {harness} session cannot be read: TMT may not open its transcript file. Check the file's owner and permissions, then try it once more with: tmt resume --retry {name}.",
        &["."],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "{name}'s remembered {harness} session looked gone when it was last resumed. Forget it with: tmt resume --forget {name}; try it once more with: tmt resume --retry {name}.",
        &[";", "."],
        &[],
    ),
];

#[cfg(test)]
mod exit_tests {
    use super::*;
    use tmt_core::{
        binding::session::{
            BindingSessionState, HarnessId, ObservedSessionKey, ProviderSessionId, RuntimeLiveness,
            RuntimeMode, SessionTransition,
        },
        endpoint::{PaneObservation, ServerEvidence},
        identity::{Lifetime, create_or_resolve},
    };

    #[test]
    fn exact_resume_requires_conclusive_process_exit() {
        use tmt_adapters::process::runtime::ProcessObservation;
        let expected = ProcessIncarnation::new(20, "old-start").unwrap();
        let reused = ProcessIncarnation::new(20, "new-start").unwrap();
        for observation in [
            ProcessObservation::Live(expected.clone()),
            ProcessObservation::Stopped(expected.clone()),
            ProcessObservation::Unknown,
        ] {
            let error = check_process("worker's seat", "%12", &expected, &observation).unwrap_err();
            assert_eq!(error.code, "RUNTIME_CONFLICT");
            assert_eq!(error.status, 5);
            assert!(error.message.contains("%12"));
            assert!(error.message.contains("Check the old pane"));
            assert!(error.message.contains("No new agent was launched"));
            assert!(error.message.contains(&format!(
                "tmt resume --move -- {}",
                crate::output::shell_word("worker's seat")
            )));
        }
        for observation in [
            ProcessObservation::Gone,
            ProcessObservation::Live(reused.clone()),
            ProcessObservation::Stopped(reused),
        ] {
            assert!(check_process("worker", "%12", &expected, &observation).is_ok());
        }
    }

    #[test]
    fn late_resume_exit_cannot_settle_a_replaced_binding_or_launch() {
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        for case in [
            "unchanged",
            "moved",
            "owner",
            "child",
            "driver",
            "unknown-child",
        ] {
            let directory = Directory(
                std::env::temp_dir().join(format!("tmt-resume-exit-{}-{case}", std::process::id())),
            );
            std::fs::create_dir(&directory.0).unwrap();
            let paths = ConfigPaths {
                global_dir: directory.0.clone(),
                global_config: directory.0.join("config.json"),
                local_config: directory.0.join("local.json"),
                database: directory.0.join("state.db"),
            };
            let mut storage = Storage::open(&paths.database).unwrap();
            let identity = create_or_resolve(&mut storage, "Worker", Lifetime::Saved)
                .unwrap()
                .identity;
            let server = ServerEvidence {
                host: tmt_core::host::HostKind::Tmux,
                server_id: "123e4567-e89b-42d3-a456-426614174000".into(),
                socket_path: "/tmp/test.sock".into(),
                server_pid: 10,
                server_start_time: "server".into(),
            };
            let mut pane = PaneObservation {
                agent_name: None,
                id: "%1".into(),
                target: None,
                cwd: None,
                command: "agent".into(),
                pane_pid: 11,
                pane_incarnation: None,
                suggested_name: None,
                marker: None,
                location: None,
            };
            let owner = ProcessIncarnation::new(20, "owner").unwrap();
            let child = ProcessIncarnation::new(21, "child").unwrap();
            let session = ProviderSessionId::new("s-1").unwrap();
            let state = BindingSessionState::default()
                .admit_launched(
                    ObservedSessionKey {
                        incarnation: child.clone(),
                        provider_session: Some(session.clone()),
                    },
                    owner.clone(),
                    SessionTransition::Resumed,
                    RuntimeLiveness::Alive,
                    RuntimeLiveness::Alive,
                )
                .unwrap();
            let mut preferences = SessionPreferences::default();
            preferences.remember(
                HarnessId::new("claude").unwrap(),
                RuntimeMode::new("default").unwrap(),
                session,
            );
            preferences
                .remembered
                .as_mut()
                .unwrap()
                .resume_pending_at_ms = Some(200);
            let original = storage
                .with_binding_transaction::<_, StorageError>(|records| {
                    let mut binding = records.insert_binding(&identity, &server, &pane)?;
                    records.set_session_state(&binding.id, &binding.session, &state)?;
                    records.set_session_preferences(&identity.id, &preferences)?;
                    binding.session = state.clone();
                    Ok(binding)
                })
                .unwrap();
            let remembered = preferences.remembered.clone().unwrap();
            if case == "unchanged" {
                let refusal = preflight_attached_with(&mut storage, &identity.id, |expected| {
                    // An independent zero-wait hook writer must be admitted while
                    // the process probe is in flight, without retry or delay.
                    let writer = rusqlite::Connection::open(&paths.database).unwrap();
                    writer.busy_timeout(std::time::Duration::ZERO).unwrap();
                    writer.execute_batch("BEGIN IMMEDIATE; COMMIT;").unwrap();
                    tmt_adapters::process::runtime::ProcessObservation::Live(expected.clone())
                })
                .unwrap_err();
                assert_eq!(refusal.code, "RUNTIME_CONFLICT");
            }
            storage
                .with_binding_transaction::<_, StorageError>(|records| {
                    let mut changed = state.clone();
                    let mut binding = original.clone();
                    match case {
                        "moved" => {
                            records.detach_binding(&original.id)?;
                            pane.id = "%2".into();
                            binding = records.insert_binding(&identity, &server, &pane)?;
                        }
                        "owner" => {
                            changed.launch_owner =
                                Some(ProcessIncarnation::new(30, "other-owner").unwrap());
                        }
                        "child" => {
                            changed.key.as_mut().unwrap().incarnation =
                                ProcessIncarnation::new(31, "other-child").unwrap();
                        }
                        "driver" => {
                            preferences.remembered.as_mut().unwrap().harness =
                                HarnessId::new("codex").unwrap();
                            records.set_session_preferences(&identity.id, &preferences)?;
                        }
                        _ => (),
                    }
                    records.set_session_state(&binding.id, &binding.session, &changed)
                })
                .unwrap();
            storage.close().unwrap();
            settle_resume(
                &paths,
                "Worker",
                ResumeExit {
                    binding: &original,
                    owner: Some(&owner),
                    child: (case != "unknown-child").then_some(&child),
                },
                &remembered,
                7,
                true,
            );
            let stored = Storage::context_by_identity(&paths.database, &identity.id, 0)
                .unwrap()
                .unwrap();
            let actual = stored.preferences.remembered.unwrap();
            if case == "unchanged" {
                assert!(actual.resume_pending_at_ms.is_none());
                assert!(actual.stale_at_ms.is_some());
            } else {
                assert_eq!(actual, preferences.remembered.unwrap(), "{case}");
            }
        }
    }
}

#[cfg(test)]
mod presence_tests {
    use super::*;
    use std::{ffi::OsString, fs, os::unix::fs::PermissionsExt, path::PathBuf};
    use tmt_core::binding::session::{HarnessId, ProviderSessionId, RuntimeMode};

    const SESSION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

    /// A private provider home; never the host's ~/.claude or ~/.codex.
    struct Home(PathBuf);
    impl Home {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tmt-resume-presence-{}-{label}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join(".claude/projects/-work")).unwrap();
            Self(path)
        }
        fn session_file(&self) -> PathBuf {
            self.0
                .join(format!(".claude/projects/-work/{SESSION}.jsonl"))
        }
        fn environment(&self) -> ProviderEnvironment {
            ProviderEnvironment::from_parts(&self.0, &self.0, Vec::new(), [])
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.session_file(), fs::Permissions::from_mode(0o600));
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn preferences(stale: bool) -> SessionPreferences {
        SessionPreferences {
            retained: Vec::new(),
            channel: None,
            preferred_harness: Some(HarnessId::new("claude").unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new("claude").unwrap(),
                mode: RuntimeMode::new(tmt_adapters::drivers::claude::MODE_DEFAULT).unwrap(),
                provider_session: ProviderSessionId::new(SESSION).unwrap(),
                state: None,
                stale_at_ms: stale.then_some(1),
                resume_pending_at_ms: None,
            }),
        }
    }

    fn select(home: &Home, retry: bool, stale: bool) -> Result<Launch, Failure> {
        select_with(home, retry, stale, false, None)
    }

    fn select_with(
        home: &Home,
        retry: bool,
        stale: bool,
        move_running: bool,
        preset: Option<&LaunchPreset>,
    ) -> Result<Launch, Failure> {
        select_command_in(
            &mut RuntimeRegistry::first_party(),
            RunRequest {
                name: "Ann",
                command: &[],
                resume: Some(super::super::Resume {
                    driver: None,
                    retry,
                    move_running,
                }),
                save: false,
                options: &Default::default(),
                channel: crate::invocation::ChannelMode::Default,
            },
            &preferences(stale),
            preset,
            Some(&home.environment()),
        )
    }

    fn refusal(result: Result<Launch, Failure>) -> (String, serde_json::Value, String) {
        let error = result.err().expect("resume must not launch");
        let document = error.document();
        (
            error.code.clone(),
            document["error"]["reason"].clone(),
            error.message.clone(),
        )
    }

    fn resumed(result: Result<Launch, Failure>) {
        let launch = result.expect("resume launches the exact session");
        assert_eq!(
            launch.command.args,
            ["--resume", SESSION].map(OsString::from)
        );
    }

    #[test]
    fn a_gone_or_expired_session_is_refused_with_its_reason() {
        let home = Home::new("missing");
        let (code, reason, message) = refusal(select(&home, false, false));
        assert_eq!(code, "RESUME_UNAVAILABLE");
        assert_eq!(reason, "session_missing");
        assert!(message.contains("gone or expired"), "{message}");
        assert!(message.contains("tmt resume --retry Ann"), "{message}");
        assert!(
            message.ends_with("Start fresh with: tmt run Ann"),
            "{message}"
        );
        // A stale mark does not hide the more exact reason.
        assert_eq!(refusal(select(&home, false, true)).1, "session_missing");
    }

    #[test]
    fn an_unreadable_session_is_refused_with_its_reason() {
        let home = Home::new("unreadable");
        fs::write(home.session_file(), b"{}\n").unwrap();
        fs::set_permissions(home.session_file(), fs::Permissions::from_mode(0o000)).unwrap();
        let (code, reason, message) = refusal(select(&home, false, false));
        assert_eq!(code, "RESUME_UNAVAILABLE");
        assert_eq!(reason, "session_unreadable");
        assert!(message.contains("cannot be read"), "{message}");
        assert!(message.contains("tmt resume --retry Ann"), "{message}");
    }

    #[test]
    fn retry_and_an_undecided_probe_keep_todays_resume() {
        let home = Home::new("retry");
        // --retry tries the exact session once more, whatever the probe says.
        resumed(select(&home, true, false));
        resumed(select(&home, true, true));
        // No provider tree: the probe cannot decide, so nothing changes.
        fs::remove_dir_all(home.0.join(".claude")).unwrap();
        resumed(select(&home, false, false));
        let (code, reason, message) = refusal(select(&home, false, true));
        assert_eq!(code, "RESUME_UNAVAILABLE");
        assert_eq!(reason, "session_stale");
        assert!(message.contains("tmt resume --retry Ann"), "{message}");
    }

    #[test]
    fn a_present_session_resumes_and_a_stale_one_still_needs_retry() {
        let home = Home::new("present");
        fs::write(home.session_file(), b"{}\n").unwrap();
        resumed(select(&home, false, false));
        assert_eq!(refusal(select(&home, false, true)).1, "session_stale");
    }

    #[test]
    fn a_move_never_probes_because_its_agent_is_running() {
        // A live session may not have written its transcript yet.
        let home = Home::new("move");
        resumed(select_with(&home, false, false, true, None));
    }

    fn replayed(executable: &str) -> LaunchPreset {
        LaunchPreset {
            executable: executable.into(),
            model: None,
            effort: None,
            env: Default::default(),
            binding: "11111111-1111-4111-8111-111111111111".into(),
            owner_pid: 1,
            owner_start: "start".into(),
            harness: Some("claude".into()),
            session: Some(SESSION.into()),
        }
    }

    #[test]
    fn a_replayed_wrapper_is_not_probed_but_the_registered_executable_is() {
        // The caller's tree exists and lacks the session file.
        let home = Home::new("wrapper");
        // A wrapper may set its own CLAUDE_CONFIG_DIR, so the caller's tree
        // proves nothing about its session.
        let wrapper = replayed("/opt/wrappers/claude-work");
        resumed(select_with(&home, false, false, false, Some(&wrapper)));
        for registered in ["claude", "/usr/local/bin/claude"] {
            let (code, reason, _) = refusal(select_with(
                &home,
                false,
                false,
                false,
                Some(&replayed(registered)),
            ));
            assert_eq!(
                (code.as_str(), reason),
                ("RESUME_UNAVAILABLE", "session_missing".into()),
                "{registered}"
            );
        }
    }
}
