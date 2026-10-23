//! Optional post-output discovery; environment coordinates never grant authority.
use crate::provider_hook_command::{
    Caller, SessionCommit, commit_session, verified_lifecycle_caller,
};
use std::{
    io::Write,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    driver_protocol::registry::DriverRecord,
    host::{CallerEnvironment, NamedHost},
    process::{
        CommandFailure, CommandRequest, CommandRunner, SupervisedProbeRunner, UnixCommandRunner,
        runtime::observe_runtime_process,
    },
    runtime::{
        RuntimeRegistry,
        caller_refusals::{Key, Refusals},
        lifecycle::{CallerSession, HostEvidence},
    },
    skill_installation::ProviderEnvironment,
    storage::{Storage, StorageError},
};
use tmt_core::{
    binding::{
        BindingEvidence, evaluate_binding,
        session::{HarnessId, RuntimeLiveness},
    },
    endpoint::EndpointProbe,
    host::HostKind,
};

const BUDGET: Duration = Duration::from_millis(500);
const REFUSAL_LAYERS: &[&str] = &[
    "environment",
    "native-admission",
    "binding",
    "main-provider",
    "replacement",
    "storage",
    "stale-commit",
    "budget",
    "worker",
    "provider-configuration",
    "provider-index-unavailable",
    "provider-index-shape",
    "provider-header-unavailable",
    "provider-header-shape",
    "provider-not-root",
];

struct Discovery<'a> {
    database: &'a std::path::Path,
    global_dir: &'a std::path::Path,
    executable: &'a std::path::Path,
    deadline: Instant,
    now_ms: u64,
}

pub fn observe() {
    if let Err(layer) = observe_with(&UnixCommandRunner) {
        debug_refusal(layer);
    }
}

fn observe_with(runner: &impl CommandRunner) -> Result<(), &'static str> {
    let registry = RuntimeRegistry::first_party();
    let candidates: Vec<_> = registry
        .harnesses()
        .filter_map(|name| {
            let harness = HarnessId::new(name).ok()?;
            let coordinates = registry.lifecycle(&harness)?.caller_session()?;
            Some((harness, coordinates))
        })
        .collect();
    let [(harness, coordinates)] = candidates.as_slice() else {
        return if candidates.is_empty()
            && !["CLAUDE_CODE_SESSION_ID", "CLAUDE_PID", "CODEX_THREAD_ID"]
                .iter()
                .any(|name| std::env::var_os(name).is_some())
        {
            Ok(())
        } else {
            Err("environment")
        };
    };
    let deadline = Instant::now() + BUDGET;
    let paths = ConfigPaths::discover().map_err(|_| "storage")?;
    let executable = std::env::current_exe().map_err(|_| "worker")?;
    observe_candidate(
        runner,
        harness,
        coordinates,
        Discovery {
            database: &paths.database,
            global_dir: &paths.global_dir,
            executable: &executable,
            deadline,
            now_ms: tmt_adapters::request_runtime::wall_time_ms(),
        },
        current_caller,
    )
}

/// Explicit binding asks for a verifiable session now, without the optional cache.
/// An invoker in another pane never supplies the target's provider coordinates.
pub(super) fn complete_binding(binding: &tmt_core::binding::Binding) -> Result<(), &'static str> {
    let registry = RuntimeRegistry::first_party();
    let candidates: Vec<_> = registry
        .harnesses()
        .filter(|name| {
            HarnessId::new(name)
                .ok()
                .and_then(|id| registry.lifecycle(&id)?.caller_session())
                .is_some()
        })
        .collect();
    let [provider] = candidates.as_slice() else {
        return Err("environment");
    };
    // An explicit bind is synchronous lifecycle admission, not the optional
    // post-output discovery attempt. Keep that optional 500 ms budget separate.
    let allowance = Duration::from_millis(crate::invocation::MAXIMUM_HOOK_WORK_BUDGET_MS);
    let result = verified_observation(provider, Instant::now() + allowance, Some(binding));
    if let Err(layer) = result {
        debug_refusal(layer);
    }
    result
}

/// The caller's environment and the approvals it was read with, once, and
/// only after the remembered-session shortcut. The approval owner may
/// resolve or adopt a first-party companion here; the precheck itself runs
/// no host driver operation.
fn current_caller() -> (CallerEnvironment, Vec<DriverRecord>) {
    let records = tmt_adapters::host::external::approved();
    (CallerEnvironment::approved(&records), records)
}

