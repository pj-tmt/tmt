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

/// The identity remembers the caller's provider session.
fn remember_session(
    storage: &mut Storage,
    id: &str,
    harness: &HarnessId,
    coordinates: &CallerSession,
) {
    let mut preferences = tmt_core::binding::session::SessionPreferences::default();
    preferences.remember(
        harness.clone(),
        tmt_core::binding::session::RuntimeMode::new("default").unwrap(),
        coordinates.session.clone(),
    );
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.set_session_preferences(id, &preferences)
        })
        .unwrap();
}

/// The identity's binding observes the caller's session as running.
fn admit_session(storage: &mut Storage, id: &str, coordinates: &CallerSession) {
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            use tmt_core::binding::session::{ObservedSessionKey, SessionTransition};
            let binding = records.entry_by_id(id)?.unwrap().binding.unwrap();
            let next = binding
                .session
                .admit(
                    ObservedSessionKey {
                        incarnation: tmt_core::endpoint::ProcessIncarnation::new(20, "provider")
                            .unwrap(),
                        provider_session: Some(coordinates.session.clone()),
                    },
                    SessionTransition::Started,
                    RuntimeLiveness::Alive,
                )
                .unwrap();
            records.set_session_state(&binding.id, &binding.session, &next)
        })
        .unwrap();
}

#[test]
fn agreeing_observed_coordinates_skip_all_host_and_provider_file_work() {
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
    remember_session(&mut storage, &id, &harness, &coordinates);
    insert_discovery_binding(&mut storage, &identity);
    admit_session(&mut storage, &id, &coordinates);
    storage.close().unwrap();
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
                executable: std::path::Path::new("nonexistent-worker"),
                deadline: Instant::now() + BUDGET,
                now_ms: 100
            },
            || panic!(
                "remembered coordinates must not capture the host environment or read approvals"
            ),
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
                executable: std::path::Path::new("worker"),
                deadline: Instant::now() + BUDGET,
                now_ms: 100
            },
            || (caller_environment(), Vec::new()),
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
                executable: std::path::Path::new("worker"),
                deadline: Instant::now(),
                now_ms: 100
            },
            || (caller_environment(), Vec::new()),
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
            agent_name: None,
            id: "%1".into(),
            target: None,
            cwd: None,
            command: "claude".into(),
            pane_pid: 11,
            pane_incarnation: Some("pane".into()),
            suggested_name: None,
            marker: None,
            location: None,
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
        if race == "unchanged" {
            // A writer that holds the lock past the hook's bounded wait makes
            // the commit busy: that is the budget, and nothing is written.
            let writer = rusqlite::Connection::open(&database).unwrap();
            writer.execute_batch("BEGIN IMMEDIATE").unwrap();
            let mut bounded =
                Storage::open_hook(&database, Instant::now() + Duration::from_millis(300)).unwrap();
            let busy = commit_session(
                &mut bounded,
                SessionCommit {
                    stored: &stored,
                    next: &next,
                    event: &event,
                    harness: &harness,
                    mode: &mode,
                    fence_preferences: true,
                },
            )
            .unwrap_err();
            assert!(busy.is_hook_allowance_spent(), "{busy:?}");
            assert_eq!(storage_layer(busy), "budget");
            drop(bounded);
            writer.execute_batch("ROLLBACK").unwrap();
        }
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
        agent_name: None,
        id: "%1".into(),
        target: None,
        cwd: None,
        command: "sh".into(),
        pane_pid: 11,
        pane_incarnation: Some("pane-start".into()),
        suggested_name: None,
        marker: None,
        location: None,
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
                        executable: std::path::Path::new("worker"),
                        deadline: Instant::now() + BUDGET,
                        now_ms: 100
                    },
                    || {
                        (
                            CallerEnvironment {
                                pane: pane.map(Into::into),
                                ..caller_environment()
                            },
                            Vec::new(),
                        )
                    },
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
                        executable: std::path::Path::new("worker"),
                        deadline: Instant::now() + BUDGET,
                        now_ms,
                    },
                    || (caller_environment(), Vec::new()),
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

