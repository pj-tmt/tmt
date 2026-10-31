//! Bounded public job views; never an admission or a second outcome tracker.
use super::{JobScheduleDocument, calendar};
use crate::{
    orchestration::{
        OrchReadFailure, OrchZone,
        wire::{self, OrchReadQuery},
    },
    storage::{JobListQuery, JobRunEvidence, Storage},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tmt_core::{
    limits::MAX_JS_SAFE_INTEGER,
    orchestration::{OrchError, OrchRoutine, effective, jobs::JobError},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    after: u64,
    room: Option<String>,
    owner: Option<String>,
    job: Option<String>,
}
fn last_run(run: JobRunEvidence) -> Value {
    let item = &run.receipt.items[0];
    let request = run
        .request
        .as_ref()
        .map(|request| crate::request_history::item_document(request, |_| None));
    let result_reference = run
        .request
        .as_ref()
        .filter(|request| {
            matches!(
                request.final_state,
                tmt_core::request::attention::FinalState::Retained { .. }
            )
        })
        .map(|request| json!({"requestId":request.request_id}));
    json!({"id":run.id,"revision":run.revision,"slotMs":run.slot_ms,"recordedAtMs":run.recorded_ms,
        "operationId":run.receipt.operation_id,"requestId":item.request_id,
        "acceptance":crate::dispatch::acceptance_value(item.acceptance),
        "request":request,"resultReference":result_reference})
}
pub(crate) fn read(
    storage: &Storage,
    query: OrchReadQuery,
    observed: u64,
) -> Result<Value, OrchReadFailure> {
    let (room, owner, job, limit, cursor, full) = match query {
        OrchReadQuery::Jobs {
            squad_room_id,
            owner_identity_id,
            job_id,
            limit,
            cursor,
        } => (
            squad_room_id,
            owner_identity_id,
            job_id,
            limit.unwrap_or(50),
            cursor,
            false,
        ),
        OrchReadQuery::Job { job_id } => (None, None, Some(job_id), 1, None, true),
        _ => return Err(OrchError::Invalid.into()),
    };
    let after = if let Some(cursor) = cursor {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| OrchError::Invalid)?;
        let decoded: Cursor = serde_json::from_slice(&bytes).map_err(|_| OrchError::Invalid)?;
        if decoded.version != 1
            || decoded.after == 0
            || decoded.after > MAX_JS_SAFE_INTEGER
            || decoded.room != room
            || decoded.owner != owner
            || decoded.job != job
        {
            return Err(OrchError::Invalid.into());
        }
        decoded.after
    } else {
        0
    };
    let mut selected = storage.read_orchestration_jobs(
        JobListQuery {
            room_id: room.as_deref(),
            owner_id: owner.as_deref(),
            job_id: job.as_deref(),
            after_number: after,
            limit,
        },
        full,
        observed,
    )?;
    if full && selected.page.items.is_empty() {
        return Err(JobError::NotFound.into());
    }
    let zone = OrchZone::load(selected.policy.time_zone.as_deref())?;
    let mut items = Vec::with_capacity(selected.page.items.len());
    for job in selected.page.items {
        let policy = if job.owner_available {
            job.owner_id
                .as_deref()
                .map(|id| effective(&selected.policy.snapshot, id, observed, &zone))
                .transpose()?
        } else {
            None
        };
        let routine = policy
            .as_ref()
            .map(|policy| wire::effective_value(policy)["routine"].clone());
        let next = if job.owner_available && job.pause.is_none() {
            let next = calendar(&job.schedule)?
                .next_after(i64::try_from(observed).map_err(|_| OrchError::Invalid)?)
                .map_err(|_| JobError::Invalid)?;
            if next.is_some_and(|next| next.unsigned_abs() > MAX_JS_SAFE_INTEGER) {
                return Err(JobError::Limit.into());
            }
            next
        } else {
            None
        };
        let held = policy
            .as_ref()
            .is_some_and(|policy| policy.routine.value == OrchRoutine::Hold);
        let state = if !job.owner_available {
            "no_owner"
        } else if job.pause.is_some() {
            "paused"
        } else if held {
            "held"
        } else {
            "on"
        };
        let schedule = serde_json::to_value(JobScheduleDocument::from(&job.schedule))
            .map_err(|_| OrchError::Invalid)?;
        let mut value = json!({"jobId":job.id,"displayId":format!("j{}",job.number),"revision":job.revision,
            "legacy":job.legacy.as_ref().map(|legacy|json!({"roomId":legacy.room_id,"cronId":legacy.cron_id})),
            "target":{"squadRoomId":job.room_id,"identityId":job.owner_id.as_ref().filter(|_|job.owner_available),"identityName":job.owner_name.as_ref().filter(|_|job.owner_available)},
            "title":job.title,"messagePreview":job.message_preview,"schedule":schedule,"state":state,
            "manualPause":job.pause.as_ref().map(|pause|json!({"by":pause.by,"atMs":pause.at_ms})),
            "heldSlot":job.pending.map(|slot|json!({"revision":slot.revision,"atMs":slot.at_ms})),"routine":routine,
            "nextRunAtMs":next,"lastRun":selected.last_runs.remove(&job.id).map(last_run)});
        if full {
            value["definition"] = json!({"ownerIdentityId":job.owner_id,"squadRoomId":job.room_id,"title":job.title,
                "message":selected.full_message.take().ok_or(JobError::NotFound)?,"schedule":schedule,
                "pause":job.pause.as_ref().map(|pause|json!({"by":pause.by,"atMs":pause.at_ms}))});
        }
        items.push(value);
    }
    let next_cursor = selected
        .page
        .next_after
        .map(|after| {
            serde_json::to_vec(&Cursor {
                version: 1,
                after,
                room,
                owner,
                job,
            })
            .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
            .map_err(|_| OrchError::Invalid)
        })
        .transpose()?;
    if full {
        Ok(
            json!({"schemaVersion":1,"observedAtMs":observed,"configurationRevision":selected.policy.snapshot.revision,
            "job":items.pop().ok_or(JobError::NotFound)?,"historyCoverage":"recorded_only"}),
        )
    } else {
        Ok(
            json!({"schemaVersion":1,"observedAtMs":observed,"configurationRevision":selected.policy.snapshot.revision,
            "jobs":items,"nextCursor":next_cursor,"historyCoverage":"recorded_only"}),
        )
    }
}
