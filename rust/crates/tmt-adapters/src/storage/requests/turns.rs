//! The first turn that began after a request was handed to its recipient (#4911). The hook's
//! runtime-observation transaction opens it when it admits a main-session turn start and closes
//! it at the turn's end; the delivery state reads it, so `working` ends with that turn.
use super::*;
use tmt_core::request::{delivery::HandoverTurn, passthrough::BOUND_WINDOW_MS};

/// A main turn started at `now`: every request already handed to `identity` that no turn has
/// accompanied yet, has no final and is no passthrough, gets this turn. A turn already running
/// when the message arrived began before it was prepared and so claims nothing; the message waits
/// for the next one.
pub(in crate::storage) fn open(
    db: &Connection,
    identity: &str,
    now: u64,
) -> Result<usize, StorageError> {
    let now_i64 = checked_i64(now, "Request turn start")?;
    let floor = checked_i64(now.saturating_sub(BOUND_WINDOW_MS), "Request turn window")?;
    db.execute(
        "INSERT INTO request_turns(request_id,started_ms)
         SELECT a.request_id,?2 FROM request_attempts a
         WHERE a.recipient_identity_id=?1 AND a.route_kind='inbox' AND a.request_kind='request'
           AND a.wake_state='sent' AND a.withdrawn_at_ms IS NULL
           AND a.prepared_at_ms<=?2 AND a.prepared_at_ms>=?3
           AND NOT EXISTS (SELECT 1 FROM request_responses r WHERE r.request_id=a.request_id)
           AND NOT EXISTS (SELECT 1 FROM request_passthrough p WHERE p.request_id=a.request_id)
           AND NOT EXISTS (SELECT 1 FROM request_turns t WHERE t.request_id=a.request_id)",
        params![identity, now_i64, floor],
    )
    .map_err(|error| classify(error, "Open the turns of handed-over requests"))
}

/// A main turn ended at `now`: the turn every open request of `identity` was waiting on is over.
pub(in crate::storage) fn close(
    db: &Connection,
    identity: &str,
    now: u64,
) -> Result<usize, StorageError> {
    db.execute(
        "UPDATE request_turns SET ended_ms=MAX(?2,started_ms) WHERE ended_ms IS NULL
           AND request_id IN (SELECT request_id FROM request_attempts WHERE recipient_identity_id=?1)",
        params![identity, checked_i64(now, "Request turn end")?],
    )
    .map_err(|error| classify(error, "Close the turns of handed-over requests"))
}

pub(in crate::storage) fn read(
    db: &Connection,
    request_id: &str,
) -> Result<HandoverTurn, StorageError> {
    let row: Option<(i64, Option<i64>)> = db
        .query_row(
            "SELECT started_ms,ended_ms FROM request_turns WHERE request_id=?",
            [request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| classify(error, "Read the turn of a handed-over request"))?;
    Ok(match row {
        None => HandoverTurn::Unrecorded,
        Some((_, Some(_))) => HandoverTurn::Ended,
        Some((started, None)) => HandoverTurn::Started {
            at_ms: u64::try_from(started).unwrap_or(0),
        },
    })
}
