//! Read-only recovery input. No capture, publication, migration or provider IO.

use crate::{config::ConfigPaths, host::Host, process::CommandRunner, storage::Storage};
use serde_json::{Value, json};
use std::{io, time::Instant};
use tmt_core::workspace::{StoredIdentity, WorkspaceSnapshot, plan::restore_plan};

pub struct WorkspacePlanInput {
    pub snapshot: WorkspaceSnapshot,
    pub identities: Vec<StoredIdentity>,
    pub current_sessions: Option<Vec<String>>,
}

pub fn read<R: CommandRunner + Clone>(
    paths: &ConfigPaths,
    socket: &str,
    host: &Host<R>,
    deadline: Instant,
) -> io::Result<WorkspacePlanInput> {
    let snapshot = super::read_snapshot(paths, socket)?;
    let ids: Vec<&str> = snapshot
        .panes
        .iter()
        .filter_map(|pane| pane.identity.as_ref().map(|identity| identity.id.as_str()))
        .collect();
    let identities = match std::fs::symlink_metadata(&paths.database) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
        Ok(_) => {
            Storage::workspace_plan_identities(&paths.database, &ids).map_err(io::Error::other)?
        }
    };
    let current_sessions = match std::fs::symlink_metadata(socket) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(Vec::new()),
        _ => host
            .workspace_session_names(socket, deadline)
            .ok()
            .flatten(),
    };
    Ok(WorkspacePlanInput {
        snapshot,
        identities,
        current_sessions,
    })
}

/// Retain the existing snapshot codec's topology rather than defining a second
/// layout format. Resume details always come from the current durable projection.
pub fn document(input: &WorkspacePlanInput) -> io::Result<Value> {
    let plan = restore_plan(
        &input.snapshot,
        &input.identities,
        input.current_sessions.as_deref(),
    );
    let snapshot: Value =
        serde_json::from_slice(&super::encode(&input.snapshot)?).map_err(io::Error::other)?;
    Ok(json!({
        "version": 1, "snapshot": snapshot,
        "sessions": plan.sessions.iter().map(|(id, action)| json!({"session": id, "action": action.as_str()})).collect::<Vec<_>>(),
        "panes": plan.panes.iter().map(|pane| json!({
            "pane": pane.pane, "action": pane.action.as_str(),
            "identity": pane.identity.map(|record| json!({"id": record.entry.identity.id, "name": record.entry.identity.name, "lifetime": record.entry.identity.lifetime.as_str(), "channel": record.preferences.channel})),
            "resume": pane.remembered.map(|session| json!({"harness": session.harness.as_str(), "session": session.provider_session.as_str(), "mode": session.mode.as_str(), "staleAtMs": session.stale_at_ms, "resumePendingAtMs": session.resume_pending_at_ms}))
        })).collect::<Vec<_>>()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        host::CallerEnvironment,
        process::{CommandError, CommandOutput, CommandRequest},
    };
    use crate::{scripted_runner::ScriptedRunner, test_support::TestDirectory};
    use tmt_core::{
        binding::{
            BindingRepository,
            session::{
                HarnessId, ProviderSessionId, RememberedSession, RuntimeMode, SessionPreferences,
            },
        },
        identity::{Lifetime, create_or_resolve},
    };

    #[derive(Clone)]
    struct PlanRunner(std::rc::Rc<ScriptedRunner>);
    impl CommandRunner for PlanRunner {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            self.0.execute(request)
        }
    }
    fn host(runner: ScriptedRunner) -> Host<PlanRunner> {
        Host::for_caller_with(
            &CallerEnvironment {
                tmux: None,
                pane: None,
                process_id: 10,
                driver_env: Default::default(),
            },
            PlanRunner(std::rc::Rc::new(runner)),
        )
    }

    #[test]
    fn readonly_projection_keeps_unbound_saved_preferences_and_excludes_retired_uuid() {
        let directory = TestDirectory::new();
        let paths = super::super::tests::paths(&directory.path);
        let mut storage = Storage::open(&paths.database).unwrap();
        let kept = create_or_resolve(&mut storage, "kept", Lifetime::Saved)
            .unwrap()
            .identity;
        let retired = create_or_resolve(&mut storage, "retired", Lifetime::Saved)
            .unwrap()
            .identity;
        let preferences = SessionPreferences {
            channel: Some(true),
            preferred_harness: Some(HarnessId::new("claude").unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new("codex").unwrap(),
                mode: RuntimeMode::new("independent").unwrap(),
                provider_session: ProviderSessionId::new("current-session").unwrap(),
                state: None,
                stale_at_ms: Some(3),
                resume_pending_at_ms: Some(2),
            }),
        };
        storage
            .with_binding_transaction(|records| {
                records.set_session_preferences(&kept.id, &preferences)
            })
            .unwrap();
        storage
            .with_binding_transaction(|records| records.retire_identity(&retired, false))
            .unwrap();
        storage.close().unwrap();
        let before = std::fs::read(&paths.database).unwrap();
        let records =
            Storage::workspace_plan_identities(&paths.database, &[&kept.id, &retired.id]).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].entry.identity.id, kept.id);
        assert!(records[0].entry.binding.is_none());
        assert_eq!(records[0].preferences, preferences);
        assert_eq!(std::fs::read(&paths.database).unwrap(), before);
    }

    #[test]
    fn complete_session_observation_skips_existing_and_failed_observation_stays_unknown() {
        let directory = TestDirectory::new();
        let paths = super::super::tests::paths(&directory.path);
        let mut snapshot = super::super::tests::sample();
        snapshot.server.socket = directory.path.join("socket").display().to_string();
        super::super::publish_event(&paths, &snapshot.server.socket, || Ok(snapshot.clone()))
            .unwrap();
        // An existing filesystem entry requires the actual bounded tmux observation.
        std::fs::write(&snapshot.server.socket, b"fixture").unwrap();
        let runner = ScriptedRunner::default();
        runner.push_output(b"workspace\nother\n".to_vec(), Vec::new());
        let input = read(
            &paths,
            &snapshot.server.socket,
            &host(runner),
            Instant::now() + super::super::CAPTURE_BUDGET,
        )
        .unwrap();
        assert_eq!(
            document(&input).unwrap()["sessions"][0]["action"],
            "skip_existing"
        );
        let runner = ScriptedRunner::default();
        runner.push_output(b"\0bad\n".to_vec(), Vec::new());
        let input = read(
            &paths,
            &snapshot.server.socket,
            &host(runner),
            Instant::now() + super::super::CAPTURE_BUDGET,
        )
        .unwrap();
        assert_eq!(
            document(&input).unwrap()["sessions"][0]["action"],
            "unknown"
        );
        assert!(!paths.database.exists());
    }
}