fn observe_candidate(
    runner: &impl CommandRunner,
    harness: &HarnessId,
    coordinates: &CallerSession,
    discovery: Discovery<'_>,
    caller_environment: impl FnOnce() -> (CallerEnvironment, Vec<DriverRecord>),
) -> Result<(), &'static str> {
    let Discovery {
        database,
        global_dir,
        executable,
        deadline,
        now_ms,
    } = discovery;
    if Storage::observes_provider_session(database, harness.as_str(), coordinates.session.as_str())
        .map_err(|_| "storage")?
    {
        // Selection is stored correlation only: agreeing observations authorize no
        // effect, so this path needs neither ps nor provider-file evidence.
        return Ok(());
    }
    let (caller, records) = caller_environment();
    let key = match caller.named_host(&records) {
        NamedHost::Nested(record) => {
            nested_caller_key(database, harness, coordinates, &caller, record)?
        }
        NamedHost::External(record) if caller.pane_locators().is_empty() => {
            external_caller_key(database, harness, coordinates, &caller, record)?
        }
        _ if caller.pane_locators().is_empty() => return Err("binding"),
        _ => {
            let Some(binding) =
                Storage::caller_session_binding(database, &caller).map_err(|_| "storage")?
            else {
                return Err("binding");
            };
            Key::new(harness, coordinates, &binding)
        }
    };
    let refusals = Refusals::in_directory(global_dir);
    if let Some(layer) = refusals.lookup(&key, now_ms)
        && let Some(layer) = REFUSAL_LAYERS.iter().copied().find(|known| *known == layer)
    {
        return Err(layer);
    }
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .as_millis();
    if remaining == 0 {
        return Err("budget");
    }
    let args = [
        "__hook".into(),
        harness.as_str().into(),
        "--worker".into(),
        "--caller-session".into(),
        "--work-budget-ms".into(),
        remaining.to_string().into(),
    ];
    let result = runner.execute(CommandRequest {
        program: executable.as_os_str(),
        args: &args,
        input: &[],
        deadline,
        max_output_bytes: 256,
    });
    let result = match result {
        Ok(_) => Ok(()),
        // The stored session is the outcome, not the worker's exit: it can end
        // badly after its commit (a fault closing the database, or the deadline
        // passing before its exit is reaped). A recorded session is admitted
        // and leaves no hint to suppress the next attempt. An unreadable store
        // proves nothing, so the refusal below stands.
        Err(_)
            if Storage::observes_provider_session(
                database,
                harness.as_str(),
                coordinates.session.as_str(),
            )
            .is_ok_and(|recorded| recorded) =>
        {
            return Ok(());
        }
        Err(error) => {
            // Worker output contains only a fixed diagnostic generated below;
            // never display a provider's stderr, argv, paths or session IDs.
            let layer = error
                .output
                .as_ref()
                .and_then(|output| {
                    REFUSAL_LAYERS.iter().copied().find(|layer| {
                        output.stderr
                            == format!("warning: caller session not recorded: {layer}\n").as_bytes()
                    })
                })
                .unwrap_or(if matches!(error.kind, CommandFailure::Timeout) {
                    "budget"
                } else {
                    "worker"
                });
            Err(layer)
        }
    };
    if let Err(layer) = result {
        refusals.remember(key, layer, now_ms);
    }
    result
}

/// An external host's caller variables hold a target, not a pane ID, so no
/// locator selects its binding here. With exactly one approved external host
/// named and no tmux, the worker resolves the pane through its driver; this
/// precheck runs no driver and only checks that the host has a binding.
fn external_caller_key(
    database: &std::path::Path,
    harness: &HarnessId,
    coordinates: &CallerSession,
    caller: &CallerEnvironment,
    record: &DriverRecord,
) -> Result<Key, &'static str> {
    let bindings = external_host_bindings(database, record)?;
    if bindings.is_empty() {
        return Err("binding");
    }
    Ok(Key::caller(harness, coordinates, record, caller, &bindings))
}

/// tmux variables beside exactly one external host's: either pane can own
/// the caller, and only the worker's nested proof tells which. So the worker
/// runs when the tmux pane or that host has a binding, and the key holds
/// both sides. The tmux binding is read through the tmux locator alone, so a
/// host variable shaped like a pane ID cannot stand in for it. This precheck
/// runs no driver and no tmux command.
fn nested_caller_key(
    database: &std::path::Path,
    harness: &HarnessId,
    coordinates: &CallerSession,
    caller: &CallerEnvironment,
    record: &DriverRecord,
) -> Result<Key, &'static str> {
    let tmux_caller = CallerEnvironment {
        tmux: caller.tmux.clone(),
        pane: caller.pane.clone(),
        process_id: caller.process_id,
        driver_env: Default::default(),
    };
    let tmux = Storage::caller_session_binding(database, &tmux_caller).map_err(|_| "storage")?;
    let bindings = external_host_bindings(database, record)?;
    if tmux.is_none() && bindings.is_empty() {
        return Err("binding");
    }
    Ok(Key::nested(
        harness,
        coordinates,
        record,
        caller,
        tmux.as_ref(),
        &bindings,
    ))
}

/// The stored bindings of non-retired identities on `record`'s host.
fn external_host_bindings(
    database: &std::path::Path,
    record: &DriverRecord,
) -> Result<Vec<String>, &'static str> {
    let Some(host @ HostKind::External(_)) = HostKind::parse(&record.name) else {
        return Err("binding");
    };
    Storage::caller_session_host_bindings(database, host).map_err(|_| "storage")
}

