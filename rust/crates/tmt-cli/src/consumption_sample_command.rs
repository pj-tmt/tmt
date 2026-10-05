//! Foreground-owned supervised sampling; no new provider hook or daemon.
use std::{
    ffi::OsString,
    io,
    time::{Duration, Instant},
};
use tmt_adapters::runtime::sampling::SamplingRequest;
use tmt_adapters::{
    config::ConfigPaths,
    drivers::Registry,
    host::{Host, OperationOptions},
    process::{
        CommandRequest, CommandRunner, SupervisedProbeRunner, UnixCommandRunner,
        runtime::observe_runtime_process,
    },
    response_input::read_stdin_bounded,
    runtime::RuntimeRegistry,
    setup::usage_hook_installed,
    skill_installation::ProviderEnvironment,
    storage::{RuntimeObservation, Storage},
};
use tmt_core::{
    binding::{
        BindingEvidence, evaluate_binding,
        session::{RuntimeLiveness, RuntimeState},
    },
    endpoint::{EndpointProbe, ProcessIncarnation},
};

const BUDGET: Duration = Duration::from_secs(2);
/// The wait callback delegates all potentially slow source/storage work to the
/// existing hard-deadline process owner; it never changes the command's result.
pub fn tick(identity: &str, binding: &str, owner: &ProcessIncarnation) {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let input=serde_json::json!({"identityId":identity,"bindingId":binding,"ownerPid":owner.pid(),"ownerStart":owner.start_identity()}).to_string();
    let _ = UnixCommandRunner.execute(CommandRequest {
        program: executable.as_os_str(),
        args: &[OsString::from("__consumption-sample")],
        input: input.as_bytes(),
        deadline: Instant::now() + BUDGET,
        max_output_bytes: 4096,
    });
}

pub fn execute() -> io::Result<u8> {
    let result = sample(Instant::now() + BUDGET - Duration::from_millis(200));
    if result.is_err() {
        let _ = SupervisedProbeRunner::abort_worker_group();
    }
    Ok(u8::from(result.is_err()))
}

fn sample(deadline: Instant) -> Result<(), ()> {
    let input = read_stdin_bounded(deadline.saturating_duration_since(Instant::now()), 4096)
        .map_err(|_| ())?;
    let request: SamplingRequest = serde_json::from_str(&input).map_err(|_| ())?;
    if !tmt_core::dispatch::canonical_id(&request.identity_id)
        || !tmt_core::dispatch::canonical_id(&request.binding_id)
    {
        return Err(());
    }
    let owner = ProcessIncarnation::new(request.owner_pid, &request.owner_start).map_err(|_| ())?;
    let paths = ConfigPaths::discover().map_err(|_| ())?;
    let now = tmt_adapters::request_runtime::wall_time_ms();
    let Some(stored) =
        Storage::context_by_identity(&paths.database, &request.identity_id, now).map_err(|_| ())?
    else {
        return Ok(());
    };
    let Some(binding) = stored.entry.binding.as_ref() else {
        return Ok(());
    };
    if binding.id != request.binding_id
        || binding.session.launch_owner.as_ref() != Some(&owner)
        || binding.session.state != RuntimeState::Running
    {
        return Ok(());
    }
    let Some(key) = binding.session.key.as_ref() else {
        return Ok(());
    };
    let Some(remembered) =
        stored.preferences.remembered.as_ref().filter(|remembered| {
            key.provider_session.as_ref() == Some(&remembered.provider_session)
        })
    else {
        return Ok(());
    };
    let registry = RuntimeRegistry::first_party();
    let Some(lifecycle) = registry.lifecycle(&remembered.harness) else {
        return Ok(());
    };
    if !Registry::builtin()
        .find(remembered.harness.as_str())
        .is_some_and(usage_hook_installed)
    {
        return Ok(());
    }
    for process in [&owner, &key.incarnation] {
        if observe_runtime_process(&SupervisedProbeRunner, process.pid(), deadline)
            .map_err(|_| ())?
            .matches(process)
            != RuntimeLiveness::Alive
        {
            return Ok(());
        }
    }
    let host = Host::for_server_with(&binding.server, SupervisedProbeRunner);
    let snapshot = host
        .observe_snapshot(OperationOptions {
            deadline: Some(deadline),
            pane_ids: Some(std::slice::from_ref(&binding.pane_id)),
        })
        .map_err(|_| ())?;
    if !matches!(
        evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot)),
        BindingEvidence::Active(_)
    ) {
        return Ok(());
    }
    let environment = ProviderEnvironment::capture().map_err(|_| ())?;
    let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
    let locator = storage
        .consumption_locator(
            &binding.identity_id,
            &binding.id,
            remembered.harness.as_str(),
            remembered.provider_session.as_str(),
        )
        .map_err(|_| ())?;
    // Source reads occur with SQLite closed and are then committed against the
    // entire snapshot. A concurrent hook makes this attempt a harmless loser.
    storage.close().map_err(|_| ())?;
    let turn = lifecycle.sampling_turn(
        &remembered.provider_session,
        locator.as_deref(),
        &environment,
        deadline,
    );
    let source = turn
        .as_ref()
        .and_then(|turn| turn.transcript.as_ref())
        .and_then(|path| {
            let payload = serde_json::json!({"transcript_path":path}).to_string();
            lifecycle.consumption_locator(payload.as_bytes(), &environment)
        });
    let next = turn.as_ref().and_then(|turn| {
        lifecycle.turn_state(turn, &environment, remembered.state.as_ref(), now, deadline)
    });
    let consumption = next
        .as_ref()
        .and_then(|state| lifecycle.state_consumption(state));
    let mut preferences = stored.preferences.clone();
    if let Some(next) = next {
        preferences.remembered.as_mut().ok_or(())?.state = Some(next);
    }
    // Source work may outlive the foreground. Revalidate the actual runtime
    // before opening the write transaction; the full stored snapshot fences
    // rebinding/session changes separately.
    for process in [&owner, &key.incarnation] {
        if observe_runtime_process(&SupervisedProbeRunner, process.pid(), deadline)
            .map_err(|_| ())?
            .matches(process)
            != RuntimeLiveness::Alive
        {
            return Ok(());
        }
    }
    if Instant::now() + Duration::from_millis(100) >= deadline {
        return Err(());
    }
    let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
    let pending = storage
        .commit_runtime_observation(RuntimeObservation {
            expected: &stored,
            preferences: &preferences,
            remember_source: true,
            locator: source.as_deref(),
            sampled: true,
            consumption,
            now_ms: tmt_adapters::request_runtime::wall_time_ms(),
            deadline,
        })
        .map_err(|_| ());
    let closed = storage.close().map_err(|_| ());
    pending.and(closed).map(|_| ())
}
