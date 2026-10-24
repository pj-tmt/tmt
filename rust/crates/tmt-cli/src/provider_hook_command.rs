//! Hook composition with a hard supervised deadline and no provider veto output.

use crate::workspace_hook::Observation;
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
    storage::{IdentityContextSnapshot, Storage, StorageError, StorageErrorCode},
};
use tmt_core::{
    binding::{
        BindingEvidence, BindingRepository, evaluate_binding,
        session::{HarnessId, ProviderSessionId, RuntimeLiveness},
    },
    endpoint::{EndpointProbe, EndpointSnapshot, ProcessIncarnation},
};

const BUDGET: Duration = Duration::from_millis(crate::invocation::MAXIMUM_HOOK_WORK_BUDGET_MS);
use crate::session_learning::Learning;
use tmt_core::binding::session::launch::{
    SessionLearningReason as Reason, SessionLearningStep as Step,
};

const ERROR_LINE: &str = "tmt: lifecycle context unavailable; continuing without context.";

/// Provider hooks always exit successfully. The existing bounded process owner
/// terminates/reaps the internal worker on timeout; no daemon, permission decision,
/// unbounded background thread or potentially late context write is introduced.
pub fn execute(
    provider: &str,
    worker: bool,
    work_budget_ms: Option<u64>,
    activity_only: bool,
) -> io::Result<u8> {
    let started = Instant::now();
    let learning = Learning::capture(provider);
    let mut diagnostic_deadline = started + BUDGET;
    let mut work_deadline = diagnostic_deadline;
    let mut failure_reason = Reason::WorkerError;
    // A turn end fires after every turn; its failures stay silent.
    let mut turn_end = false;
    let mut publication_deadline = started;
    let result = (|| {
        let registry = RuntimeRegistry::first_party();
        let harness = HarnessId::new(provider).map_err(|_| ())?;
        let lifecycle = registry.lifecycle(&harness).ok_or(())?;
        let channel_duration = (!worker).then(|| lifecycle.hook_work_duration()).flatten();
        let duration = if worker {
            crate::invocation::hook_worker_duration(work_budget_ms)
        } else {
            channel_duration.unwrap_or(BUDGET)
        };
        diagnostic_deadline = started + duration;
        publication_deadline = diagnostic_deadline;
        // Parent cleanup/diagnostics share the original budget; never renew it.
        let deadline = if worker {
            diagnostic_deadline
        } else {
            diagnostic_deadline
                .checked_sub(Duration::from_millis(200))
                .unwrap_or(started)
        };
        work_deadline = deadline;
        if let Some(learning) = &learning {
            let _ = learning.stage(Step::Input, deadline);
        }
        let input = read_stdin_bounded(
            deadline.saturating_duration_since(Instant::now()),
            tmt_adapters::runtime::hook_protocol::HOOK_INPUT_LIMIT,
        )
        .map_err(|_| ())?;
        turn_end = lifecycle.decode_turn(input.as_bytes()).is_some();
        if worker {
            return observe(provider, &input, deadline, activity_only, learning.as_ref());
        }
        if let Some(learning) = &learning {
            let _ = learning.stage(Step::DriverAdmission, deadline);
        }
        lifecycle
            .wait_for_hook_admission(input.as_bytes(), deadline)
            .map_err(|_| ())?;
        let mut args =
            worker_arguments(provider, deadline.saturating_duration_since(Instant::now()))?;
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
            .map_err(|error| {
                if error.kind == tmt_adapters::process::CommandFailure::Timeout {
                    failure_reason = Reason::BudgetTimeout;
                }
            })?;
        if !output.stderr.is_empty() {
            return Err(());
        }
        crate::workspace_hook::Observation::decode(&output.stdout)
    })();
    if !worker
        && result.is_ok()
        && let Some(learning) = &learning
    {
        let _ = learning.succeeded(diagnostic_deadline);
    }
    let failed = result.is_err();
    if failed && let Some(learning) = &learning {
        if Instant::now() >= work_deadline {
            failure_reason = Reason::BudgetTimeout;
        }
        if learning.fail(failure_reason, diagnostic_deadline).is_err() && !worker {
            let _ = writeln!(
                io::stderr().lock(),
                "tmt: session-learning diagnostic could not be saved."
            );
        }
    }
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
    activity: Option<&tmt_adapters::runtime::hook_protocol::ActivityObservation>,
    deadline: Instant,
) -> Result<Observation, ()> {
    // An interrupted turn ends activity only: no usage, consumption source or
    // final answer is read or captured for it.
    let activity_only = activity_only || turn.interrupted;
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return Ok(Observation::default());
    }
    let Caller::Bound(bound) = verified_lifecycle_caller(
        provider,
        lifecycle,
        host,
        &turn.session,
        None,
        deadline,
        SupervisedProbeRunner,
    )?
    else {
        return Ok(Observation::default());
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
        return Ok(Observation::default());
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
        return Ok(Observation::default());
    };
    let environment = ProviderEnvironment::capture().map_err(|_| ())?;
    let now_ms = tmt_adapters::request_runtime::wall_time_ms();
    let observation = (!activity_only).then(|| {
        lifecycle.turn_observation(
            turn,
            &environment,
            remembered.state.as_ref(),
            now_ms,
            deadline,
        )
    });
    let usage = observation.as_ref().and_then(|value| value.state.clone());
    let consumption = usage
        .as_ref()
        .and_then(|state| lifecycle.state_consumption(state));
    let locator = turn
        .transcript
        .as_ref()
        .filter(|_| !activity_only)
        .and_then(|path| {
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
        now_ms,
    };
    let previous = usage.as_ref().or(remembered.state.as_ref());
    let transition = activity
        .filter(|_| binding.session.state == tmt_core::binding::session::RuntimeState::Running)
        .and_then(|event| {
            lifecycle.activity_state(event, &turn.session, &process, previous, now_ms)
        });
    let response = if transition.is_some()
        && !activity_only
        && tmt_adapters::drivers::Registry::builtin()
            .find(provider)
            .is_some_and(tmt_adapters::setup::usage_hook_installed)
    {
        observation
            .and_then(|value| value.final_text)
            .map(|text| tmt_adapters::storage::ResponseCapture { text })
    } else {
        None
    };
    // An admitted turn end settles the passthrough that turn was bound to, with its final
    // message when one was read and without an answer otherwise (an interrupted turn too).
    let boundary = transition
        .is_some()
        .then(|| tmt_adapters::storage::TurnBoundary::Ended {
            turn: activity
                .and_then(|activity| activity.event.turn.as_ref())
                .map(ProviderSessionId::as_str),
            continuation: activity.is_some_and(|activity| {
                activity.event.kind
                    == tmt_core::binding::session::activity::EventKind::ContinuationEnded
            }),
        });
    let next = transition.or(usage);
    if Instant::now() >= deadline {
        return Err(());
    }
    if commit_observation(
        &paths,
        &stored,
        next,
        (!activity_only).then_some(reading),
        response,
        boundary,
        deadline,
    )? {
        return Ok(Observation {
            context: String::new(),
            workspace: Some(binding.server.clone()),
        });
    }
    Ok(Observation::default())
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

pub(super) fn verified_lifecycle_caller<R: CommandRunner + Clone>(
    provider: &str,
    lifecycle: &dyn RuntimeLifecycle,
    host: HostEvidence,
    session: &ProviderSessionId,
    verified_binding: Option<&str>,
    deadline: Instant,
    runner: R,
) -> Result<Caller, ()> {
    verified_caller(
        CallerRequest {
            provider,
            lifecycle,
            host,
            session,
            verified_binding,
            start: None,
            learning: None,
        },
        deadline,
        runner,
    )
}

struct CallerRequest<'a> {
    provider: &'a str,
    lifecycle: &'a dyn RuntimeLifecycle,
    host: HostEvidence,
    session: &'a ProviderSessionId,
    verified_binding: Option<&'a str>,
    start: Option<&'a dyn tmt_adapters::runtime::lifecycle::LifecycleObservation>,
    learning: Option<&'a Learning>,
}

