//! Hook composition with a hard supervised deadline and no provider veto output.

use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host, OperationOptions},
    process::{
        CommandRequest, CommandRunner, SupervisedProbeRunner, UnixCommandRunner,
        runtime::observe_runtime_process,
    },
    response_input::read_stdin_bounded,
    runtime::{
        RuntimeRegistry,
        lifecycle::{HostEvidence, RuntimeLifecycle, TurnEnd},
    },
    skill_installation::ProviderEnvironment,
    storage::{IdentityContextSnapshot, Storage, StorageError},
};
use tmt_core::{
    binding::{
        BindingEvidence, BindingRepository, evaluate_binding,
        session::{HarnessId, ProviderSessionId, RuntimeLiveness},
    },
    endpoint::{EndpointProbe, EndpointSnapshot, ProcessIncarnation},
};

const BUDGET: Duration = Duration::from_secs(2);
const ERROR_LINE: &str = "tmt: lifecycle context unavailable; continuing without context.";

/// Provider hooks always exit successfully. The existing bounded process owner
/// terminates/reaps the internal worker on timeout; no daemon, permission hook,
/// unbounded background thread or potentially late context write is introduced.
pub fn execute(provider: &str, worker: bool) -> io::Result<u8> {
    let deadline = Instant::now() + BUDGET;
    // A turn end fires after every turn; its failures stay silent.
    let mut turn_end = false;
    let result = (|| {
        let input = read_stdin_bounded(
            BUDGET,
            tmt_adapters::runtime::hook_protocol::HOOK_INPUT_LIMIT,
        )
        .map_err(|_| ())?;
        turn_end = RuntimeRegistry::first_party()
            .lifecycle(&HarnessId::new(provider).map_err(|_| ())?)
            .is_some_and(|lifecycle| lifecycle.decode_turn(input.as_bytes()).is_some());
        if worker {
            return observe(provider, &input, deadline);
        }
        let executable = std::env::current_exe().map_err(|_| ())?;
        let output = UnixCommandRunner
            .execute(CommandRequest {
                program: executable.as_os_str(),
                args: &["__hook".into(), provider.into(), "--worker".into()],
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
            if !turn_end {
                let _ = writeln!(io::stderr().lock(), "{ERROR_LINE}");
            }
        }
    }
    // Only the internal worker reports failure to its group-owning supervisor.
    // The provider-facing hook always succeeds and never emits a veto.
    Ok(u8::from(worker && failed))
}

/// A turn end refreshes only the remembered session's driver state (#519).
/// It needs the same verified, active binding as a lifecycle event, and the
/// binding's current conversation must be the remembered one it names: a late
/// turn end from an earlier conversation writes nothing. The write is a
/// compare-and-set on the preferences read with the binding.
fn observe_turn(
    provider: &str,
    harness: &HarnessId,
    lifecycle: &dyn RuntimeLifecycle,
    turn: &TurnEnd,
    deadline: Instant,
) -> Result<(), ()> {
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return Ok(());
    }
    let Caller::Bound(bound) = verified_caller(provider, lifecycle, host, &turn.session, deadline)?
    else {
        return Ok(());
    };
    let BoundCaller {
        paths,
        stored,
        snapshot,
        process,
        ..
    } = *bound;
    let binding = stored.entry.binding.as_ref().ok_or(())?;
    if !matches!(
        evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot)),
        BindingEvidence::Active(_)
    ) {
        return Err(());
    }
    let current = binding.session.key.as_ref().is_some_and(|key| {
        key.incarnation == process && key.provider_session.as_ref() == Some(&turn.session)
    });
    let Some(remembered) = stored.preferences.remembered.as_ref().filter(|remembered| {
        current && &remembered.harness == harness && remembered.provider_session == turn.session
    }) else {
        return Ok(());
    };
    let environment = ProviderEnvironment::capture().map_err(|_| ())?;
    let Some(next) = lifecycle.turn_state(
        turn,
        &environment,
        remembered.state.as_ref(),
        tmt_adapters::request_runtime::wall_time_ms(),
    ) else {
        return Ok(());
    };
    if Instant::now() >= deadline {
        return Err(());
    }
    let mut storage = Storage::open_hook(&paths.database).map_err(|_| ())?;
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let current = records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding);
            let mut preferences = records.session_preferences(&binding.identity_id)?;
            if current.as_ref() != Some(binding) || preferences != stored.preferences {
                return Ok(false);
            }
            if let Some(remembered) = preferences.remembered.as_mut() {
                remembered.state = Some(next);
            }
            records.set_session_preferences(&binding.identity_id, &preferences)
        })
        .map_err(|_| ())?;
    storage.close().map_err(|_| ())
}

/// The caller a hook event came from, verified the same way for every event.
enum Caller {
    /// No pane (or, in shared mode, no stored session) to attribute it to.
    Unobserved,
    /// A verified pane with no stored identity; `marked` when it still holds
    /// a TMT marker, which is inconsistent evidence.
    Unbound {
        marked: bool,
    },
    Bound(Box<BoundCaller>),
}

