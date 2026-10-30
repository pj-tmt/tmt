//! Invocation-local coordination of stored startup evidence; no delivery authority.
use std::{
    cell::{Cell, RefCell},
    time::Instant,
};
use tmt_adapters::{
    config::ConfigPaths,
    process::{
        SupervisedProbeRunner,
        ancestry::pinned_chain,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::{
        hook_protocol::HookLaunch,
        lifecycle::{HostEvidence, LifecycleObservation, RuntimeLifecycle},
    },
    storage::{IdentityContextSnapshot, Storage, StorageError},
};
use tmt_core::{
    binding::{
        BindingRecords, BindingRepository,
        session::{
            HarnessId, RuntimeState, SessionTransition,
            launch::{
                LaunchObservation, LaunchStatus, PendingStart, SessionLearningDetail,
                SessionLearningDiagnostic, SessionLearningDigestStop, SessionLearningEvent,
                SessionLearningReason, SessionLearningStep, SessionLearningStop,
            },
        },
    },
    endpoint::ProcessIncarnation,
};

pub const ENVIRONMENT: &str = "TMT_LAUNCH_OBSERVATION";
/// One change to the stored diagnostic of a launch.
enum Update {
    /// Enter a step, dropping what the previous one stopped on.
    Stage(SessionLearningStep, Option<SessionLearningDetail>),
    /// Name what the current step stopped on.
    Note(SessionLearningDetail),
    /// End the current step with this reason, unless it already has a final one.
    Fail(SessionLearningReason),
}
/// What a hook run can do to the launch it reports to. A run learns this from its payload,
/// so it starts as `Unknown`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The payload has not been read yet.
    Unknown,
    /// A lifecycle event: it can bind or end the session, so its progress is the launch's.
    Binding,
    /// A turn, prompt or activity event, or one nothing decodes: it cannot bind, and it keeps
    /// what an earlier run stored for a launch that is not admitted yet.
    Observer,
}
#[derive(Default, Clone)]
struct TerminalDiagnostic {
    step: Option<String>,
    detail: Option<String>,
    failure: Option<String>,
}
pub struct Learning {
    pub launch: HookLaunch,
    pub driver: HarnessId,
    paths: ConfigPaths,
    role: Cell<Role>,
    deferred: Cell<bool>,
    queued_capture: Cell<bool>,
    terminal: RefCell<TerminalDiagnostic>,
}
impl Learning {
    pub fn capture(provider: &str) -> Option<Self> {
        let value = std::env::var(ENVIRONMENT).ok()?;
        if value.len() > 1024 {
            return None;
        }
        let launch: HookLaunch = serde_json::from_str(&value).ok()?;
        launch.valid().then_some(())?;
        Some(Self {
            launch,
            driver: HarnessId::new(provider).ok()?,
            paths: ConfigPaths::discover().ok()?,
            role: Cell::new(Role::Unknown),
            deferred: Cell::new(false),
            queued_capture: Cell::new(false),
            terminal: RefCell::new(TerminalDiagnostic::default()),
        })
    }
    /// Stop progress stages are transient and must never take the main writer.
    pub fn defer_stop(&self) {
        self.deferred.set(true);
    }
    pub fn capture_queued(&self) {
        self.queued_capture.set(true);
    }
    pub fn queued(&self) -> tmt_adapters::storage::hook_queue::CaptureLearning {
        let TerminalDiagnostic {
            step,
            detail,
            failure,
        } = self.terminal.borrow().clone();
        tmt_adapters::storage::hook_queue::CaptureLearning {
            launch: self.launch.clone(),
            provider: self.driver.as_str().into(),
            step,
            detail,
            failure,
        }
    }
    pub fn enqueue_terminal(&self, deadline: Instant) {
        if !self.deferred.get() || self.queued_capture.get() {
            return;
        }
        let follow_up = tmt_adapters::storage::hook_queue::CaptureFollowUp {
            learning: Some(self.queued()),
            ..Default::default()
        };
        let Ok(event) = tmt_adapters::storage::hook_queue::CaptureEvent::diagnostic(
            &self.paths.database,
            follow_up,
        ) else {
            return;
        };
        if let tmt_adapters::storage::hook_queue::CapturePublication::Unpublished(error) =
            crate::hook_flush_command::enqueue(&self.paths.database, &event, deadline)
            && error.code == tmt_adapters::storage::StorageErrorCode::IncompatibleSchema
        {
            Self::apply_queued(
                &self.queued(),
                &event,
                tmt_adapters::storage::hook_queue::CaptureResult::NotApplicable,
                deadline,
            );
        }
    }
    pub fn apply_queued(
        value: &tmt_adapters::storage::hook_queue::CaptureLearning,
        event: &tmt_adapters::storage::hook_queue::CaptureEvent,
        result: tmt_adapters::storage::hook_queue::CaptureResult,
        deadline: Instant,
    ) {
        let Ok(paths) = ConfigPaths::discover() else {
            return;
        };
        let Ok(driver) = HarnessId::new(&value.provider) else {
            return;
        };
        let learning = Self {
            launch: value.launch.clone(),
            driver,
            paths,
            role: Cell::new(Role::Observer),
            deferred: Cell::new(false),
            queued_capture: Cell::new(false),
            terminal: RefCell::new(TerminalDiagnostic::default()),
        };
        let update = if result == tmt_adapters::storage::hook_queue::CaptureResult::NoAdmittedStart
        {
            Some(Update::Note(NO_ADMITTED_START))
        } else if let Some(detail) = value
            .detail
            .as_deref()
            .and_then(SessionLearningDetail::parse)
        {
            Some(
                if let Some(step) = value.step.as_deref().and_then(SessionLearningStep::parse) {
                    Update::Stage(step, Some(detail))
                } else {
                    Update::Note(detail)
                },
            )
        } else {
            value
                .failure
                .as_deref()
                .and_then(SessionLearningReason::parse)
                .map(Update::Fail)
        };
        if let Some(update) = update {
            let _ = learning.diagnostic_fenced(update, deadline, Some(event));
        } else {
            let _ = learning.succeeded_fenced(deadline, Some(event));
        }
    }