fn verified_caller<R: CommandRunner + Clone>(
    request: CallerRequest<'_>,
    deadline: Instant,
    runner: R,
) -> Result<Caller, ()> {
    let CallerRequest {
        provider,
        lifecycle,
        host,
        session,
        verified_binding,
        start,
        learning,
    } = request;
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
            if let Some(event) = start
                && let Some(observed) =
                    observed_hook_pane(&paths, deadline, runner.clone(), learning)?
                && let Some(stored) = observed.stored.as_ref()
            {
                // An ambient pane variable cannot attribute a shared server:
                // require the provider to be beneath this verified pane.
                let process = lifecycle.observe_in_pane(
                    &runner,
                    u64::from(std::process::id()),
                    observed.pid,
                    deadline,
                );
                if process.as_ref().is_some_and(|process| {
                    host.runtime_pid()
                        .is_some_and(|pid| process.pid() == u64::from(pid))
                }) {
                    let _ = record_start_refusal(
                        &paths,
                        stored,
                        &observed.snapshot,
                        (
                            provider,
                            event.session(),
                            tmt_core::binding::session::StartRefusalReason::SharedRuntime,
                        ),
                        deadline,
                    );
                }
            }
            return Ok(Caller::Unobserved);
        };
        let binding = stored.entry.binding.as_ref().ok_or(())?;
        let pane = &binding.pane_id;
        let EndpointProbe::Live(snapshot) = Host::for_server_with(&binding.server, runner.clone())
            .with_drivers(tmt_adapters::host::external::approved())
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
        let Some(observed) = observed_hook_pane(&paths, deadline, runner.clone(), learning)? else {
            return Ok(Caller::Unobserved);
        };
        let HookPane {
            stored,
            snapshot,
            pid,
            marked,
        } = observed;
        if let Some(learning) = learning {
            let _ = learning.stage(Step::ProviderVerification, deadline);
        }
        let process = lifecycle
            .observe_in_pane(&runner, u64::from(std::process::id()), pid, deadline)
            .ok_or_else(|| {
                if let (Some(event), Some(stored)) = (start, stored.as_ref()) {
                    let _ = record_start_refusal(
                        &paths,
                        stored,
                        &snapshot,
                        (provider,
                        event.session(),
                        tmt_core::binding::session::StartRefusalReason::PrivateProcessUnverified),
                        deadline,
                    );
                }
            })?;
        if host
            .runtime_pid()
            .is_some_and(|pid| process.pid() != u64::from(pid))
        {
            if let (Some(event), Some(stored)) = (start, stored.as_ref()) {
                let _ = record_start_refusal(
                    &paths,
                    stored,
                    &snapshot,
                    (
                        provider,
                        event.session(),
                        tmt_core::binding::session::StartRefusalReason::PrivateProcessUnverified,
                    ),
                    deadline,
                );
            }
            return Err(());
        }
        let Some(stored) = stored else {
            return Ok(Caller::Unbound { marked });
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

struct HookPane {
    stored: Option<IdentityContextSnapshot>,
    snapshot: EndpointSnapshot,
    pid: u64,
    marked: bool,
}

/// Host-neutral caller ancestry and a fresh pane snapshot; neither is session authority.
fn observed_hook_pane<R: CommandRunner + Clone>(
    paths: &ConfigPaths,
    deadline: Instant,
    runner: R,
    learning: Option<&Learning>,
) -> Result<Option<HookPane>, ()> {
    if let Some(learning) = learning {
        let _ = learning.stage(Step::HostDiscovery, deadline);
    }
    let environment = CallerEnvironment::current();
    let panes = Host::for_verified_caller(
        &environment,
        runner.clone(),
        tmt_adapters::host::external::approved(),
        &mut tmt_adapters::storage::RecordedHostServers::new(&paths.database),
    )
    .map_err(|_| ())?;
    let Some(pane) = panes.caller_pane(&environment).map_err(|_| ())? else {
        return if Instant::now() < deadline {
            Ok(None)
        } else {
            Err(())
        };
    };
    if let Some(learning) = learning {
        let _ = learning.stage(Step::PaneSnapshot, deadline);
    }
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
    // A nested caller's host was chosen from one pinned ancestry; the
    // pane it reports now must be the pane shell that ancestry held.
    if panes
        .pinned_caller_pane()
        .is_some_and(|pid| pid != observed_pane.pane_pid)
    {
        return Err(());
    }
    if let Some(learning) = learning {
        let _ = learning.stage(Step::Storage, deadline);
    }
    let stored = Storage::context_by_pane(
        &paths.database,
        snapshot.server.host,
        &pane,
        &snapshot.server.server_id,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .map_err(|_| ())?;
    let pid = observed_pane.pane_pid;
    let marked = observed_pane.marker.is_some();
    Ok(Some(HookPane {
        stored,
        snapshot,
        pid,
        marked,
    }))
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
    let Caller::Bound(bound) = verified_lifecycle_caller(
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
    learning: Option<&Learning>,
) -> Result<Observation, ()> {
    if let Some(learning) = learning {
        let _ = learning.stage(Step::Decode, deadline);
    }
    let registry = RuntimeRegistry::first_party();
    let harness = HarnessId::new(provider).map_err(|_| ())?;
    let lifecycle = registry.lifecycle(&harness).ok_or(())?;
    let activity = lifecycle.decode_activity(input.as_bytes());
    // Notification observers receive non-permission notifications too. Declared
    // activity-only events that carry no admitted main event are quiet no-ops.
    let activity_only_event = serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| value["hook_event_name"].as_str().map(str::to_owned))
        .is_some_and(|event| lifecycle.activity_events().contains(&event.as_str()));
    if activity_only_event && activity.is_none() {
        return Ok(Observation::default());
    }
    if let Some(turn) = lifecycle.decode_turn(input.as_bytes()) {
        return observe_turn(
            provider,
            &harness,
            lifecycle,
            &turn,
            activity_only,
            activity.as_ref(),
            deadline,
        );
    }
    if let Some(session) = lifecycle.decode_prompt(input.as_bytes()) {
        return observe_main_event(
            provider,
            &harness,
            lifecycle,
            &session,
            activity.as_ref(),
            Some(input.as_bytes()),
            deadline,
        );
    }
    if let Some(session) = lifecycle.decode_activity_session(input.as_bytes()) {
        return observe_activity_event(
            &harness,
            lifecycle,
            &session,
            activity.as_ref().ok_or(())?,
            deadline,
            || ConfigPaths::discover().map_err(|_| ()),
        );
    }
    let event = lifecycle.decode(input.as_bytes()).ok_or(())?;
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return Ok(Observation::default());
    }
    let BoundCaller {
        host,
        paths,
        stored,
        snapshot,
        process,
    } = match verified_caller(
        CallerRequest {
            provider,
            lifecycle,
            host,
            session: event.session(),
            verified_binding: event.verified_binding(),
            start: event.starting().then_some(event.as_ref()),
            learning,
        },
        deadline,
        SupervisedProbeRunner,
    )? {
        Caller::Unobserved => return Ok(Observation::default()),
        // With no stored identity, only emit the fixed naming hint. The
        // command-derived suggested_name (for example "claude") is not a
        // binding marker and supplies no identity or context to this branch.
        Caller::Unbound { marked: true } => return Err(()),
        Caller::Unbound { marked: false } => {
            return if event.starting() {
                Ok(Observation {
                    context: lifecycle
                        .encode_context(&crate::context_command::unbound_text().map_err(|_| ())?)
                        .unwrap_or_default(),
                    workspace: None,
                })
            } else {
                Ok(Observation::default())
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
    if event.transition() == Some(tmt_core::binding::session::SessionTransition::Ended)
        && let Some(learning) = learning
        && learning.ended(&stored, &process, event.as_ref(), deadline)?
    {
        return Ok(Observation::default());
    }
    if event.starting()
        && let Some(learning) = learning
    {
        let _ = learning.stage(Step::Storage, deadline);
        if learning.defer(&stored, &process, host, event.as_ref(), lifecycle, deadline)? {
            return Ok(Observation::default());
        }
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
    let proposed = event.propose(&binding.session, &process, previous, host, owned_resume);
    // A verified channel maps its server back to the already-admitted
    // foreground. Its proposal proves the private record/owner fences; it is
    // not adoption of the server process as a new pane attachment.
    let keeps_foreground = event.verified_binding() == Some(binding.id.as_str())
        && binding.session.key.is_some()
        && proposed
            .as_ref()
            .is_some_and(|next| next.key == binding.session.key);
    if event.starting()
        && !keeps_foreground
        && let Some(reason) = refused_start(
            StartCandidate {
                stored: &stored,
                process: &process,
                previous,
                host,
            },
            lifecycle,
            &harness,
            event.as_ref(),
            deadline,
        )
    {
        record_start_refusal(
            &paths,
            &stored,
            &snapshot,
            (provider, event.session(), reason),
            deadline,
        )?;
        return Err(());
    }
    let Some(mut next) = proposed else {
        if event.starting() {
            record_start_refusal(
                &paths,
                &stored,
                &snapshot,
                (
                    provider,
                    event.session(),
                    tmt_core::binding::session::StartRefusalReason::AdmissionRefused,
                ),
                deadline,
            )?;
        }
        return Err(());
    };
    if event.starting() {
        next.start_refusal = None;
    }
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
            fence_preferences: event.starting() || owned_resume,
        },
    )
    .map_err(|_| ())?;
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
                response: None,
                boundary: None,
                family: None,
                now_ms: 0,
                deadline,
            })
            .map_err(|_| ())?;
    }
    storage.close().map_err(|_| ())?;
    if !changed || Instant::now() >= deadline {
        return Err(());
    }
    // Only a committed, verified lifecycle observation reaches this branch.
    // Recovery is advisory and still independently fences the native server.
    if !event.starting() {
        refresh_pane_badge(&paths, binding, deadline);
        return Ok(Observation {
            context: String::new(),
            workspace: Some(binding.server.clone()),
        });
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
    if let Some(learning) = learning {
        let _ = learning.stage(Step::Context, deadline);
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
    refresh_pane_badge(&paths, binding, deadline);
    Ok(Observation {
        context: encoded,
        workspace: Some(binding.server.clone()),
    })
}

/// Complete an existing binding only for its driver and an ended old session.
/// Managed launches already admitted their exact child and retain their own fence.
pub(super) struct StartCandidate<'a> {
    pub stored: &'a IdentityContextSnapshot,
    pub process: &'a ProcessIncarnation,
    pub previous: RuntimeLiveness,
    pub host: HostEvidence,
}

pub(super) fn refused_start(
    candidate: StartCandidate<'_>,
    lifecycle: &dyn RuntimeLifecycle,
    harness: &HarnessId,
    event: &dyn tmt_adapters::runtime::lifecycle::LifecycleObservation,
    deadline: Instant,
) -> Option<tmt_core::binding::session::StartRefusalReason> {
    use tmt_core::binding::session::{
        ObservedSessionKey, RuntimeState, SessionTransition, StartRefusalReason as Why,
    };
    let StartCandidate {
        stored,
        process,
        previous,
        host,
    } = candidate;
    let binding = stored.entry.binding.as_ref()?;
    let key = binding.session.key.as_ref();
    let managed = binding.session.launch_owner.is_some()
        && key.is_some_and(|key| {
            key.incarnation == *process
                && (key.provider_session.is_none()
                    || key.provider_session.as_ref() == Some(event.session()))
        })
        && stored.preferences.preferred_harness.as_ref() == Some(harness);
    if managed {
        return None;
    }
    let recorded = stored
        .preferences
        .remembered
        .as_ref()
        .map(|r| &r.harness)
        .or(stored.preferences.preferred_harness.as_ref());
    if recorded.is_some_and(|recorded| recorded != harness) {
        return Some(Why::DriverMismatch);
    }
    // Conversation turnover inside the same verified provider remains the
    // driver's transition policy. Only a new process needs hand-adoption.
    if key.is_some_and(|key| key.incarnation == *process) {
        return None;
    }
    if host.shared() {
        // Existing exact-session routing may move a known thread from a gone
        // embedded process to its shared server. The driver proposal and caller
        // attribution still enforce that session; this grants no new adoption.
        return (!(previous == RuntimeLiveness::Gone
            && key.is_some_and(|key| key.provider_session.as_ref() == Some(event.session()))))
        .then_some(Why::SharedRuntime);
    }
    if process.pid() == binding.pane_pid {
        return Some(Why::PaneProcess);
    }
    if key.is_some() && previous == RuntimeLiveness::Unknown {
        return Some(Why::PreviousSessionUnknown);
    }
    let ended = binding.session.state == RuntimeState::Ended
        || binding.session.last_transition == Some(SessionTransition::Ended);
    if key.is_some() && !ended && previous != RuntimeLiveness::Gone {
        return Some(if previous == RuntimeLiveness::Alive {
            Why::PreviousSessionLive
        } else {
            Why::PreviousSessionUnknown
        });
    }
    let mut candidate = binding.clone();
    candidate.session.key = Some(ObservedSessionKey {
        incarnation: process.clone(),
        provider_session: Some(event.session().clone()),
    });
    let mut preferences = stored.preferences.clone();
    let Some(mode) = lifecycle.mode(host) else {
        return Some(Why::PrivateProcessUnverified);
    };
    preferences.remember(harness.clone(), mode, event.session().clone());
    (lifecycle
        .movable_process(&SupervisedProbeRunner, &candidate, &preferences, deadline)
        .as_ref()
        != Some(process))
    .then_some(Why::PrivateProcessUnverified)
}

/// A refusal is diagnostic only. The captured binding and preferences fence it;
/// it changes neither admission, identity nor remembered provider coordinates.
pub(super) fn record_start_refusal(
    paths: &ConfigPaths,
    stored: &IdentityContextSnapshot,
    snapshot: &EndpointSnapshot,
    observation: (
        &str,
        &ProviderSessionId,
        tmt_core::binding::session::StartRefusalReason,
    ),
    deadline: Instant,
) -> Result<(), ()> {
    let (provider, session, reason) = observation;
    use tmt_core::binding::session::StartRefusal;
    let binding = stored.entry.binding.as_ref().ok_or(())?;
    if !matches!(
        evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot.clone())),
        BindingEvidence::Active(_)
    ) {
        return Err(());
    }
    let mut next = binding.session.clone();
    next.start_refusal = Some(StartRefusal {
        harness: HarnessId::new(provider).map_err(|_| ())?,
        session: session.clone(),
        reason,
        observed_at_ms: tmt_adapters::request_runtime::wall_time_ms(),
    });
    let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            if records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding)
                .as_ref()
                != Some(binding)
                || records.session_preferences(&binding.identity_id)? != stored.preferences
            {
                return Ok(false);
            }
            records.set_session_state(&binding.id, &binding.session, &next)
        })
        .map_err(|_| ())?;
    storage.close().map_err(|_| ())
}

