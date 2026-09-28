//! One consistent read-only projection for rehydration; no reconciliation or GC.

use super::{Storage, StorageError, bindings::BindingRows, errors::classify};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{path::Path, time::Duration};
use tmt_core::binding::{BindingEntry, BindingRecords};

const ROLE_LIMIT: usize = 500;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ContextRequests {
    pub originated: u64,
    pub incoming: u64,
}

#[derive(Debug)]
pub struct IdentityContextSnapshot {
    /// Stored evidence, not a claim of current presence. The caller must apply
    /// the core's pure binding evaluator to a fresh driver observation.
    pub entry: BindingEntry,
    pub role: Option<String>,
    pub role_truncated: bool,
    pub requests: ContextRequests,
}

impl Storage {
    /// No create, migration, permission changes, acknowledgment or retention
    /// writes. Missing/incompatible storage is an error for the caller to map
    /// to a quiet unavailable context, never an invitation to initialize it.
    pub fn context_by_pane(
        path: &Path,
        pane: &str,
        server: &str,
        now_ms: u64,
    ) -> Result<Option<IdentityContextSnapshot>, StorageError> {
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| classify(error, "Open context storage read-only"))?;
        connection
            .busy_timeout(Duration::from_millis(100))
            .map_err(|error| classify(error, "Bound context storage wait"))?;
        let transaction = connection
            .transaction()
            .map_err(|error| classify(error, "Begin context read snapshot"))?;
        super::migrations::require_current(&transaction)?;
        let Some(entry) = BindingRows(&transaction).entry_by_pane(pane, server)? else {
            return Ok(None);
        };
        let role = transaction
            .query_row(
                "SELECT content FROM role_profiles WHERE identity_id = ?",
                [&entry.identity.id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| classify(error, "Read context role"))?;
        let role_truncated = role
            .as_ref()
            .is_some_and(|text| text.chars().count() > ROLE_LIMIT);
        let role = role.map(|text| text.chars().take(ROLE_LIMIT).collect());
        let requests = requests(&transaction, &entry.identity.id, now_ms)?;
        Ok(Some(IdentityContextSnapshot {
            entry,
            role,
            role_truncated,
            requests,
        }))
    }
}

fn requests(
    connection: &Connection,
    identity: &str,
    now_ms: u64,
) -> Result<ContextRequests, StorageError> {
    let now = i64::try_from(now_ms).map_err(|_| {
        classify(
            rusqlite::Error::InvalidQuery,
            "Context retention cutoff out of range",
        )
    })?;
    // Match the two X list scopes, including both per-item and bulk acknowledgments.
    let originated = count(
        connection,
        identity,
        now,
        "a.originator_identity_id = ?1
         AND a.attention_acknowledged_revision < a.attention_revision
         AND COALESCE((SELECT acknowledged_through FROM request_attention_identities
                       WHERE identity_id = ?1), 0) < a.attention_revision",
    )?;
    let incoming = count(
        connection,
        identity,
        now,
        "a.recipient_identity_id = ?1 AND a.route_kind = 'inbox' AND a.status = 'queued'
         AND a.recipient_attention_acknowledged_revision < a.recipient_attention_revision
         AND COALESCE((SELECT acknowledged_through FROM request_recipient_attention_identities
                       WHERE identity_id = ?1), 0) < a.recipient_attention_revision",
    )?;
    Ok(ContextRequests {
        originated,
        incoming,
    })
}

fn count(
    connection: &Connection,
    identity: &str,
    now: i64,
    condition: &str,
) -> Result<u64, StorageError> {
    // Only counts enter context: no bodies, receipts or request IDs are loaded.
    let query = format!(
        "SELECT COUNT(*) FROM request_attempts AS a
         WHERE a.retention_expires_at_ms > ?2 AND ({condition})"
    );
    connection
        .query_row(&query, params![identity, now], |row| {
            u64::try_from(row.get::<_, i64>(0)?).map_err(|_| rusqlite::Error::InvalidQuery)
        })
        .map_err(|error| classify(error, "Read context request count"))
}

#[cfg(test)]
mod tests;
