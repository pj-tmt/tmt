//! Final capture owns its activity transition. Optional usage writers cannot
//! invalidate it or have their newer observations overwritten by an old snapshot.
use super::super::hook_queue::CaptureResult;
use super::super::{
    IdentityContextSnapshot, Storage, StorageError,
    bindings::BindingRows,
    identities::{commit_immediate_transaction, with_immediate_transaction},
};
use crate::runtime::{driver_state, hook_protocol::ActivityObservation};
use rusqlite::{Connection, OptionalExtension, params};
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
    /// An ordinary Stop offers no new final in its exact recorded batch.
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
        event_id: &str,
        recorded_batch: Option<&str>,
        capture: TurnCapture<'_>,
    ) -> Result<TurnCaptureOutcome, StorageError> {
        let Some(binding) = capture.expected.entry.binding.as_ref() else {
            return Ok(TurnCaptureOutcome::NotApplicable);
        };
        if Instant::now() >= capture.deadline {
            return Ok(TurnCaptureOutcome::NotApplicable);
        }
        if !matches!(
            capture.activity.event.kind,
            EventKind::TurnEnded | EventKind::ContinuationEnded
        ) || binding.session.state != RuntimeState::Running
        {
            return Ok(TurnCaptureOutcome::NotApplicable);
        }
        with_immediate_transaction(self, "capture turn final", |db| {
            if let Some(previous) = capture_receipt(db, event_id)? {
                return Ok(match previous {
                    CaptureResult::Committed => TurnCaptureOutcome::Committed,
                    CaptureResult::Duplicate => TurnCaptureOutcome::Duplicate,
                    CaptureResult::NoAdmittedStart => TurnCaptureOutcome::NoAdmittedStart,
                    _ => TurnCaptureOutcome::NotApplicable,
                });
            }
            let outcome = commit_turn_end(db, &capture, recorded_batch)?;
            let remembered = capture.expected.preferences.remembered.as_ref();
            finish_capture_disposition(
                db,
                &CaptureDisposition {
                    event_id,
                    identity: &binding.identity_id,
                    binding: &binding.id,
                    batch: recorded_batch,
                    provider: remembered.map(|r| r.harness.as_str()),
                    session: remembered.map(|r| r.provider_session.as_str()),
                    process: capture.process,
                    response: capture.response.as_ref(),
                    interrupted: capture.interrupted,
                    now_ms: capture.now_ms,
                    outcome: CaptureResult::from(outcome),
                },
            )?;
            if Instant::now() >= capture.deadline {
                return Err(StorageError::new(
                    super::super::StorageErrorCode::Busy,
                    "Capture disposition exceeded its hook budget",
                ));
            }
            Ok(outcome)
        })
    }
}