pub fn worker(provider: &str, budget_ms: Option<u64>) -> u8 {
    let deadline = Instant::now() + Duration::from_millis(budget_ms.unwrap_or(0)).min(BUDGET);
    let result = verified_observation(provider, deadline, None);
    if let Err(layer) = result {
        // Private supervisor evidence is always captured, never forwarded as
        // default diagnostics. Cache hits retain the original refusing layer.
        let mut stderr = tmt_cli_style::stream::stderr();
        let _ = writeln!(stderr, "warning: caller session not recorded: {layer}");
        let _ = SupervisedProbeRunner::abort_worker_group();
    }
    u8::from(result.is_err())
}

fn verified_observation(
    provider: &str,
    deadline: Instant,
    expected_binding: Option<&tmt_core::binding::Binding>,
) -> Result<(), &'static str> {
    let registry = RuntimeRegistry::first_party();
    let harness = HarnessId::new(provider).map_err(|_| "environment")?;
    let lifecycle = registry.lifecycle(&harness).ok_or("environment")?;
    let coordinates: CallerSession = lifecycle.caller_session().ok_or("environment")?;
    let host = HostEvidence::Independent {
        runtime_pid: coordinates.runtime_pid,
    };
    let Caller::Bound(bound) = verified_lifecycle_caller(
        provider,
        lifecycle,
        host,
        &coordinates.session,
        None,
        deadline,
        SupervisedProbeRunner,
    )
    .map_err(|_| "native-admission")?
    else {
        return Err("native-admission");
    };
    let binding = bound.stored.entry.binding.as_ref().ok_or("binding")?;
    if expected_binding.is_some_and(|expected| binding != expected) {
        return Err("binding");
    }
    if !matches!(
        evaluate_binding(
            &bound.stored.entry,
            &EndpointProbe::Live(bound.snapshot.clone())
        ),
        BindingEvidence::Active(_)
    ) {
        return Err("binding");
    }
    let process = lifecycle
        .observe_main_caller(
            &SupervisedProbeRunner,
            u64::from(std::process::id()),
            binding.pane_pid,
            deadline,
        )
        .ok_or("main-provider")?;
    if process != bound.process {
        return Err("main-provider");
    }
    let previous = match &binding.session.key {
        Some(key) if key.incarnation == process => RuntimeLiveness::Alive,
        Some(key) => {
            observe_runtime_process(&SupervisedProbeRunner, key.incarnation.pid(), deadline)
                .map_err(|_| "native-admission")?
                .matches(&key.incarnation)
        }
        None => RuntimeLiveness::Unknown,
    };
    let environment = ProviderEnvironment::capture().map_err(|_| "provider-configuration")?;
    let event = lifecycle
        .caller_session_observation(&coordinates, &environment, deadline)
        .map_err(|error| error.layer())?;
    if let Some(reason) = crate::provider_hook_command::refused_start(
        crate::provider_hook_command::StartCandidate {
            stored: &bound.stored,
            process: &process,
            previous,
            host,
        },
        lifecycle,
        &harness,
        event.as_ref(),
        deadline,
    ) {
        crate::provider_hook_command::record_start_refusal(
            &bound.paths,
            &bound.stored,
            &bound.snapshot,
            (provider, event.session(), reason),
            deadline,
        )
        .map_err(|_| "storage")?;
        return Err("replacement");
    }
    let next = event
        .propose(&binding.session, &process, previous, host, false)
        .ok_or("replacement")?;
    let mode = lifecycle.mode(host).ok_or("native-admission")?;
    if Instant::now() >= deadline {
        return Err("budget");
    }
    let mut storage = Storage::open_hook(&bound.paths.database, deadline).map_err(storage_layer)?;
    let changed = commit_session(
        &mut storage,
        SessionCommit {
            stored: &bound.stored,
            next: &next,
            event: event.as_ref(),
            harness: &harness,
            mode: &mode,
            fence_preferences: true,
        },
    );
    let closed = storage.close().map_err(storage_layer);
    if !changed.map_err(storage_layer)? {
        return Err("stale-commit");
    }
    closed
}

/// Only a hook connection's spent writer allowance is the budget; any other
/// busy, locked or close fault is a storage refusal, unless the commit landed
/// (see `observe_candidate`).
fn storage_layer(error: StorageError) -> &'static str {
    if error.is_hook_allowance_spent() {
        "budget"
    } else {
        "storage"
    }
}

fn debug_refusal(layer: &str) {
    if std::env::var_os("TMT_CALLER_SESSION_DEBUG").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    let _ = tmt_cli_style::message::warning(
        &mut stderr,
        terminal,
        &format!("caller session not recorded: {layer}"),
        None,
    );
}

#[cfg(test)]
mod tests;