/// The badge is a tmux state projection. An external pane has only its
/// marker's name, which bind, rename and `tmt run` keep current, and delivery
/// repairs when it admits a replacement runtime. A hook renames nothing, so it
/// starts no driver for it.
fn refresh_pane_badge(
    paths: &ConfigPaths,
    binding: &tmt_core::binding::Binding,
    deadline: Instant,
) {
    if let tmt_core::host::HostKind::External(_) = binding.server.host {
        return;
    }
    crate::pane_badge::refresh(
        paths,
        &Host::for_server_with(&binding.server, SupervisedProbeRunner),
        binding,
        deadline,
    );
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
) -> Result<bool, StorageError> {
    let binding = proposed.stored.entry.binding.as_ref().ok_or_else(|| {
        StorageError::new(
            StorageErrorCode::Unknown,
            "The stored context has no binding",
        )
    })?;
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
                if !records.set_session_preferences(&binding.identity_id, &preferences)? {
                    return Ok(false);
                }
                if let Some(owner) = proposed.next.launch_owner.as_ref()
                    && let Some(observed) = records.launch_observation(&binding.id, owner)?
                {
                    crate::session_learning::settle(
                        records,
                        &observed,
                        tmt_core::binding::session::launch::LaunchStatus::Admitted,
                        None,
                    )?;
                }
                Ok(true)
            } else {
                Ok(true)
            }
        })
        .map_err(|error| storage.hook_allowance(error))
}

