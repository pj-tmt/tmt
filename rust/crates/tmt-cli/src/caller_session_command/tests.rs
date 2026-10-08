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
    refusal: Option<&'static str>,
}
impl CommandRunner for Worker {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert!(
            self.allowed,
            "unchanged coordinates must not verify native ancestry or read provider files"
        );
        self.calls.set(self.calls.get() + 1);
        assert_eq!(request.args[0], "__hook");
        assert!(["claude", "codex"].contains(&request.args[1].to_str().unwrap()));
        assert_eq!(
            request.args[2..4],
            ["--worker", "--caller-session"].map(std::ffi::OsString::from)
        );
        assert!(request.input.is_empty());
        assert!(request.deadline.saturating_duration_since(Instant::now()) <= BUDGET);
        if let Some(layer) = self.refusal {
            let args = [
                "-c".into(),
                format!(
                    "printf '%s\\n' 'warning: caller session not recorded: {layer}' >&2; exit 1"
                )
                .into(),
            ];
            return UnixCommandRunner.execute(CommandRequest {
                program: std::ffi::OsStr::new("/bin/sh"),
                args: &args,
                input: &[],
                deadline: request.deadline,
                max_output_bytes: 256,
            });
        }
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
    let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
        .unwrap()
        .identity;
    let id = identity.id.clone();
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
    let caller = caller_environment();
    let worker = Worker {
        calls: Cell::new(0),
        allowed: false,
        refusal: None,
    };
    assert_eq!(
        observe_candidate(
            &worker,
            &harness,
            &coordinates,
            Discovery {
                database: &database,
                global_dir: &directory.0,
                caller: &caller,
                executable: std::path::Path::new("nonexistent-worker"),
                deadline: Instant::now() + BUDGET,
                now_ms: 100
            }
        ),
        Ok(())
    );
    assert_eq!(worker.calls.get(), 0);
    let mut storage = Storage::open(&database).unwrap();
    insert_discovery_binding(&mut storage, &identity);
    storage.close().unwrap();
    let changed = CallerSession {
        session: tmt_core::binding::session::ProviderSessionId::new("changed").unwrap(),
        ..coordinates
    };
    let worker = Worker {
        calls: Cell::new(0),
        allowed: true,
        refusal: None,
    };
    assert_eq!(
        observe_candidate(
            &worker,
            &harness,
            &changed,
            Discovery {
                database: &database,
                global_dir: &directory.0,
                caller: &caller,
                executable: std::path::Path::new("worker"),
                deadline: Instant::now() + BUDGET,
                now_ms: 100
            }
        ),
        Ok(())
    );
    assert_eq!(worker.calls.get(), 1);
    let worker = Worker {
        calls: Cell::new(0),
        allowed: false,
        refusal: None,
    };
    assert_eq!(
        observe_candidate(
            &worker,
            &harness,
            &changed,
            Discovery {
                database: &database,
                global_dir: &directory.0,
                caller: &caller,
                executable: std::path::Path::new("worker"),
                deadline: Instant::now(),
                now_ms: 100
            }
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

fn caller_environment() -> CallerEnvironment {
    CallerEnvironment {
        tmux: Some("/tmp/test.sock,10,0".into()),
        pane: Some("%1".into()),
        process_id: 100,
        driver_env: Default::default(),
    }
}

fn insert_discovery_binding(storage: &mut Storage, identity: &tmt_core::identity::Identity) {
    let server = tmt_core::endpoint::ServerEvidence {
        host: tmt_core::host::HostKind::Tmux,
        server_id: "server".into(),
        socket_path: "/tmp/test.sock".into(),
        server_pid: 10,
        server_start_time: "server-start".into(),
    };
    let pane = tmt_core::endpoint::PaneObservation {
        id: "%1".into(),
        target: None,
        cwd: None,
        command: "sh".into(),
        pane_pid: 11,
        pane_incarnation: Some("pane-start".into()),
        suggested_name: None,
        marker: None,
    };
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.insert_binding(identity, &server, &pane).map(|_| ())
        })
        .unwrap();
}

#[test]
fn unbound_or_missing_pane_context_never_spawns_even_on_second_call() {
    let directory = Directory::new();
    let database = directory.0.join("state.db");
    let mut storage = Storage::open(&database).unwrap();
    create_or_resolve(&mut storage, "Direct", Lifetime::Saved).unwrap();
    storage.close().unwrap();
    for pane in [None, Some("%1"), Some("%2")] {
        let mut caller = caller_environment();
        caller.pane = pane.map(Into::into);
        let worker = Worker {
            calls: Cell::new(0),
            allowed: false,
            refusal: None,
        };
        for _ in 0..2 {
            assert_eq!(
                observe_candidate(
                    &worker,
                    &HarnessId::new("claude").unwrap(),
                    &CallerSession {
                        session: tmt_core::binding::session::ProviderSessionId::new("thread")
                            .unwrap(),
                        runtime_pid: Some(20)
                    },
                    Discovery {
                        database: &database,
                        global_dir: &directory.0,
                        caller: &caller,
                        executable: std::path::Path::new("worker"),
                        deadline: Instant::now() + BUDGET,
                        now_ms: 100
                    }
                ),
                Err("binding")
            );
        }
        assert_eq!(worker.calls.get(), 0);
        assert!(!directory.0.join("caller-session-refusals.json").exists());
    }
}

#[test]
fn refused_callers_skip_worker_until_ttl_or_coordinates_change() {
    for (provider, pid) in [("claude", Some(20)), ("codex", None)] {
        for layer in [
            "native-admission",
            "main-provider",
            "provider-not-root",
            "provider-header-shape",
            "budget",
        ] {
            let directory = Directory::new();
            let database = directory.0.join("state.db");
            let mut storage = Storage::open(&database).unwrap();
            let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
                .unwrap()
                .identity;
            insert_discovery_binding(&mut storage, &identity);
            storage.close().unwrap();
            let caller = caller_environment();
            let worker = Worker {
                calls: Cell::new(0),
                allowed: true,
                refusal: Some(layer),
            };
            let harness = HarnessId::new(provider).unwrap();
            let coordinates = CallerSession {
                session: tmt_core::binding::session::ProviderSessionId::new("thread").unwrap(),
                runtime_pid: pid,
            };
            let attempt = |coordinates: &CallerSession, now_ms| {
                observe_candidate(
                    &worker,
                    &harness,
                    coordinates,
                    Discovery {
                        database: &database,
                        global_dir: &directory.0,
                        caller: &caller,
                        executable: std::path::Path::new("worker"),
                        deadline: Instant::now() + BUDGET,
                        now_ms,
                    },
                )
            };
            assert_eq!(attempt(&coordinates, 100), Err(layer));
            assert_eq!(worker.calls.get(), 1);
            assert_eq!(attempt(&coordinates, 101), Err(layer));
            assert_eq!(worker.calls.get(), 1, "second call must not spawn a worker");
            assert_eq!(
                attempt(
                    &coordinates,
                    100 + tmt_adapters::runtime::caller_refusals::TTL_MS
                ),
                Err(layer)
            );
            assert_eq!(
                worker.calls.get(),
                2,
                "expiry must re-run admission without sleeps"
            );
            let changed = CallerSession {
                session: tmt_core::binding::session::ProviderSessionId::new("different").unwrap(),
                runtime_pid: pid,
            };
            assert_eq!(
                attempt(
                    &changed,
                    100 + tmt_adapters::runtime::caller_refusals::TTL_MS
                ),
                Err(layer)
            );
            assert_eq!(worker.calls.get(), 3);
        }
    }
}
