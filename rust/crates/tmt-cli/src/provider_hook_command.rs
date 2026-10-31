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
use crate::session_learning::{Learning, Role};
use tmt_core::binding::session::launch::SessionLearningDigestStop as DigestStop;
use tmt_core::binding::session::launch::{
    SessionLearningDetail, SessionLearningEvent, SessionLearningReason as Reason,
    SessionLearningStep as Step, SessionLearningStop as Stop,
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
    let mut empty_hook_output = "";
    let mut publication_deadline = started;
    let result = (|| {
        let registry = RuntimeRegistry::first_party();
        let harness = HarnessId::new(provider).map_err(|_| ())?;
        let lifecycle = registry.lifecycle(&harness).ok_or(())?;
        empty_hook_output = lifecycle.empty_hook_output();
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
        let input = read_stdin_bounded(
            deadline.saturating_duration_since(Instant::now()),
            tmt_adapters::runtime::hook_protocol::HOOK_INPUT_LIMIT,
        )
        .map_err(|_| ())?;
        turn_end = lifecycle.decode_turn(input.as_bytes()).is_some();
        if let Some(learning) = &learning {
            if turn_end || named_turn_end(input.as_bytes()) {
                learning.defer_stop();
            } else {
                let _ = learning.stage(Step::Input, deadline);
            }
            // Only a lifecycle event can bind the launch; every other run keeps what an earlier
            // run stored for it (see `Learning::set_role`).
            let role = hook_role(lifecycle, input.as_bytes());
            learning.set_role(role);
            if role == Role::Binding {
                let _ = learning.stage(Step::Input, deadline);
            }
        }
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
    if (worker || failed)
        && let Some(learning) = &learning
    {
        learning.enqueue_terminal(diagnostic_deadline);
    }
    match result {
        Ok(mut observation) => {
            if worker {
                let _ = io::stdout().lock().write_all(&observation.encode());
            } else {
                if observation.context.is_empty() && !empty_hook_output.is_empty() {
                    observation.context = empty_hook_output.to_owned();
                }
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
            } else if !empty_hook_output.is_empty() {
                let _ = io::stdout().lock().write_all(empty_hook_output.as_bytes());
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

/// What a hook payload can do to its launch. Only a lifecycle event can bind it, by its name even
/// when no decoder accepts the payload (that refusal is the launch's own, #5016); a turn, prompt
/// or activity event cannot, and neither can a payload of no known event.
fn named_turn_end(input: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(input)
        .is_ok_and(|v| matches!(v["hook_event_name"].as_str(), Some("Stop" | "Interrupt")))
}

fn hook_role(lifecycle: &dyn RuntimeLifecycle, input: &[u8]) -> Role {
    let name = serde_json::from_slice::<serde_json::Value>(input)
        .ok()
        .and_then(|value| value["hook_event_name"].as_str().map(str::to_owned));
    let event = SessionLearningEvent::from_hook_name(name.as_deref());
    let observer = event.decoder().is_some_and(|decoder| {
        decoder != tmt_core::binding::session::launch::SessionLearningDecoder::Lifecycle
    }) || name
        .as_deref()
        .is_some_and(|event| lifecycle.activity_events().contains(&event))
        || lifecycle.decode_turn(input).is_some()
        || lifecycle.decode_prompt(input).is_some()
        || lifecycle.decode_activity_session(input).is_some();
    let lifecycle_event = event.decoder()
        == Some(tmt_core::binding::session::launch::SessionLearningDecoder::Lifecycle)
        || lifecycle.decode(input).is_some();
    if !observer && lifecycle_event {
        Role::Binding
    } else {
        Role::Observer
    }
}

/// A turn end refreshes only the remembered session's driver state (#519).
/// It needs the same verified, active binding as a lifecycle event, and the
/// binding's current conversation must be the remembered one it names: a late
/// turn end from an earlier conversation writes nothing. The write is a
/// compare-and-set on the preferences read with the binding.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TurnMode {
    Full,
    ActivityOnly,
    BeforeDigest,
}

/// Only a capture failure after current binding/session authority was proved
/// may defer Digest. Inapplicable observations grant nothing to either hook.
enum TurnFailure {
    NotApplicable,
    CaptureFailed,
}

fn observe_turn(
    provider: &str,
    harness: &HarnessId,
    lifecycle: &dyn RuntimeLifecycle,
    turn: &TurnEnd,
    mode: TurnMode,
    activity: Option<&tmt_adapters::runtime::hook_protocol::ActivityObservation>,
    run: Run<'_>,
) -> Result<Observation, TurnFailure> {
    let deadline = run.deadline;
    let activity_only = mode == TurnMode::ActivityOnly;
    // An interrupted turn ends activity only: no usage, consumption source or
    // final answer is read or captured for it.
    let activity_only = activity_only || turn.interrupted;
    // An installed answer observer owns Stop. A launch-only activity observer
    // must not race it to Idle or append a duplicate continued final. Digest
    // captures ordinary Stop before blocking; its decoder excludes continuations.
    if mode == TurnMode::ActivityOnly
        && tmt_adapters::drivers::Registry::builtin()
            .find(provider)
            .is_some_and(tmt_adapters::setup::stop_hook_installed)
    {
        run.stopped(Stop::ObserverOwnsStop);
        return Ok(Observation::default());
    }
    let capture_on = !activity_only;
    let host = lifecycle
        .host_evidence()
        .map_err(|_| TurnFailure::NotApplicable)?;
    if matches!(host, HostEvidence::Unsupported) {
        return unsupported_host(run).map_err(|_| TurnFailure::NotApplicable);
    }
    let Caller::Bound(bound) = verified_caller(
        CallerRequest {
            provider,
            lifecycle,
            host,
            session: &turn.session,
            verified_binding: None,
            start: None,
            learning: run.learning,
        },
        deadline,
        SupervisedProbeRunner,
    )
    .map_err(|_| TurnFailure::NotApplicable)?
    else {
        run.stopped(Stop::UnboundSession);
        return Ok(Observation::default());
    };
    let BoundCaller {
        paths,
        stored,
        snapshot,
        process,
        ..
    } = *bound;
    let binding = stored
        .entry
        .binding
        .as_ref()
        .ok_or(TurnFailure::NotApplicable)?;
    let Some(process) =
        lifecycle.activity_process(&binding.session, &process, &turn.session, host, deadline)
    else {
        run.stopped(Stop::NotCurrent);
        return Ok(Observation::default());
    };
    if !matches!(
        evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot.clone())),
        BindingEvidence::Active(_)
    ) {
        return Err(TurnFailure::NotApplicable);
    }
    let stored = continue_awaiting_session(
        Continuation {
            paths: &paths,
            stored: &stored,
            snapshot: &snapshot,
            process: &process,
            harness,
            mode: lifecycle.mode(host),
            launch: run.learning.map(|learning| &learning.launch),
            session: &turn.session,
        },
        deadline,
    )
    .map_err(|_| TurnFailure::NotApplicable)?
    .unwrap_or(stored);
    let binding = stored
        .entry
        .binding
        .as_ref()
        .ok_or(TurnFailure::NotApplicable)?;
    let current = binding.session.key.as_ref().is_some_and(|key| {
        key.incarnation == process && key.provider_session.as_ref() == Some(&turn.session)
    });
    let Some(remembered) = stored.preferences.remembered.as_ref().filter(|remembered| {
        current && &remembered.harness == harness && remembered.provider_session == turn.session
    }) else {
        run.stopped(Stop::NotCurrent);
        return Ok(Observation::default());
    };
    let usage_on = !activity_only
        && tmt_adapters::drivers::Registry::builtin()
            .find(provider)
            .is_some_and(tmt_adapters::setup::usage_consented);
    // Only older payloads missing the final need the consented driver reader
    // before capture; optional usage never precedes capture of supplied text.
    let queued = (|| -> Result<bool, ()> {
        // Some provider Stops have no activity event (for example Codex without
        // a turn id), but their existing consented usage follow-up still runs.
        let no_activity = tmt_adapters::runtime::hook_protocol::ActivityObservation {
            event: tmt_core::binding::session::activity::Event {
                kind: tmt_core::binding::session::activity::EventKind::TurnEnded,
                turn: None,
            },
            model: None,
        };
        let fallback = if turn.final_text.is_none() && usage_on {
            ProviderEnvironment::capture().ok().and_then(|environment| {
                lifecycle
                    .turn_observation(
                        turn,
                        &environment.with_data_root(&paths.global_dir),
                        remembered.state.as_ref(),
                        tmt_adapters::request_runtime::wall_time_ms(),
                        deadline,
                    )
                    .final_text
            })
        } else {
            None
        };
        let response = kept_answer(capture_on, Some(fallback), turn.final_text.as_ref())
            .map(|text| tmt_adapters::storage::ResponseCapture { text });
        let follow_up = tmt_adapters::storage::hook_queue::CaptureFollowUp {
            usage: usage_on && mode == TurnMode::Full,
            transcript: turn.transcript.clone(),
            learning: run.learning.map(Learning::queued),
        };
        let capture = tmt_adapters::storage::TurnCapture {
            expected: &stored,
            process: &process,
            activity: activity.unwrap_or(&no_activity),
            response,
            interrupted: turn.interrupted,
            now_ms: tmt_adapters::request_runtime::wall_time_ms(),
            deadline,
        };
        let prepared = tmt_adapters::storage::hook_queue::CaptureEvent::prepare(
            &paths.database,
            &capture,
            follow_up,
        );
        let Ok(event) = prepared else {
            return Ok(false);
        };
        let mut event = if activity.is_none() {
            event.usage_only()
        } else {
            event
        };
        // A refused interrupt has no final, usage or terminal observation to
        // retain. Keep its existing guarantee of leaving durable state unchanged.
        if event.interrupted()
            && event.admission() != tmt_adapters::storage::hook_queue::CaptureResult::Committed
        {
            return Ok(false);
        }
        use tmt_adapters::storage::hook_queue::{CapturePublication, CaptureResult};
        let publication = crate::hook_flush_command::enqueue(&paths.database, &event, deadline);
        let captured = match publication {
            CapturePublication::Queued => matches!(
                event.admission(),
                CaptureResult::Committed | CaptureResult::Duplicate
            ),
            CapturePublication::Recorded(outcome) => {
                matches!(outcome, CaptureResult::Committed | CaptureResult::Duplicate)
            }
            CapturePublication::Unpublished(error)
                if error.code == StorageErrorCode::IncompatibleSchema =>
            {
                // Only a missing/incompatible queue selects the direct path;
                // an uncertain append never authorizes capture replay.
                let direct = (|| -> Result<_, StorageError> {
                    let mut storage = Storage::open_hook(&paths.database, deadline)?;
                    let outcome = if activity.is_some() {
                        storage.capture_turn_end(event.id(), event.recorded_batch(), capture)
                    } else {
                        Ok(tmt_adapters::storage::TurnCaptureOutcome::NotApplicable)
                    };
                    let closed = storage.close();
                    outcome.and_then(|value| closed.map(|()| value))
                })();
                let outcome = match direct {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        // A committed direct receipt wins over this diagnostic,
                        // including when only connection cleanup failed.
                        let _ = CapturePublication::Unpublished(error).record_failure(
                            &paths.database,
                            &event,
                            deadline,
                        );
                        return Err(());
                    }
                };
                let result = CaptureResult::from(outcome);
                if outcome == tmt_adapters::storage::TurnCaptureOutcome::Committed {
                    let _ = event.adopt_direct_batch(&paths.database);
                }
                flush_capture_follow_up(&paths, &event, result, deadline);
                matches!(result, CaptureResult::Committed | CaptureResult::Duplicate)
            }
            unpublished @ CapturePublication::Unpublished(_) => {
                let _ = unpublished.record_failure(&paths.database, &event, deadline);
                return Err(());
            }
            CapturePublication::Unknown { .. } => return Err(()),
        };
        if let Some(learning) = run.learning {
            learning.capture_queued();
        }
        Ok(captured)
    })();
    let captured = match queued {
        Ok(queued) => queued,
        Err(()) => return Err(TurnFailure::CaptureFailed),
    };
    Ok(Observation {
        context: String::new(),
        workspace: captured.then(|| binding.server.clone()),
    })
}

/// Optional Stop follow-up runs only after the final and its receipt commit.
pub(crate) fn flush_capture_follow_up(
    paths: &ConfigPaths,
    event: &tmt_adapters::storage::hook_queue::CaptureEvent,
    result: tmt_adapters::storage::hook_queue::CaptureResult,
    deadline: Instant,
) {
    if !event.is_current_batch(&paths.database).unwrap_or(false) {
        return;
    }
    if let Some(learning) = &event.follow_up.learning {
        Learning::apply_queued(learning, event, result, deadline);
    }
    if !event.follow_up.usage || event.interrupted() {
        return;
    }
    let _ = (|| -> Result<(), ()> {
        let provider = event.provider().ok_or(())?;
        if !tmt_adapters::drivers::Registry::builtin()
            .find(provider)
            .is_some_and(tmt_adapters::setup::usage_consented)
        {
            return Ok(());
        }
        let harness = HarnessId::new(provider).map_err(|_| ())?;
        let registry = RuntimeRegistry::first_party();
        let lifecycle = registry.lifecycle(&harness).ok_or(())?;
        let refreshed = event
            .follow_up_context(&paths.database)
            .map_err(|_| ())?
            .ok_or(())?;
        let previous = refreshed.preferences.remembered.as_ref().ok_or(())?;
        if previous.harness != harness
            || previous.provider_session.as_str() != event.session().ok_or(())?
        {
            return Ok(());
        }
        let turn = TurnEnd {
            session: previous.provider_session.clone(),
            transcript: event.follow_up.transcript.clone(),
            final_text: None,
            interrupted: false,
        };
        let environment = ProviderEnvironment::capture()
            .map_err(|_| ())?
            .with_data_root(&paths.global_dir);
        let now_ms = tmt_adapters::request_runtime::wall_time_ms();
        let observation = lifecycle.turn_observation(
            &turn,
            &environment,
            previous.state.as_ref(),
            now_ms,
            deadline,
        );
        let consumption = observation
            .state
            .as_ref()
            .and_then(|state| lifecycle.sampled_consumption(state, &environment));
        let locator = turn.transcript.as_ref().and_then(|path| {
            lifecycle.consumption_locator(
                serde_json::json!({"transcript_path":path})
                    .to_string()
                    .as_bytes(),
                &environment,
            )
        });
        commit_observation(
            paths,
            &refreshed,
            observation.state,
            Some(ConsumptionReading {
                locator,
                consumption,
                now_ms,
            }),
            None,
            None,
            deadline,
        )?;
        Ok(())
    })();
}

/// Blocking feedback is downstream of the observer's single capture entry.
/// Digest never installs an answer hook or grants transcript consent.
pub(crate) fn capture_before_digest(
    provider: &str,
    input: &[u8],
    deadline: Instant,
) -> Result<(), DigestStop> {
    if !tmt_adapters::drivers::Registry::builtin()
        .find(provider)
        .is_some_and(tmt_adapters::setup::stop_hook_installed)
    {
        return Ok(());
    }
    let registry = RuntimeRegistry::first_party();
    let Ok(harness) = HarnessId::new(provider) else {
        return Ok(());
    };
    let Some(lifecycle) = registry.lifecycle(&harness) else {
        return Ok(());
    };
    let Some(turn) = lifecycle.decode_turn(input) else {
        return Ok(());
    };
    let activity = lifecycle.decode_activity(input);
    match observe_turn(
        provider,
        &harness,
        lifecycle,
        &turn,
        TurnMode::BeforeDigest,
        activity.as_ref(),
        Run {
            deadline,
            learning: None,
            event: SessionLearningEvent::Stop,
        },
    ) {
        Ok(_) | Err(TurnFailure::NotApplicable) => Ok(()),
        Err(TurnFailure::CaptureFailed) => Err(DigestStop::CaptureFailed),
    }
}

/// Prefer the observed final when usage is on; otherwise keep the Stop payload's own
/// final text, including when the observation found none. With usage off no observation
/// is made. A turn end that was not admitted, or an interrupted one, keeps nothing.
fn kept_answer(
    admitted: bool,
    observed: Option<Option<tmt_core::responses::FinalText>>,
    payload: Option<&tmt_core::responses::FinalText>,
) -> Option<tmt_core::responses::FinalText> {
    if !admitted {
        return None;
    }
    observed.flatten().or_else(|| payload.cloned())
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

#[derive(Debug, Clone, Copy)]
pub(super) enum CallerProbeFailure {
    Config,
    Host,
    Pane,
    Storage(StorageErrorCode),
    Process,
}

pub(super) fn verified_lifecycle_caller<R: CommandRunner + Clone>(
    provider: &str,
    lifecycle: &dyn RuntimeLifecycle,
    host: HostEvidence,
    session: &ProviderSessionId,
    verified_binding: Option<&str>,
    deadline: Instant,
    runner: R,
) -> Result<Caller, CallerProbeFailure> {
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
) -> Result<Caller, CallerProbeFailure> {
    let CallerRequest {
        provider,
        lifecycle,
        host,
        session,
        verified_binding,
        start,
        learning,
    } = request;
    let paths = ConfigPaths::discover().map_err(|_| CallerProbeFailure::Config)?;
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
        .map_err(|error| CallerProbeFailure::Storage(error.code))?
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
        let binding = stored
            .entry
            .binding
            .as_ref()
            .ok_or(CallerProbeFailure::Storage(StorageErrorCode::Unknown))?;
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
            .map_err(|_| CallerProbeFailure::Pane)?
        else {
            return Err(CallerProbeFailure::Pane);
        };
        let pid = u64::from(host.runtime_pid().ok_or(CallerProbeFailure::Process)?);
        let tmt_adapters::process::runtime::ProcessObservation::Live(process) =
            observe_runtime_process(&runner, pid, deadline)
                .map_err(|_| CallerProbeFailure::Process)?
        else {
            return Err(CallerProbeFailure::Process);
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
            panes,
            pane,
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
                CallerProbeFailure::Process
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
            return Err(CallerProbeFailure::Process);
        }
        let (stored, snapshot) = match stored {
            Some(stored) => (stored, snapshot),
            // A hand-started provider in a pane with no binding binds the saved identity that
            // remembers this very session. Any refusal is the unbound hint.
            None => match start.filter(|_| !marked).and_then(|event| {
                bind_remembered(&paths, &panes, &pane, provider, event.session(), deadline)
            }) {
                Some(bound) => bound,
                None => return Ok(Caller::Unbound { marked }),
            },
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

struct HookPane<R: CommandRunner + Clone> {
    stored: Option<IdentityContextSnapshot>,
    snapshot: EndpointSnapshot,
    pid: u64,
    marked: bool,
    panes: Host<R>,
    pane: String,
}

/// The one saved identity that remembers exactly this provider session binds to the hook's
/// pane (#5401). The pane and the provider's place in it are verified before this runs, so it
/// adds only the identity choice; `bind_existing_identity` refuses an identity that is active
/// anywhere else and fences the pane again inside its own transaction. The binding carries no
/// session key yet: the caller's ordinary start admission decides that, as it does after
/// `tmt this`. Every refusal returns `None`.
fn bind_remembered<R: CommandRunner + Clone>(
    paths: &ConfigPaths,
    panes: &Host<R>,
    pane: &String,
    harness: &str,
    session: &ProviderSessionId,
    deadline: Instant,
) -> Option<(IdentityContextSnapshot, EndpointSnapshot)> {
    let now = tmt_adapters::request_runtime::wall_time_ms();
    let candidate =
        Storage::context_by_remembered_session(&paths.database, harness, session.as_str(), now)
            .ok()??;
    let mut storage = Storage::open_hook(&paths.database, deadline).ok()?;
    let bound = tmt_core::binding::bind_existing_identity(
        &mut storage,
        &mut panes.session(),
        pane,
        &candidate.entry.identity.id,
        false,
    );
    let closed = storage.close();
    bound.ok()?;
    closed.ok()?;
    let snapshot = panes
        .observe_snapshot(OperationOptions {
            deadline: Some(deadline),
            pane_ids: Some(std::slice::from_ref(pane)),
        })
        .ok()?;
    let stored = Storage::context_by_pane(
        &paths.database,
        snapshot.server.host,
        pane,
        &snapshot.server.server_id,
        now,
    )
    .ok()??;
    (stored.entry.identity.id == candidate.entry.identity.id).then_some((stored, snapshot))
}

/// Host-neutral caller ancestry and a fresh pane snapshot; neither is session authority.
fn observed_hook_pane<R: CommandRunner + Clone>(
    paths: &ConfigPaths,
    deadline: Instant,
    runner: R,
    learning: Option<&Learning>,
) -> Result<Option<HookPane<R>>, CallerProbeFailure> {
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
    .map_err(|_| CallerProbeFailure::Host)?;
    let Some(pane) = panes
        .caller_pane(&environment)
        .map_err(|_| CallerProbeFailure::Pane)?
    else {
        return if Instant::now() < deadline {
            Ok(None)
        } else {
            Err(CallerProbeFailure::Pane)
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
        .map_err(|_| CallerProbeFailure::Pane)?;
    let observed_pane = snapshot
        .panes
        .iter()
        .find(|item| item.id == pane)
        .ok_or(CallerProbeFailure::Pane)?;
    // A nested caller's host was chosen from one pinned ancestry; the
    // pane it reports now must be the pane shell that ancestry held.
    if panes
        .pinned_caller_pane()
        .is_some_and(|pid| pid != observed_pane.pane_pid)
    {
        return Err(CallerProbeFailure::Pane);
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
    .map_err(|error| CallerProbeFailure::Storage(error.code))?;
    let pid = observed_pane.pane_pid;
    let marked = observed_pane.marker.is_some();
    Ok(Some(HookPane {
        stored,
        snapshot,
        pid,
        marked,
        panes,
        pane,
    }))
}

/// Discover or verify this exact foreground launch in one native admission pass.
/// Supplied argv and discovered coordinates are locators, never authority. A refusal names
/// the check that failed, so a Digest Stop that ends with no output can say why.
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
    DigestStop,
> {
    if launch.is_some_and(|v| !v.valid()) {
        return Err(DigestStop::LaunchArgument);
    }
    let host = lifecycle
        .host_evidence()
        .map_err(|_| DigestStop::LaunchHost)?;
    let mut bound = caller_check(
        verified_lifecycle_caller(
            provider,
            lifecycle,
            host,
            session,
            None,
            deadline,
            runner.clone(),
        ),
        deadline,
    )?;
    let binding = bound
        .stored
        .entry
        .binding
        .as_ref()
        .ok_or(DigestStop::LaunchBinding)?;
    let launch = match launch {
        Some(launch) => launch.clone(),
        None => {
            let owner = binding
                .session
                .launch_owner
                .as_ref()
                .ok_or(DigestStop::LaunchBinding)?;
            tmt_adapters::runtime::hook_protocol::HookLaunch {
                identity_id: binding.identity_id.clone(),
                binding_id: binding.id.clone(),
                owner_pid: owner.pid(),
                owner_start: owner.start_identity().to_owned(),
            }
        }
    };
    let owner = launch
        .valid()
        .then(|| launch.owner())
        .flatten()
        .ok_or(DigestStop::LaunchArgument)?;
    let process = lifecycle
        .digest_process(&binding.session, &bound.process, session, host, deadline)
        .ok_or(DigestStop::LaunchProcess)?;
    let harness = HarnessId::new(provider).map_err(|_| DigestStop::LaunchBinding)?;
    if let Some(stored) = continue_awaiting_session(
        Continuation {
            paths: &bound.paths,
            stored: &bound.stored,
            snapshot: &bound.snapshot,
            process: &process,
            harness: &harness,
            mode: lifecycle.mode(host),
            launch: Some(&launch),
            session,
        },
        deadline,
    )
    .map_err(|error| continuation_failure(error, deadline))?
    {
        bound.stored = stored;
    }
    let binding = bound
        .stored
        .entry
        .binding
        .as_ref()
        .ok_or(DigestStop::LaunchBinding)?;
    if binding.id != launch.binding_id
        || binding.identity_id != launch.identity_id
        || binding.session.state != tmt_core::binding::session::RuntimeState::Running
        || binding.session.launch_owner.as_ref() != Some(&owner)
    {
        return Err(DigestStop::LaunchBinding);
    }
    if !binding.session.key.as_ref().is_some_and(|key| {
        key.incarnation == process && key.provider_session.as_ref() == Some(session)
    }) {
        return Err(DigestStop::LaunchSession);
    }
    if !bound
        .stored
        .preferences
        .remembered
        .as_ref()
        .is_some_and(|remembered| {
            remembered.harness.as_str() == provider && &remembered.provider_session == session
        })
    {
        return Err(DigestStop::LaunchRemembered);
    }
    if !matches!(
        evaluate_binding(&bound.stored.entry, &EndpointProbe::Live(bound.snapshot)),
        BindingEvidence::Active(_)
    ) {
        return Err(DigestStop::LaunchEndpoint);
    }
    owner_check(
        observe_runtime_process(&runner, owner.pid(), deadline).map_err(|_| ()),
        &owner,
        deadline,
    )?;
    if Instant::now() >= deadline {
        return Err(DigestStop::LaunchDeadline);
    }
    Ok((launch, bound.paths, bound.stored))
}

/// A probe that failed is not a refused caller: it names the deadline once that passed.
fn probe_failure(probe: DigestStop, deadline: Instant) -> DigestStop {
    if Instant::now() >= deadline {
        DigestStop::LaunchDeadline
    } else {
        probe
    }
}

fn caller_check(
    caller: Result<Caller, CallerProbeFailure>,
    deadline: Instant,
) -> Result<Box<BoundCaller>, DigestStop> {
    match caller {
        Ok(Caller::Bound(bound)) => Ok(bound),
        Ok(_) => Err(DigestStop::LaunchCaller),
        Err(error) => Err(probe_failure(
            match error {
                CallerProbeFailure::Config => DigestStop::LaunchCallerConfig,
                CallerProbeFailure::Host => DigestStop::LaunchCallerHost,
                CallerProbeFailure::Pane => DigestStop::LaunchCallerPane,
                CallerProbeFailure::Storage(StorageErrorCode::Busy) => {
                    DigestStop::LaunchCallerStorageBusy
                }
                CallerProbeFailure::Storage(_) => DigestStop::LaunchCallerStorage,
                CallerProbeFailure::Process => DigestStop::LaunchCallerProcess,
            },
            deadline,
        )),
    }
}

/// Only a live owner with a different start is `launch:owner`, the start identity check
/// itself; a probe with no answer and an owner that is not live are named apart.
fn owner_check(
    observed: Result<tmt_adapters::process::runtime::ProcessObservation, ()>,
    owner: &ProcessIncarnation,
    deadline: Instant,
) -> Result<(), DigestStop> {
    use tmt_adapters::process::runtime::ProcessObservation as Observed;
    match observed {
        Ok(Observed::Live(current)) if current == *owner => Ok(()),
        Ok(Observed::Live(_)) => Err(DigestStop::LaunchOwner),
        Ok(Observed::Gone | Observed::Stopped(_) | Observed::UnreapedZombie(_)) => {
            Err(DigestStop::LaunchOwnerNotLive)
        }
        Ok(Observed::Unknown) | Err(()) => {
            Err(probe_failure(DigestStop::LaunchOwnerProbe, deadline))
        }
    }
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
    // A following event must see the preceding queued Stop before taking its
    // snapshot (notably Claude prompts without turn IDs). This explicit drain
    // shares the hook budget; failure leaves recovery to __flush and the event
    // proceeds. Mutation wrappers must never drain after this snapshot.
    if lifecycle.decode_turn(input.as_bytes()).is_none()
        && !named_turn_end(input.as_bytes())
        && let Ok(paths) = ConfigPaths::discover()
        && tmt_adapters::storage::hook_queue::HookQueue::exists(&paths.database)
        && let Ok(mut storage) = Storage::open_hook(&paths.database, deadline)
    {
        let _ = storage.ingest_pending_hook_captures();
        let _ = storage.close();
    }
    // Notification observers receive non-permission notifications too. Declared
    // activity-only events that carry no admitted main event are quiet no-ops.
    let event_name = serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| value["hook_event_name"].as_str().map(str::to_owned));
    let activity_only_event = event_name
        .as_deref()
        .is_some_and(|event| lifecycle.activity_events().contains(&event));
    let run = Run {
        deadline,
        learning,
        event: SessionLearningEvent::from_hook_name(event_name.as_deref()),
    };
    if activity_only_event && activity.is_none() {
        run.stopped(Stop::ActivityMissing);
        return Ok(Observation::default());
    }
    if let Some(turn) = lifecycle.decode_turn(input.as_bytes()) {
        return observe_turn(
            provider,
            &harness,
            lifecycle,
            &turn,
            if activity_only {
                TurnMode::ActivityOnly
            } else {
                TurnMode::Full
            },
            activity.as_ref(),
            run,
        )
        .map_err(|_| ());
    }
    if let Some(session) = lifecycle.decode_prompt(input.as_bytes()) {
        return observe_main_event(
            provider,
            &harness,
            lifecycle,
            &session,
            activity.as_ref(),
            Some(input.as_bytes()),
            run,
        );
    }
    if let Some(session) = lifecycle.decode_activity_session(input.as_bytes()) {
        let Some(activity) = activity.as_ref() else {
            run.stopped(Stop::ActivityMissing);
            return Err(());
        };
        return observe_activity_event(&harness, lifecycle, &session, activity, run, || {
            ConfigPaths::discover().map_err(|_| ())
        });
    }
    let Some(event) = lifecycle.decode(input.as_bytes()) else {
        // Nothing accepted the payload: say which event it was, never what it held.
        if let Some(learning) = learning {
            let event = SessionLearningEvent::from_hook_name(event_name.as_deref());
            let _ = learning.note(SessionLearningDetail::Refused(event), deadline);
        }
        return Err(());
    };
    observe_lifecycle_event(
        provider,
        harness,
        lifecycle,
        input.as_bytes(),
        event,
        deadline,
        learning,
    )
}

/// One hook run: its deadline and the launch diagnostic it reports to, if any.
#[derive(Clone, Copy)]
struct Run<'a> {
    deadline: Instant,
    learning: Option<&'a Learning>,
    /// The hook event being run, by name only; `Unknown` where none applies.
    event: SessionLearningEvent,
}
impl Run<'_> {
    /// Names why this decoded event, which cannot bind, admitted nothing (#5125). Diagnostics
    /// only: what the hook admits is unchanged.
    fn stopped(&self, stop: Stop) {
        if let Some(learning) = self.learning
            && let Some(detail) = SessionLearningDetail::stopped(self.event, stop)
        {
            let _ = learning.note(detail, self.deadline);
        }
    }
}

/// The driver found no provider process among the hook's ancestors, so the hook
/// observes nothing. Say so in the launch's diagnostic instead of stopping silently.
fn unsupported_host(run: Run<'_>) -> Result<Observation, ()> {
    if let Some(learning) = run.learning {
        let _ = learning.stage_detail(
            Step::HostDiscovery,
            SessionLearningDetail::HostUnsupported,
            run.deadline,
        );
    }
    Ok(Observation::default())
}

fn observe_lifecycle_event(
    provider: &str,
    harness: HarnessId,
    lifecycle: &dyn RuntimeLifecycle,
    input: &[u8],
    event: Box<dyn tmt_adapters::runtime::lifecycle::LifecycleObservation>,
    deadline: Instant,
    learning: Option<&Learning>,
) -> Result<Observation, ()> {
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return unsupported_host(Run {
            deadline,
            learning,
            event: SessionLearningEvent::Unknown,
        });
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
    )
    .map_err(|_| ())?
    {
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
            &RuntimeRegistry::first_party(),
        )
        .map_err(|_| ())?;
    }
    if changed
        && event.starting()
        && let Ok(environment) = ProviderEnvironment::capture()
        && tmt_adapters::drivers::Registry::builtin()
            .find(provider)
            .is_some_and(tmt_adapters::setup::usage_consented)
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
        let locator = lifecycle.consumption_locator(input, &environment);
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
    // A plain program is not a session: the first agent in the same verified pane replaces it,
    // whatever this identity ran before, and its admission clears `plain_program` in the same CAS.
    let plain = binding.session.plain_program.is_some();
    if !plain && recorded.is_some_and(|recorded| recorded != harness) {
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
    if !plain && key.is_some() && previous == RuntimeLiveness::Unknown {
        return Some(Why::PreviousSessionUnknown);
    }
    let ended = binding.session.state == RuntimeState::Ended
        || binding.session.last_transition == Some(SessionTransition::Ended);
    if !plain && key.is_some() && !ended && previous != RuntimeLiveness::Gone {
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
/// A launch-owned binding awaiting its next session continues with the session its own
/// verified process reports (#5703): a `/clear` whose start was never admitted otherwise
/// leaves every later event of that process refused. The launch claim must name this binding
/// and its live owner, as the launch's own hooks do; nothing else continues it. Returns the
/// refreshed context once committed, `None` when the binding is not awaiting this session.
fn continue_awaiting_session(
    continuation: Continuation<'_>,
    deadline: Instant,
) -> Result<Option<IdentityContextSnapshot>, ContinuationFailure> {
    let Continuation {
        paths,
        stored,
        snapshot,
        process,
        harness,
        mode,
        launch,
        session,
    } = continuation;
    let binding = stored
        .entry
        .binding
        .as_ref()
        .ok_or(ContinuationFailure::Binding)?;
    let (Some(launch), Some(mode)) = (launch, mode) else {
        return Ok(None);
    };
    if !binding.session.awaiting_next_session()
        || !launch.valid()
        || launch.binding_id != binding.id
        || launch.identity_id != binding.identity_id
        || launch.owner().as_ref() != binding.session.launch_owner.as_ref()
    {
        return Ok(None);
    }
    let Some(next) =
        binding
            .session
            .continue_session(tmt_core::binding::session::ObservedSessionKey {
                incarnation: process.clone(),
                provider_session: Some(session.clone()),
            })
    else {
        return Ok(None);
    };
    let owner = binding
        .session
        .launch_owner
        .as_ref()
        .ok_or(ContinuationFailure::Binding)?;
    if !matches!(
        observe_runtime_process(&SupervisedProbeRunner, owner.pid(), deadline).map_err(|_| ContinuationFailure::OwnerProbe)?,
        tmt_adapters::process::runtime::ProcessObservation::Live(current) if current == *owner
    ) || !matches!(
        evaluate_binding(&stored.entry, &EndpointProbe::Live(snapshot.clone())),
        BindingEvidence::Active(_)
    ) {
        return Ok(None);
    }
    let mut storage = Storage::open_hook(&paths.database, deadline)
        .map_err(|error| ContinuationFailure::Open(error.code))?;
    let committed = storage
        .with_binding_transaction::<_, StorageError>(|records| {
            if records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding)
                .as_ref()
                != Some(binding)
                || records.session_preferences(&binding.identity_id)? != stored.preferences
                || !records.set_session_state(&binding.id, &binding.session, &next)?
            {
                return Ok(false);
            }
            let mut preferences = stored.preferences.clone();
            preferences.remember(harness.clone(), mode, session.clone());
            records.set_session_preferences(&binding.identity_id, &preferences)
        })
        .map_err(|error| ContinuationFailure::Write(error.code));
    let closed = storage
        .close()
        .map_err(|error| ContinuationFailure::Close(error.code));
    if !committed? {
        return Ok(None);
    }
    closed?;
    Storage::context_by_identity(
        &paths.database,
        &binding.identity_id,
        tmt_adapters::request_runtime::wall_time_ms(),
    )
    .map_err(|error| ContinuationFailure::Read(error.code))
}

#[derive(Debug, Clone, Copy)]
enum ContinuationFailure {
    Binding,
    OwnerProbe,
    Open(StorageErrorCode),
    Write(StorageErrorCode),
    Close(StorageErrorCode),
    Read(StorageErrorCode),
}

fn continuation_failure(error: ContinuationFailure, deadline: Instant) -> DigestStop {
    let stop = match error {
        ContinuationFailure::Binding => DigestStop::LaunchBinding,
        ContinuationFailure::OwnerProbe => DigestStop::LaunchContinuationOwnerProbe,
        ContinuationFailure::Open(StorageErrorCode::Busy) => DigestStop::LaunchContinuationOpenBusy,
        ContinuationFailure::Open(_) => DigestStop::LaunchContinuationOpen,
        ContinuationFailure::Write(StorageErrorCode::Busy) => {
            DigestStop::LaunchContinuationWriteBusy
        }
        ContinuationFailure::Write(_) => DigestStop::LaunchContinuationWrite,
        ContinuationFailure::Close(StorageErrorCode::Busy) => {
            DigestStop::LaunchContinuationCloseBusy
        }
        ContinuationFailure::Close(_) => DigestStop::LaunchContinuationClose,
        ContinuationFailure::Read(StorageErrorCode::Busy) => DigestStop::LaunchContinuationReadBusy,
        ContinuationFailure::Read(_) => DigestStop::LaunchContinuationRead,
    };
    probe_failure(stop, deadline)
}

struct Continuation<'a> {
    paths: &'a ConfigPaths,
    stored: &'a IdentityContextSnapshot,
    snapshot: &'a EndpointSnapshot,
    process: &'a ProcessIncarnation,
    harness: &'a HarnessId,
    mode: Option<tmt_core::binding::session::RuntimeMode>,
    launch: Option<&'a tmt_adapters::runtime::hook_protocol::HookLaunch>,
    session: &'a ProviderSessionId,
}

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
    run: Run<'_>,
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
            run.stopped(Stop::NotWaiting);
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
        run,
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
    run: Run<'_>,
) -> Result<Observation, ()> {
    let deadline = run.deadline;
    let host = lifecycle.host_evidence().map_err(|_| ())?;
    if matches!(host, HostEvidence::Unsupported) {
        return unsupported_host(run);
    }
    let Caller::Bound(bound) = verified_caller(
        CallerRequest {
            provider,
            lifecycle,
            host,
            session,
            verified_binding: None,
            start: None,
            learning: run.learning,
        },
        deadline,
        SupervisedProbeRunner,
    )
    .map_err(|_| ())?
    else {
        run.stopped(Stop::UnboundSession);
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
        run.stopped(Stop::NotCurrent);
        return Ok(Observation::default());
    };
    let stored = continue_awaiting_session(
        Continuation {
            paths: &paths,
            stored: &stored,
            snapshot: &snapshot,
            process: &process,
            harness,
            mode: lifecycle.mode(host),
            launch: run.learning.map(|learning| &learning.launch),
            session,
        },
        deadline,
    )
    .map_err(|_| ())?
    .unwrap_or(stored);
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
        if let Some(bytes) = payload
            && let Some(event) = lifecycle.decode(bytes)
            && event.starting()
            && stored
                .preferences
                .remembered
                .as_ref()
                .is_some_and(|remembered| {
                    &remembered.harness == harness && &remembered.provider_session == session
                })
        {
            return observe_lifecycle_event(
                provider,
                harness.clone(),
                lifecycle,
                bytes,
                event,
                deadline,
                None,
            );
        }
        run.stopped(Stop::NotCurrent);
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
            .is_some_and(tmt_adapters::setup::usage_consented)
    {
        ProviderEnvironment::capture().ok().and_then(|environment| {
            let environment = environment.with_data_root(&paths.global_dir);
            let locator = lifecycle.consumption_locator(payload?, &environment);
            let turn =
                lifecycle.sampling_turn(session, locator.as_deref(), &environment, deadline)?;
            let now = tmt_adapters::request_runtime::wall_time_ms();
            let state = lifecycle.turn_state(&turn, &environment, previous, now, deadline);
            let consumption = state
                .as_ref()
                .and_then(|state| lifecycle.sampled_consumption(state, &environment));
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
    let activity_next = activity.and_then(|event| {
        lifecycle.activity_state(
            event,
            session,
            &process,
            observed,
            tmt_adapters::request_runtime::wall_time_ms(),
        )
    });
    let next = activity_next
        .clone()
        .or_else(|| reading.as_ref().and_then(|(state, _)| state.clone()));
    // An admitted turn start carries the prompt the provider is about to run; a passthrough
    // typed with that text is bound to this turn.
    let prompt = activity
        .filter(|activity| {
            activity.event.kind == tmt_core::binding::session::activity::EventKind::TurnStarted
        })
        .and(payload)
        .and_then(|bytes| lifecycle.decode_prompt_text(bytes));
    let boundary = activity
        .filter(|event| {
            event.event.kind == tmt_core::binding::session::activity::EventKind::TurnStarted
        })
        .filter(|_| activity_next.is_some())
        .map(|event| {
            let turn = event.event.turn.as_ref().map(ProviderSessionId::as_str);
            match prompt.as_deref() {
                Some(prompt) => tmt_adapters::storage::TurnBoundary::Started { turn, prompt },
                None => tmt_adapters::storage::TurnBoundary::StartedWithoutPrompt { turn },
            }
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
        // A kept answer is stamped even when no usage was read.
        now_ms: reading
            .as_ref()
            .map_or_else(tmt_adapters::request_runtime::wall_time_ms, |r| r.now_ms),
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
mod decode_tests;

#[cfg(test)]
mod interrupt_tests;

#[cfg(test)]
mod digest_launch_tests {
    use super::*;
    use tmt_adapters::process::runtime::ProcessObservation as Observed;

    #[test]
    fn caller_and_continuation_faults_have_distinct_bounded_details() {
        let later = Instant::now() + Duration::from_secs(5);
        let passed = Instant::now();
        for (failure, expected) in [
            (CallerProbeFailure::Config, DigestStop::LaunchCallerConfig),
            (CallerProbeFailure::Host, DigestStop::LaunchCallerHost),
            (CallerProbeFailure::Pane, DigestStop::LaunchCallerPane),
            (
                CallerProbeFailure::Storage(StorageErrorCode::Busy),
                DigestStop::LaunchCallerStorageBusy,
            ),
            (
                CallerProbeFailure::Storage(StorageErrorCode::Unknown),
                DigestStop::LaunchCallerStorage,
            ),
            (CallerProbeFailure::Process, DigestStop::LaunchCallerProcess),
        ] {
            assert_eq!(caller_check(Err(failure), later).err(), Some(expected));
            assert_eq!(
                caller_check(Err(failure), passed).err(),
                Some(DigestStop::LaunchDeadline)
            );
        }
        for (failure, expected) in [
            (
                ContinuationFailure::OwnerProbe,
                DigestStop::LaunchContinuationOwnerProbe,
            ),
            (
                ContinuationFailure::Open(StorageErrorCode::Busy),
                DigestStop::LaunchContinuationOpenBusy,
            ),
            (
                ContinuationFailure::Open(StorageErrorCode::Unknown),
                DigestStop::LaunchContinuationOpen,
            ),
            (
                ContinuationFailure::Write(StorageErrorCode::Busy),
                DigestStop::LaunchContinuationWriteBusy,
            ),
            (
                ContinuationFailure::Write(StorageErrorCode::Unknown),
                DigestStop::LaunchContinuationWrite,
            ),
            (
                ContinuationFailure::Close(StorageErrorCode::Busy),
                DigestStop::LaunchContinuationCloseBusy,
            ),
            (
                ContinuationFailure::Close(StorageErrorCode::Unknown),
                DigestStop::LaunchContinuationClose,
            ),
            (
                ContinuationFailure::Read(StorageErrorCode::Busy),
                DigestStop::LaunchContinuationReadBusy,
            ),
            (
                ContinuationFailure::Read(StorageErrorCode::Unknown),
                DigestStop::LaunchContinuationRead,
            ),
        ] {
            assert_eq!(continuation_failure(failure, later), expected);
            assert_eq!(
                continuation_failure(failure, passed),
                DigestStop::LaunchDeadline
            );
            assert_eq!(DigestStop::from_worker(expected.name()), Some(expected));
        }
        // The existing clean-owner control below and the ordinary hook paths use the
        // same verification: diagnostics add no success or fallback authority.
    }

    #[test]
    fn each_owner_and_caller_outcome_has_its_own_name() {
        let owner = ProcessIncarnation::new(7, "ps-v1:Sat Oct 24 11:40:50 2026").unwrap();
        let other = ProcessIncarnation::new(7, "ps-v1:Sat Oct 24 11:40:51 2026").unwrap();
        let later = Instant::now() + Duration::from_secs(5);
        let passed = Instant::now();
        let check = |observed, deadline| owner_check(observed, &owner, deadline);
        assert_eq!(check(Ok(Observed::Live(owner.clone())), later), Ok(()));
        assert_eq!(
            check(Ok(Observed::Live(other.clone())), later),
            Err(DigestStop::LaunchOwner),
            "only a live owner with a different start is the start identity check"
        );
        for gone in [
            Observed::Gone,
            Observed::Stopped(owner.clone()),
            Observed::UnreapedZombie(owner.clone()),
        ] {
            assert_eq!(check(Ok(gone), later), Err(DigestStop::LaunchOwnerNotLive));
        }
        assert_eq!(check(Err(()), later), Err(DigestStop::LaunchOwnerProbe));
        assert_eq!(
            check(Ok(Observed::Unknown), later),
            Err(DigestStop::LaunchOwnerProbe)
        );
        assert_eq!(check(Err(()), passed), Err(DigestStop::LaunchDeadline));
        assert_eq!(
            caller_check(Ok(Caller::Unobserved), later).err(),
            Some(DigestStop::LaunchCaller)
        );
        assert_eq!(
            caller_check(Ok(Caller::Unbound { marked: false }), later).err(),
            Some(DigestStop::LaunchCaller)
        );
        assert_eq!(
            caller_check(Err(CallerProbeFailure::Process), later).err(),
            Some(DigestStop::LaunchCallerProcess)
        );
        assert_eq!(
            caller_check(Err(CallerProbeFailure::Process), passed).err(),
            Some(DigestStop::LaunchDeadline)
        );
    }
}

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

#[cfg(test)]
mod answer_tests {
    use super::kept_answer;
    use tmt_core::responses::FinalText;

    #[test]
    fn observed_final_takes_precedence_over_the_stop_payload() {
        let payload = FinalText::new("from the payload").unwrap();
        let observed = FinalText::new("from the transcript").unwrap();
        assert_eq!(
            kept_answer(true, Some(Some(observed.clone())), Some(&payload)),
            Some(observed.clone())
        );
        assert_eq!(
            kept_answer(true, Some(Some(observed.clone())), None),
            Some(observed)
        );
    }

    #[test]
    fn missing_observed_final_falls_back_to_the_stop_payload() {
        let payload = FinalText::new("from the payload").unwrap();
        // Usage on: an observation was made but found no final text.
        assert_eq!(
            kept_answer(true, Some(None), Some(&payload)),
            Some(payload.clone())
        );
        // Usage off: the observation was not made.
        assert_eq!(kept_answer(true, None, Some(&payload)), Some(payload));
    }

    #[test]
    fn a_turn_without_either_final_source_keeps_nothing() {
        assert_eq!(kept_answer(true, Some(None), None), None);
        assert_eq!(kept_answer(true, None, None), None);
    }

    #[test]
    fn a_turn_that_was_not_admitted_keeps_neither_final_source() {
        let payload = FinalText::new("from the payload").unwrap();
        let observed = FinalText::new("from the transcript").unwrap();
        // Stale, duplicate and interrupted ends never reach answer capture.
        for observation in [None, Some(None), Some(Some(observed))] {
            assert_eq!(kept_answer(false, observation, Some(&payload)), None);
        }
    }
}
