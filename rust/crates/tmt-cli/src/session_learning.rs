//! Invocation-local coordination of stored startup evidence; no delivery authority.
use std::time::Instant;
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
                LaunchObservation, LaunchStatus, PendingStart, SessionLearningDiagnostic,
                SessionLearningReason, SessionLearningStep,
            },
        },
    },
    endpoint::ProcessIncarnation,
};

pub const ENVIRONMENT: &str = "TMT_LAUNCH_OBSERVATION";
pub struct Learning {
    pub launch: HookLaunch,
    pub driver: HarnessId,
    paths: ConfigPaths,
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
        })
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
        self.diagnostic(Some(step), None, deadline)
    }
    pub fn fail(&self, reason: SessionLearningReason, deadline: Instant) -> Result<(), ()> {
        self.diagnostic(None, Some(reason), deadline)
    }
    fn diagnostic(
        &self,
        step: Option<SessionLearningStep>,
        reason: Option<SessionLearningReason>,
        deadline: Instant,
    ) -> Result<(), ()> {
        let owner = self.owner(deadline)?;
        let mut storage = Storage::open_hook(&self.paths.database, deadline).map_err(|_| ())?;
        let result = storage
            .with_binding_transaction::<_, StorageError>(|records| {
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

                let mut next = observed.clone();
                let previous = observed.diagnostic.unwrap_or(SessionLearningDiagnostic {
                    step: SessionLearningStep::Input,
                    reason: SessionLearningReason::NotAdmitted,
                });
                next.diagnostic = Some(SessionLearningDiagnostic {
                    step: step.unwrap_or(previous.step),
                    reason: if step.is_some() {
                        if observed.status == LaunchStatus::Admitted {
                            SessionLearningReason::InProgress
                        } else {
                            SessionLearningReason::NotAdmitted
                        }
                    } else if matches!(
                        previous.reason,
                        SessionLearningReason::NotAdmitted | SessionLearningReason::InProgress
                    ) {
                        reason.unwrap_or(previous.reason)
                    } else {
                        previous.reason
                    },
                });
                records.set_launch_observation(&observed, &next)
            })
            .map_err(|_| ());
        let closed = storage.close().map_err(|_| ());
        match result {
            Ok(true) => closed,
            _ => Err(()),
        }
    }

    pub fn succeeded(&self, deadline: Instant) -> Result<(), ()> {
        let owner = self.owner(deadline)?;
        let mut storage = Storage::open_hook(&self.paths.database, deadline).map_err(|_| ())?;
        let result = storage
            .with_binding_transaction::<_, StorageError>(|records| {
                let Some(observed) = records.launch_observation(&self.launch.binding_id, &owner)?
                else {
                    return Ok(false);
                };
                if observed.driver != self.driver {
                    return Ok(false);
                }
                if observed.status != LaunchStatus::Admitted || observed.diagnostic.is_none() {
                    return Ok(true);
                }
                let mut next = observed.clone();
                next.diagnostic = None;
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
                next.diagnostic = Some(SessionLearningDiagnostic {
                    step: SessionLearningStep::Storage,
                    reason: SessionLearningReason::LaunchEnded,
                });
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
                        Some(SessionLearningDiagnostic {
                            step: SessionLearningStep::Storage,
                            reason: SessionLearningReason::Conflict,
                        }),
                    )?;
                    return Ok(false);
                }
                let mut next = observed.clone();
                next.status = LaunchStatus::Pending;
                next.pending = Some(pending);
                next.diagnostic = Some(SessionLearningDiagnostic {
                    step: SessionLearningStep::Storage,
                    reason: SessionLearningReason::NotAdmitted,
                });
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
            diagnostic.or(Some(SessionLearningDiagnostic {
                step: SessionLearningStep::Storage,
                reason: SessionLearningReason::LaunchEnded,
            }))
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
    pub next_step: String,
}
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
    if observed.status == LaunchStatus::Admitted {
        return None;
    }
    observed.diagnostic.map(|diagnostic| LearningView {
        step: diagnostic.step.as_str(),
        reason: diagnostic.reason.as_str(),
        next_step: tmt_core::binding::session::launch::session_learning_recovery(
            identity,
            &observed.driver,
            preferences,
        ),
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

pub fn document(view: &LearningView) -> serde_json::Value {
    serde_json::json!({"step":view.step,"reason":view.reason,"nextStep":view.next_step})
}

#[cfg(test)]
mod tests;
