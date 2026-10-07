//! Indexed retained-history reads over canonical requests, including acknowledged work.

use super::{checked_i64, checked_limit, checked_now, rows};
use crate::{
    request_text::normalized,
    storage::{StorageError, errors::classify},
};
use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::request::{
    attention::AttentionRecord,
    history::*,
    inbox::{ANSWERABLE, OpenQuery},
};

// Read a bounded UTF-8 prefix as bytes: SQLite TEXT substr stops at embedded NUL.
const PREVIEW_BYTES: usize = HISTORY_PREVIEW_CHARS * 4;

// Four extra bytes give the decoder lookahead past 160 four-byte scalars.
const RESPONSE_PREVIEW_BYTES: usize = PREVIEW_BYTES + 4;

fn utf8_prefix(bytes: &[u8], cap: usize, column: usize) -> rusqlite::Result<&str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if bytes.len() == cap && error.error_len().is_none() => {
            // The byte cap may split only the last scalar; preceding text stays exact.
            Ok(std::str::from_utf8(&bytes[..error.valid_up_to()]).expect("validated UTF-8 prefix"))
        }
        Err(error) => Err(rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Blob,
            Box::new(error),
        )),
    }
}

fn history_preview(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<String>> {
    let column = rows::ATTEMPT_COLUMN_COUNT + 4;
    row.get::<_, Option<Vec<u8>>>(column)?
        .map(|bytes| {
            Ok(utf8_prefix(&bytes, PREVIEW_BYTES, column)?
                .chars()
                .take(HISTORY_PREVIEW_CHARS)
                .collect())
        })
        .transpose()
}

fn response_preview(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<ResponsePreview>> {
    let column = rows::ATTEMPT_COLUMN_COUNT + 5;
    let Some(bytes) = row.get::<_, Option<Vec<u8>>>(column)? else {
        return Ok(None);
    };
    let text = utf8_prefix(&bytes, RESPONSE_PREVIEW_BYTES, column)?;
    let body_bytes = rows::u64_at(row, rows::ATTEMPT_COLUMN_COUNT + 2)?;
    let mut chars = normalized(text);
    let mut preview = String::new();
    let mut truncated = body_bytes > text.len() as u64;
    for _ in 0..HISTORY_PREVIEW_CHARS {
        match chars.next() {
            None => {
                return Ok(Some(ResponsePreview {
                    text: preview,
                    truncated,
                }));
            }
            Some('\n') => {
                return Ok(Some(ResponsePreview {
                    text: preview,
                    truncated: true,
                }));
            }
            Some(c) => preview.push(c),
        }
    }
    truncated |= chars.next().is_some();
    Ok(Some(ResponsePreview {
        text: preview,
        truncated,
    }))
}

fn history_select(source: &str, condition: &str, preview: &str, response_preview: &str) -> String {
    format!("SELECT {}, COALESCE(state.acknowledged_through,0),
        response.submitted_at_ms, response.body_bytes, response.response_expires_at_ms,
        {preview}, {response_preview}
        FROM {source}
        LEFT JOIN request_recipient_attention_identities AS state ON state.identity_id=a.recipient_identity_id
        LEFT JOIN request_responses AS response ON response.request_id=a.request_id
        WHERE {condition}", rows::qualified_attempt_columns())
}

pub(super) fn history_query(scope: &HistoryScope, has_cursor: bool) -> String {
    let results = matches!(scope, HistoryScope::OriginatorResults(_));
    let time = if results {
        "response_submitted_at_ms"
    } else {
        "prepared_at_ms"
    };
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
        HistoryScope::OriginatorResults(_) => (
            "request_history_originator_results",
            "a.originator_identity_id=?1 AND ?2 IS NULL AND a.response_submitted_at_ms IS NOT NULL",
        ),
    };
    let cursor = if has_cursor {
        format!("(a.{time},a.request_id) < (?3,?4)")
    } else {
        "?3 IS NULL AND ?4 IS NULL".into()
    };
    let select = history_select(
        &format!("request_attempts AS a INDEXED BY {index}"),
        &format!("{condition} AND a.retention_expires_at_ms > ?6 AND {cursor}"),
        &format!(
            "CASE WHEN a.message_expires_at_ms > ?6 THEN substr(CAST(a.message_text AS BLOB),1,{PREVIEW_BYTES}) ELSE NULL END"
        ),
        &if results {
            format!(
                "CASE WHEN response.response_expires_at_ms > ?6 THEN substr(CAST(response.body AS BLOB),1,{RESPONSE_PREVIEW_BYTES}) ELSE NULL END"
            )
        } else {
            "NULL".into()
        },
    );
    format!("{select} ORDER BY a.{time} DESC,a.request_id DESC LIMIT ?5")
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
        HistoryScope::OriginatorResults(id) => (Some(id.as_str()), None),
    };
    let before = query
        .before
        .as_ref()
        .map(|cursor| checked_i64(cursor.timestamp(), "History cursor"))
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
                query.before.as_ref().map(|cursor| cursor.request_id()),
                checked_limit(limit)?,
                checked_now(now_ms, "History retention cutoff")?
            ],
            |row| {
                Ok(HistoryRecord {
                    attention: rows::recipient_attention_row(row)?,
                    preview: history_preview(row)?,
                    response_preview: response_preview(row)?,
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
             AND a.withdrawn_at_ms IS NULL
             AND a.status IN ({statuses}) AND a.retention_expires_at_ms > ?3
             AND (a.expires_at_ms > ?3 OR a.prepared_at_ms > ?4)"
        ),
        &format!(
            "CASE WHEN a.message_expires_at_ms > ?3 THEN substr(CAST(a.message_text AS BLOB),1,{PREVIEW_BYTES}) ELSE NULL END"
        ),
        "NULL",
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
                    response_preview: None,
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
            &history_select("request_attempts AS a", "a.request_id=?", "NULL", "NULL"),
            [request_id],
            rows::recipient_attention_row,
        )
        .optional()
        .map_err(|error| classify(error, "Find retained request history"))
}
