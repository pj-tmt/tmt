//! Strict SQLite row decoding for request state.

use rusqlite::Row;
use tmt_core::{
    endpoint::ServerEvidence,
    host::HostKind,
    limits::is_valid_js_safe_integer,
    request::{
        AttemptStatus, FinalResponse, Originator, RawRequestContext, RequestAttempt,
        RequestEndpoint, RequestKind, RequestOrigin, RequestRoute,
        attention::{AttentionRecord, ResponseMetadata},
    },
};

pub(super) const ATTEMPT_COLUMNS: &str = "
    attempt_id, request_id, originator_kind, originator_identity_id,
    recipient_identity_id, nonce, identity_id, route_kind, server_id, socket_path,
    server_pid, server_start_time, pane_id, pane_pid, wait_active, status,
    preamble_every, inject_preamble, cadence_reserved, prepared_at_ms,
    sending_at_ms, settled_at_ms, wait_released_at_ms, response_submitted_at_ms,
    expires_at_ms, retention_days, retention_expires_at_ms,
    attention_revision, attention_acknowledged_revision,
    recipient_attention_revision, recipient_attention_acknowledged_revision, request_kind, room_id,
    host, withdrawn_at_ms, withdrawal_reason, origin,
    remote_ref, remote_target_id, remote_target_name, remote_sender_name";
pub(super) const ATTEMPT_COLUMN_COUNT: usize = 41;
/// Where `qualified_attempt_columns` selects the originator's attention revision and its
/// acknowledgment.
pub(super) const ATTENTION_REVISION_COLUMN: usize = 27;
pub(super) const ATTENTION_ACKNOWLEDGED_COLUMN: usize = 28;
pub(super) fn qualified_attempt_columns() -> String {
    ATTEMPT_COLUMNS
        .split(',')
        .map(|column| format!("a.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}
/// A final's own columns, then the route columns of its attempt: a remote final keeps its route
/// on the attempt row, so the join reaches it by `attempt_id` and no other final needs it. An
/// attempt is deleted only once its last unexpired final is gone, so a remote final the join
/// cannot complete has expired and the reader treats it as absent.
pub(super) const RESPONSE_COLUMNS: &str = "
    r.request_id, r.attempt_id, r.route_kind, r.route_recipient_identity_id,
    r.server_id, r.socket_path, r.server_pid, r.server_start_time,
    r.pane_id, r.pane_pid, r.body, r.body_bytes, r.submitted_at_ms, r.response_expires_at_ms,
    r.host, a.remote_ref, a.remote_target_id, a.remote_target_name, a.remote_sender_name";
pub(super) const RESPONSE_FROM: &str = "request_responses r
    LEFT JOIN request_attempts a ON a.attempt_id = r.attempt_id AND r.route_kind = 'remote'";

fn invalid_row() -> rusqlite::Error {
    rusqlite::Error::InvalidQuery
}

fn checked_u64(value: i64) -> rusqlite::Result<u64> {
    let value = u64::try_from(value).map_err(|_| invalid_row())?;
    if !is_valid_js_safe_integer(value) {
        Err(invalid_row())
    } else {
        Ok(value)
    }
}

pub(super) fn u64_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<u64> {
    checked_u64(row.get(offset)?)
}

pub(super) fn optional_u64_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(offset)?
        .map(checked_u64)
        .transpose()
}

fn bool_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<bool> {
    match row.get::<_, i64>(offset)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid_row()),
    }
}

fn status_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<AttemptStatus> {
    match row.get::<_, String>(offset)?.as_str() {
        "prepared" => Ok(AttemptStatus::Prepared),
        "sending" => Ok(AttemptStatus::Sending),
        "sent" => Ok(AttemptStatus::Sent),
        "queued" => Ok(AttemptStatus::Queued),
        "uncertain" => Ok(AttemptStatus::Uncertain),
        "definitely_failed" => Ok(AttemptStatus::DefinitelyFailed),
        _ => Err(invalid_row()),
    }
}