#[test]
fn remembered_history_does_not_skip_an_incomplete_current_binding() {
    let directory = Directory::new();
    let database = directory.0.join("state.db");
    let mut storage = Storage::open(&database).unwrap();
    let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_discovery_binding(&mut storage, &identity);
    let harness = HarnessId::new("codex").unwrap();
    let coordinates = CallerSession {
        session: tmt_core::binding::session::ProviderSessionId::new("remembered-thread").unwrap(),
        runtime_pid: None,
    };
    let mut preferences = tmt_core::binding::session::SessionPreferences::default();
    preferences.remember(
        harness.clone(),
        tmt_core::binding::session::RuntimeMode::new("embedded").unwrap(),
        coordinates.session.clone(),
    );
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.set_session_preferences(&identity.id, &preferences)
        })
        .unwrap();
    storage.close().unwrap();
    let worker = Worker {
        calls: Cell::new(0),
        allowed: true,
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
                executable: std::path::Path::new("worker"),
                deadline: Instant::now() + BUDGET,
                now_ms: 100,
            },
            || (caller_environment(), Vec::new())
        ),
        Ok(())
    );
    assert_eq!(
        worker.calls.get(),
        1,
        "history is not a current observed session"
    );
}

/// An approved external host named `name` that declares `variable` for
/// `caller`, as `tmt driver install` records it.
fn external_host(directory: &Directory, name: &str, variable: &str) -> DriverRecord {
    let path = directory.0.join(format!("driver-{name}"));
    fs::write(&path, b"driver").unwrap();
    DriverRecord {
        name: name.into(),
        path: path.clone(),
        digest: "digest".into(),
        fingerprint: tmt_adapters::executable_trust::Fingerprint::of(
            &fs::metadata(&path).unwrap(),
        ),
        protocol: 1,
        locations: None,
        capabilities: serde_json::from_value(serde_json::json!({"protocols":[1],"kind":"host",
            "name":name,"version":"0.0.0-test","ops":["caller"],"paneId":{"prefix":format!("{name}-")},
            "target":"f{n}","callerEnv":[variable]}))
        .unwrap(),
        approved_at_ms: 1,
        source: Default::default(),
    }
}

/// A caller in an external pane: its variables hold a target, never a pane
/// ID, so it has no pane locator.
fn external_caller(variables: &[(&str, &str)]) -> CallerEnvironment {
    CallerEnvironment {
        tmux: None,
        pane: None,
        process_id: 100,
        driver_env: variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).into()))
            .collect(),
    }
}

/// Binds `identity` to `pane` on the external host `host`.
fn insert_external_binding(
    storage: &mut Storage,
    identity: &tmt_core::identity::Identity,
    host: &str,
    pane: &str,
) {
    let server = tmt_core::endpoint::ServerEvidence {
        host: HostKind::parse(host).unwrap(),
        server_id: format!("{host}-server"),
        socket_path: format!("/tmp/{host}.sock"),
        server_pid: 10,
        server_start_time: "server-start".into(),
    };
    let pane = tmt_core::endpoint::PaneObservation {
        agent_name: None,
        id: pane.into(),
        target: None,
        cwd: None,
        command: "sh".into(),
        pane_pid: 11,
        pane_incarnation: Some("pane-start".into()),
        suggested_name: None,
        marker: None,
        location: None,
    };
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.insert_binding(identity, &server, &pane).map(|_| ())
        })
        .unwrap();
}

fn claude_session() -> (HarnessId, CallerSession) {
    (
        HarnessId::new("claude").unwrap(),
        CallerSession {
            session: tmt_core::binding::session::ProviderSessionId::new("thread").unwrap(),
            runtime_pid: Some(20),
        },
    )
}

/// One external caller attempt at `now_ms`.
fn attempt_external(
    worker: &Worker,
    directory: &Directory,
    records: &[DriverRecord],
    caller: impl FnOnce() -> CallerEnvironment,
    now_ms: u64,
) -> Result<(), &'static str> {
    let (harness, coordinates) = claude_session();
    observe_candidate(
        worker,
        &harness,
        &coordinates,
        Discovery {
            database: &directory.0.join("state.db"),
            global_dir: &directory.0,
            executable: std::path::Path::new("worker"),
            deadline: Instant::now() + BUDGET,
            now_ms,
        },
        || (caller(), records.to_vec()),
    )
}

