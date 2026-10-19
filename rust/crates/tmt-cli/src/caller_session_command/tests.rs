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
    insert_discovery_binding(&mut storage, &identity);
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            use tmt_core::binding::session::{ObservedSessionKey, SessionTransition};
            let binding = records.entry_by_id(&id)?.unwrap().binding.unwrap();
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

/// Only exactly one approved external host and no nonempty tmux variable
/// take the external path; every other caller keeps the current refusal.
#[test]
fn only_one_named_external_host_without_tmux_reaches_the_worker() {
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
            false,
        ),
        (
            "one host and a TMUX that is not text",
            vec![fake.clone()],
            vec![("FAKE_PANE_ID", "f1")],
            not_text(),
            None,
            false,
        ),
        (
            "one host and a tmux pane",
            vec![fake.clone()],
            vec![("FAKE_PANE_ID", "f1")],
            Some("/tmp/test.sock,10,0".into()),
            Some("%1".into()),
            false,
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
fn a_spent_budget_is_the_budget_layer_and_other_storage_refusals_stay_storage() {
    use tmt_adapters::storage::StorageErrorCode;
    // No budget left: refused before the file is touched, so no database is needed.
    let spent = Storage::open_hook(std::path::Path::new("/nonexistent/tmt.db"), Instant::now())
        .err()
        .unwrap();
    assert_eq!(spent.code, StorageErrorCode::Busy);
    assert_eq!(storage_layer(&spent), "budget");
    // The same `Busy` code from contention, and a real failure, are storage.
    let busy = StorageError::new(StorageErrorCode::Busy, "database is locked");
    assert_eq!(storage_layer(&busy), "storage");
    let corrupt = StorageError::new(StorageErrorCode::Corrupt, "malformed");
    assert_eq!(storage_layer(&corrupt), "storage");
}