pub(super) fn attempt_row(row: &Row<'_>) -> rusqlite::Result<RequestAttempt> {
    let originator_kind = row.get::<_, String>(2)?;
    let originator_identity_id = row.get::<_, Option<String>>(3)?;
    let originator = match originator_kind.as_str() {
        "unknown" if originator_identity_id.is_none() => Originator::Unknown,
        "explicit" => Originator::Explicit(originator_identity_id.ok_or_else(invalid_row)?),
        "verified" => Originator::Verified(originator_identity_id.ok_or_else(invalid_row)?),
        _ => return Err(invalid_row()),
    };
    let route = match row.get::<_, String>(7)?.as_str() {
        "pane" => pane_route_at(row, 8, 33)?,
        "inbox" => RequestRoute::Inbox {
            recipient_identity_id: row.get::<_, Option<String>>(4)?.ok_or_else(invalid_row)?,
        },
        "remote" => RequestRoute::Remote {
            remote: row.get::<_, Option<String>>(37)?.ok_or_else(invalid_row)?,
            target_id: row.get(38)?,
            target_name: row.get::<_, Option<String>>(39)?.ok_or_else(invalid_row)?,
            sender_name: row.get::<_, Option<String>>(40)?.ok_or_else(invalid_row)?,
        },
        _ => return Err(invalid_row()),
    };
    let attention_revision = u64_at(row, ATTENTION_REVISION_COLUMN)?;
    let attention_acknowledged_revision = u64_at(row, ATTENTION_ACKNOWLEDGED_COLUMN)?;
    if attention_acknowledged_revision > attention_revision {
        return Err(invalid_row());
    }
    let recipient_attention_revision = u64_at(row, 29)?;
    let recipient_attention_acknowledged_revision = u64_at(row, 30)?;
    if recipient_attention_acknowledged_revision > recipient_attention_revision {
        return Err(invalid_row());
    }
    let preamble_every = optional_u64_at(row, 16)?;
    if preamble_every == Some(0) {
        return Err(invalid_row());
    }
    let room_id: Option<String> = row.get(32)?;
    if room_id
        .as_deref()
        .is_some_and(|id| !tmt_core::dispatch::canonical_id(id))
    {
        return Err(invalid_row());
    }
    Ok(RequestAttempt {
        room_id,
        kind: match row.get::<_, String>(31)?.as_str() {
            "request" => RequestKind::Request,
            "announcement" => RequestKind::Announcement,
            _ => return Err(invalid_row()),
        },
        origin: match row.get::<_, Option<String>>(36)?.as_deref() {
            None => RequestOrigin::Unknown,
            Some("person") => RequestOrigin::Person,
            Some("automation") => RequestOrigin::Automation,
            Some(_) => return Err(invalid_row()),
        },
        attempt_id: row.get(0)?,
        request_id: row.get(1)?,
        originator,
        recipient_identity_id: row.get(4)?,
        nonce: row.get(5)?,
        identity_id: row.get(6)?,
        route,
        wait_active: bool_at(row, 14)?,
        status: status_at(row, 15)?,
        preamble_every,
        inject_preamble: bool_at(row, 17)?,
        cadence_reserved: bool_at(row, 18)?,
        prepared_at_ms: u64_at(row, 19)?,
        sending_at_ms: optional_u64_at(row, 20)?,
        settled_at_ms: optional_u64_at(row, 21)?,
        wait_released_at_ms: optional_u64_at(row, 22)?,
        response_submitted_at_ms: optional_u64_at(row, 23)?,
        withdrawal: match (optional_u64_at(row, 34)?, row.get::<_, Option<String>>(35)?) {
            (None, None) => None,
            (Some(withdrawn_at_ms), Some(reason))
                if withdrawn_at_ms > 0
                    && !reason.is_empty()
                    && reason.len() <= tmt_core::request::MAX_WITHDRAWAL_REASON_BYTES
                    && optional_u64_at(row, 23)?.is_none() =>
            {
                Some(tmt_core::request::Withdrawal {
                    reason,
                    withdrawn_at_ms,
                })
            }
            _ => return Err(invalid_row()),
        },
        expires_at_ms: u64_at(row, 24)?,
        retention_days: u64_at(row, 25)?,
        retention_expires_at_ms: u64_at(row, 26)?,
    })
}

pub(super) fn response_row(row: &Row<'_>) -> rusqlite::Result<FinalResponse> {
    Ok(FinalResponse {
        request_id: row.get(0)?,
        attempt_id: row.get(1)?,
        route: match row.get::<_, String>(2)?.as_str() {
            "pane" => pane_route_at(row, 4, 14)?,
            "inbox" => RequestRoute::Inbox {
                recipient_identity_id: row.get::<_, Option<String>>(3)?.ok_or_else(invalid_row)?,
            },
            "remote" => RequestRoute::Remote {
                remote: row.get::<_, Option<String>>(15)?.ok_or_else(invalid_row)?,
                target_id: row.get(16)?,
                target_name: row.get::<_, Option<String>>(17)?.ok_or_else(invalid_row)?,
                sender_name: row.get::<_, Option<String>>(18)?.ok_or_else(invalid_row)?,
            },
            _ => return Err(invalid_row()),
        },
        body: row.get(10)?,
        body_bytes: u64_at(row, 11)?,
        submitted_at_ms: u64_at(row, 12)?,
        response_expires_at_ms: u64_at(row, 13)?,
    })
}