    /// Says what this run can do, once its payload is known.
    pub fn set_role(&self, role: Role) {
        self.role.set(role);
    }
    fn owner(&self, deadline: Instant) -> Result<ProcessIncarnation, ()> {
        let owner = self.launch.owner().ok_or(())?;
        match observe_runtime_process(&SupervisedProbeRunner, owner.pid(), deadline)
            .map_err(|_| ())?
        {
            ProcessObservation::Live(current) if current == owner => Ok(owner),
            _ => Err(()),
        }
    }
    /// Diagnostics select only a pre-existing launch row. An environment value
    /// never creates one or changes a runtime, preference or delivery decision.
    pub fn stage(&self, step: SessionLearningStep, deadline: Instant) -> Result<(), ()> {
        self.diagnostic(Update::Stage(step, None), deadline)
    }
    /// A stage that already knows why it will not go on.
    pub fn stage_detail(
        &self,
        step: SessionLearningStep,
        detail: SessionLearningDetail,
        deadline: Instant,
    ) -> Result<(), ()> {
        self.diagnostic(Update::Stage(step, Some(detail)), deadline)
    }
    /// Names what the current step stopped on, keeping its step and reason.
    pub fn note(&self, detail: SessionLearningDetail, deadline: Instant) -> Result<(), ()> {
        self.diagnostic(Update::Note(detail), deadline)
    }
    pub fn fail(&self, reason: SessionLearningReason, deadline: Instant) -> Result<(), ()> {
        self.diagnostic(Update::Fail(reason), deadline)
    }
    fn diagnostic(&self, update: Update, deadline: Instant) -> Result<(), ()> {
        self.diagnostic_fenced(update, deadline, None)
    }
    fn diagnostic_fenced(
        &self,
        update: Update,
        deadline: Instant,
        event: Option<&tmt_adapters::storage::hook_queue::CaptureEvent>,
    ) -> Result<(), ()> {
        if self.deferred.get() {
            let mut terminal = self.terminal.borrow_mut();
            match update {
                Update::Stage(_, None) => {}
                Update::Stage(step, Some(detail)) => {
                    terminal.step = Some(step.as_str().into());
                    terminal.detail = Some(detail.as_str().into());
                }
                Update::Note(detail) => terminal.detail = Some(detail.as_str().into()),
                Update::Fail(reason) => terminal.failure = Some(reason.as_str().into()),
            }
            return Ok(());
        }
        let owner = self.owner(deadline)?;
        let mut storage = Storage::open_hook(&self.paths.database, deadline).map_err(|_| ())?;
        let operation = |records: &mut dyn BindingRecords<Error = StorageError>| {
            let Some(observed) = records.launch_observation(&self.launch.binding_id, &owner)?
            else {
                return Ok(false);
            };
            if observed.driver != self.driver {
                return Ok(false);
            }
            if observed.status == LaunchStatus::Settled {
                return Ok(observed.diagnostic.is_some());
            }
            // Only a Digest Stop sets or clears its own reason on an admitted launch.
            if observed.status == LaunchStatus::Admitted && digest_stopped(&observed) {
                return Ok(true);
            }

            // A run that cannot bind never overwrites what an earlier run stored for a launch
            // that is not admitted yet: the SessionStart refusal is the one worth reading.
            // Such a run names its own reason only when nothing more specific is stored.
            let keeps = self.role.get() != Role::Binding
                && observed.status != LaunchStatus::Admitted
                && observed.diagnostic.is_some();
            if keeps {
                let named = observed.diagnostic.is_some_and(|d| d.detail.is_some());
                if !matches!(update, Update::Note(_)) || named {
                    return Ok(true);
                }
            }
            let mut next = observed.clone();
            let previous = observed
                .diagnostic
                .unwrap_or(SessionLearningDiagnostic::new(
                    SessionLearningStep::Input,
                    SessionLearningReason::NotAdmitted,
                ));
            next.diagnostic = Some(match update {
                Update::Stage(step, detail) => SessionLearningDiagnostic {
                    step,
                    reason: if observed.status == LaunchStatus::Admitted {
                        SessionLearningReason::InProgress
                    } else {
                        SessionLearningReason::NotAdmitted
                    },
                    detail,
                },
                Update::Note(detail)
                    if detail == NO_ADMITTED_START && observed.status == LaunchStatus::Admitted =>
                {
                    SessionLearningDiagnostic {
                        step: SessionLearningStep::Storage,
                        reason: SessionLearningReason::NotAdmitted,
                        detail: Some(detail),
                    }
                }
                Update::Note(detail) => SessionLearningDiagnostic {
                    detail: Some(detail),
                    ..previous
                },
                Update::Fail(reason) => SessionLearningDiagnostic {
                    reason: if matches!(
                        previous.reason,
                        SessionLearningReason::NotAdmitted | SessionLearningReason::InProgress
                    ) {
                        reason
                    } else {
                        previous.reason
                    },
                    ..previous
                },
            });
            records.set_launch_observation(&observed, &next)
        };
        let result = match event {
            Some(event) => storage.update_hook_capture_learning(event, operation),
            None => storage.with_binding_transaction::<_, StorageError>(operation),
        }
        .map_err(|_| ());
        let closed = storage.close().map_err(|_| ());
        match result {
            Ok(true) => closed,
            _ => Err(()),
        }
    }

