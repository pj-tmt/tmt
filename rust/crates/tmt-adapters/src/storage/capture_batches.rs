//! Conversation correlation is committed with the facts it references. No
//! provider, clock window or message text is used to reconstruct placement.
mod read;

use super::{StorageError, bindings::BindingRows, classify};
use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::{
    binding::{Binding, BindingRecords, session::RememberedSession},
    responses::RETENTION_MS,
};

pub(super) fn current(db: &Connection, identity: &str) -> Result<Option<String>, StorageError> {
    let rows = BindingRows(db);
    let Some(binding) = rows.entry_by_id(identity)?.and_then(|entry| entry.binding) else {
        return Ok(None);
    };
    let preferences = rows.session_preferences(identity)?;
    let Some(remembered) = preferences.remembered else {
        return Ok(None);
    };
    let Some(key) = binding.session.key else {
        return Ok(None);
    };
    db.query_row("SELECT id FROM capture_batches WHERE identity_id=?1 AND closed_ms IS NULL AND binding_id=?2 AND driver=?3 AND session_id=?4 AND runtime_pid=?5 AND runtime_start=?6",
        params![identity,binding.id,remembered.harness.as_str(),remembered.provider_session.as_str(),key.incarnation.pid() as i64,key.incarnation.start_identity()], |row|row.get(0))
        .optional().map_err(|e|classify(e,"Read current input batch"))
}

pub(super) fn close(db: &Connection, identity: &str, now: u64) -> Result<(), StorageError> {
    db.execute(
        "UPDATE capture_batches SET closed_ms=?2 WHERE identity_id=?1 AND closed_ms IS NULL",
        params![identity, now as i64],
    )
    .map_err(|e| classify(e, "Close input batch"))?;
    Ok(())
}

pub(super) fn begin(
    db: &Connection,
    binding: &Binding,
    remembered: &RememberedSession,
    turn: Option<&str>,
    now: u64,
) -> Result<String, StorageError> {
    db.execute(
        "UPDATE capture_batches SET closed_ms=?2 WHERE identity_id=?1 AND closed_ms IS NULL",
        params![binding.identity_id, now as i64],
    )
    .map_err(|e| classify(e, "Close earlier input batch"))?;
    let key = binding.session.key.as_ref().ok_or_else(|| {
        super::StorageError::new(
            super::StorageErrorCode::Corrupt,
            "Input batch has no admitted process",
        )
    })?;
    let id = tmt_core::operation::new_operation_id();
    db.execute("INSERT INTO capture_batches(id,identity_id,binding_id,driver,session_id,runtime_pid,runtime_start,provider_turn,created_ms) VALUES(?,?,?,?,?,?,?,?,?)",
        params![id,binding.identity_id,binding.id,remembered.harness.as_str(),remembered.provider_session.as_str(),key.incarnation.pid() as i64,key.incarnation.start_identity(),turn,now as i64])
        .map_err(|e|classify(e,"Open input batch"))?;
    prune(db, &binding.identity_id, now)?;
    Ok(id)
}

pub(super) fn ensure(
    db: &Connection,
    binding: &Binding,
    remembered: &RememberedSession,
    turn: Option<&str>,
    now: u64,
) -> Result<String, StorageError> {
    match current(db, &binding.identity_id)? {
        Some(id) => Ok(id),
        None => begin(db, binding, remembered, turn, now),
    }
}

pub(super) fn input(
    db: &Connection,
    identity: &str,
    batch: &str,
    request: &str,
    source: &str,
    delivery: &str,
    now: u64,
) -> Result<(), StorageError> {
    db.execute("INSERT INTO capture_entries(identity_id,batch_id,kind,request_id,source,delivery,created_ms) VALUES(?1,?2,'input',?3,?4,?5,?6) ON CONFLICT DO NOTHING",
        params![identity,batch,request,source,delivery,now as i64]).map_err(|e|classify(e,"Record delivered conversation input"))?;
    Ok(())
}