/// Activity-only callbacks share main-event admission; path discovery is lazy.
fn observe_activity_event(
    harness: &HarnessId,
    lifecycle: &dyn RuntimeLifecycle,
    session: &ProviderSessionId,
    activity: &tmt_adapters::runtime::hook_protocol::ActivityObservation,
    deadline: Instant,
    paths: impl FnOnce() -> Result<ConfigPaths, ()>,
) -> Result<Observation, ()> {
    use tmt_core::binding::session::activity::{ActivityPhase, EventKind};
    if activity.event.kind == EventKind::ToolFinished {
        // This is only a read-only no-op shortcut, never caller admission. A
        // stale miss leaves activity for the next event; a stale hit still
        // needs all of the existing live verification and CAS below.
        let waiting = paths()
            .ok()
            .and_then(|paths| {
                Storage::context_by_provider_session(
                    &paths.database,
                    harness.as_str(),
                    session.as_str(),
                    tmt_adapters::request_runtime::wall_time_ms(),
                )
                .ok()
                .flatten()
            })
            .and_then(|stored| stored.preferences.remembered?.state)
            .and_then(|state| lifecycle.state_activity(&state))
            .is_some_and(|activity| {
                activity.session == *session && activity.phase == ActivityPhase::Waiting
            });
        if !waiting {
            return Ok(Observation::default());
        }
    }
    observe_main_event(
        harness.as_str(),
        harness,
        lifecycle,
        session,
        Some(activity),
        None,
        deadline,
    )
}

