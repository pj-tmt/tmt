//! Job admission, canonical inbox acceptance and run evidence share one commit.
use super::*;
use crate::orchestration::OrchZone;
use tmt_core::{
    dispatch::{DispatchInput, DispatchReceipt, DispatchRoom},
    orchestration::{
        OrchDecision, OrchDecisionAction, OrchDecisionOutcome, OrchDecisionSource, OrchPolicyField,
        OrchSubjectKind, effective,
    },
    request::{Originator, RequestKind, RequestOrigin, digest::DeliveryMode},
    retention::valid_retention_days,
};

pub struct JobRunInput {
    pub job_id: String,
    pub expected_revision: u64,
    /// The observed slot fences repeated calls, including a held-slot catch-up.
    pub slot_ms: i64,
    /// Explicit send has an action UUID and never changes pause or catch-up.
    pub manual_operation_id: Option<String>,
    pub originator: Originator,
}
#[derive(Debug)]
pub enum JobRunResult {
    NoOwner,
    Paused,
    Held {
        slot: HeldJobSlot,
        journal_id: Option<u64>,
    },
    Accepted {
        receipt: DispatchReceipt,
        created: bool,
        run_id: u64,
    },
}
fn selected_zone(db: &Connection) -> Result<Option<String>, StorageError> {
    db.query_row(
        "SELECT time_zone FROM orchestration_settings WHERE id=1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| classify(error, "Read job policy zone"))
    .map(Option::flatten)
}
fn existing_run(
    db: &Connection,
    input: &JobRunInput,
    job: &OrchJob,
) -> Result<Option<JobRunResult>, JobStoreError> {
    let row = if let Some(operation) = &input.manual_operation_id {
        db.query_row("SELECT id,job_id,revision,operation_id FROM orchestration_job_runs WHERE operation_id=?",[operation],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?,row.get::<_,String>(3)?))).optional()
    } else {
        db.query_row("SELECT id,job_id,revision,operation_id FROM orchestration_job_runs WHERE job_id=? AND observed_slot_ms=? AND manual=0",params![input.job_id,input.slot_ms],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?,row.get::<_,String>(3)?))).optional()
    }.map_err(|error|classify(error,"Recover job run"))?;
    row.map(|(id, job_id, revision, operation)| {
        if job_id != input.job_id || unsigned(revision)? != input.expected_revision {
            return Err(JobError::Conflict.into());
        }
        let (digest, receipt) = crate::storage::dispatch::read_dispatch_operation(db, &operation)?
            .ok_or_else(corrupt)?;
        if digest != crate::dispatch::intent_digest(&job_dispatch(job, input, operation)?) {
            return Err(crate::storage::DispatchError::IdempotencyConflict.into());
        }
        Ok(JobRunResult::Accepted {
            receipt,
            created: false,
            run_id: unsigned(id)?,
        })
    })
    .transpose()
}
fn job_dispatch(
    job: &OrchJob,
    input: &JobRunInput,
    operation_id: String,
) -> Result<DispatchInput, JobError> {
    let owner = job.owner_id.as_ref().ok_or(JobError::NoOwner)?;
    Ok(DispatchInput {
        originator: input.originator.clone(),
        kind: RequestKind::Request,
        origin: RequestOrigin::Automation,
        operation_id,
        recipient_ids: vec![owner.clone()],
        message: job.message.clone(),
        room: job.room_id.as_ref().map(|id| DispatchRoom::Direct {
            room_id: id.clone(),
        }),
        mode: DeliveryMode::Request,
    })
}
impl Storage {
    /// The scheduler supplies an observed due slot; it never reconstructs old
    /// slots after absence. This owner fences current policy/definition and
    /// joins canonical dispatch. A failed commit cannot leave a run or request.
    pub fn run_orchestration_job(
        &mut self,
        input: JobRunInput,
        retention_days: u64,
        clock: impl Fn() -> u64,
    ) -> Result<JobRunResult, JobStoreError> {
        if !canonical_id(&input.job_id)
            || input.expected_revision == 0
            || input.expected_revision > MAX_JS_SAFE_INTEGER
            || input.slot_ms.unsigned_abs() > MAX_JS_SAFE_INTEGER
            || input
                .manual_operation_id
                .as_deref()
                .is_some_and(|id| !canonical_id(id))
            || !valid_retention_days(retention_days)
        {
            return Err(JobError::Invalid.into());
        }
        let configured_zone = selected_zone(self.connection()?)?;
        let zone = OrchZone::load(configured_zone.as_deref()).map_err(|_| JobError::Invalid)?;
        let receiving = super::super::ReceivingCalendar::load(self.connection()?)?;
        let tx = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Begin job admission"))?;
        let job = read_job(&tx, &input.job_id)?.ok_or(JobError::NotFound)?;
        if job.revision != input.expected_revision {
            return Err(JobError::Conflict.into());
        }
        if let Some(result) = existing_run(&tx, &input, &job)? {
            return Ok(result);
        }
        let Some(owner) = job.owner_id.as_deref() else {
            return Ok(JobRunResult::NoOwner);
        };
        if !owner_exists(&tx, Some(owner))? {
            return Ok(JobRunResult::NoOwner);
        }
        if let Some(room_id) = &job.room_id {
            let room = crate::storage::room::read_room(&tx, room_id)?;
            if room.is_none_or(|room| !room.member_ids.iter().any(|id| id == owner)) {
                return Ok(JobRunResult::NoOwner);
            }
        }
        let now = clock();
        if now == 0 || now > MAX_JS_SAFE_INTEGER || i128::from(input.slot_ms) > i128::from(now) {
            return Err(JobError::Invalid.into());
        }
        let state = super::super::read_snapshot(&tx, None, Some(owner))?;
        if state.time_zone != configured_zone {
            return Err(JobError::Conflict.into());
        }
        let policy = effective(&state.snapshot, owner, now, &zone).map_err(|_| corrupt())?;
        let decision = admit_job(
            &job,
            &JobAdmission {
                expected_revision: input.expected_revision,
                owner_active: true,
                routine: policy.routine.value,
                slot_ms: input.slot_ms,
                manual: input.manual_operation_id.is_some(),
            },
        )?;
        let mut event = OrchDecision {
            recorded_at_ms: now,
            subject_kind: OrchSubjectKind::Job,
            subject_id: job.id.clone(),
            actor_id: input.originator.identity_id().map(str::to_owned),
            recipient_id: Some(owner.into()),
            action: OrchDecisionAction::JobRun,
            outcome: OrchDecisionOutcome::Allowed,
            reason: "job_dispatch_accepted".into(),
            configuration_revision: state.snapshot.revision,
            time_zone: zone.name().into(),
            sources: vec![OrchDecisionSource {
                field: OrchPolicyField::Routine,
                source: policy.routine.source,
            }],
        };
        let result = match decision {
            JobDecision::NoOwner => JobRunResult::NoOwner,
            JobDecision::Paused => JobRunResult::Paused,
            JobDecision::Hold(slot) => {
                let journal_id = if job.pending == Some(slot) {
                    None
                } else {
                    tx.execute(
                        "UPDATE orchestration_jobs SET pending_revision=?,pending_ms=? WHERE id=?",
                        params![slot.revision as i64, slot.at_ms, job.id],
                    )
                    .map_err(|error| classify(error, "Hold job slot"))?;
                    event.outcome = OrchDecisionOutcome::Held;
                    event.reason = "routine_hold".into();
                    Some(super::super::append(&tx, &event)?)
                };
                JobRunResult::Held { slot, journal_id }
            }
            JobDecision::Dispatch { slot_ms } => {
                let operation = input
                    .manual_operation_id
                    .clone()
                    .unwrap_or_else(|| job_operation_id(&job, slot_ms));
                let dispatch = job_dispatch(&job, &input, operation.clone())?;
                let (receipt, created) = crate::storage::dispatch::dispatch_in_transaction(
                    &tx,
                    dispatch,
                    retention_days,
                    || now,
                    &receiving,
                )?;
                tx.execute("INSERT INTO orchestration_job_runs(job_id,revision,slot_ms,observed_slot_ms,manual,operation_id,recorded_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![job.id,job.revision as i64,slot_ms,input.slot_ms,input.manual_operation_id.is_some(),operation,now as i64])
                    .map_err(|error|classify(error,"Record job acceptance"))?;
                let run_id = unsigned(tx.last_insert_rowid())?;
                super::super::append(&tx, &event)?;
                if input.manual_operation_id.is_none() {
                    tx.execute("UPDATE orchestration_jobs SET pending_revision=NULL,pending_ms=NULL WHERE id=?",[&job.id])
                        .map_err(|error|classify(error,"Consume held job slot"))?;
                }
                JobRunResult::Accepted {
                    receipt,
                    created,
                    run_id,
                }
            }
        };
        tx.commit()
            .map_err(|error| classify(error, "Commit job admission"))?;
        Ok(result)
    }
}
