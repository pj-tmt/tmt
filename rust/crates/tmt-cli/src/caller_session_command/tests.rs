use super::*;
use std::{
    cell::Cell,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tmt_adapters::{
    process::{CommandError, CommandOutput},
    storage::StorageError,
};
use tmt_core::{
    binding::BindingRepository,
    identity::{Lifetime, create_or_resolve},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-caller-session-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct Worker {
    calls: Cell<usize>,
    allowed: bool,
}
impl CommandRunner for Worker {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert!(
            self.allowed,
            "unchanged coordinates must not verify native ancestry or read provider files"
        );
        self.calls.set(self.calls.get() + 1);
        assert_eq!(
            request.args[..4],
            ["__hook", "claude", "--worker", "--caller-session"].map(std::ffi::OsString::from)
        );
        assert!(request.input.is_empty());
        assert!(request.deadline.saturating_duration_since(Instant::now()) <= BUDGET);
        Ok(CommandOutput {
            stdout: vec![],
            stderr: vec![],
        })
    }
}

#[test]
fn identical_coordinates_skip_all_host_and_provider_file_work_even_without_binding() {
    let directory = Directory::new();
    let database = directory.0.join("state.db");
    let mut storage = Storage::open(&database).unwrap();
    let id = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    let harness = HarnessId::new("claude").unwrap();
    let coordinates = CallerSession {
        session: tmt_core::binding::session::ProviderSessionId::new(
            "12345678-1234-4234-8234-123456789abc",
        )
        .unwrap(),
        runtime_pid: Some(1234),
    };
    let mut preferences = tmt_core::binding::session::SessionPreferences::default();
    preferences.remember(
        harness.clone(),
        tmt_core::binding::session::RuntimeMode::new("default").unwrap(),
        coordinates.session.clone(),
    );
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.set_session_preferences(&id, &preferences)
        })
        .unwrap();
    storage.close().unwrap();
    let worker = Worker {
        calls: Cell::new(0),
        allowed: false,
    };
    assert_eq!(
        observe_candidate(
            &worker,
            &database,
            &harness,
            &coordinates,
            std::path::Path::new("nonexistent-worker"),
            Instant::now() + BUDGET
        ),
        Ok(())
    );
    assert_eq!(worker.calls.get(), 0);
    let changed = CallerSession {
        session: tmt_core::binding::session::ProviderSessionId::new("changed").unwrap(),
        ..coordinates
    };
    let worker = Worker {
        calls: Cell::new(0),
        allowed: true,
    };
    assert_eq!(
        observe_candidate(
            &worker,
            &database,
            &harness,
            &changed,
            std::path::Path::new("worker"),
            Instant::now() + BUDGET
        ),
        Ok(())
    );
    assert_eq!(worker.calls.get(), 1);
    let worker = Worker {
        calls: Cell::new(0),
        allowed: false,
    };
    assert_eq!(
        observe_candidate(
            &worker,
            &database,
            &harness,
            &changed,
            std::path::Path::new("worker"),
            Instant::now()
        ),
        Err("budget")
    );
    assert_eq!(worker.calls.get(), 0);
}

#[test]
fn caller_commit_fences_binding_and_preferences_and_preserves_driver_state() {
    use tmt_adapters::{
        drivers::claude::ClaudeObservation, runtime::lifecycle::LifecycleObservation,
    };
    use tmt_core::{
        binding::session::{DriverState, ProviderSessionId, RuntimeMode, SessionTransition},
        endpoint::{PaneObservation, ProcessIncarnation, ServerEvidence},
    };
    for race in ["unchanged", "preferences", "binding"] {
        let directory = Directory::new();
        let database = directory.0.join("state.db");
        let mut storage = Storage::open(&database).unwrap();
        let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
            .unwrap()
            .identity;
        let server = ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "123e4567-e89b-42d3-a456-426614174000".into(),
            socket_path: "/tmp/test.sock".into(),
            server_pid: 10,
            server_start_time: "server".into(),
        };
        let pane = PaneObservation {
            id: "%1".into(),
            target: None,
            cwd: None,
            command: "claude".into(),
            pane_pid: 11,
            pane_incarnation: Some("pane".into()),
            suggested_name: None,
            marker: None,
        };
        let harness = HarnessId::new("claude").unwrap();
        let mode = RuntimeMode::new("default").unwrap();
        let mut preferences = tmt_core::binding::session::SessionPreferences::default();
        preferences.remember(
            harness.clone(),
            mode.clone(),
            ProviderSessionId::new("old").unwrap(),
        );
        let remembered = preferences.remembered.as_mut().unwrap();
        remembered.stale_at_ms = Some(10);
        remembered.resume_pending_at_ms = Some(11);
        remembered.state = Some(DriverState::new(1, "{\"model\":\"previous\"}").unwrap());
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                records.insert_binding(&identity, &server, &pane)?;
                records.set_session_preferences(&identity.id, &preferences)
            })
            .unwrap();
        storage.close().unwrap();
        let stored = Storage::context_by_identity(&database, &identity.id, 0)
            .unwrap()
            .unwrap();
        let event = ClaudeObservation {
            session: ProviderSessionId::new("new").unwrap(),
            model: None,
            context_tokens: None,
            starting: true,
            transition: SessionTransition::Started,
        };
        let next = LifecycleObservation::propose(
            &event,
            &stored.entry.binding.as_ref().unwrap().session,
            &ProcessIncarnation::new(20, "runtime").unwrap(),
            RuntimeLiveness::Unknown,
            HostEvidence::Independent {
                runtime_pid: Some(20),
            },
            false,
        )
        .unwrap();
        let mut storage = Storage::open(&database).unwrap();
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                match race {
                    "preferences" => {
                        preferences.remember(
                            harness.clone(),
                            mode.clone(),
                            ProviderSessionId::new("concurrent").unwrap(),
                        );
                        records.set_session_preferences(&identity.id, &preferences)?;
                    }
                    "binding" => {
                        records.detach_binding(&stored.entry.binding.as_ref().unwrap().id)?;
                        records.insert_binding(&identity, &server, &pane)?;
                    }
                    _ => (),
                }
                Ok(())
            })
            .unwrap();
        let before = Storage::context_by_identity(&database, &identity.id, 0)
            .unwrap()
            .unwrap();
        let applied = commit_session(
            &mut storage,
            SessionCommit {
                stored: &stored,
                next: &next,
                event: &event,
                harness: &harness,
                mode: &mode,
                fence_preferences: true,
            },
        )
        .unwrap();
        storage.close().unwrap();
        let after = Storage::context_by_identity(&database, &identity.id, 0)
            .unwrap()
            .unwrap();
        assert_eq!(applied, race == "unchanged");
        if applied {
            assert_eq!(after.entry.binding.unwrap().session, next);
            let remembered = after.preferences.remembered.unwrap();
            assert_eq!(remembered.provider_session.as_str(), "new");
            assert_eq!(remembered.state, preferences.remembered.unwrap().state);
            assert_eq!(remembered.stale_at_ms, None);
            assert_eq!(remembered.resume_pending_at_ms, None);
        } else {
            assert_eq!(after.entry, before.entry);
            assert_eq!(after.preferences, before.preferences);
        }
    }
}
