//! Indexed retained-history reads over canonical requests, including acknowledged work.

use super::{checked_i64, checked_limit, checked_now, rows};
use crate::storage::{StorageError, errors::classify};
use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::request::{
    attention::AttentionRecord,
    history::*,
    inbox::{ANSWERABLE, OpenQuery},
};

// Read a bounded UTF-8 prefix as bytes: SQLite TEXT substr stops at embedded NUL.
const PREVIEW_BYTES: usize = HISTORY_PREVIEW_CHARS * 4;

fn history_preview(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<String>> {
    let column = rows::ATTEMPT_COLUMN_COUNT + 4;
    let Some(bytes) = row.get::<_, Option<Vec<u8>>>(column)? else {
        return Ok(None);
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) if bytes.len() == PREVIEW_BYTES && error.error_len().is_none() => {
            // The byte cap may split only the last scalar; preceding text stays exact.
            std::str::from_utf8(&bytes[..error.valid_up_to()]).expect("validated UTF-8 prefix")
        }
        Err(error) => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Blob,
                Box::new(error),
            ));
        }
    };
    Ok(Some(text.chars().take(HISTORY_PREVIEW_CHARS).collect()))
}

fn history_select(source: &str, condition: &str, preview: &str) -> String {
    format!("SELECT {}, COALESCE(state.acknowledged_through,0),
        response.submitted_at_ms, response.body_bytes, response.response_expires_at_ms,
        {preview}
        FROM {source}
        LEFT JOIN request_recipient_attention_identities AS state ON state.identity_id=a.recipient_identity_id
        LEFT JOIN request_responses AS response ON response.request_id=a.request_id
        WHERE {condition}", rows::qualified_attempt_columns())
}

pub(super) fn history_query(scope: &HistoryScope, has_cursor: bool) -> String {
    let (index, condition) = match scope {
        HistoryScope::Recipient { room_id: None, .. } => (
            "request_history_recipient",
            "a.recipient_identity_id=?1 AND ?2 IS NULL",
        ),
        HistoryScope::Recipient {
            room_id: Some(_), ..
        } => (
            "request_history_recipient_room",
            "a.recipient_identity_id=?1 AND a.room_id=?2",
        ),
        HistoryScope::Room(_) => ("request_history_room", "?1 IS NULL AND a.room_id=?2"),
    };
    let cursor = if has_cursor {
        "(a.prepared_at_ms,a.request_id) < (?3,?4)"
    } else {
        "?3 IS NULL AND ?4 IS NULL"
    };
    let select = history_select(
        &format!("request_attempts AS a INDEXED BY {index}"),
        &format!("{condition} AND a.retention_expires_at_ms > ?6 AND {cursor}"),
        &format!(
            "CASE WHEN a.message_expires_at_ms > ?6 THEN substr(CAST(a.message_text AS BLOB),1,{PREVIEW_BYTES}) ELSE NULL END"
        ),
    );
    format!("{select} ORDER BY a.prepared_at_ms DESC,a.request_id DESC LIMIT ?5")
}

pub(super) fn list_request_history(
    connection: &Connection,
    query: &HistoryQuery,
    limit: u64,
    now_ms: u64,
) -> Result<Vec<HistoryRecord>, StorageError> {
    let (recipient, room) = match &query.scope {
        HistoryScope::Recipient {
            identity_id,
            room_id,
        } => (Some(identity_id.as_str()), room_id.as_deref()),
        HistoryScope::Room(id) => (None, Some(id.as_str())),
    };
    let before = query
        .before
        .as_ref()
        .map(|cursor| checked_i64(cursor.prepared_at_ms, "History cursor"))
        .transpose()?;
    let mut statement = connection
        .prepare(&history_query(&query.scope, query.before.is_some()))
        .map_err(|error| classify(error, "Prepare request history query"))?;
    statement
        .query_map(
            params![
                recipient,
                room,
                before,
                query
                    .before
                    .as_ref()
                    .map(|cursor| cursor.request_id.as_str()),
                checked_limit(limit)?,
                checked_now(now_ms, "History retention cutoff")?
            ],
            |row| {
                Ok(HistoryRecord {
                    attention: rows::recipient_attention_row(row)?,
                    preview: history_preview(row)?,
                })
            },
        )
        .map_err(|error| classify(error, "Read request history"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| classify(error, "Decode request history"))
}

/// Open requests to one recipient, oldest first. This narrows by the same
/// columns the service's acceptance rule reads; the service decides.
pub(super) fn list_open_requests(
    connection: &Connection,
    query: &OpenQuery,
    now_ms: u64,
) -> Result<Vec<HistoryRecord>, StorageError> {
    let statuses = ANSWERABLE
        .iter()
        .map(|status| format!("'{}'", status.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let select = history_select(
        "request_attempts AS a INDEXED BY request_history_recipient",
        &format!(
            "a.recipient_identity_id=?1 AND (?2 IS NULL OR a.originator_identity_id=?2)
             AND a.request_kind='request' AND a.response_submitted_at_ms IS NULL
             AND a.status IN ({statuses}) AND a.retention_expires_at_ms > ?3
             AND (a.expires_at_ms > ?3 OR a.prepared_at_ms > ?4)"
        ),
        &format!(
            "CASE WHEN a.message_expires_at_ms > ?3 THEN substr(CAST(a.message_text AS BLOB),1,{PREVIEW_BYTES}) ELSE NULL END"
        ),
    );
    let mut statement = connection
        .prepare(&format!(
            "{select} ORDER BY a.prepared_at_ms, a.request_id LIMIT ?5"
        ))
        .map_err(|error| classify(error, "Prepare open request query"))?;
    statement
        .query_map(
            params![
                query.recipient_identity_id,
                query.originator_identity_id,
                checked_now(now_ms, "Open request cutoff")?,
                checked_i64(query.window_start_ms, "Open request window")?,
                checked_limit(query.limit)?
            ],
            |row| {
                Ok(HistoryRecord {
                    attention: rows::recipient_attention_row(row)?,
                    preview: history_preview(row)?,
                })
            },
        )
        .map_err(|error| classify(error, "Read open requests"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| classify(error, "Decode open requests"))
}

pub(super) fn find_request_history(
    connection: &Connection,
    request_id: &str,
) -> Result<Option<AttentionRecord>, StorageError> {
    connection
        .query_row(
            &history_select("request_attempts AS a", "a.request_id=?", "NULL"),
            [request_id],
            rows::recipient_attention_row,
        )
        .optional()
        .map_err(|error| classify(error, "Find retained request history"))
}
