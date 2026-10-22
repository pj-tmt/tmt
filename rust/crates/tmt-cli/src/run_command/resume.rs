//! Fresh command selection and exact resume settlement share the existing session policy.

use super::{Launch, Resume, diagnostic, storage_failure, warn};
use crate::output::Failure;
use std::ffi::OsString;
use tmt_adapters::{
    config::ConfigPaths,
    runtime::{RuntimeCommand, RuntimeError, RuntimeRegistry},
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

/// Check the original attachment before binding reconciliation can discard its
/// process key. The native read and captured row share the existing writer lock;
/// binding and final launch admission still recheck their own current evidence.
pub(super) fn preflight_attached(storage: &mut Storage, identity_id: &str) -> Result<(), Failure> {
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let Some(entry) = records.entry_by_id(identity_id)? else {
                return Ok(Err(Failure::new(
                    "RUNTIME_CONFLICT",
                    "The identity changed before resume. No new agent was launched; check the old pane and retry.",
                    5,
                )));
            };
            let preferences = records.session_preferences(identity_id)?;
            let Some(binding) = entry.binding else { return Ok(Ok(())); };
            let Some(session) = preferences.remembered else { return Ok(Ok(())); };
            let Some(key) = binding.session.key.as_ref().filter(|key| {
                key.provider_session.as_ref() == Some(&session.provider_session)
            }) else { return Ok(Ok(())); };
            let observation = tmt_adapters::process::runtime::observe_runtime_process(
                &tmt_adapters::process::UnixCommandRunner,
                key.incarnation.pid(),
                std::time::Instant::now() + std::time::Duration::from_secs(3),
            ).unwrap_or(tmt_adapters::process::runtime::ProcessObservation::Unknown);
            Ok(check_process(&entry.identity.name, &binding.pane_id, &key.incarnation, &observation))
        })
        .map_err(storage_failure)?
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

/// A resume never falls back to a fresh start: an unavailable or unsupported
/// session is reported, and a fresh start stays explicit (`tmt run <name>`).
pub(super) fn select_command(
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