    pub fn succeeded(&self, deadline: Instant) -> Result<(), ()> {
        self.succeeded_fenced(deadline, None)
    }
    fn succeeded_fenced(
        &self,
        deadline: Instant,
        event: Option<&tmt_adapters::storage::hook_queue::CaptureEvent>,
    ) -> Result<(), ()> {
        if self.deferred.get() {
            return Ok(());
        }
        let owner = self.owner(deadline)?;
        let mut storage = Storage::open_hook(&self.paths.database, deadline).map_err(|_| ())?;
        let operation = |records: &mut dyn BindingRecords<Error = StorageError>| {
            let Some(observed) = records.launch_observation(&self.launch.binding_id, &owner)?
            else {
                return Ok(false);
            };
            if observed.driver != self.driver {
                return Ok(false);
            }
            if observed.status != LaunchStatus::Admitted
                || observed.diagnostic.is_none()
                || digest_stopped(&observed)
                || no_admitted_start(&observed)
            {
                return Ok(true);
            }
            let mut next = observed.clone();
            next.diagnostic = None;
            records.set_launch_observation(&observed, &next)
        };
        let result = match event {
            Some(event) => storage.update_hook_capture_learning(event, operation),
            None => storage.with_binding_transaction::<_, StorageError>(operation),
        }
        .map_err(|_| ());
        let closed = storage.close().map_err(|_| ());
        match result {
            Ok(true) => closed,
            _ => Err(()),
        }
    }

