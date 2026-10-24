//! The post-attribution startup boundary with real storage/native ownership.
//! Host discovery/main-provider attribution has its separate real-host scenarios.
use super::*;
use crate::run_command::admission::{ForegroundAdmission, admit_foreground};
use std::{ffi::OsString, fs, io::Write, time::Duration};
use tmt_adapters::{
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
    runtime::RuntimeRegistry,
};
use tmt_core::{
    binding::BindingRepository,
    driver::ActionResult,
    endpoint::{PaneObservation, ServerEvidence},
    host::HostKind,
    identity::{Lifetime, create_or_resolve},
};

fn wait_file(path: &std::path::Path) -> String {
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

struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn early_start(host: HostKind, provider: &str, ended: bool, preference_edit: bool) {
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
    };
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
