//! One foreground admission transaction, shared by the launcher and its race controls.
use super::channel::{admitted_key, claim_is_current};
use tmt_adapters::{
    process::runtime::ProcessObservation,
    runtime::lifecycle::RuntimeLifecycle,
    storage::{Storage, StorageError},
};
use tmt_core::{
    binding::{
        Binding, BindingRepository,
        session::{
            HarnessId, ObservedSessionKey, RememberedSession, RuntimeLiveness, RuntimeState,
            SessionPreferences, SessionTransition,
        },
    },
    endpoint::ProcessIncarnation,
};

pub(crate) struct ForegroundAdmission<'a> {
    pub binding: &'a Binding,
    pub owner: Option<&'a ProcessIncarnation>,
    pub child: &'a ProcessObservation,
    pub driver: Option<&'a HarnessId>,
    pub resumed: Option<&'a RememberedSession>,
    pub lifecycle: &'a dyn RuntimeLifecycle,
    pub preferences: &'a SessionPreferences,
    pub lease_session: Option<&'a tmt_core::binding::session::ProviderSessionId>,
    pub channel: Option<bool>,
    pub can_admit: bool,
}

/// The caller supplies the owned child's native evidence. No host/process probe
/// or callback runs while this writer is held; a conflict never replays a launch.
pub(crate) fn admit_foreground(
    storage: &mut Storage,
    input: ForegroundAdmission<'_>,
) -> Result<Option<(ObservedSessionKey, RuntimeState)>, StorageError> {
    let binding = input.binding;
    let owner = input.owner.cloned();
    let claim = input.driver.cloned();
    let lifecycle = input.lifecycle;
    let preferences = input.preferences;
    let can_admit = input.can_admit;
    let child_incarnation = match input.child {
        ProcessObservation::Live(child)
        | ProcessObservation::Stopped(child)
        | ProcessObservation::UnreapedZombie(child) => Some(child.clone()),
        _ => None,
    };
    let already_exited = matches!(input.child, ProcessObservation::UnreapedZombie(_));
    let already_stopped = matches!(input.child, ProcessObservation::Stopped(_));
    storage.with_binding_transaction::<_, StorageError>(|records| {
        let Some(current) = records
            .entry_by_id(&binding.identity_id)?
            .and_then(|entry| entry.binding)
            .filter(|current| current.id == binding.id)
        else {
            return Ok(None);
        };
        if !claim_is_current(&current, binding)
            && !current
                .session
                .key
                .as_ref()
                .is_some_and(|key| Some(&key.incarnation) == child_incarnation.as_ref())
        {
            return Ok(None);
        }
        let learning = match &owner {
            Some(owner) => records.launch_observation(&binding.id, owner)?,
            None => None,
        };
        if learning.as_ref().is_some_and(|observation| {
            observation.status == tmt_core::binding::session::launch::LaunchStatus::Settled
                && observation.diagnostic.is_some_and(|diagnostic| {
                    diagnostic.reason
                        == tmt_core::binding::session::launch::SessionLearningReason::Conflict
                })
        }) {
            return Ok(None);
        }
        let pending = match (&learning, &owner, &child_incarnation, &claim) {
            (Some(observed), Some(owner), Some(child), Some(driver))
                if records.launch_preferences_match(observed)? =>
            {
                observed
                    .pending_for(
                        owner,
                        child,
                        driver,
                        tmt_adapters::request_runtime::wall_time_ms(),
                    )
                    .cloned()
            }
            _ => None,
        };
        let (true, Some(owner), Some(child)) = (can_admit, &owner, &child_incarnation) else {
            remember_driver(records, &binding.identity_id, claim.as_ref())?;
            return Ok(None);
        };
        // A hook may have attached its provider session before this probe.
        // Exact resume coordinates came from the driver-owned remembered
        // record, a channel launch's from its lease: never from user argv or
        // a hook pane. A session conflicting with hook evidence is not admitted.
        let Some(mut key) = admitted_key(
            current.session.key.as_ref(),
            child,
            input
                .resumed
                .as_ref()
                .filter(|session| Some(&session.harness) == claim.as_ref())
                .map(|session| session.provider_session.clone()),
            input.lease_session,
        ) else {
            remember_driver(records, &binding.identity_id, claim.as_ref())?;
            return Ok(None);
        };
        if let Some(observed) = &learning
            && observed.pending.is_some()
            && pending.as_ref().is_none_or(|pending| {
                key.provider_session
                    .as_ref()
                    .is_some_and(|session| session != &pending.session)
            })
        {
            crate::session_learning::settle(
                records,
                observed,
                tmt_core::binding::session::launch::LaunchStatus::Settled,
                Some(
                    tmt_core::binding::session::launch::SessionLearningDiagnostic {
                        step: tmt_core::binding::session::launch::SessionLearningStep::Storage,
                        reason: tmt_core::binding::session::launch::SessionLearningReason::Conflict,
                    },
                ),
            )?;
            return Ok(None);
        }
        let pending = pending.filter(|_| !already_exited && !already_stopped);
        if let Some(pending) = &pending {
            key.provider_session = Some(pending.session.clone());
        }
        let next = if already_exited {
            lifecycle.client_exit(&current.session, key, owner.clone(), preferences)
        } else if already_stopped {
            // Keep exact ownership for duplicate refusal, without treating
            // a stopped child as live evidence or authorizing delivery.
            current.session.record_stopped_launch(
                key,
                owner.clone(),
                if input.resumed.is_some() {
                    SessionTransition::Resumed
                } else {
                    SessionTransition::Started
                },
            )
        } else {
            current.session.admit_launched(
                key,
                owner.clone(),
                if input.resumed.is_some() {
                    SessionTransition::Resumed
                } else {
                    SessionTransition::Started
                },
                RuntimeLiveness::Alive,
                RuntimeLiveness::Alive,
            )
        };
        let Some(mut next) = next else {
            if pending.is_none() {
                remember_driver(records, &binding.identity_id, claim.as_ref())?;
            }
            return Ok(None);
        };
        if pending
            .as_ref()
            .is_some_and(|pending| pending.transition == SessionTransition::Ended)
        {
            let Some(ended) = next
                .key
                .as_ref()
                .and_then(|key| next.transition(key, SessionTransition::Ended, None))
            else {
                return Ok(None);
            };
            next = ended;
        }
        if !records.set_session_state(&binding.id, &current.session, &next)? {
            if pending.is_none() {
                remember_driver(records, &binding.identity_id, claim.as_ref())?;
            }
            return Ok(None);
        }
        remember_driver(records, &binding.identity_id, claim.as_ref())?;
        if let (Some(pending), Some(driver)) = (&pending, &claim) {
            let mut preferences = records.session_preferences(&binding.identity_id)?;
            preferences.remember(
                driver.clone(),
                pending.mode.clone(),
                pending.session.clone(),
            );
            if let Some(remembered) = preferences.remembered.as_mut() {
                remembered.state = pending.state.clone();
            }
            if !records.set_session_preferences(&binding.identity_id, &preferences)? {
                return Err(conflict());
            }
        }
        if let Some(observed) = &learning {
            use tmt_core::binding::session::launch::{
                LaunchStatus, SessionLearningDiagnostic, SessionLearningReason, SessionLearningStep,
            };
            let learned = next
                .key
                .as_ref()
                .and_then(|key| key.provider_session.as_ref())
                .is_some();
            if learned {
                crate::session_learning::settle(records, observed, LaunchStatus::Admitted, None)?;
            } else if already_exited || already_stopped || observed.pending.is_some() {
                crate::session_learning::settle(
                    records,
                    observed,
                    LaunchStatus::Settled,
                    Some(SessionLearningDiagnostic {
                        step: SessionLearningStep::Storage,
                        reason: if already_exited {
                            SessionLearningReason::LaunchEnded
                        } else {
                            SessionLearningReason::Conflict
                        },
                    }),
                )?;
            }
        }
        if let Some(channel) = input.channel.filter(|_| claim.is_some()) {
            let mut preferences = records.session_preferences(&binding.identity_id)?;
            preferences.channel = Some(channel);
            records.set_session_preferences(&binding.identity_id, &preferences)?;
        }
        Ok(next.key.map(|key| (key, next.state)))
    })
}

fn conflict() -> StorageError {
    StorageError::new(
        tmt_adapters::storage::StorageErrorCode::Unknown,
        "The launch admission changed while committing its session",
    )
}
/// A confirmed spawn retains its selected driver even when ordinary admission
/// is unavailable; a conflicting provisional observation never changes it.
fn remember_driver(
    records: &mut dyn tmt_core::binding::BindingRecords<Error = StorageError>,
    identity: &str,
    driver: Option<&HarnessId>,
) -> Result<(), StorageError> {
    if let Some(driver) = driver {
        let mut preferences = records.session_preferences(identity)?;
        preferences.launched(driver);
        if !records.set_session_preferences(identity, &preferences)? {
            return Err(conflict());
        }
    }
    Ok(())
}