    /// Names why a Digest Stop on this admitted launch ended with no output. It probes no
    /// process: the refused check may be the owner's own start identity, and a fixed token
    /// on a pre-existing admitted row is the only write.
    pub fn digest_stop(
        &self,
        stop: SessionLearningDigestStop,
        deadline: Instant,
    ) -> Result<(), ()> {
        self.digest_outcome(Some(stop.diagnostic()), deadline)
    }
    /// A delivered Digest Stop clears the reason an earlier one stored.
    pub fn digest_delivered(&self, deadline: Instant) -> Result<(), ()> {
        self.digest_outcome(None, deadline)
    }
    fn digest_outcome(
        &self,
        diagnostic: Option<SessionLearningDiagnostic>,
        deadline: Instant,
    ) -> Result<(), ()> {
        let owner = self.launch.owner().ok_or(())?;
        let mut storage = Storage::open_hook(&self.paths.database, deadline).map_err(|_| ())?;
        let result = storage
            .with_binding_transaction::<_, StorageError>(|records| {
                let Some(observed) = records.launch_observation(&self.launch.binding_id, &owner)?
                else {
                    return Ok(false);
                };
                if observed.driver != self.driver || observed.status != LaunchStatus::Admitted {
                    return Ok(false);
                }
                if diagnostic.is_none() && !digest_stopped(&observed) {
                    return Ok(true);
                }
                let mut next = observed.clone();
                next.diagnostic = diagnostic;
                records.set_launch_observation(&observed, &next)
            })
            .map_err(|_| ());
        let closed = storage.close().map_err(|_| ());
        match result {
            Ok(true) => closed,
            _ => Err(()),
        }
    }

    pub fn ended(
        &self,
        stored: &IdentityContextSnapshot,
        process: &ProcessIncarnation,
        event: &dyn LifecycleObservation,
        deadline: Instant,
    ) -> Result<bool, ()> {
        let binding = stored.entry.binding.as_ref().ok_or(())?;
        let owner = self.owner(deadline)?;
        if binding.id != self.launch.binding_id || binding.identity_id != self.launch.identity_id {
            return Err(());
        }
        let mut storage = Storage::open_hook(&self.paths.database, deadline).map_err(|_| ())?;
        let result = storage
            .with_binding_transaction::<_, StorageError>(|records| {
                if records
                    .entry_by_id(&binding.identity_id)?
                    .and_then(|entry| entry.binding)
                    .as_ref()
                    != Some(binding)
                {
                    return Ok(false);
                }
                let Some(observed) = records.launch_observation(&binding.id, &owner)? else {
                    return Ok(false);
                };
                let Some(pending) = observed.pending.as_ref().filter(|pending| {
                    pending.process == *process && pending.session == *event.session()
                }) else {
                    return Ok(false);
                };
                if observed.status != LaunchStatus::Pending || observed.driver != self.driver {
                    return Ok(false);
                }
                let mut next = observed.clone();
                let mut ended = pending.clone();
                ended.transition = SessionTransition::Ended;
                next.pending = Some(ended);
                next.diagnostic = Some(SessionLearningDiagnostic::new(
                    SessionLearningStep::Storage,
                    SessionLearningReason::LaunchEnded,
                ));
                records.set_launch_observation(&observed, &next)
            })
            .map_err(|_| ());
        let closed = storage.close().map_err(|_| ());
        result.and_then(|changed| closed.map(|()| changed))
    }