fn commit_turn_end(
    db: &Connection,
    capture: &TurnCapture<'_>,
    recorded_batch: Option<&str>,
) -> Result<TurnCaptureOutcome, StorageError> {
    let binding = capture
        .expected
        .entry
        .binding
        .as_ref()
        .expect("verified binding");

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
    let next = driver_state::after_activity(
        remembered.state.as_ref(),
        &remembered.provider_session,
        capture.process,
        &capture.activity.event,
        capture.activity.model.as_deref(),
        capture.now_ms,
    );
    let batch = super::super::capture_batches::current(db, &binding.identity_id)?;
    if recorded_batch.is_some_and(|recorded| batch.as_deref() != Some(recorded)) {
        return Ok(TurnCaptureOutcome::NotApplicable);
    }
    if next.is_none() {
        // Idle activity alone cannot distinguish a retried Stop from
        // a later final whose prompt boundary was missed. Keep its
        // verified text and compare complete identity under this writer.
        let previous = remembered
            .state
            .as_ref()
            .and_then(driver_state::state_activity)
            .filter(|activity| activity.matches(&remembered.provider_session, capture.process));
        let idle_end = capture.activity.event.kind == EventKind::TurnEnded
            && previous.as_ref().is_some_and(|activity| {
                activity.phase == tmt_core::binding::session::activity::ActivityPhase::Idle
                    && activity.turn == capture.activity.event.turn
                    && capture.now_ms >= activity.last_activity_ms
            });
        if !idle_end || capture.interrupted || capture.response.is_none() || batch.is_none() {
            return Ok(if idle_end {
                TurnCaptureOutcome::Duplicate
            } else if !capture.interrupted
                && previous
                    .as_ref()
                    .is_none_or(|activity| activity.turn != capture.activity.event.turn)
            {
                TurnCaptureOutcome::NoAdmittedStart
            } else {
                TurnCaptureOutcome::NotApplicable
            });
        }
    }
    if capture.activity.event.kind == EventKind::TurnEnded
        && let Some(response) = capture.response.as_ref()
        && super::same_final_in_batch(db, batch.as_deref(), response)?
    {
        return Ok(TurnCaptureOutcome::Duplicate);
    }
    let driver = remembered.harness.as_str().to_owned();
    let session = remembered.provider_session.as_str().to_owned();
    if let Some(next) = next {
        current.remembered.as_mut().unwrap().state = Some(next);
    }
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
        super::super::requests::end_digest_continuation(db, &binding.identity_id, capture.now_ms)?;
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
        commit_immediate_transaction(self, "capture failure", |db| {
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

/// A verified capture's recorded coordinates, never a request to alter current activity.
pub(in crate::storage) struct CaptureDisposition<'a> {
    pub event_id: &'a str,
    pub identity: &'a str,
    pub binding: &'a str,
    pub batch: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub session: Option<&'a str>,
    pub process: &'a ProcessIncarnation,
    pub response: Option<&'a super::ResponseCapture>,
    pub interrupted: bool,
    pub now_ms: u64,
    pub outcome: CaptureResult,
}

pub(in crate::storage) fn capture_receipt(
    db: &Connection,
    event_id: &str,
) -> Result<Option<CaptureResult>, StorageError> {
    let outcome: Option<String> = db
        .query_row(
            "SELECT outcome FROM hook_capture_receipts WHERE event_id=?",
            [event_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| super::super::classify(e, "Read capture disposition receipt"))?;
    outcome
        .map(|outcome| {
            serde_json::from_value(serde_json::Value::String(outcome)).map_err(|_| {
                StorageError::new(
                    super::super::StorageErrorCode::Corrupt,
                    "Invalid capture receipt",
                )
            })
        })
        .transpose()
}

/// Called inside the capture writer on both paths. A receipt fences each event,
/// while the old recorded batch and exact binding fence where a miss can appear.
pub(in crate::storage) fn finish_capture_disposition(
    db: &Connection,
    capture: &CaptureDisposition<'_>,
) -> Result<(), StorageError> {
    if capture_receipt(db, capture.event_id)?.is_some() {
        return Ok(());
    }
    if matches!(
        capture.outcome,
        CaptureResult::NoAdmittedStart | CaptureResult::NotApplicable
    ) && !capture.interrupted
        && let (Some(batch), Some(response)) = (capture.batch, capture.response)
    {
        let recorded: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM capture_batches b JOIN identities i ON i.id=b.identity_id AND i.retired_at_ms IS NULL JOIN bindings current ON current.identity_id=i.id AND current.id=b.binding_id
             WHERE b.id=?1 AND b.identity_id=?2 AND b.binding_id=?3 AND b.driver=?4 AND b.session_id=?5 AND b.runtime_pid=?6 AND b.runtime_start=?7)",
            params![batch,capture.identity,capture.binding,capture.provider,capture.session,capture.process.pid() as i64,capture.process.start_identity()],
            |row| row.get(0),
        ).map_err(|e| super::super::classify(e, "Check refused capture's recorded batch"))?;
        if recorded && !super::same_final_in_batch(db, Some(batch), response)? {
            db.execute("INSERT INTO capture_entries(identity_id,batch_id,kind,failure,created_ms) VALUES(?1,?2,'failure',?3,?4)",
                params![capture.identity,batch,CaptureFailure::Worker.as_str(),capture.now_ms as i64])
                .map_err(|e| super::super::classify(e, "Record refused final capture"))?;
        }
    }
    db.execute("INSERT INTO hook_capture_receipts(event_id,identity_id,batch_id,outcome,committed_ms) VALUES(?,?,?,?,?)",
        params![capture.event_id,capture.identity,capture.batch,capture.outcome.token(),crate::request_runtime::wall_time_ms() as i64])
        .map_err(|e| super::super::classify(e, "Commit capture disposition receipt"))?;
    Ok(())
}
