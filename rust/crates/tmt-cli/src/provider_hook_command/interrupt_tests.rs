//! An interrupt's commit shares the turn-end CAS (#3207): a binding or
//! preferences change between the hook's read and its commit wins.
use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tmt_core::{
    binding::session::{
        RuntimeMode, SessionPreferences,
        activity::{ActivityPhase, Event},
    },
    endpoint::{PaneObservation, ServerEvidence},
    identity::{Lifetime, create_or_resolve},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-activity-cas-{}-{}",
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

fn turn(
    phase: ActivityPhase,
    id: &str,
) -> tmt_adapters::runtime::hook_protocol::ActivityObservation {
    tmt_adapters::runtime::hook_protocol::ActivityObservation {
        event: Event {
            kind: match phase {
                ActivityPhase::Working => {
                    tmt_core::binding::session::activity::EventKind::TurnStarted
                }
                ActivityPhase::Waiting => {
                    tmt_core::binding::session::activity::EventKind::ApprovalPrompt
                }
                ActivityPhase::Idle => tmt_core::binding::session::activity::EventKind::TurnEnded,
            },
            turn: Some(ProviderSessionId::new(id).unwrap()),
        },
        model: None,
    }
}

fn rows(database: &std::path::Path, table: &str) -> i64 {
    rusqlite::Connection::open(database)
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn a_racing_binding_or_preferences_change_wins_over_an_interrupt() {
    assert_activity_cas("codex", "Interrupt", ActivityPhase::Idle);
}

#[test]
fn a_racing_binding_or_preferences_change_wins_over_an_approval_notification() {
    assert_activity_cas("claude", "Notification", ActivityPhase::Waiting);
}

fn assert_activity_cas(provider: &str, event: &str, expected_phase: ActivityPhase) {
    let registry = RuntimeRegistry::first_party();
    let driver = registry
        .lifecycle(&HarnessId::new(provider).unwrap())
        .unwrap();
    let session = ProviderSessionId::new("s-1").unwrap();
    let process = ProcessIncarnation::new(20, "runtime").unwrap();
    let interrupt = driver
        .decode_activity(
            serde_json::json!({
                "hook_event_name":event, "session_id":"s-1", "turn_id":"a",
                "notification_type":"permission_prompt", "model":"hook-model",
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let start = |id| {
        let mut event = turn(ActivityPhase::Working, id);
        if provider == "claude" {
            event.event.turn = None;
        }
        event
    };
    let observed = |state: &tmt_core::binding::session::DriverState| {
        driver
            .state_observed_model(state)
            .map(|observed| observed.model)
    };
    for race in ["unchanged", "preferences", "binding"] {
        let directory = Directory::new();
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
        let pane = PaneObservation {
            agent_name: None,
            id: "%1".into(),
            target: None,
            cwd: None,
            command: provider.into(),
            pane_pid: 11,
            pane_incarnation: Some("pane".into()),
            suggested_name: None,
            marker: None,
            location: None,
        };
        let harness = HarnessId::new(provider).unwrap();
        let mode = RuntimeMode::new("embedded").unwrap();
        let mut preferences = SessionPreferences::default();
        preferences.remember(harness.clone(), mode.clone(), session.clone());
        let working = driver
            .activity_state(&start("a"), &session, &process, None, 100)
            .unwrap();
        preferences.remembered.as_mut().unwrap().state = Some(working.clone());
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                records.insert_binding(&identity, &server, &pane)?;
                records.set_session_preferences(&identity.id, &preferences)
            })
            .unwrap();
        storage.close().unwrap();
        let stored = Storage::context_by_identity(&paths.database, &identity.id, 0)
            .unwrap()
            .unwrap();
        let next = driver.activity_state(&interrupt, &session, &process, Some(&working), 200);
        assert!(next.is_some());

        // The concurrent writer commits between the hook's read and its commit.
        let mut storage = Storage::open(&paths.database).unwrap();
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                match race {
                    "preferences" => {
                        let mut racer = start("b");
                        racer.model = (provider == "codex").then(|| "racer-model".into());
                        let later =
                            driver.activity_state(&racer, &session, &process, Some(&working), 150);
                        preferences.remembered.as_mut().unwrap().state = later;
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
        storage.close().unwrap();
        let before = Storage::context_by_identity(&paths.database, &identity.id, 0)
            .unwrap()
            .unwrap();
        let applied = commit_observation(
            &paths,
            &stored,
            next.clone(),
            None,
            None,
            None,
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        let after = Storage::context_by_identity(&paths.database, &identity.id, 0)
            .unwrap()
            .unwrap();
        assert_eq!(applied, race == "unchanged", "{race}");
        if applied {
            let state = after.preferences.remembered.unwrap().state.unwrap();
            assert_eq!(Some(&state), next.as_ref());
            assert_eq!(driver.state_activity(&state).unwrap().phase, expected_phase);
            assert_eq!(
                observed(&state).as_deref(),
                (provider == "codex").then_some("hook-model")
            );
        } else {
            assert_eq!(after.entry, before.entry, "{race}");
            assert_eq!(after.preferences, before.preferences, "{race}");
            // The losing hook's model is never stored. A racing Claude prompt
            // can clear optional state, which must stay cleared after this CAS.
            let state = after.preferences.remembered.unwrap().state;
            let expected = (race == "preferences" && provider == "codex").then_some("racer-model");
            assert_eq!(
                state.as_ref().and_then(observed).as_deref(),
                expected,
                "{race}"
            );
        }
        // An interrupt never adds a consumption source or a response.
        assert_eq!(rows(&paths.database, "consumption_sources"), 0);
        assert_eq!(rows(&paths.database, "recent_responses"), 0);
    }
}

#[test]
fn tool_completions_probe_only_when_storage_reports_waiting_and_never_write_the_precheck() {
    use tmt_core::binding::session::{
        DriverState,
        activity::{Activity, EventKind},
    };
    struct Probe(AtomicU64);
    impl RuntimeLifecycle for Probe {
        fn host_evidence(
            &self,
        ) -> Result<HostEvidence, tmt_adapters::runtime::lifecycle::LifecycleUnavailable> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(HostEvidence::Unsupported)
        }
        fn state_activity(&self, state: &DriverState) -> Option<Activity> {
            tmt_adapters::runtime::driver_state::state_activity(state)
        }
    }
    let registry = RuntimeRegistry::first_party();
    let claude = registry
        .lifecycle(&HarnessId::new("claude").unwrap())
        .unwrap();
    let harness = HarnessId::new("claude").unwrap();
    let session = ProviderSessionId::new("session").unwrap();
    let process = ProcessIncarnation::new(20, "runtime").unwrap();
    for (phase, probes) in [
        (ActivityPhase::Working, 0),
        (ActivityPhase::Waiting, 1),
        (ActivityPhase::Idle, 0),
    ] {
        let directory = Directory::new();
        let paths = ConfigPaths {
            global_dir: directory.0.clone(),
            global_config: directory.0.join("config.json"),
            local_config: directory.0.join("local.json"),
            database: directory.0.join("state.db"),
        };
        let mut storage = Storage::open(&paths.database).unwrap();
        let identity = create_or_resolve(&mut storage, "Tool agent", Lifetime::Saved)
            .unwrap()
            .identity;
        let start = tmt_adapters::runtime::hook_protocol::ActivityObservation {
            event: Event {
                kind: EventKind::TurnStarted,
                turn: None,
            },
            model: None,
        };
        let working = claude
            .activity_state(&start, &session, &process, None, 100)
            .unwrap();
        let state = if phase == ActivityPhase::Working {
            working
        } else {
            claude
                .activity_state(
                    &tmt_adapters::runtime::hook_protocol::ActivityObservation {
                        event: Event {
                            kind: if phase == ActivityPhase::Waiting {
                                EventKind::ApprovalPrompt
                            } else {
                                EventKind::TurnEnded
                            },
                            turn: None,
                        },
                        model: None,
                    },
                    &session,
                    &process,
                    Some(&working),
                    101,
                )
                .unwrap()
        };
        storage.close().unwrap();
        let connection = rusqlite::Connection::open(&paths.database).unwrap();
        connection.execute("INSERT INTO bindings (id, identity_id, transport, pane_id, server_id, socket_path, server_pid, server_start_time, pane_pid, bound_at, last_verified_at, runtime_pid, runtime_start_identity, observed_provider_session_id) VALUES ('binding', ?1, 'tmux', '%1', 'server', '/tmp/test.sock', 10, 'start', 11, 'original', 'original', 20, 'runtime', 'session')", [&identity.id]).unwrap();
        connection.execute("INSERT INTO identity_session_preferences (identity_id, remembered_harness, runtime_mode, provider_session_id, driver_state_version, driver_state) VALUES (?1, 'claude', 'embedded', 'session', ?2, ?3)", rusqlite::params![identity.id, state.version(), state.document()]).unwrap();
        drop(connection);
        let before = fs::read(&paths.database).unwrap();
        for event in ["PostToolUse", "PostToolUseFailure"] {
            let activity = claude
                .decode_activity(
                    serde_json::json!({"hook_event_name":event,"session_id":"session"})
                        .to_string()
                        .as_bytes(),
                )
                .unwrap();
            let probe = Probe(AtomicU64::new(0));
            let output = observe_activity_event(
                &harness,
                &probe,
                &session,
                &activity,
                Instant::now() + Duration::from_secs(2),
                || Ok(paths.clone()),
            )
            .unwrap();
            assert_eq!(output.context, "");
            assert!(
                output.workspace.is_none(),
                "unadmitted/no-op tool events cannot request recovery probes"
            );
            assert_eq!(probe.0.load(Ordering::Relaxed), probes, "{phase:?} {event}");
            assert_eq!(fs::read(&paths.database).unwrap(), before);
        }
    }
}

/// Initial learning is one binding/preferences CAS, not a replacement path.
/// Even saved channel and preferred-driver choices survive learning a session.
#[test]
fn initial_session_adoption_preserves_preferences_and_loses_to_concurrent_changes() {
    use tmt_core::binding::session::{
        BindingSessionState, ObservedSessionKey, RuntimeState, SessionTransition,
    };
    for case in [
        "unchanged",
        "preferences",
        "binding",
        "observed",
        "ended",
        "remembered",
    ] {
        let directory = Directory::new();
        let database = directory.0.join("state.db");
        let mut storage = Storage::open(&database).unwrap();
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
        let session = ProviderSessionId::new("s-1").unwrap();
        let mode = RuntimeMode::new("default").unwrap();
        let mut preferences = SessionPreferences {
            channel: Some(false),
            preferred_harness: Some(HarnessId::new("codex").unwrap()),
            remembered: None,
        };
        let next = BindingSessionState::default()
            .admit(
                ObservedSessionKey {
                    incarnation: ProcessIncarnation::new(20, "runtime").unwrap(),
                    provider_session: Some(session.clone()),
                },
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                let binding = records.insert_binding(&identity, &server, &pane)?;
                if matches!(case, "observed" | "ended") {
                    let mut old = next.clone();
                    if case == "ended" {
                        old.state = RuntimeState::Ended;
                        old.last_transition = Some(SessionTransition::Ended);
                    }
                    records.set_session_state(&binding.id, &binding.session, &old)?;
                }
                if case == "remembered" {
                    preferences.remember(
                        harness.clone(),
                        mode.clone(),
                        ProviderSessionId::new("old").unwrap(),
                    );
                }
                records.set_session_preferences(&identity.id, &preferences)
            })
            .unwrap();
        storage.close().unwrap();
        let stored = Storage::context_by_identity(&database, &identity.id, 0)
            .unwrap()
            .unwrap();
        let mut proposed_preferences = stored.preferences.clone();
        proposed_preferences.remember(harness, mode, session);
        let mut storage = Storage::open(&database).unwrap();
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                match case {
                    "preferences" => {
                        let mut changed = preferences.clone();
                        changed.channel = Some(true);
                        records.set_session_preferences(&identity.id, &changed)?;
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
        storage.close().unwrap();
        let before = Storage::context_by_identity(&database, &identity.id, 0)
            .unwrap()
            .unwrap();
        let mut storage = Storage::open(&database).unwrap();
        let changed =
            commit_initial_session(&mut storage, &stored, &next, &proposed_preferences).unwrap();
        storage.close().unwrap();
        let after = Storage::context_by_identity(&database, &identity.id, 0)
            .unwrap()
            .unwrap();
        assert_eq!(changed, case == "unchanged", "{case}");
        if changed {
            assert_eq!(after.entry.binding.unwrap().session, next);
            assert_eq!(after.preferences, proposed_preferences);
            assert_eq!(after.preferences.channel, Some(false));
            assert_eq!(
                after.preferences.preferred_harness.unwrap().as_str(),
                "codex"
            );
        } else {
            assert_eq!(after.entry, before.entry, "{case}");
            assert_eq!(after.preferences, before.preferences, "{case}");
        }
    }
}