    /// Retain the single early start only after the ordinary main-provider
    /// verification. The pending row itself never admits or remembers anything.
    pub fn defer(
        &self,
        stored: &IdentityContextSnapshot,
        process: &ProcessIncarnation,
        host: HostEvidence,
        event: &dyn LifecycleObservation,
        lifecycle: &dyn RuntimeLifecycle,
        deadline: Instant,
    ) -> Result<bool, ()> {
        if !matches!(host, HostEvidence::Independent { .. }) {
            return Ok(false);
        }
        let binding = stored.entry.binding.as_ref().ok_or(())?;
        let owner = self.owner(deadline)?;
        if binding.id != self.launch.binding_id || binding.identity_id != self.launch.identity_id {
            return Err(());
        }
        if binding.session.state == RuntimeState::Running
            && binding.session.launch_owner.as_ref() == Some(&owner)
            && binding
                .session
                .key
                .as_ref()
                .is_some_and(|key| &key.incarnation == process)
        {
            return Ok(false);
        }
        let chain =
            pinned_chain(&SupervisedProbeRunner, process.pid(), deadline).map_err(|_| ())?;
        let provider = chain.position(process.pid()).ok_or(())?;
        let launch = chain.position(owner.pid()).ok_or(())?;
        if provider >= launch
            || chain.links()[provider].process != *process
            || chain.links()[launch].process != owner
        {
            return Err(());
        }
        let mode = lifecycle.mode(host).ok_or(())?;
        let previous = stored
            .preferences
            .remembered
            .as_ref()
            .filter(|r| r.harness == self.driver)
            .and_then(|r| r.state.as_ref());
        let now = tmt_adapters::request_runtime::wall_time_ms();
        let pending = PendingStart {
            process: process.clone(),
            transition: event.transition().ok_or(())?,
            session: event.session().clone(),
            mode,
            state: event.driver_state(previous, now),
            observed_at_ms: now,
        };
        let mut storage = Storage::open_hook(&self.paths.database, deadline).map_err(|_| ())?;
        let result = storage
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
                let Some(observed) = records.launch_observation(&binding.id, &owner)? else {
                    return Ok(false);
                };
                if observed.driver != self.driver
                    || now >= observed.expires_at_ms
                    || !records.launch_preferences_match(&observed)?
                    || !matches!(
                        observed.status,
                        LaunchStatus::Intent | LaunchStatus::Pending
                    )
                {
                    return Ok(false);
                }
                if let Some(previous) = &observed.pending {
                    if previous.process == pending.process
                        && previous.session == pending.session
                        && previous.transition == pending.transition
                    {
                        return Ok(true);
                    }
                    settle(
                        records,
                        &observed,
                        LaunchStatus::Settled,
                        Some(SessionLearningDiagnostic::new(
                            SessionLearningStep::Storage,
                            SessionLearningReason::Conflict,
                        )),
                    )?;
                    return Ok(false);
                }
                let mut next = observed.clone();
                next.status = LaunchStatus::Pending;
                next.pending = Some(pending);
                next.diagnostic = Some(SessionLearningDiagnostic::new(
                    SessionLearningStep::Storage,
                    SessionLearningReason::NotAdmitted,
                ));
                records.set_launch_observation(&observed, &next)
            })
            .map_err(|_| ());
        let closed = storage.close().map_err(|_| ());
        match result {
            Ok(true) => closed.map(|()| true),
            _ => Err(()),
        }
    }
}

fn digest_stopped(observed: &LaunchObservation) -> bool {
    observed
        .diagnostic
        .is_some_and(|d| matches!(d.detail, Some(SessionLearningDetail::DigestStop(_))))
}

const NO_ADMITTED_START: SessionLearningDetail = SessionLearningDetail::Stopped(
    SessionLearningEvent::Stop,
    SessionLearningStop::NoAdmittedStart,
);

fn no_admitted_start(observed: &LaunchObservation) -> bool {
    observed.diagnostic.and_then(|diagnostic| diagnostic.detail) == Some(NO_ADMITTED_START)
}

/// Called only inside the same writer transaction as session admission/settlement.
pub fn settle(
    records: &mut dyn BindingRecords<Error = StorageError>,
    observed: &LaunchObservation,
    status: LaunchStatus,
    diagnostic: Option<SessionLearningDiagnostic>,
) -> Result<(), StorageError> {
    let mut next = observed.clone();
    next.status = status;
    next.pending = None;
    next.diagnostic = diagnostic;
    if records.set_launch_observation(observed, &next)? {
        Ok(())
    } else {
        Err(StorageError::new(
            tmt_adapters::storage::StorageErrorCode::Unknown,
            "The session-learning observation changed while committing admission",
        ))
    }
}