pub(super) fn attention_row(row: &Row<'_>) -> rusqlite::Result<AttentionRecord> {
    let revision = u64_at(row, 27)?;
    let acknowledged_revision = u64_at(row, 28)?;
    let attempt = attempt_row(row)?;
    let acknowledged_through = u64_at(row, ATTEMPT_COLUMN_COUNT)?;
    let submitted_at_ms = optional_u64_at(row, ATTEMPT_COLUMN_COUNT + 1)?;
    let body_bytes = optional_u64_at(row, ATTEMPT_COLUMN_COUNT + 2)?;
    let expires_at_ms = optional_u64_at(row, ATTEMPT_COLUMN_COUNT + 3)?;
    let response_metadata = match (submitted_at_ms, body_bytes, expires_at_ms) {
        (None, None, None) => None,
        (Some(_), Some(body_bytes), Some(expires_at_ms)) => Some(ResponseMetadata {
            body_bytes,
            expires_at_ms,
        }),
        _ => return Err(invalid_row()),
    };
    Ok(AttentionRecord {
        attempt,
        revision,
        acknowledged_revision,
        acknowledged_through,
        response_metadata,
    })
}

pub(super) fn recipient_attention_row(row: &Row<'_>) -> rusqlite::Result<AttentionRecord> {
    let revision = u64_at(row, 29)?;
    let acknowledged_revision = u64_at(row, 30)?;
    let attempt = attempt_row(row)?;
    let acknowledged_through = u64_at(row, ATTEMPT_COLUMN_COUNT)?;
    let submitted_at_ms = optional_u64_at(row, ATTEMPT_COLUMN_COUNT + 1)?;
    let body_bytes = optional_u64_at(row, ATTEMPT_COLUMN_COUNT + 2)?;
    let expires_at_ms = optional_u64_at(row, ATTEMPT_COLUMN_COUNT + 3)?;
    let response_metadata = match (submitted_at_ms, body_bytes, expires_at_ms) {
        (None, None, None) => None,
        (Some(_), Some(body_bytes), Some(expires_at_ms)) => Some(ResponseMetadata {
            body_bytes,
            expires_at_ms,
        }),
        _ => return Err(invalid_row()),
    };
    Ok(AttentionRecord {
        attempt,
        revision,
        acknowledged_revision,
        acknowledged_through,
        response_metadata,
    })
}

pub(super) fn context_row(row: &Row<'_>) -> rusqlite::Result<RawRequestContext> {
    let attempt = attempt_row(row)?;
    let message: Option<String> = row.get(ATTEMPT_COLUMN_COUNT)?;
    let message_bytes = optional_u64_at(row, ATTEMPT_COLUMN_COUNT + 1)?;
    let expires_at_ms = optional_u64_at(row, ATTEMPT_COLUMN_COUNT + 2)?;
    // Prompt scrubbing removes text and byte count but intentionally retains
    // the expiry marker, allowing reads to distinguish expired from historical
    // content that was never stored.
    if message.is_some() != message_bytes.is_some()
        || (message.is_some() && expires_at_ms.is_none())
    {
        return Err(invalid_row());
    }
    Ok(RawRequestContext {
        attempt,
        message,
        message_bytes,
        expires_at_ms,
    })
}

/// `host_at` holds the fence's host; NULL is tmux, which wrote every fence
/// before schema 39. An unknown host is invalid, never a guess.
fn pane_route_at(row: &Row<'_>, offset: usize, host_at: usize) -> rusqlite::Result<RequestRoute> {
    let host = match row.get::<_, Option<String>>(host_at)? {
        None => HostKind::Tmux,
        Some(host) => HostKind::parse(&host).ok_or_else(invalid_row)?,
    };
    let server_pid = u64_at(row, offset + 2)?;
    let pane_pid = u64_at(row, offset + 5)?;
    if server_pid == 0 || pane_pid == 0 {
        return Err(invalid_row());
    }
    Ok(RequestRoute::Pane(RequestEndpoint {
        server: ServerEvidence {
            host,
            server_id: row.get(offset)?,
            socket_path: row.get(offset + 1)?,
            server_pid,
            server_start_time: row.get(offset + 3)?,
        },
        pane_id: row.get(offset + 4)?,
        pane_pid,
    }))
}