struct BoundCaller {
    host: HostEvidence,
    paths: ConfigPaths,
    stored: IdentityContextSnapshot,
    snapshot: EndpointSnapshot,
    process: ProcessIncarnation,
}

fn verified_caller(
    provider: &str,
    lifecycle: &dyn RuntimeLifecycle,
    host: HostEvidence,
    session: &ProviderSessionId,
    deadline: Instant,
) -> Result<Caller, ()> {
    let paths = ConfigPaths::discover().map_err(|_| ())?;
    let now = tmt_adapters::request_runtime::wall_time_ms();
    let shared = host.shared();
    let (stored, snapshot, process) = if shared {
        let Some(stored) =
            Storage::context_by_provider_session(&paths.database, provider, session.as_str(), now)
                .map_err(|_| ())?
        else {
            return Ok(Caller::Unobserved);
        };
        let binding = stored.entry.binding.as_ref().ok_or(())?;
        let pane = &binding.pane_id;
        let EndpointProbe::Live(snapshot) =
            Host::for_server_with(&binding.server, SupervisedProbeRunner)
                .probe(
                    &binding.server,
                    OperationOptions {
                        deadline: Some(deadline),
                        pane_ids: Some(std::slice::from_ref(pane)),
                    },
                )
                .map_err(|_| ())?
        else {
            return Err(());
        };
        let pid = u64::from(host.runtime_pid().ok_or(())?);
        let tmt_adapters::process::runtime::ProcessObservation::Live(process) =
            observe_runtime_process(&SupervisedProbeRunner, pid, deadline).map_err(|_| ())?
        else {
            return Err(());
        };
        (stored, snapshot, process)
    } else {
        let environment = CallerEnvironment::current();
        let panes = Host::for_caller_with(&environment, SupervisedProbeRunner);
        let Some(pane) = panes.caller_pane(&environment).map_err(|_| ())? else {
            return if Instant::now() < deadline {
                Ok(Caller::Unobserved)
            } else {
                Err(())
            };
        };
        let snapshot = panes
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
        let process = lifecycle
            .observe_in_pane(
                u64::from(std::process::id()),
                observed_pane.pane_pid,
                deadline,
            )
            .ok_or(())?;
        if host
            .runtime_pid()
            .is_some_and(|pid| process.pid() != u64::from(pid))
        {
            return Err(());
        }
        let stored = Storage::context_by_pane(
            &paths.database,
            snapshot.server.host,
            &pane,
            &snapshot.server.server_id,
            tmt_adapters::request_runtime::wall_time_ms(),
        )
        .map_err(|_| ())?;
        let Some(stored) = stored else {
            return Ok(Caller::Unbound {
                marked: observed_pane.marker.is_some(),
            });
        };
        (stored, snapshot, process)
    };
    Ok(Caller::Bound(Box::new(BoundCaller {
        host,
        paths,
        stored,
        snapshot,
        process,
    })))
}

