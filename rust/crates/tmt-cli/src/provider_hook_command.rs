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

const BUDGET: Duration = Duration::from_millis(crate::invocation::MAXIMUM_HOOK_WORK_BUDGET_MS);
const ERROR_LINE: &str = "tmt: lifecycle context unavailable; continuing without context.";

/// Provider hooks always exit successfully. The existing bounded process owner
/// terminates/reaps the internal worker on timeout; no daemon, permission hook,
/// unbounded background thread or potentially late context write is introduced.
pub fn execute(
    provider: &str,
    worker: bool,
    work_budget_ms: Option<u64>,
    activity_only: bool,
) -> io::Result<u8> {
    let started = Instant::now();
    // A turn end fires after every turn; its failures stay silent.
    let mut turn_end = false;
    let mut publication_deadline = started;
    let result = (|| {
        let registry = RuntimeRegistry::first_party();
        let harness = HarnessId::new(provider).map_err(|_| ())?;
        let lifecycle = registry.lifecycle(&harness).ok_or(())?;
        let channel_duration = (!worker).then(|| lifecycle.hook_work_duration()).flatten();
        let duration = if worker {
            worker_duration(work_budget_ms)
        } else {
            channel_duration.unwrap_or(BUDGET)
        };
        let deadline = started + duration;
        publication_deadline = deadline;
        let input = read_stdin_bounded(
            if channel_duration.is_some() || work_budget_ms.is_some() {
                deadline.saturating_duration_since(Instant::now())
            } else {
                BUDGET
            },
            tmt_adapters::runtime::hook_protocol::HOOK_INPUT_LIMIT,
        )
        .map_err(|_| ())?;
        turn_end = lifecycle.decode_turn(input.as_bytes()).is_some();
        if worker {
            let mut workspace = None;
            let context = observe(provider, &input, deadline, activity_only, &mut workspace)?;
            return Ok(crate::workspace_hook::Observation { context, workspace });
        }
        lifecycle
            .wait_for_hook_admission(input.as_bytes(), deadline)
            .map_err(|_| ())?;
        let mut args = worker_arguments(provider, deadline)?;
        if activity_only {
            args.push("--activity-only".into());
        }
        let executable = std::env::current_exe().map_err(|_| ())?;
        let output = UnixCommandRunner
            .execute(CommandRequest {
                program: executable.as_os_str(),
                args: &args,
                input: input.as_bytes(),
                deadline,
                max_output_bytes: 64 * 1024,
            })
            .map_err(|_| ())?;
        if !output.stderr.is_empty() {
            return Err(());
        }
        crate::workspace_hook::Observation::decode(&output.stdout)
    })();
    let failed = result.is_err();
    match result {
        Ok(observation) => {
            if worker {
                let _ = io::stdout().lock().write_all(&observation.encode());
            } else {
                crate::workspace_hook::publish(
                    &mut io::stdout().lock(),
                    observation,
                    publication_deadline,
                );
            }
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
    activity_only: bool,
    activity: Option<&tmt_core::binding::session::activity::Event>,
    deadline: Instant,
) -> Result<(), ()> {
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return Ok(());
    }
    let Caller::Bound(bound) = verified_caller(
        provider,
        lifecycle,
        host,
        &turn.session,
        None,
        deadline,
        SupervisedProbeRunner,
    )?
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
    let Some(process) =
        lifecycle.activity_process(&binding.session, &process, &turn.session, host, deadline)
    else {
        return Ok(());
    };
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
    let usage = (!activity_only)
        .then(|| {
            lifecycle.turn_state(
                turn,
                &environment,
                remembered.state.as_ref(),
                tmt_adapters::request_runtime::wall_time_ms(),
                deadline,
            )
        })
        .flatten();
    let consumption = usage
        .as_ref()
        .and_then(|state| lifecycle.state_consumption(state));
    let locator = turn.transcript.as_ref().and_then(|path| {
        lifecycle.consumption_locator(
            serde_json::json!({"transcript_path":path})
                .to_string()
                .as_bytes(),
            &environment,
        )
    });
    let reading = ConsumptionReading {
        locator,
        consumption,
        now_ms: tmt_adapters::request_runtime::wall_time_ms(),
    };
    let previous = usage.as_ref().or(remembered.state.as_ref());
    let next = activity
        .filter(|_| binding.session.state == tmt_core::binding::session::RuntimeState::Running)
        .and_then(|event| {
            lifecycle.activity_state(
                event,
                &turn.session,
                &process,
                previous,
                tmt_adapters::request_runtime::wall_time_ms(),
            )
        })
        .or(usage);
    if Instant::now() >= deadline {
        return Err(());
    }
    commit_observation(
        &paths,
        &stored,
        next,
        (!activity_only).then_some(reading),
        deadline,
    )
    .map(|_| ())
}

/// The caller a hook event came from, verified the same way for every event.
pub(super) enum Caller {
    /// No pane (or, in shared mode, no stored session) to attribute it to.
    Unobserved,
    /// A verified pane with no stored identity; `marked` when it still holds
    /// a TMT marker, which is inconsistent evidence.
    Unbound {
        marked: bool,
    },
    Bound(Box<BoundCaller>),
}

pub(super) struct BoundCaller {
    pub(super) host: HostEvidence,
    pub(super) paths: ConfigPaths,
    pub(super) stored: IdentityContextSnapshot,
    pub(super) snapshot: EndpointSnapshot,
    pub(super) process: ProcessIncarnation,
}

pub(super) fn verified_caller<R: CommandRunner + Clone>(
    provider: &str,
    lifecycle: &dyn RuntimeLifecycle,
    host: HostEvidence,
    session: &ProviderSessionId,
    verified_binding: Option<&str>,
    deadline: Instant,
    runner: R,
) -> Result<Caller, ()> {
    let paths = ConfigPaths::discover().map_err(|_| ())?;
    let now = tmt_adapters::request_runtime::wall_time_ms();
    let shared = host.shared();
    let (stored, snapshot, process) = if shared {
        let Some(stored) = match verified_binding {
            Some(binding) => Storage::context_by_binding(&paths.database, binding, now),
            None => Storage::context_by_provider_session(
                &paths.database,
                provider,
                session.as_str(),
                now,
            ),
        }
        .map_err(|_| ())?
        else {
            return Ok(Caller::Unobserved);
        };
        let binding = stored.entry.binding.as_ref().ok_or(())?;
        let pane = &binding.pane_id;
        let EndpointProbe::Live(snapshot) = Host::for_server_with(&binding.server, runner.clone())
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
            observe_runtime_process(&runner, pid, deadline).map_err(|_| ())?
        else {
            return Err(());
        };
        (stored, snapshot, process)
    } else {
        let environment = CallerEnvironment::current();
        let panes = Host::for_caller_with(&environment, runner.clone());
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
                &runner,
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

/// Discover or verify this exact foreground launch in one native admission pass.
/// Supplied argv and discovered coordinates are locators, never authority.
pub(crate) fn verified_digest_launch<R: CommandRunner + Clone>(
    provider: &str,
    lifecycle: &dyn RuntimeLifecycle,
    launch: Option<&tmt_adapters::runtime::hook_protocol::HookLaunch>,
    session: &ProviderSessionId,
    deadline: Instant,
    runner: R,
) -> Result<
    (
        tmt_adapters::runtime::hook_protocol::HookLaunch,
        ConfigPaths,
        IdentityContextSnapshot,
    ),
    (),
> {
    if launch.is_some_and(|v| !v.valid()) {
        return Err(());
    }
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    let Caller::Bound(bound) = verified_caller(
        provider,
        lifecycle,
        host,
        session,
        None,
        deadline,
        runner.clone(),
    )?
    else {
        return Err(());
    };
    let binding = bound.stored.entry.binding.as_ref().ok_or(())?;
    let launch = match launch {
        Some(launch) => launch.clone(),
        None => {
            let owner = binding.session.launch_owner.as_ref().ok_or(())?;
            tmt_adapters::runtime::hook_protocol::HookLaunch {
                identity_id: binding.identity_id.clone(),
                binding_id: binding.id.clone(),
                owner_pid: owner.pid(),
                owner_start: owner.start_identity().to_owned(),
            }
        }
    };
    if !launch.valid() {
        return Err(());
    }
    let owner = launch.owner().ok_or(())?;
    let process = lifecycle
        .digest_process(&binding.session, &bound.process, session, host, deadline)
        .ok_or(())?;
    let expected = binding.id == launch.binding_id
        && binding.identity_id == launch.identity_id
        && binding.session.state == tmt_core::binding::session::RuntimeState::Running
        && binding.session.launch_owner.as_ref() == Some(&owner)
        && binding.session.key.as_ref().is_some_and(|key| {
            key.incarnation == process && key.provider_session.as_ref() == Some(session)
        })
        && bound
            .stored
            .preferences
            .remembered
            .as_ref()
            .is_some_and(|remembered| {
                remembered.harness.as_str() == provider && &remembered.provider_session == session
            })
        && matches!(
            evaluate_binding(&bound.stored.entry, &EndpointProbe::Live(bound.snapshot)),
            BindingEvidence::Active(_)
        );
    if !expected {
        return Err(());
    }
    let alive = matches!(observe_runtime_process(&runner, owner.pid(), deadline).map_err(|_| ())?,
        tmt_adapters::process::runtime::ProcessObservation::Live(current) if current == owner);
    if !alive || Instant::now() >= deadline {
        return Err(());
    }
    Ok((launch, bound.paths, bound.stored))
}

fn observe(
    provider: &str,
    input: &str,
    deadline: Instant,
    activity_only: bool,
    workspace: &mut Option<tmt_core::endpoint::ServerEvidence>,
) -> Result<String, ()> {
    let registry = RuntimeRegistry::first_party();
    let harness = HarnessId::new(provider).map_err(|_| ())?;
    let lifecycle = registry.lifecycle(&harness).ok_or(())?;
    let activity = lifecycle.decode_activity(input.as_bytes());
    if let Some(turn) = lifecycle.decode_turn(input.as_bytes()) {
        return observe_turn(
            provider,
            &harness,
            lifecycle,
            &turn,
            activity_only,
            activity.as_ref(),
            deadline,
        )
        .map(|()| String::new());
    }
    if let Some(session) = lifecycle.decode_prompt(input.as_bytes()) {
        return observe_prompt(
            provider,
            &harness,
            lifecycle,
            &session,
            activity.as_ref(),
            input.as_bytes(),
            deadline,
        );
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
    } = match verified_caller(
        provider,
        lifecycle,
        host,
        event.session(),
        event.verified_binding(),
        deadline,
        SupervisedProbeRunner,
    )? {
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
    let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
    let changed = commit_session(
        &mut storage,
        SessionCommit {
            stored: &stored,
            next: &next,
            event: event.as_ref(),
            harness: &harness,
            mode: &mode,
            fence_preferences: owned_resume,
        },
    )?;
    if changed
        && event.starting()
        && let Some(owner) = next.launch_owner.as_ref()
        && let Some(session) = storage
            .session_preferences(&binding.identity_id)
            .map_err(|_| ())?
            .remembered
            .as_ref()
    {
        tmt_adapters::runtime::launch_preset::associate(
            &mut storage,
            &binding.identity_id,
            &binding.id,
            owner,
            session,
            &registry,
        )
        .map_err(|_| ())?;
    }
    if changed
        && event.starting()
        && let Ok(environment) = ProviderEnvironment::capture()
        && tmt_adapters::drivers::Registry::builtin()
            .find(provider)
            .is_some_and(tmt_adapters::setup::usage_hook_installed)
        && let Some(refreshed) = Storage::context_by_identity(
            &paths.database,
            &binding.identity_id,
            tmt_adapters::request_runtime::wall_time_ms(),
        )
        .map_err(|_| ())?
        && refreshed
            .entry
            .binding
            .as_ref()
            .is_some_and(|b| b.id == binding.id && b.session == next)
        && refreshed
            .preferences
            .remembered
            .as_ref()
            .is_some_and(|r| r.harness == harness && &r.provider_session == event.session())
    {
        let locator = lifecycle.consumption_locator(input.as_bytes(), &environment);
        storage
            .commit_runtime_observation(tmt_adapters::storage::RuntimeObservation {
                expected: &refreshed,
                preferences: &refreshed.preferences,
                remember_source: true,
                locator: locator.as_deref(),
                sampled: false,
                consumption: None,
                now_ms: 0,
                deadline,
            })
            .map_err(|_| ())?;
    }
    storage.close().map_err(|_| ())?;
    if !changed || Instant::now() >= deadline {
        return Err(());
    }
    // decode_turn and decode_prompt returned above; only documented session
    // lifecycle boundaries reach this admitted branch (including clear/compact).
    let capture_workspace = event.starting()
        || event.transition() == Some(tmt_core::binding::session::SessionTransition::Ended);
    if !event.starting() {
        if capture_workspace {
            *workspace = Some(binding.server.clone());
        }
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
        event.transition() == Some(tmt_core::binding::session::SessionTransition::Compacted),
    )
    .map_err(|_| ())?;
    let encoded = lifecycle.encode_context(&context).ok_or(())?;
    crate::pane_badge::refresh(
        &paths,
        &Host::for_server_with(&binding.server, SupervisedProbeRunner),
        binding,
        deadline,
    );
    if capture_workspace {
        *workspace = Some(binding.server.clone());
    }
    Ok(encoded)
}

pub(super) struct SessionCommit<'a> {
    pub stored: &'a IdentityContextSnapshot,
    pub next: &'a tmt_core::binding::session::BindingSessionState,
    pub event: &'a dyn tmt_adapters::runtime::lifecycle::LifecycleObservation,
    pub harness: &'a HarnessId,
    pub mode: &'a tmt_core::binding::session::RuntimeMode,
    pub fence_preferences: bool,
}

/// Both starting hooks and optional caller discovery share this exact commit.
pub(super) fn commit_session(
    storage: &mut Storage,
    proposed: SessionCommit<'_>,
) -> Result<bool, ()> {
    let binding = proposed.stored.entry.binding.as_ref().ok_or(())?;
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let current = records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding);
            if current.as_ref() != Some(binding)
                || (proposed.fence_preferences
                    && records.session_preferences(&binding.identity_id)?
                        != proposed.stored.preferences)
            {
                return Ok(false);
            }
            if !records.set_session_state(&binding.id, &binding.session, proposed.next)? {
                return Ok(false);
            }
            if proposed.event.starting() {
                let mut preferences = records.session_preferences(&binding.identity_id)?;
                preferences.launched(proposed.harness);
                preferences.remember(
                    proposed.harness.clone(),
                    proposed.mode.clone(),
                    proposed.event.session().clone(),
                );
                if let Some(remembered) = preferences.remembered.as_mut() {
                    remembered.state = proposed.event.driver_state(
                        remembered.state.as_ref(),
                        tmt_adapters::request_runtime::wall_time_ms(),
                    );
                }
                records.set_session_preferences(&binding.identity_id, &preferences)
            } else {
                Ok(true)
            }
        })
        .map_err(|_| ())
}

