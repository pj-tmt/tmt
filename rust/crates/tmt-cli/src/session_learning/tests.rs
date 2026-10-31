//! The post-attribution startup boundary with real storage/native ownership.
//! Host discovery/main-provider attribution has its separate real-host scenarios.
use super::*;
use crate::run_command::admission::{ForegroundAdmission, admit_foreground};
use std::{ffi::OsString, fs, io::Write, time::Duration};
use tmt_adapters::{
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
    runtime::RuntimeRegistry,
};
use tmt_core::binding::session::launch::{SessionLearningEvent, SessionLearningStop};
use tmt_core::{
    binding::BindingRepository,
    driver::ActionResult,
    endpoint::{PaneObservation, ServerEvidence},
    host::HostKind,
    identity::{Lifetime, create_or_resolve},
};

pub(super) fn wait_file(path: &std::path::Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(value) = fs::read_to_string(path)
            && !value.trim().is_empty()
        {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "private provider did not publish {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn one_early_start_is_retained_until_the_exact_foreground_is_admitted() {
    for host in [HostKind::Tmux, HostKind::pty()] {
        for provider in ["claude", "codex"] {
            early_start(host, provider, false, false);
            early_start(host, provider, true, false);
        }
    }
}

#[test]
fn a_concurrent_preference_edit_wins_over_an_early_start() {
    for provider in ["claude", "codex"] {
        early_start(HostKind::pty(), provider, false, true);
    }
}

pub(crate) struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

/// A saved identity bound to a pane, with a launch row owned by this test process.
pub(crate) struct Launch {
    pub(crate) directory: Directory,
    pub(crate) paths: ConfigPaths,
    pub(crate) storage: Storage,
    pub(crate) identity: tmt_core::identity::Identity,
    pub(crate) binding: tmt_core::binding::Binding,
    pub(crate) owner: ProcessIncarnation,
    pub(crate) driver: HarnessId,
    pub(crate) learning: Learning,
    pub(crate) deadline: Instant,
}
impl Launch {
    /// What the launch's diagnostic says now.
    pub(crate) fn diagnostic(&self) -> Option<LearningView> {
        view(
            &self.storage,
            Some(&self.binding),
            &self.identity,
            &Default::default(),
        )
        .unwrap()
    }
}

pub(crate) fn launch(host: HostKind, provider: &str) -> Launch {
    let directory = Directory(std::env::temp_dir().join(format!(
        "tmt-early-session-{}",
        tmt_core::operation::new_operation_id()
    )));
    fs::create_dir(&directory.0).unwrap();
    let root = directory.0.as_path();
    let paths = ConfigPaths {
        global_dir: root.into(),
        global_config: root.join("config.json"),
        local_config: root.join("local.json"),
        database: root.join("state.db"),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let owner =
        match observe_runtime_process(&UnixCommandRunner, u64::from(std::process::id()), deadline)
            .unwrap()
        {
            ProcessObservation::Live(owner) => owner,
            other => panic!("owned coordinator not observable: {other:?}"),
        };
    let mut storage = Storage::open(&paths.database).unwrap();
    let identity = create_or_resolve(&mut storage, "Alpha", Lifetime::Saved)
        .unwrap()
        .identity;
    let binding = storage
        .with_binding_transaction(|records| {
            records.insert_binding(
                &identity,
                &ServerEvidence {
                    host,
                    server_id: tmt_core::operation::new_operation_id(),
                    socket_path: root.join("private.sock").to_string_lossy().into(),
                    server_pid: owner.pid(),
                    server_start_time: owner.start_identity().into(),
                },
                &PaneObservation {
                    id: "owned-pane".into(),
                    target: None,
                    cwd: None,
                    command: provider.into(),
                    pane_pid: owner.pid(),
                    pane_incarnation: Some(owner.start_identity().into()),
                    suggested_name: None,
                    marker: None,
                    location: None,
                    agent_name: None,
                },
            )
        })
        .unwrap();
    let driver = HarnessId::new(provider).unwrap();
    assert!(
        storage
            .with_binding_transaction(|records| records.begin_launch_observation(
                &binding,
                &owner,
                &driver,
                tmt_adapters::request_runtime::wall_time_ms()
            ))
            .unwrap()
    );
    let learning = Learning {
        launch: HookLaunch {
            identity_id: identity.id.clone(),
            binding_id: binding.id.clone(),
            owner_pid: owner.pid(),
            owner_start: owner.start_identity().into(),
        },
        driver: driver.clone(),
        paths: paths.clone(),
        role: Cell::new(Role::Binding),
        deferred: Cell::new(false),
        queued_capture: Cell::new(false),
        terminal: RefCell::new(TerminalDiagnostic::default()),
    };
    Launch {
        directory,
        paths,
        storage,
        identity,
        binding,
        owner,
        driver,
        learning,
        deadline,
    }
}

fn early_start(host: HostKind, provider: &str, ended: bool, preference_edit: bool) {
    let Launch {
        directory,
        paths,
        mut storage,
        identity,
        binding,
        owner,
        driver,
        learning,
        deadline,
    } = launch(host, provider);
    let root = directory.0.as_path();
    if preference_edit {
        learning
            .stage(SessionLearningStep::ProviderVerification, deadline)
            .unwrap();
        learning
            .fail(SessionLearningReason::BudgetTimeout, deadline)
            .unwrap();
        learning
            .fail(SessionLearningReason::WorkerError, deadline)
            .unwrap();
        let diagnostic = view(&storage, Some(&binding), &identity, &Default::default())
            .unwrap()
            .unwrap();
        assert_eq!(diagnostic.step, "provider_verification");
        assert_eq!(
            diagnostic.reason, "budget_timeout",
            "parent failure must not erase the first failure code"
        );
        assert!(
            storage
                .session_preferences(&identity.id)
                .unwrap()
                .remembered
                .is_none()
        );
    }
    let fifo = root.join("release");
    UnixCommandRunner
        .execute(CommandRequest {
            program: "/usr/bin/mkfifo".as_ref(),
            args: &[fifo.as_os_str().into()],
            input: &[],
            deadline,
            max_output_bytes: 1024,
        })
        .unwrap();
    let session = "123e4567-e89b-42d3-a456-426614174001";
    let payload=serde_json::json!({"hook_event_name":"SessionStart","source":"startup","session_id":session}).to_string();
    // One event only. The private FIFO keeps the owned peer alive; no timer
    // controls the event/admission interleaving, and Drop reaps it on panic.
    let args:Vec<OsString>=vec!["-i".into(),"PATH=/usr/bin:/bin".into(),"TMT_AGENT=1".into(),"BROWSER=/usr/bin/true".into(),
        format!("HOME={}",root.display()).into(),format!("TMPDIR={}",root.display()).into(),
        "/bin/sh".into(),"-c".into(),r#"printf '%s\n' "$$" > "$1/pid.tmp"; /bin/mv "$1/pid.tmp" "$1/pid"; printf '%s\n' "$2" > "$1/start.tmp"; /bin/mv "$1/start.tmp" "$1/start.json"; IFS= read -r release < "$1/release""#.into(),
        "private-provider".into(),root.as_os_str().into(),payload.into()];
    let child = UnixCommandRunner
        .start(CommandRequest {
            program: "/usr/bin/env".as_ref(),
            args: &args,
            input: &[],
            deadline,
            max_output_bytes: 1024,
        })
        .unwrap();
    let pid = wait_file(&root.join("pid")).trim().parse().unwrap();
    let bytes = wait_file(&root.join("start.json"));
    let process = match observe_runtime_process(&UnixCommandRunner, pid, deadline).unwrap() {
        ProcessObservation::Live(process) => process,
        other => panic!("owned provider not observable: {other:?}"),
    };
    let mut registry = RuntimeRegistry::first_party();
    let lifecycle = registry.lifecycle(&driver).unwrap();
    let event = lifecycle.decode(bytes.as_bytes()).unwrap();
    let stored = Storage::context_by_identity(
        &paths.database,
        &identity.id,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .unwrap()
    .unwrap();
    assert!(stored.preferences.remembered.is_none());
    assert!(stored.entry.binding.as_ref().unwrap().session.key.is_none());
    assert!(
        learning
            .defer(
                &stored,
                &process,
                HostEvidence::Independent {
                    runtime_pid: Some(pid as u32)
                },
                event.as_ref(),
                lifecycle,
                Instant::now() + Duration::from_secs(2)
            )
            .unwrap()
    );
    let pending = storage
        .with_binding_transaction(|records| records.launch_observation(&binding.id, &owner))
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, LaunchStatus::Pending);
    assert_eq!(pending.pending.as_ref().unwrap().session.as_str(), session);
    assert!(
        storage
            .session_preferences(&identity.id)
            .unwrap()
            .remembered
            .is_none(),
        "pending is not admission"
    );
    assert!(
        storage
            .with_binding_transaction(|records| records.entry_by_id(&identity.id))
            .unwrap()
            .unwrap()
            .binding
            .unwrap()
            .session
            .key
            .is_none()
    );
    if ended {
        let end=lifecycle.decode(serde_json::json!({"hook_event_name":"SessionEnd","session_id":session,"reason":"other"}).to_string().as_bytes()).unwrap();
        assert!(
            learning
                .ended(
                    &stored,
                    &process,
                    end.as_ref(),
                    Instant::now() + Duration::from_secs(2)
                )
                .unwrap()
        );
    }
    if preference_edit {
        storage
            .with_binding_transaction(|records| {
                let mut preferences = records.session_preferences(&identity.id)?;
                preferences.launched(&driver);
                preferences.channel = Some(true);
                records.set_session_preferences(&identity.id, &preferences)
            })
            .unwrap();
    }
    // Admission is deliberately invoked after that one event has completed.
    let admitted = admit_foreground(
        &mut storage,
        ForegroundAdmission {
            binding: &binding,
            owner: Some(&owner),
            child: &ProcessObservation::Live(process.clone()),
            driver: Some(&driver),
            resumed: None,
            lifecycle,
            preferences: &stored.preferences,
            lease_session: None,
            channel: Some(false),
            can_admit: true,
            plain: None,
        },
    )
    .unwrap();
    if preference_edit {
        assert!(
            admitted.is_none(),
            "conflicting evidence grants no admission"
        );
        let preferences = storage.session_preferences(&identity.id).unwrap();
        assert_eq!(preferences.channel, Some(true));
        assert!(preferences.remembered.is_none());
        assert!(
            storage
                .with_binding_transaction(|records| records.entry_by_id(&identity.id))
                .unwrap()
                .unwrap()
                .binding
                .unwrap()
                .session
                .key
                .is_none()
        );
        let learning = view(&storage, Some(&binding), &identity, &preferences)
            .unwrap()
            .unwrap();
        assert_eq!(learning.reason, "conflict");
        assert!(learning.next_step.contains("tmt run"));
        assert!(!learning.next_step.contains("tmt resume"));
    } else {
        let admitted = admitted.expect("admitted once");
        assert_eq!(
            admitted.0.provider_session.as_ref().unwrap().as_str(),
            session
        );
        assert_eq!(
            admitted.1,
            if ended {
                RuntimeState::Unknown
            } else {
                RuntimeState::Running
            }
        );
        let preferences = storage.session_preferences(&identity.id).unwrap();
        let remembered = preferences
            .remembered
            .as_ref()
            .expect("the first event survives admission");
        assert_eq!(remembered.provider_session.as_str(), session);
        let observed = storage
            .with_binding_transaction(|records| records.launch_observation(&binding.id, &owner))
            .unwrap()
            .unwrap();
        assert_eq!(observed.status, LaunchStatus::Admitted);
        assert!(observed.pending.is_none());
        assert!(
            view(&storage, Some(&binding), &identity, &preferences)
                .unwrap()
                .is_none()
        );
        learning
            .stage(SessionLearningStep::Context, deadline)
            .unwrap();
        learning
            .fail(SessionLearningReason::WorkerError, deadline)
            .unwrap();
        assert!(
            view(&storage, Some(&binding), &identity, &preferences)
                .unwrap()
                .is_none(),
            "optional context failure cannot hide the committed session"
        );
        assert_eq!(
            storage.session_preferences(&identity.id).unwrap(),
            preferences
        );
        match registry.resume(remembered) {
            ActionResult::Completed(command) => {
                assert!(command.args.iter().any(|arg| arg == session))
            }
            other => panic!("exact resume unavailable: {other:?}"),
        }
    }
    let mut release = fs::OpenOptions::new().write(true).open(fifo).unwrap();
    release.write_all(b"release\n").unwrap();
    drop(release);
    child.wait().unwrap();
    assert!(matches!(
        observe_runtime_process(&UnixCommandRunner, pid, deadline).unwrap(),
        ProcessObservation::Gone
    ));
    storage.close().unwrap();
}

#[test]
fn a_detail_stays_with_its_step_until_the_next_stage() {
    let launch = launch(HostKind::pty(), "codex");
    let Launch {
        learning, deadline, ..
    } = &launch;
    learning
        .stage_detail(
            SessionLearningStep::HostDiscovery,
            SessionLearningDetail::HostUnsupported,
            *deadline,
        )
        .unwrap();
    let staged = launch.diagnostic().unwrap();
    assert_eq!(
        (staged.step, staged.reason, staged.detail),
        ("host_discovery", "not_admitted", Some("host_unsupported"))
    );
    assert_eq!(
        staged.cause(),
        "host_discovery: not_admitted, host_unsupported"
    );
    // The parent's failure keeps the step and what it stopped on.
    learning
        .fail(SessionLearningReason::WorkerError, *deadline)
        .unwrap();
    let failed = launch.diagnostic().unwrap();
    assert_eq!(
        (failed.step, failed.reason, failed.detail),
        ("host_discovery", "worker_error", Some("host_unsupported"))
    );
    // A later step is a new question: it does not inherit the old answer.
    learning
        .stage(SessionLearningStep::Storage, *deadline)
        .unwrap();
    let next = launch.diagnostic().unwrap();
    assert_eq!(
        (next.step, next.reason, next.detail),
        ("storage", "not_admitted", None)
    );
    assert_eq!(next.cause(), "storage: not_admitted");
    // A note names the current step without moving it.
    learning
        .note(
            SessionLearningDetail::Refused(SessionLearningEvent::Stop),
            *deadline,
        )
        .unwrap();
    let noted = launch.diagnostic().unwrap();
    assert_eq!(
        (noted.step, noted.reason, noted.detail),
        ("storage", "not_admitted", Some("refused:turn:Stop"))
    );
    assert_eq!(
        document(&noted)["detail"],
        "refused:turn:Stop",
        "the public document carries the stored token"
    );
}

#[test]
fn a_diagnostic_without_a_detail_leaves_it_out_of_the_document() {
    let launch = launch(HostKind::pty(), "claude");
    launch
        .learning
        .stage(SessionLearningStep::Input, launch.deadline)
        .unwrap();
    let staged = launch.diagnostic().unwrap();
    assert_eq!(staged.detail, None);
    assert!(document(&staged).get("detail").is_none());
}

fn stored(launch: &Launch) -> String {
    launch.diagnostic().unwrap().cause()
}

#[test]
fn a_run_that_cannot_bind_keeps_what_the_session_start_stored() {
    // A SessionStart that stopped at host discovery, as the parent's failure leaves it.
    let launch = launch(HostKind::pty(), "claude");
    let Launch {
        learning, deadline, ..
    } = &launch;
    learning
        .stage_detail(
            SessionLearningStep::HostDiscovery,
            SessionLearningDetail::HostUnsupported,
            *deadline,
        )
        .unwrap();
    learning
        .fail(SessionLearningReason::WorkerError, *deadline)
        .unwrap();
    let expected = "host_discovery: worker_error, host_unsupported";
    assert_eq!(stored(&launch), expected);

    // Every later turn, prompt or activity run goes through the same stages, and leaves it.
    let stopped = SessionLearningDetail::stopped(
        SessionLearningEvent::UserPromptSubmit,
        SessionLearningStop::UnboundSession,
    )
    .unwrap();
    for role in [Role::Unknown, Role::Observer] {
        learning.set_role(role);
        for step in [
            SessionLearningStep::Input,
            SessionLearningStep::DriverAdmission,
            SessionLearningStep::Decode,
            SessionLearningStep::ProviderVerification,
        ] {
            learning.stage(step, *deadline).unwrap();
            assert_eq!(stored(&launch), expected, "{step:?} by {}", role_name(role));
        }
        learning
            .stage_detail(
                SessionLearningStep::Storage,
                SessionLearningDetail::Refused(SessionLearningEvent::Stop),
                *deadline,
            )
            .unwrap();
        learning.note(stopped, *deadline).unwrap();
        learning
            .fail(SessionLearningReason::BudgetTimeout, *deadline)
            .unwrap();
        assert_eq!(stored(&launch), expected, "{}", role_name(role));
    }

    // A SessionStart that runs again is the launch's own progress and starts over.
    learning.set_role(Role::Binding);
    learning
        .stage(SessionLearningStep::Input, *deadline)
        .unwrap();
    assert_eq!(stored(&launch), "input: not_admitted");
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Unknown => "a run whose payload is not read yet",
        Role::Binding => "a binding run",
        Role::Observer => "an observer run",
    }
}

#[test]
fn a_run_that_cannot_bind_names_its_own_reason_only_when_nothing_more_specific_is_stored() {
    let launch = launch(HostKind::pty(), "claude");
    let Launch {
        learning, deadline, ..
    } = &launch;
    // The binding run stopped at a step without saying why.
    learning
        .stage(SessionLearningStep::ProviderVerification, *deadline)
        .unwrap();
    learning.set_role(Role::Observer);
    let first =
        SessionLearningDetail::stopped(SessionLearningEvent::Stop, SessionLearningStop::NotCurrent)
            .unwrap();
    learning.note(first, *deadline).unwrap();
    assert_eq!(
        stored(&launch),
        "provider_verification: not_admitted, stopped:turn:Stop:not_current",
        "the step stays the binding run's; only the missing detail is filled"
    );
    // The next run does not replace it.
    let second = SessionLearningDetail::stopped(
        SessionLearningEvent::UserPromptSubmit,
        SessionLearningStop::UnboundSession,
    )
    .unwrap();
    learning.note(second, *deadline).unwrap();
    assert_eq!(
        stored(&launch),
        "provider_verification: not_admitted, stopped:turn:Stop:not_current"
    );
}

#[test]
fn a_run_that_cannot_bind_leaves_a_launch_nothing_has_bound_at_its_first_step() {
    // The launch row starts at the input step: a run that cannot bind does not move it, and
    // says why it stopped on that step.
    let launch = launch(HostKind::pty(), "claude");
    let Launch {
        learning, deadline, ..
    } = &launch;
    learning.set_role(Role::Observer);
    assert_eq!(stored(&launch), "input: not_admitted");
    learning
        .stage(SessionLearningStep::Decode, *deadline)
        .unwrap();
    assert_eq!(stored(&launch), "input: not_admitted");
    learning
        .note(
            SessionLearningDetail::stopped(
                SessionLearningEvent::Notification,
                SessionLearningStop::ActivityMissing,
            )
            .unwrap(),
            *deadline,
        )
        .unwrap();
    assert_eq!(
        stored(&launch),
        "input: not_admitted, stopped:activity_session:Notification:activity_missing"
    );
}

fn admit(launch: &mut Launch) {
    let (binding, owner) = (launch.binding.id.clone(), launch.owner.clone());
    assert!(
        launch
            .storage
            .with_binding_transaction(|records| {
                let observed = records.launch_observation(&binding, &owner)?.unwrap();
                let mut admitted = observed.clone();
                admitted.status = LaunchStatus::Admitted;
                admitted.diagnostic = None;
                records.set_launch_observation(&observed, &admitted)
            })
            .unwrap()
    );
}

#[test]
fn a_digest_stop_names_its_refusal_on_an_admitted_launch_until_one_delivers() {
    let mut launch = launch(HostKind::pty(), "claude");
    admit(&mut launch);
    let Launch {
        learning, deadline, ..
    } = &launch;
    assert!(
        launch.diagnostic().is_none(),
        "an admitted launch shows nothing"
    );
    // Nothing stored: a delivered Stop changes nothing.
    learning.digest_delivered(*deadline).unwrap();
    assert!(launch.diagnostic().is_none());

    learning
        .digest_stop(SessionLearningDigestStop::LaunchOwner, *deadline)
        .unwrap();
    let stopped = launch.diagnostic().unwrap();
    assert_eq!(
        (
            stopped.step,
            stopped.reason,
            stopped.detail,
            stopped.unbound
        ),
        (
            "provider_verification",
            "worker_error",
            Some("digest_stop:launch:owner"),
            false
        )
    );
    assert_eq!(stopped.next_step, DIGEST_STOP_NEXT_STEP);
    assert_eq!(
        stopped.warning(Some("Alpha")),
        "The last Digest Stop for 'Alpha' ended without the Digest (provider_verification: worker_error, digest_stop:launch:owner)."
    );
    // Another hook run on the admitted launch neither replaces nor clears it.
    learning
        .stage(SessionLearningStep::Input, *deadline)
        .unwrap();
    learning.succeeded(*deadline).unwrap();
    assert_eq!(
        launch.diagnostic().unwrap().detail,
        Some("digest_stop:launch:owner")
    );
    // A later refusal replaces it; a delivered Digest Stop clears it.
    learning
        .digest_stop(SessionLearningDigestStop::WorkerTimeout, *deadline)
        .unwrap();
    assert_eq!(
        launch.diagnostic().unwrap().cause(),
        "context: budget_timeout, digest_stop:worker_timeout"
    );
    learning.digest_delivered(*deadline).unwrap();
    assert!(launch.diagnostic().is_none());
}

#[test]
fn a_digest_stop_never_writes_to_a_launch_that_is_not_admitted() {
    let launch = launch(HostKind::pty(), "codex");
    let before = stored(&launch);
    assert!(
        launch
            .learning
            .digest_stop(SessionLearningDigestStop::WorkerError, launch.deadline)
            .is_err()
    );
    assert!(launch.learning.digest_delivered(launch.deadline).is_err());
    assert_eq!(stored(&launch), before);
    assert!(launch.diagnostic().unwrap().unbound);
}

#[test]
fn a_missing_start_diagnostic_survives_hook_success_until_the_next_stage() {
    let mut launch = launch(HostKind::pty(), "claude");
    admit(&mut launch);
    let learning = &launch.learning;
    learning.set_role(Role::Observer);
    learning.note(NO_ADMITTED_START, launch.deadline).unwrap();
    learning.succeeded(launch.deadline).unwrap();
    let view = launch.diagnostic().unwrap();
    assert_eq!(
        (view.step, view.reason, view.detail, view.unbound),
        (
            "storage",
            "not_admitted",
            Some("stopped:turn:Stop:no_admitted_start"),
            false
        )
    );
    assert_eq!(view.next_step, NO_ADMITTED_START_NEXT_STEP);
    assert_eq!(
        view.warning(Some("Alpha")),
        "The last Stop for 'Alpha' had no admitted start (storage: not_admitted, stopped:turn:Stop:no_admitted_start)."
    );
    learning
        .stage(SessionLearningStep::Input, launch.deadline)
        .unwrap();
    assert!(launch.diagnostic().is_none());
    learning.succeeded(launch.deadline).unwrap();
    assert!(launch.diagnostic().is_none());
}

#[test]
fn a_missing_start_does_not_replace_a_digest_stop_diagnostic() {
    let mut launch = launch(HostKind::pty(), "claude");
    admit(&mut launch);
    launch
        .learning
        .digest_stop(SessionLearningDigestStop::WorkerTimeout, launch.deadline)
        .unwrap();
    launch
        .learning
        .note(NO_ADMITTED_START, launch.deadline)
        .unwrap();
    launch.learning.succeeded(launch.deadline).unwrap();
    assert_eq!(
        launch.diagnostic().unwrap().detail,
        Some("digest_stop:worker_timeout")
    );
}
