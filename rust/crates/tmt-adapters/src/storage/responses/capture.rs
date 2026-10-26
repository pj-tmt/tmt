//! Final capture owns its activity transition. Optional usage writers cannot
//! invalidate it or have their newer observations overwritten by an old snapshot.
use super::super::{
    IdentityContextSnapshot, Storage, StorageError, bindings::BindingRows,
    identities::with_immediate_transaction,
};
use crate::runtime::{driver_state, hook_protocol::ActivityObservation};
use std::time::Instant;
use tmt_core::{
    binding::{
        BindingRecords,
        session::{RuntimeState, activity::EventKind},
    },
    endpoint::ProcessIncarnation,
};

#[derive(Clone, Copy)]
pub enum CaptureFailure {
    Worker,
    Budget,
    Storage,
}
impl CaptureFailure {
    fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker_error",
            Self::Budget => "budget_timeout",
            Self::Storage => "storage_error",
        }
    }
}

pub struct TurnCapture<'a> {
    pub expected: &'a IdentityContextSnapshot,
    pub process: &'a ProcessIncarnation,
    pub activity: &'a ActivityObservation,
    pub response: Option<super::ResponseCapture>,
    pub interrupted: bool,
    pub now_ms: u64,
    pub deadline: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCaptureOutcome {
    Committed,
    /// An ordinary Stop for this exact idle turn was already committed.
    Duplicate,
    /// The deadline, authority or activity transition does not permit a commit.
    NotApplicable,
    /// A non-interrupted end has no start in its exact session/process/turn.
    NoAdmittedStart,
}

impl Storage {
    /// The verified authority is rechecked under the writer. Activity is
    /// recomputed from current state, independently of optional usage changes.
    /// A duplicate ordinary Stop is already settled; a stale authority refuses.
    pub fn capture_turn_end(
        &mut self,
        capture: TurnCapture<'_>,
    ) -> Result<TurnCaptureOutcome, StorageError> {
        let Some(binding) = capture.expected.entry.binding.as_ref() else {
            return Ok(TurnCaptureOutcome::NotApplicable);
        };
        if !matches!(
            capture.activity.event.kind,
            EventKind::TurnEnded | EventKind::ContinuationEnded
        ) || binding.session.state != RuntimeState::Running
        {
            return Ok(TurnCaptureOutcome::NotApplicable);
        }
        with_immediate_transaction(self, "capture turn final", |db| {
            if Instant::now() >= capture.deadline {
                return Ok(TurnCaptureOutcome::NotApplicable);
            }
            let mut records = BindingRows(db);
            if records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding)
                .as_ref()
                != Some(binding)
            {
                return Ok(TurnCaptureOutcome::NotApplicable);
            }
            let mut current = records.session_preferences(&binding.identity_id)?;
            let Some(remembered) = current.remembered.as_ref() else {
                return Ok(TurnCaptureOutcome::NotApplicable);
            };
            let Some(expected) = capture.expected.preferences.remembered.as_ref() else {
                return Ok(TurnCaptureOutcome::NotApplicable);
            };
            let mut selection = current.clone();
            selection.remembered.as_mut().unwrap().state = expected.state.clone();
            if selection != capture.expected.preferences
                || !binding.session.key.as_ref().is_some_and(|key| {
                    &key.incarnation == capture.process
                        && key.provider_session.as_ref() == Some(&remembered.provider_session)
                })
            {
                return Ok(TurnCaptureOutcome::NotApplicable);
            }
            let Some(next) = driver_state::after_activity(
                remembered.state.as_ref(),
                &remembered.provider_session,
                capture.process,
                &capture.activity.event,
                capture.activity.model.as_deref(),
                capture.now_ms,
            ) else {
                // No writer repeats a normal end that another observer has
                // already committed for this exact active session/process.
                let previous = remembered
                    .state
                    .as_ref()
                    .and_then(driver_state::state_activity)
                    .filter(|activity| {
                        activity.matches(&remembered.provider_session, capture.process)
                    });
                return Ok(
                    if capture.activity.event.kind == EventKind::TurnEnded
                        && previous.as_ref().is_some_and(|activity| {
                            activity.phase
                                == tmt_core::binding::session::activity::ActivityPhase::Idle
                                && activity.turn == capture.activity.event.turn
                        })
                    {
                        TurnCaptureOutcome::Duplicate
                    } else if !capture.interrupted
                        && previous
                            .as_ref()
                            .is_none_or(|activity| activity.turn != capture.activity.event.turn)
                    {
                        TurnCaptureOutcome::NoAdmittedStart
                    } else {
                        TurnCaptureOutcome::NotApplicable
                    },
                );
            };
            let driver = remembered.harness.as_str().to_owned();
            let session = remembered.provider_session.as_str().to_owned();
            current.remembered.as_mut().unwrap().state = Some(next);
            if !records.set_session_preferences(&binding.identity_id, &current)? {
                return Ok(TurnCaptureOutcome::NotApplicable);
            }
            super::super::capture_batches::ensure(
                db,
                binding,
                current.remembered.as_ref().unwrap(),
                capture
                    .activity
                    .event
                    .turn
                    .as_ref()
                    .map(|turn| turn.as_str()),
                capture.now_ms,
            )?;
            let retained = capture
                .response
                .as_ref()
                .filter(|_| !capture.interrupted)
                .map(|response| {
                    super::append(
                        db,
                        &binding.identity_id,
                        &driver,
                        &session,
                        capture.now_ms,
                        response,
                    )
                })
                .transpose()?;
            let turn = capture
                .activity
                .event
                .turn
                .as_ref()
                .map(|turn| turn.as_str());
            if capture.activity.event.kind == EventKind::ContinuationEnded {
                super::super::requests::end_digest_continuation(
                    db,
                    &binding.identity_id,
                    capture.now_ms,
                )?;
            }
            super::super::requests::turns::close(db, &binding.identity_id, capture.now_ms)?;
            super::super::requests::passthrough::settle_turn(
                db,
                &binding.identity_id,
                &session,
                turn,
                retained
                    .zip(capture.response.as_ref())
                    .map(
                        |(response, text)| super::super::requests::passthrough::SettledFinal {
                            text: &text.text,
                            provider: &driver,
                            response,
                        },
                    ),
                capture.now_ms,
            )?;
            if capture.interrupted {
                super::super::capture_batches::close(db, &binding.identity_id, capture.now_ms)?;
            }
            if Instant::now() >= capture.deadline {
                return Err(super::super::StorageError::new(
                    super::super::StorageErrorCode::Busy,
                    "Final capture exceeded its hook budget",
                ));
            }
            Ok(TurnCaptureOutcome::Committed)
        })
    }
}