/// Prompt submission only contributes context to an already admitted session.
/// It cannot name, bind, resume or revive an identity.
fn observe_prompt(
    provider: &str,
    harness: &HarnessId,
    lifecycle: &dyn RuntimeLifecycle,
    session: &ProviderSessionId,
    activity: Option<&tmt_core::binding::session::activity::Event>,
    payload: &[u8],
    deadline: Instant,
) -> Result<String, ()> {
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return Ok(String::new());
    }
    let Caller::Bound(bound) = verified_caller(
        provider,
        lifecycle,
        host,
        session,
        None,
        deadline,
        SupervisedProbeRunner,
    )?
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
    let Some(process) =
        lifecycle.activity_process(&binding.session, &process, session, host, deadline)
    else {
        return Ok(String::new());
    };
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
    let context = crate::context_command::render_prompt(
        &stored,
        &paths,
        deadline
            .checked_sub(Duration::from_millis(200))
            .unwrap_or(deadline),
    );
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
    let previous = stored
        .preferences
        .remembered
        .as_ref()
        .and_then(|r| r.state.as_ref());
    let reading = if binding.session.launch_owner.is_some()
        && previous
            .and_then(|state| lifecycle.state_consumption(state))
            .is_none()
        && tmt_adapters::drivers::Registry::builtin()
            .find(provider)
            .is_some_and(tmt_adapters::setup::usage_hook_installed)
    {
        ProviderEnvironment::capture().ok().and_then(|environment| {
            let locator = lifecycle.consumption_locator(payload, &environment);
            let turn =
                lifecycle.sampling_turn(session, locator.as_deref(), &environment, deadline)?;
            let now = tmt_adapters::request_runtime::wall_time_ms();
            let state = lifecycle.turn_state(&turn, &environment, previous, now, deadline);
            let consumption = state
                .as_ref()
                .and_then(|state| lifecycle.state_consumption(state));
            Some((
                state,
                ConsumptionReading {
                    locator,
                    consumption,
                    now_ms: now,
                },
            ))
        })
    } else {
        None
    };
    let observed = reading
        .as_ref()
        .and_then(|(state, _)| state.as_ref())
        .or(previous);
    let next = activity
        .and_then(|event| {
            lifecycle.activity_state(
                event,
                session,
                &process,
                observed,
                tmt_adapters::request_runtime::wall_time_ms(),
            )
        })
        .or_else(|| reading.as_ref().and_then(|(state, _)| state.clone()));
    if (next.is_some() || reading.is_some())
        && !commit_observation(
            &paths,
            &stored,
            next,
            reading.map(|(_, reading)| reading),
            deadline,
        )?
    {
        return Err(());
    }
    if context.is_empty() {
        return Ok(String::new());
    }
    lifecycle.encode_prompt_context(&context).ok_or(())
}