/// Main events observe an already admitted session; only prompts contribute context.
/// They cannot name, bind, resume or revive an identity.
fn observe_main_event(
    provider: &str,
    harness: &HarnessId,
    lifecycle: &dyn RuntimeLifecycle,
    session: &ProviderSessionId,
    activity: Option<&tmt_adapters::runtime::hook_protocol::ActivityObservation>,
    payload: Option<&[u8]>,
    deadline: Instant,
) -> Result<Observation, ()> {
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return Ok(Observation::default());
    }
    let Caller::Bound(bound) = verified_lifecycle_caller(
        provider,
        lifecycle,
        host,
        session,
        None,
        deadline,
        SupervisedProbeRunner,
    )?
    else {
        return Ok(Observation::default());
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
        return Ok(Observation::default());
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
        return Ok(Observation::default());
    }
    let context = if payload.is_some() {
        crate::context_command::render_prompt(
            &stored,
            &paths,
            deadline
                .checked_sub(Duration::from_millis(200))
                .unwrap_or(deadline),
        )
    } else {
        String::new()
    };
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
    let reading = if payload.is_some()
        && binding.session.launch_owner.is_some()
        && previous
            .and_then(|state| lifecycle.state_consumption(state))
            .is_none()
        && tmt_adapters::drivers::Registry::builtin()
            .find(provider)
            .is_some_and(tmt_adapters::setup::usage_hook_installed)
    {
        ProviderEnvironment::capture().ok().and_then(|environment| {
            let locator = lifecycle.consumption_locator(payload?, &environment);
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
    // An admitted turn start carries the prompt the provider is about to run; a passthrough
    // typed with that text is bound to this turn.
    let prompt = activity
        .filter(|activity| {
            activity.event.kind == tmt_core::binding::session::activity::EventKind::TurnStarted
        })
        .and(payload)
        .and_then(tmt_adapters::runtime::hook_protocol::decode_prompt_text);
    let boundary = prompt
        .as_deref()
        .map(|prompt| tmt_adapters::storage::TurnBoundary::Started {
            turn: activity
                .and_then(|activity| activity.event.turn.as_ref())
                .map(ProviderSessionId::as_str),
            prompt,
        });
    if (next.is_some() || reading.is_some())
        && !commit_observation(
            &paths,
            &stored,
            next,
            reading.map(|(_, reading)| reading),
            None,
            boundary,
            deadline,
        )?
    {
        return Err(());
    }
    let context = if context.is_empty() {
        context
    } else {
        lifecycle.encode_prompt_context(&context).ok_or(())?
    };
    Ok(Observation {
        context,
        workspace: Some(binding.server.clone()),
    })
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
    response: Option<tmt_adapters::storage::ResponseCapture>,
    boundary: Option<tmt_adapters::storage::TurnBoundary<'_>>,
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
        family: None,
        response,
        boundary,
        now_ms: reading.as_ref().map_or(0, |r| r.now_ms),
        deadline,
    });
    let cleanup = storage.close();
    pending
        .and_then(|changed| cleanup.map(|()| changed))
        .map_err(|_| ())
}