impl Storage {
    /// Best-effort diagnostics never renew the hook budget, replay capture or
    /// attach a failure to a binding/session that replaced the verified one.
    pub fn record_capture_failure(
        &mut self,
        expected: &IdentityContextSnapshot,
        reason: CaptureFailure,
        now: u64,
        deadline: Instant,
    ) -> Result<(), StorageError> {
        let Some(binding) = expected.entry.binding.as_ref() else {
            return Ok(());
        };
        with_immediate_transaction(self, "capture failure", |db| {
            if Instant::now() >= deadline {
                return Ok(());
            }
            let rows = BindingRows(db);
            if rows
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding)
                .as_ref()
                != Some(binding)
            {
                return Ok(());
            }
            let current = rows.session_preferences(&binding.identity_id)?;
            let pair = |preferences: &tmt_core::binding::session::SessionPreferences| {
                preferences
                    .remembered
                    .as_ref()
                    .map(|r| (r.harness.clone(), r.provider_session.clone()))
            };
            if pair(&current) != pair(&expected.preferences) {
                return Ok(());
            }
            if let Some(batch) = super::super::capture_batches::current(db, &binding.identity_id)? {
                db.execute("INSERT INTO capture_entries(identity_id,batch_id,kind,failure,created_ms)
                    SELECT ?1,?2,'failure',?3,?4 WHERE NOT EXISTS(SELECT 1 FROM capture_entries WHERE batch_id=?2 AND kind='failure')",
                    rusqlite::params![binding.identity_id,batch,reason.as_str(),now as i64])
                    .map_err(|e|super::super::classify(e,"Record capture failure"))?;
            }
            if Instant::now() >= deadline {
                return Err(StorageError::new(
                    super::super::StorageErrorCode::Busy,
                    "Capture diagnostic exceeded its budget",
                ));
            }
            Ok(())
        })
    }
}