/// Shared publication boundary: a current binding/preferences snapshot and enough
/// time for the bounded hook-storage transaction. No work is queued after return.
struct ConsumptionReading {
    locator: Option<String>,
    consumption: Option<tmt_adapters::runtime::consumption::Consumption>,
    now_ms: u64,
}

fn commit_observation(
    paths: &ConfigPaths,
    stored: &IdentityContextSnapshot,
    next: Option<tmt_core::binding::session::DriverState>,
    reading: Option<ConsumptionReading>,
    deadline: Instant,
) -> Result<bool, ()> {
    if Instant::now() + Duration::from_millis(100) >= deadline {
        return Err(());
    }
    let mut preferences = stored.preferences.clone();
    if let Some(next) = next
        && let Some(remembered) = preferences.remembered.as_mut()
    {
        remembered.state = Some(next);
    }
    let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
    let pending = storage.commit_runtime_observation(tmt_adapters::storage::RuntimeObservation {
        expected: stored,
        preferences: &preferences,
        remember_source: reading.is_some(),
        locator: reading.as_ref().and_then(|r| r.locator.as_deref()),
        sampled: reading.is_some(),
        consumption: reading.as_ref().and_then(|r| r.consumption.clone()),
        now_ms: reading.as_ref().map_or(0, |r| r.now_ms),
        deadline,
    });
    let cleanup = storage.close();
    pending
        .and_then(|changed| cleanup.map(|()| changed))
        .map_err(|_| ())
}

