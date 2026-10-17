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
                ActivityPhase::Working => tmt_core::binding::session::activity::EventKind::TurnStarted,
                ActivityPhase::Waiting => tmt_core::binding::session::activity::EventKind::ApprovalPrompt,
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
        driver.state_observed_model(state).map(|observed| observed.model)
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
                        let later = driver.activity_state(&racer, &session, &process, Some(&working), 150);
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
            assert_eq!(observed(&state).as_deref(), (provider == "codex").then_some("hook-model"));
        } else {
            assert_eq!(after.entry, before.entry, "{race}");
            assert_eq!(after.preferences, before.preferences, "{race}");
            // The losing hook's model is never stored.
            let state = after.preferences.remembered.unwrap().state.unwrap();
            let expected = (race == "preferences" && provider == "codex").then_some("racer-model");
            assert_eq!(observed(&state).as_deref(), expected, "{race}");
        }
        // An interrupt never adds a consumption source or a response.
        assert_eq!(rows(&paths.database, "consumption_sources"), 0);
        assert_eq!(rows(&paths.database, "recent_responses"), 0);
    }
}