/// The worker resolves an external caller's pane itself, so the precheck
/// only needs a binding on that host; a refusal is then remembered by the
/// caller's variables and the host's bindings.
#[test]
fn an_external_caller_reaches_the_worker_and_its_refusal_is_remembered() {
    let directory = Directory::new();
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
        .unwrap()
        .identity;
    // A binding whose runtime is still unknown passes: the gate is not
    // presence.
    insert_external_binding(&mut storage, &identity, "fake", "fake-1");
    storage.close().unwrap();
    let records = [external_host(&directory, "fake", "FAKE_PANE_ID")];
    let worker = Worker {
        calls: Cell::new(0),
        allowed: true,
        refusal: Some("native-admission"),
    };
    let caller = || external_caller(&[("FAKE_PANE_ID", "f1")]);
    let attempt = |caller: &dyn Fn() -> CallerEnvironment, now_ms| {
        attempt_external(&worker, &directory, &records, caller, now_ms)
    };
    assert_eq!(attempt(&caller, 100), Err("native-admission"));
    assert_eq!(worker.calls.get(), 1);
    assert_eq!(attempt(&caller, 101), Err("native-admission"));
    assert_eq!(worker.calls.get(), 1, "a kept hint suppresses the worker");
    let expired = 100 + tmt_adapters::runtime::caller_refusals::TTL_MS;
    assert_eq!(attempt(&caller, expired), Err("native-admission"));
    assert_eq!(worker.calls.get(), 2, "expiry runs admission again");
    // Another pane bound on that host changes the binding set.
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    let other = create_or_resolve(&mut storage, "Other", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_external_binding(&mut storage, &other, "fake", "fake-2");
    storage.close().unwrap();
    assert_eq!(attempt(&caller, expired + 1), Err("native-admission"));
    assert_eq!(worker.calls.get(), 3, "a changed binding set runs it again");
    let moved = || external_caller(&[("FAKE_PANE_ID", "f3")]);
    assert_eq!(attempt(&moved, expired + 2), Err("native-admission"));
    assert_eq!(worker.calls.get(), 4, "a changed variable runs it again");
}

/// Exactly one approved external host, alone or beside tmux variables
/// (nested), reaches the worker when that host has a binding; no host
/// variable, an unapproved host or two hosts keep the refusal.
#[test]
fn one_named_external_host_reaches_the_worker_alone_or_nested() {
    use std::os::unix::ffi::OsStringExt;
    let directory = Directory::new();
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_external_binding(&mut storage, &identity, "fake", "fake-1");
    storage.close().unwrap();
    let fake = external_host(&directory, "fake", "FAKE_PANE_ID");
    let other = external_host(&directory, "other", "OTHER_PANE_ID");
    let not_text = || Some(std::ffi::OsString::from_vec(vec![b'/', 0xff]));
    type Row = (
        &'static str,
        Vec<DriverRecord>,
        Vec<(&'static str, &'static str)>,
        Option<std::ffi::OsString>,
        Option<std::ffi::OsString>,
        bool,
    );
    let rows: Vec<Row> = vec![
        (
            "no host variable",
            vec![fake.clone()],
            vec![],
            None,
            None,
            false,
        ),
        (
            "an unapproved host",
            vec![],
            vec![("FAKE_PANE_ID", "f1")],
            None,
            None,
            false,
        ),
        (
            "one host",
            vec![fake.clone()],
            vec![("FAKE_PANE_ID", "f1")],
            None,
            None,
            true,
        ),
        (
            "one host, empty TMUX",
            vec![fake.clone()],
            vec![("FAKE_PANE_ID", "f1")],
            Some("".into()),
            None,
            true,
        ),
        (
            "one host, empty TMUX_PANE",
            vec![fake.clone()],
            vec![("FAKE_PANE_ID", "f1")],
            None,
            Some("".into()),
            true,
        ),
        (
            "one host and TMUX",
            vec![fake.clone()],
            vec![("FAKE_PANE_ID", "f1")],
            Some("/tmp/t.sock,1,0".into()),
            None,
            true,
        ),
        (
            "one host and a TMUX that is not text",
            vec![fake.clone()],
            vec![("FAKE_PANE_ID", "f1")],
            not_text(),
            None,
            true,
        ),
        (
            "one host and a tmux pane",
            vec![fake.clone()],
            vec![("FAKE_PANE_ID", "f1")],
            Some("/tmp/test.sock,10,0".into()),
            Some("%1".into()),
            true,
        ),
        (
            "two hosts",
            vec![fake.clone(), other.clone()],
            vec![("FAKE_PANE_ID", "f1"), ("OTHER_PANE_ID", "o1")],
            None,
            None,
            false,
        ),
    ];
    for (row, records, variables, tmux, pane, reaches) in rows {
        let worker = Worker {
            calls: Cell::new(0),
            allowed: reaches,
            refusal: None,
        };
        let caller = || CallerEnvironment {
            tmux,
            pane,
            ..external_caller(&variables)
        };
        let result = attempt_external(&worker, &directory, &records, caller, 100);
        assert_eq!(
            (result, worker.calls.get()),
            if reaches {
                (Ok(()), 1)
            } else {
                (Err("binding"), 0)
            },
            "{row}"
        );
    }
    assert!(!directory.0.join("caller-session-refusals.json").exists());
}

/// The gate counts stored bindings of identities that are not retired, on
/// the caller's own host only.
#[test]
fn an_external_caller_without_a_binding_on_its_host_never_spawns() {
    let directory = Directory::new();
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_discovery_binding(&mut storage, &identity);
    let retired = create_or_resolve(&mut storage, "Retired", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_external_binding(&mut storage, &retired, "fake", "fake-1");
    storage.close().unwrap();
    let records = [external_host(&directory, "fake", "FAKE_PANE_ID")];
    let worker = Worker {
        calls: Cell::new(0),
        allowed: false,
        refusal: None,
    };
    let caller = || external_caller(&[("FAKE_PANE_ID", "f1")]);
    // Only a tmux binding, and a retired identity's binding on this host.
    rusqlite::Connection::open(directory.0.join("state.db"))
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms = 123 WHERE id = ?1",
            [&retired.id],
        )
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            attempt_external(&worker, &directory, &records, caller, 100),
            Err("binding")
        );
    }
    assert_eq!(worker.calls.get(), 0);
    assert!(!directory.0.join("caller-session-refusals.json").exists());
}