fn observe(provider: &str, input: &str, deadline: Instant) -> Result<String, ()> {
    let registry = RuntimeRegistry::first_party();
    let harness = HarnessId::new(provider).map_err(|_| ())?;
    let lifecycle = registry.lifecycle(&harness).ok_or(())?;
    if let Some(turn) = lifecycle.decode_turn(input.as_bytes()) {
        return observe_turn(provider, &harness, lifecycle, &turn, deadline)
            .map(|()| String::new());
    }
    if let Some(session) = lifecycle.decode_prompt(input.as_bytes()) {
        return observe_prompt(provider, &harness, lifecycle, &session, deadline);
    }
    let event = lifecycle.decode(input.as_bytes()).ok_or(())?;
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return Ok(String::new());
    }
    let BoundCaller {
        host,
        paths,
        stored,
        snapshot,
        process,
    } = match verified_caller(provider, lifecycle, host, event.session(), deadline)? {
        Caller::Unobserved => return Ok(String::new()),
        // With no stored identity, only emit the fixed naming hint. The
        // command-derived suggested_name (for example "claude") is not a
        // binding marker and supplies no identity or context to this branch.
        Caller::Unbound { marked: true } => return Err(()),
        Caller::Unbound { marked: false } => {
            return if event.starting() {
                Ok(lifecycle
                    .encode_context(&crate::context_command::unbound_text().map_err(|_| ())?)
                    .unwrap_or_default())
            } else {
                Ok(String::new())
            };
        }
        Caller::Bound(bound) => *bound,
    };
    let binding = stored.entry.binding.as_ref().ok_or(())?;
    let pane = binding.pane_id.clone();
    if !matches!(
        evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot.clone())),
        BindingEvidence::Active(_)
    ) {
        return Err(());
    }
    let previous = match &binding.session.key {
        Some(key) if key.incarnation == process => RuntimeLiveness::Alive,
        Some(key) => {
            observe_runtime_process(&SupervisedProbeRunner, key.incarnation.pid(), deadline)
                .map_err(|_| ())?
                .matches(&key.incarnation)
        }
        None => RuntimeLiveness::Unknown,
    };
    let owned_resume = lifecycle.permits_owned_resume(&stored.preferences, event.session(), host)
        && binding.session.launch_owner.as_ref().is_some_and(|owner| {
            observe_runtime_process(&SupervisedProbeRunner, owner.pid(), deadline)
                .is_ok_and(|value| value.matches(owner) == RuntimeLiveness::Alive)
        });
    let next = event
        .propose(&binding.session, &process, previous, host, owned_resume)
        .ok_or(())?;
    if Instant::now() >= deadline {
        return Err(());
    }
    let mode = lifecycle.mode(host).ok_or(())?;
    let mut storage = Storage::open_hook(&paths.database).map_err(|_| ())?;
    let changed = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let current = records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding);
            if current.as_ref() != Some(binding) {
                return Ok(false);
            }
            if owned_resume
                && records.session_preferences(&binding.identity_id)? != stored.preferences
            {
                return Ok(false);
            }
            if !records.set_session_state(&binding.id, &binding.session, &next)? {
                return Ok(false);
            }
            if event.starting() {
                let mut preferences = records.session_preferences(&binding.identity_id)?;
                preferences.preferred_harness = Some(harness.clone());
                preferences.remember(harness.clone(), mode.clone(), event.session().clone());
                // Only this starting event's reported fields update driver
                // state; `remember` kept the same driver's previous state.
                if let Some(remembered) = preferences.remembered.as_mut() {
                    remembered.state = event.driver_state(
                        remembered.state.as_ref(),
                        tmt_adapters::request_runtime::wall_time_ms(),
                    );
                }
                return records.set_session_preferences(&binding.identity_id, &preferences);
            }
            Ok(true)
        })
        .map_err(|_| ())?;
    storage.close().map_err(|_| ())?;
    if !changed || Instant::now() >= deadline {
        return Err(());
    }
    if !event.starting() {
        crate::pane_badge::refresh(
            &paths,
            &Host::for_server_with(&binding.server, SupervisedProbeRunner),
            binding,
            deadline,
        );
        return Ok(String::new());
    }
    // Re-read the bounded projection after acknowledgment. Do not inject a
    // different binding if a concurrent rebind occurred during the callback.
    let refreshed = Storage::context_by_pane(
        &paths.database,
        snapshot.server.host,
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
    // Extension contributions share the hook's remaining budget.
    let context = crate::context_command::render_verified(
        refreshed,
        &paths,
        deadline
            .checked_sub(Duration::from_millis(200))
            .unwrap_or(deadline),
    )
    .map_err(|_| ())?;
    let encoded = lifecycle.encode_context(&context).ok_or(())?;
    crate::pane_badge::refresh(
        &paths,
        &Host::for_server_with(&binding.server, SupervisedProbeRunner),
        binding,
        deadline,
    );
    Ok(encoded)
}

/// Prompt submission only contributes context to an already admitted session.
/// It cannot name, bind, resume or revive an identity.
fn observe_prompt(
    provider: &str,
    harness: &HarnessId,
    lifecycle: &dyn RuntimeLifecycle,
    session: &ProviderSessionId,
    deadline: Instant,
) -> Result<String, ()> {
    let paths = ConfigPaths::discover().map_err(|_| ())?;
    if !tmt_adapters::extension_hooks::has_context_consent(&paths.global_dir) {
        return Ok(String::new());
    }
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return Ok(String::new());
    }
    let Caller::Bound(bound) = verified_caller(provider, lifecycle, host, session, deadline)?
    else {
        return Ok(String::new());
    };
    let BoundCaller {
        paths,
        stored,
        snapshot,
        process,
        ..
    } = *bound;
    let binding = stored.entry.binding.as_ref().ok_or(())?;
    if binding.session.state != tmt_core::binding::session::RuntimeState::Running
        || !binding.session.key.as_ref().is_some_and(|key| {
            key.incarnation == process && key.provider_session.as_ref() == Some(session)
        })
        || !stored
            .preferences
            .remembered
            .as_ref()
            .is_some_and(|remembered| {
                &remembered.harness == harness && &remembered.provider_session == session
            })
        || !matches!(
            evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot.clone())),
            BindingEvidence::Active(_)
        )
    {
        return Ok(String::new());
    }
    let context = crate::context_command::render_extensions(
        &binding.identity_id,
        &paths,
        deadline
            .checked_sub(Duration::from_millis(200))
            .unwrap_or(deadline),
    );
    if context.is_empty() {
        return Ok(String::new());
    }
    // A callback can run public commands: do not hand its context to a binding
    // or conversation that changed while the callback was running.
    let refreshed = Storage::context_by_pane(
        &paths.database,
        snapshot.server.host,
        &binding.pane_id,
        &snapshot.server.server_id,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .map_err(|_| ())?
    .ok_or(())?;
    if Instant::now() >= deadline
        || refreshed.entry.binding.as_ref() != Some(binding)
        || refreshed.preferences != stored.preferences
        || !matches!(
            evaluate_binding(&refreshed.entry, &EndpointProbe::Live(snapshot)),
            BindingEvidence::Active(_)
        )
    {
        return Err(());
    }
    lifecycle.encode_prompt_context(&context).ok_or(())
}
