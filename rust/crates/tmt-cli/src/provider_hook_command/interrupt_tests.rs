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
            "tmt-interrupt-cas-{}-{}",
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
            phase,
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
    let codex = RuntimeRegistry::first_party();
    let codex = codex.lifecycle(&HarnessId::new("codex").unwrap()).unwrap();
    let session = ProviderSessionId::new("s-1").unwrap();
    let process = ProcessIncarnation::new(20, "runtime").unwrap();
    let interrupt = codex
        .decode_activity(br#"{"hook_event_name":"Interrupt","session_id":"s-1","turn_id":"a"}"#)
        .unwrap();
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
            command: "codex".into(),
            pane_pid: 11,
            pane_incarnation: Some("pane".into()),
            suggested_name: None,
            marker: None,
            location: None,
        };
        let harness = HarnessId::new("codex").unwrap();
        let mode = RuntimeMode::new("embedded").unwrap();
        let mut preferences = SessionPreferences::default();
        preferences.remember(harness.clone(), mode.clone(), session.clone());
        let working = codex
            .activity_state(
                &turn(ActivityPhase::Working, "a"),
                &session,
                &process,
                None,
                100,
            )
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
        let next = codex.activity_state(&interrupt, &session, &process, Some(&working), 200);
        assert!(next.is_some());

        // The concurrent writer commits between the hook's read and its commit.
        let mut storage = Storage::open(&paths.database).unwrap();
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                match race {
                    "preferences" => {
                        let later = codex.activity_state(
                            &turn(ActivityPhase::Working, "b"),
                            &session,
                            &process,
                            Some(&working),
                            150,
                        );
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
            assert_eq!(
                codex.state_activity(&state).unwrap().phase,
                ActivityPhase::Idle
            );
        } else {
            assert_eq!(after.entry, before.entry, "{race}");
            assert_eq!(after.preferences, before.preferences, "{race}");
        }
        // An interrupt never adds a consumption source or a response.
        assert_eq!(rows(&paths.database, "consumption_sources"), 0);
        assert_eq!(rows(&paths.database, "recent_responses"), 0);
    }
}