pub fn finish_records(
    records: &mut dyn BindingRecords<Error = StorageError>,
    binding: &str,
    owner: &ProcessIncarnation,
    diagnostic: Option<SessionLearningDiagnostic>,
) -> Result<(), StorageError> {
    if let Some(observed) = records.launch_observation(binding, owner)?
        && observed.status != LaunchStatus::Settled
    {
        let diagnostic = if observed.status == LaunchStatus::Admitted {
            None
        } else {
            diagnostic.or(Some(SessionLearningDiagnostic::new(
                SessionLearningStep::Storage,
                SessionLearningReason::LaunchEnded,
            )))
        };
        settle(records, &observed, LaunchStatus::Settled, diagnostic)?;
    }
    Ok(())
}
pub fn finish_launch(
    storage: &mut Storage,
    binding: &str,
    owner: &ProcessIncarnation,
    diagnostic: Option<SessionLearningDiagnostic>,
) -> Result<(), StorageError> {
    storage.with_binding_transaction(|records| finish_records(records, binding, owner, diagnostic))
}

#[derive(Debug)]
pub struct LearningView {
    pub step: &'static str,
    pub reason: &'static str,
    pub detail: Option<&'static str>,
    pub next_step: String,
    /// Whether the launch is not admitted. An admitted turn/Digest Stop diagnostic is not.
    pub unbound: bool,
}

const DIGEST_STOP_NEXT_STEP: &str = "Nothing to do now; the next turn end tries the Digest again.";
const NO_ADMITTED_START_NEXT_STEP: &str = "Check the provider's start/stop hook ordering.";
pub fn view(
    storage: &Storage,
    binding: Option<&tmt_core::binding::Binding>,
    identity: &tmt_core::identity::Identity,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> Result<Option<LearningView>, StorageError> {
    let Some(binding) = binding else {
        return Ok(None);
    };
    let Some(observed) =
        storage.session_learning(&binding.id, tmt_adapters::request_runtime::wall_time_ms())?
    else {
        return Ok(None);
    };
    Ok(from_observation(observed, identity, preferences))
}

fn from_observation(
    observed: LaunchObservation,
    identity: &tmt_core::identity::Identity,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> Option<LearningView> {
    // An admitted session still shows a dropped final or a Digest Stop's own reason.
    let admitted = observed.status == LaunchStatus::Admitted;
    let missing_start = no_admitted_start(&observed);
    if admitted && !digest_stopped(&observed) && !missing_start {
        return None;
    }
    observed.diagnostic.map(|diagnostic| LearningView {
        step: diagnostic.step.as_str(),
        reason: diagnostic.reason.as_str(),
        detail: diagnostic.detail.map(SessionLearningDetail::as_str),
        next_step: if admitted && missing_start {
            NO_ADMITTED_START_NEXT_STEP.to_owned()
        } else if admitted {
            DIGEST_STOP_NEXT_STEP.to_owned()
        } else {
            tmt_core::binding::session::launch::session_learning_recovery(
                identity,
                &observed.driver,
                preferences,
            )
        },
        unbound: !admitted,
    })
}
pub fn identity_view(
    storage: &Storage,
    identity: &tmt_core::identity::Identity,
    preferences: &tmt_core::binding::session::SessionPreferences,
) -> Result<Option<LearningView>, StorageError> {
    Ok(storage
        .identity_session_learning(&identity.id, tmt_adapters::request_runtime::wall_time_ms())?
        .and_then(|(_, observed)| from_observation(observed, identity, preferences)))
}

impl LearningView {
    /// The one warning a listing or show gives, for one named identity or for its own.
    pub fn warning(&self, name: Option<&str>) -> String {
        let whose = name
            .map(|name| format!(" for '{name}'"))
            .unwrap_or_default();
        if self.unbound {
            format!(
                "The provider session{whose} was not recorded ({}). Resume may be unavailable.",
                self.cause()
            )
        } else if self.detail == Some(NO_ADMITTED_START.as_str()) {
            format!(
                "The last Stop{whose} had no admitted start ({}).",
                self.cause()
            )
        } else {
            format!(
                "The last Digest Stop{whose} ended without the Digest ({}).",
                self.cause()
            )
        }
    }
    /// `step: reason`, with what the step stopped on when it is known.
    pub fn cause(&self) -> String {
        match self.detail {
            Some(detail) => format!("{}: {}, {detail}", self.step, self.reason),
            None => format!("{}: {}", self.step, self.reason),
        }
    }
}

pub fn document(view: &LearningView) -> serde_json::Value {
    let mut document =
        serde_json::json!({"step":view.step,"reason":view.reason,"nextStep":view.next_step});
    if let Some(detail) = view.detail {
        document["detail"] = detail.into();
    }
    document
}

#[cfg(test)]
pub(crate) mod tests;