fn worker_arguments(provider: &str, deadline: Instant) -> Result<Vec<std::ffi::OsString>, ()> {
    let mut args = vec!["__hook".into(), provider.into(), "--worker".into()];
    // Every worker shares its supervisor's remaining budget, including plain
    // hooks. The parent remains the hard deadline owner through child startup.
    let budget = deadline
        .saturating_duration_since(Instant::now())
        .min(BUDGET)
        .as_millis();
    if budget == 0 {
        return Err(());
    }
    args.extend(["--work-budget-ms".into(), budget.to_string().into()]);
    Ok(args)
}

fn worker_duration(work_budget_ms: Option<u64>) -> Duration {
    work_budget_ms
        .map(Duration::from_millis)
        .unwrap_or(BUDGET)
        .min(BUDGET)
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn full_admission_gate_leaves_only_the_actual_worker_remainder() {
        let started =
            Instant::now() - tmt_adapters::drivers::codex::MAXIMUM_HOOK_ADMISSION_DURATION;
        let deadline = started + tmt_adapters::drivers::codex::MAXIMUM_HOOK_WORK_DURATION;
        let args = worker_arguments("codex", deadline).unwrap();
        let remaining = args[4].to_str().unwrap().parse::<u64>().unwrap();
        assert!(remaining > 0 && remaining <= 500);
        assert_eq!(
            worker_duration(Some(remaining)),
            Duration::from_millis(remaining)
        );
        assert!(worker_arguments("codex", Instant::now()).is_err());
    }

    #[test]
    fn plain_hooks_pass_the_parent_remainder_instead_of_restarting_two_seconds() {
        let args = worker_arguments("claude", Instant::now() + Duration::from_millis(731)).unwrap();
        assert_eq!(
            &args[..4],
            &["__hook", "claude", "--worker", "--work-budget-ms"].map(std::ffi::OsString::from)
        );
        let remaining = args[4].to_str().unwrap().parse::<u64>().unwrap();
        assert!(remaining > 0 && remaining <= 731);
        assert!(worker_arguments("claude", Instant::now()).is_err());
    }

    #[test]
    fn absent_worker_budget_keeps_two_seconds() {
        assert_eq!(worker_duration(None), Duration::from_secs(2));
    }

    #[test]
    fn worker_budget_preserves_remaining_time_and_caps_direct_calls() {
        assert_eq!(worker_duration(Some(0)), Duration::ZERO);
        assert_eq!(worker_duration(Some(731)), Duration::from_millis(731));
        assert_eq!(worker_duration(Some(u64::MAX)), BUDGET);
    }
}