/// Records the forwarded deadline and budget, then lets the worker run past
/// its deadline.
struct Slow {
    seen: std::cell::RefCell<Option<(Instant, u128, Instant)>>,
}

impl CommandRunner for Slow {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let budget: u128 = request.args[5].to_str().unwrap().parse().unwrap();
        *self.seen.borrow_mut() = Some((request.deadline, budget, Instant::now()));
        UnixCommandRunner.execute(CommandRequest {
            program: std::ffi::OsStr::new("/bin/sleep"),
            args: &["5".into()],
            input: &[],
            deadline: request.deadline,
            max_output_bytes: 256,
        })
    }
}

/// The worker gets the precheck's own deadline, and its remaining budget
/// in milliseconds; a worker that outlives it is a remembered `budget`.
#[test]
fn an_external_worker_gets_the_precheck_deadline_and_a_timeout_is_remembered() {
    let directory = Directory::new();
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_external_binding(&mut storage, &identity, "fake", "fake-1");
    storage.close().unwrap();
    let records = [external_host(&directory, "fake", "FAKE_PANE_ID")];
    let (harness, coordinates) = claude_session();
    let slow = Slow {
        seen: Default::default(),
    };
    let started = Instant::now();
    let deadline = started + BUDGET;
    let result = observe_candidate(
        &slow,
        &harness,
        &coordinates,
        Discovery {
            database: &directory.0.join("state.db"),
            global_dir: &directory.0,
            executable: std::path::Path::new("worker"),
            deadline,
            now_ms: 100,
        },
        || (external_caller(&[("FAKE_PANE_ID", "f1")]), records.to_vec()),
    );
    assert_eq!(result, Err("budget"));
    let (forwarded, budget, spawned) = slow.seen.borrow().unwrap();
    assert_eq!(forwarded, deadline, "the absolute deadline is forwarded");
    // The budget was computed between the start and the spawn.
    let latest = deadline.saturating_duration_since(started).as_millis();
    let earliest = deadline.saturating_duration_since(spawned).as_millis();
    assert!(
        (earliest..=latest).contains(&budget) && latest <= BUDGET.as_millis(),
        "{earliest} <= {budget} <= {latest}"
    );
    let worker = Worker {
        calls: Cell::new(0),
        allowed: false,
        refusal: None,
    };
    assert_eq!(
        attempt_external(
            &worker,
            &directory,
            &records,
            || external_caller(&[("FAKE_PANE_ID", "f1")]),
            101
        ),
        Err("budget")
    );
}