pub(super) fn prompt_inputs(
    db: &Connection,
    identity: &str,
    batch: &str,
    now: u64,
) -> Result<(), StorageError> {
    let mut query=db.prepare("SELECT a.request_id FROM request_attempts a WHERE a.recipient_identity_id=?1 AND (
        EXISTS(SELECT 1 FROM request_passthrough p WHERE p.request_id=a.request_id AND p.state='bound' AND p.bound_at_ms=?2)
        OR EXISTS(SELECT 1 FROM request_turns t WHERE t.request_id=a.request_id AND t.started_ms=?2)) ORDER BY a.prepared_at_ms,a.request_id")
        .map_err(|e|classify(e,"Find admitted prompt inputs"))?;
    let ids = query
        .query_map(params![identity, now as i64], |row| row.get::<_, String>(0))
        .map_err(|e| classify(e, "Read admitted prompt inputs"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| classify(e, "Decode admitted prompt inputs"))?;
    for request in ids {
        input(db, identity, batch, &request, "incoming", "delivered", now)?;
    }
    Ok(())
}

/// Ordinary delivery joins only a genuinely working turn. An idle handoff
/// waits for its admitted prompt; Digest records its explicit continuation.
pub(super) fn delivered(
    db: &Connection,
    request: &str,
    delivery: &str,
    now: u64,
) -> Result<(), StorageError> {
    let identity: Option<String> = db
        .query_row(
            "SELECT recipient_identity_id FROM request_attempts WHERE request_id=?",
            [request],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| classify(e, "Read delivered input recipient"))?
        .flatten();
    let Some(identity) = identity else {
        return Ok(());
    };
    let rows = BindingRows(db);
    let Some(binding) = rows.entry_by_id(&identity)?.and_then(|entry| entry.binding) else {
        return Ok(());
    };
    if binding.session.state != tmt_core::binding::session::RuntimeState::Running {
        return Ok(());
    }
    let prefs = rows.session_preferences(&identity)?;
    let Some(remembered) = prefs.remembered.as_ref() else {
        return Ok(());
    };
    let Some(key) = binding.session.key.as_ref() else {
        return Ok(());
    };
    let activity = remembered
        .state
        .as_ref()
        .and_then(crate::runtime::driver_state::state_activity);
    if let Some(activity) = activity.filter(|a| {
        a.phase == tmt_core::binding::session::activity::ActivityPhase::Working
            && a.matches(&remembered.provider_session, &key.incarnation)
            && key.provider_session.as_ref() == Some(&remembered.provider_session)
    }) {
        let batch = ensure(
            db,
            &binding,
            remembered,
            activity.turn.as_ref().map(|turn| turn.as_str()),
            now,
        )?;
        input(db, &identity, &batch, request, "incoming", delivery, now)?;
    }
    Ok(())
}

pub(super) fn final_entry(
    db: &Connection,
    identity: &str,
    response: u64,
    now: u64,
) -> Result<(), StorageError> {
    let batch = current(db, identity)?;
    db.execute("INSERT INTO capture_entries(identity_id,batch_id,kind,response_sequence,created_ms) VALUES(?1,?2,'final',?3,?4)",params![identity,batch,response as i64,now as i64])
        .map_err(|e|classify(e,"Record captured final order"))?;
    prune(db, identity, now)
}

pub(super) fn reply(db: &Connection, request: &str, now: u64) -> Result<(), StorageError> {
    let identity: Option<String> = db
        .query_row(
            "SELECT recipient_identity_id FROM request_attempts WHERE request_id=?",
            [request],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| classify(e, "Read reply author"))?
        .flatten();
    let Some(identity) = identity else {
        return Ok(());
    };
    let batch:Option<String>=db.query_row("SELECT batch_id FROM capture_entries WHERE identity_id=?1 AND kind='input' AND request_id=?2 AND source='incoming'",params![identity,request],|row|row.get(0)).optional()
        .map_err(|e|classify(e,"Read submitted reply batch"))?.flatten();
    db.execute("INSERT INTO capture_entries(identity_id,batch_id,kind,request_id,created_ms) VALUES(?1,?2,'reply',?3,?4) ON CONFLICT DO NOTHING",params![identity,batch,request,now as i64])
        .map_err(|e|classify(e,"Record submitted reply order"))?;
    prune(db, &identity, now)
}

pub(super) fn prune(db: &Connection, identity: &str, now: u64) -> Result<(), StorageError> {
    let floor = now.saturating_sub(RETENTION_MS) as i64;
    db.execute("INSERT INTO capture_floor(identity_id,through_sequence) SELECT identity_id,MAX(sequence) FROM capture_entries WHERE identity_id=?1 AND created_ms<=?2 GROUP BY identity_id
        ON CONFLICT(identity_id) DO UPDATE SET through_sequence=MAX(through_sequence,excluded.through_sequence)",params![identity,floor]).map_err(|e|classify(e,"Remember dropped conversation coverage"))?;
    db.execute(
        "DELETE FROM capture_entries WHERE identity_id=?1 AND created_ms<=?2",
        params![identity, floor],
    )
    .map_err(|e| classify(e, "Expire captured conversation links"))?;
    db.execute("DELETE FROM capture_batches WHERE identity_id=?1 AND created_ms<=?2 AND NOT EXISTS(SELECT 1 FROM capture_entries e WHERE e.batch_id=capture_batches.id)",params![identity,floor])
        .map_err(|e|classify(e,"Expire empty input batches"))?;
    Ok(())
}

/// Freeze the revision in the same transaction that produced this body. A
/// later submitted reply must never change an older captured final's key.
pub(super) fn acknowledgeable(
    db: &Connection,
    request: &str,
    revision: u64,
) -> Result<(), StorageError> {
    db.execute(
        "INSERT INTO capture_acknowledgements(entry_sequence,request_id,identity_id,revision)
        SELECT e.sequence,a.request_id,a.originator_identity_id,?2 FROM request_attempts a
        JOIN capture_entries e ON e.identity_id=a.recipient_identity_id AND (
          (a.response_submitted_at_ms IS NOT NULL AND e.kind='reply' AND e.request_id=a.request_id)
          OR (a.response_submitted_at_ms IS NULL AND e.kind='final' AND e.response_sequence=(
            SELECT p.answer_response FROM request_passthrough p WHERE p.request_id=a.request_id)))
        WHERE a.request_id=?1 AND a.originator_identity_id IS NOT NULL ON CONFLICT DO NOTHING",
        params![request, revision as i64],
    )
    .map_err(|e| classify(e, "Freeze painted-answer acknowledgement"))?;
    Ok(())
}

/// Reserve causal input positions before the provider can see the checklist.
/// Pending positions are invisible until the publication outcome is settled.
pub(super) fn reserve_checklist(
    db: &Connection,
    identity: &str,
    checklist: &str,
    now: u64,
) -> Result<(), StorageError> {
    let Some(batch) = current(db, identity)? else {
        return Ok(());
    };
    db.execute("INSERT INTO capture_entries(identity_id,batch_id,kind,request_id,source,delivery,checklist_id,created_ms)
        SELECT ?1,?2,'input',request_id,source,'pending',?3,?4 FROM focus_items WHERE checklist_id=?3 ORDER BY sequence
        ON CONFLICT DO NOTHING",params![identity,batch,checklist,now as i64])
        .map_err(|e|classify(e,"Reserve checklist input order"))?;
    Ok(())
}
pub(super) fn settle_checklist(
    db: &Connection,
    checklist: &str,
    delivery: Option<&str>,
) -> Result<(), StorageError> {
    match delivery {
        Some(delivery) => {
            db.execute("UPDATE capture_entries SET delivery=?2 WHERE checklist_id=?1 AND delivery='pending'",params![checklist,delivery])
            .map_err(|e|classify(e,"Publish checklist input positions"))?;
        }
        None => {
            db.execute(
                "DELETE FROM capture_entries WHERE checklist_id=?1 AND delivery='pending'",
                [checklist],
            )
            .map_err(|e| classify(e, "Discard unpublished checklist inputs"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
