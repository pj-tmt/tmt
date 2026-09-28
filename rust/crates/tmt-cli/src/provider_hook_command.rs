//! Hook composition with a hard supervised deadline and no provider veto output.

use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    process::{
        CommandRequest, CommandRunner, SupervisedProbeRunner, UnixCommandRunner,
        runtime::observe_runtime_process,
    },
    response_input::read_stdin_bounded,
    runtime::{CLAUDE_MODE_DEFAULT, claude},
    storage::{Storage, StorageError},
    tmux::{CallerEnvironment, OperationOptions, Tmux},
};
use tmt_core::{
    binding::{
        BindingEvidence, BindingRepository, evaluate_binding,
        session::{HarnessId, RememberedSession, RuntimeLiveness, RuntimeMode},
    },
    endpoint::EndpointProbe,
};

const BUDGET: Duration = Duration::from_secs(2);
const ERROR_LINE: &str = "tmt: lifecycle context unavailable; continuing without context.";

/// Provider hooks always exit successfully. The existing bounded process owner
/// terminates/reaps the internal worker on timeout; no daemon, permission hook,
/// unbounded background thread or potentially late context write is introduced.
pub fn execute(worker: bool) -> io::Result<u8> {
    let deadline = Instant::now() + BUDGET;
    let result = (|| {
        let input = read_stdin_bounded(BUDGET, claude::HOOK_INPUT_LIMIT).map_err(|_| ())?;
        if worker {
            return observe(&input, deadline);
        }
        let executable = std::env::current_exe().map_err(|_| ())?;
        let output = UnixCommandRunner
            .execute(CommandRequest {
                program: executable.as_os_str(),
                args: &["__hook".into(), "claude".into(), "--worker".into()],
                input: input.as_bytes(),
                deadline,
                max_output_bytes: 32 * 1024,
            })
            .map_err(|_| ())?;
        if !output.stderr.is_empty() {
            return Err(());
        }
        String::from_utf8(output.stdout).map_err(|_| ())
    })();
    let failed = result.is_err();
    match result {
        Ok(context) => {
            let _ = io::stdout().lock().write_all(context.as_bytes());
        }
        Err(()) => {
            if worker {
                // Cleanup must precede worker exit: the supervisor cannot
                // safely signal a group after reaping its numeric leader.
                let _ = SupervisedProbeRunner::abort_worker_group();
            }
            let _ = writeln!(io::stderr().lock(), "{ERROR_LINE}");
        }
    }
    // Only the internal worker reports failure to its group-owning supervisor.
    // The provider-facing hook always succeeds and never emits a veto.
    Ok(u8::from(worker && failed))
}

fn observe(input: &str, deadline: Instant) -> Result<String, ()> {
    let event = claude::decode_hook(input.as_bytes()).map_err(|_| ())?;
    let tmux = Tmux::new(SupervisedProbeRunner);
    let pane = tmux
        .caller_pane(&CallerEnvironment::current())
        .map_err(|_| ())?
        .ok_or(())?;
    let snapshot = tmux
        .observe_snapshot(OperationOptions {
            deadline: Some(deadline),
            pane_ids: Some(std::slice::from_ref(&pane)),
        })
        .map_err(|_| ())?;
    let observed_pane = snapshot
        .panes
        .iter()
        .find(|item| item.id == pane)
        .ok_or(())?;
    let process = claude::observe_in_pane(
        &SupervisedProbeRunner,
        u64::from(std::process::id()),
        observed_pane.pane_pid,
        deadline,
    )
    .ok_or(())?;
    let paths = ConfigPaths::discover().map_err(|_| ())?;
    let stored = Storage::context_by_pane(
        &paths.database,
        &pane,
        &snapshot.server.server_id,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .map_err(|_| ())?;
    let Some(stored) = stored else {
        // With no stored identity, only emit the fixed naming hint. The
        // command-derived suggested_name (for example "claude") is not a
        // binding marker and supplies no identity or context to this branch.
        if observed_pane.marker.is_some() {
            return Err(());
        }
        return if event.starting {
            Ok(
                claude::encode_context(&crate::context_command::unbound_text().map_err(|_| ())?)
                    .unwrap_or_default(),
            )
        } else {
            Ok(String::new())
        };
    };
    if !matches!(
        evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot.clone())),
        BindingEvidence::Active(_)
    ) {
        return Err(());
    }
    let binding = stored.entry.binding.as_ref().ok_or(())?;
    let previous = match &binding.session.key {
        Some(key) if key.incarnation == process => RuntimeLiveness::Alive,
        Some(key) => {
            observe_runtime_process(&SupervisedProbeRunner, key.incarnation.pid(), deadline)
                .map_err(|_| ())?
                .matches(&key.incarnation)
        }
        None => RuntimeLiveness::Unknown,
    };
    let next = event
        .propose(&binding.session, &process, previous)
        .ok_or(())?;
    if Instant::now() >= deadline {
        return Err(());
    }
    let mut storage = Storage::open_hook(&paths.database).map_err(|_| ())?;
    let changed = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let current = records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding);
            if current.as_ref() != Some(binding) {
                return Ok(false);
            }
            if !records.set_session_state(&binding.id, &binding.session, &next)? {
                return Ok(false);
            }
            if event.starting {
                let mut preferences = records.session_preferences(&binding.identity_id)?;
                let harness = HarnessId::new("claude").expect("first-party harness");
                preferences.preferred_harness = Some(harness.clone());
                preferences.remembered = Some(RememberedSession {
                    harness,
                    mode: RuntimeMode::new(CLAUDE_MODE_DEFAULT).expect("first-party mode"),
                    provider_session: event.session.clone(),
                });
                return records.set_session_preferences(&binding.identity_id, &preferences);
            }
            Ok(true)
        })
        .map_err(|_| ())?;
    storage.close().map_err(|_| ())?;
    if !changed || Instant::now() >= deadline {
        return Err(());
    }
    if !event.starting {
        return Ok(String::new());
    }
    // Re-read the bounded projection after acknowledgment. Do not inject a
    // different binding if a concurrent rebind occurred during the callback.
    let refreshed = Storage::context_by_pane(
        &paths.database,
        &pane,
        &snapshot.server.server_id,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .map_err(|_| ())?
    .ok_or(())?;
    if refreshed
        .entry
        .binding
        .as_ref()
        .is_none_or(|value| value.id != binding.id || value.session != next)
        || !matches!(
            evaluate_binding(&refreshed.entry, &EndpointProbe::Live(snapshot)),
            BindingEvidence::Active(_)
        )
    {
        return Err(());
    }
    let context = crate::context_command::render_verified(refreshed, &paths).map_err(|_| ())?;
    claude::encode_context(&context).ok_or(())
}