#[test]
fn an_exhausted_budget_at_the_storage_open_is_a_budget_refusal() {
    let directory = Directory::new();
    let database = directory.0.join("tmt.db");
    Storage::open(&database).unwrap().close().unwrap();
    let exhausted = Storage::open_hook(&database, Instant::now()).err().unwrap();
    assert_eq!(storage_layer(exhausted), "budget");
    let missing = Storage::open_hook(
        &directory.0.join("missing.db"),
        Instant::now() + Duration::from_secs(5),
    )
    .err()
    .unwrap();
    assert_eq!(storage_layer(missing), "storage");
    // A busy or locked result that is not the spent allowance, such as a
    // close fault, stays a storage refusal.
    assert_eq!(
        storage_layer(StorageError::new(
            tmt_adapters::storage::StorageErrorCode::Busy,
            "Close storage failed",
        )),
        "storage"
    );
}

/// How a worker that already stored its session ends.
#[derive(Clone, Copy)]
enum Ending {
    /// A fault closing the database, reported as a refusal.
    CloseFault,
    /// The deadline passes before its exit is reaped.
    PastDeadline,
}

/// A worker that records the caller's session and then ends badly.
struct CommitsThenEnds {
    database: PathBuf,
    ending: Ending,
}

impl CommandRunner for CommitsThenEnds {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let (harness, coordinates) = claude_session();
        let mut storage = Storage::open(&self.database).unwrap();
        let id = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        remember_session(&mut storage, &id, &harness, &coordinates);
        admit_session(&mut storage, &id, &coordinates);
        storage.close().unwrap();
        let args: [std::ffi::OsString; 2] = match self.ending {
            Ending::CloseFault => [
                "-c".into(),
                "printf '%s\\n' 'warning: caller session not recorded: storage' >&2; exit 1".into(),
            ],
            Ending::PastDeadline => ["-c".into(), "sleep 5".into()],
        };
        UnixCommandRunner.execute(CommandRequest {
            program: std::ffi::OsStr::new("/bin/sh"),
            args: &args,
            input: &[],
            deadline: request.deadline,
            max_output_bytes: 256,
        })
    }
}

/// The stored session is the outcome of an attempt. A worker that recorded it
/// is admitted however it ends, and leaves no refusal hint behind for the next
/// attempt to be suppressed by (#3908).
fn assert_a_stored_session_is_admitted(ending: Ending) {
    {
        let directory = Directory::new();
        let database = directory.0.join("state.db");
        let mut storage = Storage::open(&database).unwrap();
        let identity = create_or_resolve(&mut storage, "Direct", Lifetime::Saved)
            .unwrap()
            .identity;
        insert_external_binding(&mut storage, &identity, "fake", "fake-1");
        storage.close().unwrap();
        let records = [external_host(&directory, "fake", "FAKE_PANE_ID")];
        let (harness, coordinates) = claude_session();
        let worker = CommitsThenEnds {
            database: database.clone(),
            ending,
        };
        let result = observe_candidate(
            &worker,
            &harness,
            &coordinates,
            Discovery {
                database: &database,
                global_dir: &directory.0,
                executable: std::path::Path::new("worker"),
                deadline: Instant::now() + Duration::from_millis(300),
                now_ms: 100,
            },
            || (external_caller(&[("FAKE_PANE_ID", "f1")]), records.to_vec()),
        );
        assert_eq!(result, Ok(()));
        assert!(
            Storage::observes_provider_session(
                &database,
                harness.as_str(),
                coordinates.session.as_str()
            )
            .unwrap(),
            "the worker stored the session"
        );
        assert!(
            !directory.0.join("caller-session-refusals.json").exists(),
            "an admitted attempt remembers no refusal"
        );
    }
}