fn worker_arguments(provider: &str, remaining: Duration) -> Result<Vec<std::ffi::OsString>, ()> {
    let mut args = vec!["__hook".into(), provider.into(), "--worker".into()];
    // Every worker shares its supervisor's remaining budget, including plain
    // hooks. The parent remains the hard deadline owner through child startup.
    let budget = remaining.min(BUDGET).as_millis();
    if budget == 0 {
        return Err(());
    }
    args.extend(["--work-budget-ms".into(), budget.to_string().into()]);
    Ok(args)
}

#[cfg(test)]
mod interrupt_tests;

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn full_admission_gate_leaves_only_the_actual_worker_remainder() {
        let remaining_budget = tmt_adapters::drivers::codex::MAXIMUM_HOOK_WORK_DURATION
            - tmt_adapters::drivers::codex::MAXIMUM_HOOK_ADMISSION_DURATION;
        let args = worker_arguments("codex", remaining_budget).unwrap();
        let remaining = args[4].to_str().unwrap().parse::<u64>().unwrap();
        assert_eq!(remaining, 500);
        assert_eq!(
            crate::invocation::hook_worker_duration(Some(remaining)),
            Duration::from_millis(remaining)
        );
        assert!(worker_arguments("codex", Duration::ZERO).is_err());
    }

    #[test]
    fn plain_hooks_pass_the_parent_remainder_instead_of_restarting_two_seconds() {
        let args = worker_arguments("claude", Duration::from_millis(731)).unwrap();
        assert_eq!(
            &args[..4],
            &["__hook", "claude", "--worker", "--work-budget-ms"].map(std::ffi::OsString::from)
        );
        let remaining = args[4].to_str().unwrap().parse::<u64>().unwrap();
        assert_eq!(remaining, 731);
        assert!(worker_arguments("claude", Duration::ZERO).is_err());
    }

    #[test]
    fn absent_worker_budget_keeps_two_seconds() {
        assert_eq!(
            crate::invocation::hook_worker_duration(None),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn worker_budget_preserves_remaining_time_and_caps_direct_calls() {
        assert_eq!(
            crate::invocation::hook_worker_duration(Some(0)),
            Duration::ZERO
        );
        assert_eq!(
            crate::invocation::hook_worker_duration(Some(731)),
            Duration::from_millis(731)
        );
        assert_eq!(
            crate::invocation::hook_worker_duration(Some(u64::MAX)),
            BUDGET
        );
    }

    #[test]
    fn worker_arguments_cap_and_floor_the_remaining_budget() {
        let args = worker_arguments("claude", Duration::from_secs(3)).unwrap();
        assert_eq!(args[4], "2000");
        let args = worker_arguments("claude", Duration::from_micros(1_999)).unwrap();
        assert_eq!(args[4], "1");
        assert!(worker_arguments("claude", Duration::from_micros(999)).is_err());
    }
}