#[test]
fn a_worker_that_stored_its_session_then_faults_closing_the_database_is_admitted() {
    assert_a_stored_session_is_admitted(Ending::CloseFault);
}

#[test]
fn a_worker_that_stored_its_session_then_outlives_the_deadline_is_admitted() {
    assert_a_stored_session_is_admitted(Ending::PastDeadline);
}

/// A caller with tmux variables beside the `fake` host's `FAKE_PANE_ID`.
fn nested_caller(
    tmux: Option<std::ffi::OsString>,
    pane: Option<std::ffi::OsString>,
    variable: &str,
) -> CallerEnvironment {
    CallerEnvironment {
        tmux,
        pane,
        ..external_caller(&[("FAKE_PANE_ID", variable)])
    }
}

/// The nested caller tmux pane `%1` on `/tmp/test.sock`, as
/// `insert_discovery_binding` binds it, in fake pane target `f1`.
fn valid_nested_caller() -> CallerEnvironment {
    nested_caller(Some("/tmp/test.sock,10,0".into()), Some("%1".into()), "f1")
}

/// The target of the newest refusal hint, read only.
fn newest_hint_target(directory: &Directory) -> serde_json::Value {
    let text = fs::read_to_string(directory.0.join("caller-session-refusals.json")).unwrap();
    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    document["entries"].as_array().unwrap().last().unwrap()["key"]["target"].clone()
}

fn update_database(directory: &Directory, sql: &str) {
    let connection = rusqlite::Connection::open(directory.0.join("state.db")).unwrap();
    assert_eq!(connection.execute(sql, []).unwrap(), 1, "{sql}");
}

/// Only the worker's nested proof tells which pane owns a nested caller, so
/// a refusal is kept under both sides: the tmux binding when the tmux pane
/// is bound, and the external host's variables and bindings with the raw
/// tmux variables. A change to any of them runs admission again; nothing
/// else does.
#[test]
fn a_nested_caller_refusal_is_kept_under_its_tmux_binding_and_its_external_host() {
    use std::os::unix::ffi::OsStringExt;
    let directory = Directory::new();
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    let outer = create_or_resolve(&mut storage, "Outer", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_discovery_binding(&mut storage, &outer);
    let hosted = create_or_resolve(&mut storage, "Hosted", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_external_binding(&mut storage, &hosted, "fake", "fake-1");
    storage.close().unwrap();
    let records = [external_host(&directory, "fake", "FAKE_PANE_ID")];
    let worker = Worker {
        calls: Cell::new(0),
        allowed: true,
        refusal: Some("native-admission"),
    };
    let calls = Cell::new(0);
    // Each attempt is refused; `runs` says whether it spawned a worker.
    let attempt = |caller: &dyn Fn() -> CallerEnvironment, now_ms, runs: bool, case: &str| {
        let result = attempt_external(&worker, &directory, &records, caller, now_ms);
        calls.set(calls.get() + usize::from(runs));
        assert_eq!(
            (result, worker.calls.get()),
            (Err("native-admission"), calls.get()),
            "{case}"
        );
    };
    attempt(&valid_nested_caller, 100, true, "both bound");
    attempt(&valid_nested_caller, 101, false, "unchanged");
    let target = newest_hint_target(&directory);
    let tmux_binding: String = rusqlite::Connection::open(directory.0.join("state.db"))
        .unwrap()
        .query_row(
            "SELECT id FROM bindings WHERE transport = 'tmux'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        target["nested"]["tmux"]["binding"], tmux_binding,
        "{target}"
    );
    assert_eq!(target["nested"]["tmux"]["pane"], "%1", "{target}");
    assert_eq!(target["nested"]["host"], "fake", "{target}");

    update_database(
        &directory,
        "UPDATE bindings SET pane_incarnation = 'pane-restart' WHERE transport = 'tmux'",
    );
    attempt(
        &valid_nested_caller,
        102,
        true,
        "a new tmux pane incarnation",
    );
    attempt(&valid_nested_caller, 103, false, "unchanged");
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    let other = create_or_resolve(&mut storage, "Other", Lifetime::Saved)
        .unwrap()
        .identity;
    insert_external_binding(&mut storage, &other, "fake", "fake-2");
    storage.close().unwrap();
    attempt(&valid_nested_caller, 104, true, "another external binding");
    let other_fake = || nested_caller(Some("/tmp/test.sock,10,0".into()), Some("%1".into()), "f2");
    attempt(&other_fake, 105, true, "another external target");
    update_database(&directory, "DELETE FROM bindings WHERE transport = 'tmux'");
    attempt(&valid_nested_caller, 106, true, "the tmux binding removed");
    assert_eq!(
        newest_hint_target(&directory)["nested"]["tmux"],
        serde_json::Value::Null
    );

    // A host variable shaped like a pane ID that is bound on its host is not
    // a tmux binding: the tmux side is read through the tmux locator alone.
    let shaped = || {
        nested_caller(
            Some("/tmp/test.sock,10,0".into()),
            Some("%1".into()),
            "fake-1",
        )
    };
    attempt(&shaped, 107, true, "a pane-shaped host variable");
    let target = newest_hint_target(&directory);
    assert_eq!(
        target["nested"]["tmux"],
        serde_json::Value::Null,
        "{target}"
    );

    // With no tmux binding, a malformed, partial or stale tmux environment
    // is kept apart from the corrected one, in both variables.
    let later = 200 + tmt_adapters::runtime::caller_refusals::TTL_MS;
    let not_text = |bytes: &[u8]| Some(std::ffi::OsString::from_vec(bytes.to_vec()));
    let tmux = || Some(std::ffi::OsString::from("/tmp/test.sock,10,0"));
    let pane = || Some(std::ffi::OsString::from("%1"));
    type Caller = Box<dyn Fn() -> CallerEnvironment>;
    let wrong: Vec<(&str, Caller)> = vec![
        (
            "TMUX not text",
            Box::new(move || nested_caller(not_text(b"/tmp/\xff"), pane(), "f1")),
        ),
        (
            "TMUX_PANE not text",
            Box::new(move || nested_caller(tmux(), not_text(b"%\xff"), "f1")),
        ),
        (
            "TMUX unset",
            Box::new(move || nested_caller(None, pane(), "f1")),
        ),
        (
            "TMUX_PANE unset",
            Box::new(move || nested_caller(tmux(), None, "f1")),
        ),
        (
            "TMUX empty",
            Box::new(move || nested_caller(Some("".into()), pane(), "f1")),
        ),
        (
            "a stale TMUX",
            Box::new(move || nested_caller(Some("/tmp/old.sock,9,0".into()), pane(), "f1")),
        ),
        (
            "another TMUX_PANE",
            Box::new(move || nested_caller(tmux(), Some("%2".into()), "f1")),
        ),
    ];
    for (offset, (case, caller)) in (0..).zip(&wrong) {
        attempt(caller.as_ref(), later + offset, true, case);
    }
    attempt(
        &valid_nested_caller,
        later + 10,
        true,
        "the corrected environment",
    );
    attempt(&valid_nested_caller, later + 11, false, "unchanged");
    for (case, caller) in &wrong {
        attempt(caller.as_ref(), later + 12, false, case);
    }
}

/// A nested caller runs the worker when either side has a binding: the tmux
/// pane alone (a tmux-owned shape, as before) is enough. With neither, it
/// never spawns and keeps no hint.
#[test]
fn a_nested_caller_reaches_the_worker_when_either_pane_is_bound() {
    let directory = Directory::new();
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    let outer = create_or_resolve(&mut storage, "Outer", Lifetime::Saved)
        .unwrap()
        .identity;
    storage.close().unwrap();
    let records = [external_host(&directory, "fake", "FAKE_PANE_ID")];
    let worker = Worker {
        calls: Cell::new(0),
        allowed: false,
        refusal: Some("native-admission"),
    };
    assert_eq!(
        attempt_external(&worker, &directory, &records, valid_nested_caller, 100),
        Err("binding")
    );
    assert!(!directory.0.join("caller-session-refusals.json").exists());
    let mut storage = Storage::open(directory.0.join("state.db")).unwrap();
    insert_discovery_binding(&mut storage, &outer);
    storage.close().unwrap();
    let worker = Worker {
        allowed: true,
        ..worker
    };
    assert_eq!(
        attempt_external(&worker, &directory, &records, valid_nested_caller, 101),
        Err("native-admission")
    );
    assert_eq!(worker.calls.get(), 1);
}
