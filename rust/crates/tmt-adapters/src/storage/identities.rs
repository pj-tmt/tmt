//! Identity SQL shares the invocation-owned connection and schema history.

use std::{path::Path, time::Duration};

use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior, params,
};
use tmt_core::{
    identity::{Identity, IdentityReader, IdentityRepository, IdentityWriter, Lifetime},
    names::{ValidatedName, normalize_name},
};

use super::{Storage, StorageError, StorageErrorCode, errors::classify};

const COLUMNS: &str = "id, name, canonical_name, lifetime, created_at, updated_at";

pub(super) struct IdentityRecords<'a>(pub(super) &'a Connection);

pub(super) fn identity_row_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<Identity> {
    let lifetime = match row.get::<_, String>(offset + 3)?.as_str() {
        "temporary" => Lifetime::Temporary,
        "saved" => Lifetime::Saved,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Identity {
        id: row.get(offset)?,
        name: row.get(offset + 1)?,
        canonical_name: row.get(offset + 2)?,
        lifetime,
        created_at: row.get(offset + 4)?,
        updated_at: row.get(offset + 5)?,
    })
}

fn identity_row(row: &Row<'_>) -> rusqlite::Result<Identity> {
    identity_row_at(row, 0)
}

impl IdentityReader for IdentityRecords<'_> {
    type Error = StorageError;

    fn find_identity(&self, canonical_name: &str) -> Result<Option<Identity>, Self::Error> {
        self.0
            .query_row(
                &format!("SELECT {COLUMNS} FROM identities WHERE canonical_name = ? AND retired_at_ms IS NULL"),
                [canonical_name],
                identity_row,
            )
            .optional()
            .map_err(|error| classify(error, "Find identity"))
    }

    fn list_identities(&self) -> Result<Vec<Identity>, Self::Error> {
        let mut statement = self.0.prepare(&format!(
            "SELECT {COLUMNS} FROM identities WHERE retired_at_ms IS NULL ORDER BY canonical_name COLLATE BINARY"
        )).map_err(|error| classify(error, "Prepare identity list"))?;
        statement
            .query_map([], identity_row)
            .and_then(|rows| rows.collect())
            .map_err(|error| classify(error, "List identities"))
    }
}

impl IdentityWriter for IdentityRecords<'_> {
    fn insert_identity(
        &mut self,
        name: &ValidatedName,
        lifetime: Lifetime,
    ) -> Result<Identity, Self::Error> {
        self.0.query_row(
            &format!("INSERT INTO identities (id, name, canonical_name, lifetime, created_at, updated_at) \
                VALUES (?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) RETURNING {COLUMNS}"),
            params![uuid::Uuid::new_v4().to_string(), name.display_name(), name.canonical_name(), lifetime.as_str()],
            identity_row,
        ).map_err(|error| classify(error, "Create identity"))
    }

    fn save_identity(&mut self, identity: &Identity) -> Result<Identity, Self::Error> {
        self.0.query_row(
            &format!("UPDATE identities SET lifetime = 'saved', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                WHERE id = ? AND retired_at_ms IS NULL AND lifetime = 'temporary' RETURNING {COLUMNS}"),
            [&identity.id], identity_row,
        ).map_err(|error| classify(error, "Save identity"))
    }
}

impl IdentityReader for Storage {
    type Error = StorageError;

    fn find_identity(&self, canonical_name: &str) -> Result<Option<Identity>, Self::Error> {
        IdentityRecords(self.connection()?).find_identity(canonical_name)
    }

    fn list_identities(&self) -> Result<Vec<Identity>, Self::Error> {
        IdentityRecords(self.connection()?).list_identities()
    }
}

impl Storage {
    /// Shared discovery for interactive completion and identity pickers. This
    /// never creates or migrates storage, reconciles bindings, or probes tmux.
    /// Callers may treat unavailable storage as no suggestions, not as absence
    /// of an identity for a subsequent mutation.
    pub fn identity_candidates(
        path: &Path,
        prefix: &str,
        remembered_only: bool,
    ) -> Result<Vec<Identity>, StorageError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| classify(error, "Open identity discovery"))?;
        connection
            .busy_timeout(Duration::from_millis(100))
            .map_err(|error| classify(error, "Bound identity discovery wait"))?;
        let prefix = normalize_name(prefix);
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM identities WHERE retired_at_ms IS NULL
             AND substr(canonical_name, 1, length(?1)) = ?1
             AND (?2 = 0 OR EXISTS (
               SELECT 1 FROM identity_session_preferences p
               WHERE p.identity_id = identities.id AND p.provider_session_id IS NOT NULL
             ))
             ORDER BY lifetime = 'saved' DESC, canonical_name COLLATE BINARY"
            ))
            .map_err(|error| classify(error, "Prepare identity discovery"))?;
        statement
            .query_map(params![prefix, remembered_only], identity_row)
            .and_then(|rows| rows.collect())
            .map_err(|error| classify(error, "Read identity discovery"))
    }

    /// Request history keeps participant UUIDs after retirement, so compact
    /// projections can continue to name the exact historical participant.
    pub fn find_identity_by_id(&self, id: &str) -> Result<Option<Identity>, StorageError> {
        identity_by_id(self.connection()?, id)
    }

    /// Office revalidates the originally selected UUID across separate calls;
    /// a same-name replacement must never inherit a pending proof or credential.
    pub fn find_active_identity_by_id(&self, id: &str) -> Result<Option<Identity>, StorageError> {
        active_identity_by_id(self.connection()?, id)
    }

    /// Explicit CLI/API selection prefers an active UUID over a UUID-shaped
    /// display name. Unknown or retired UUIDs retain normalized name lookup.
    pub fn resolve_identity(&self, selector: &str) -> Result<Option<Identity>, StorageError> {
        if tmt_core::dispatch::canonical_id(selector)
            && let Some(identity) = self.find_active_identity_by_id(selector)?
        {
            return Ok(Some(identity));
        }
        self.find_identity(&normalize_name(selector))
    }
}

pub(super) fn identity_by_id(
    connection: &Connection,
    id: &str,
) -> Result<Option<Identity>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM identities WHERE id = ?"),
            [id],
            identity_row,
        )
        .optional()
        .map_err(|error| classify(error, "Find identity by ID"))
}

pub(super) fn active_identity_by_id(
    connection: &Connection,
    id: &str,
) -> Result<Option<Identity>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM identities WHERE id = ? AND retired_at_ms IS NULL"),
            [id],
            identity_row,
        )
        .optional()
        .map_err(|error| classify(error, "Find active identity by ID"))
}

impl IdentityRepository for Storage {
    fn with_identity_transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn IdentityWriter<Error = Self::Error>) -> Result<T, Self::Error>,
    ) -> Result<T, Self::Error> {
        with_immediate_transaction(self, "identity", |transaction| {
            operation(&mut IdentityRecords(transaction))
        })
    }
}

pub(super) fn with_immediate_transaction<T, E>(
    storage: &mut Storage,
    operation_name: &str,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<StorageError>,
{
    let connection = storage.connection.as_mut().ok_or_else(|| {
        E::from(StorageError::new(
            StorageErrorCode::Closed,
            "Storage is already closed",
        ))
    })?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            E::from(classify(
                error,
                &format!("Begin {operation_name} transaction"),
            ))
        })?;
    let result = operation(&transaction)?;
    transaction.commit().map_err(|error| {
        E::from(classify(
            error,
            &format!("Commit {operation_name} transaction"),
        ))
    })?;
    Ok(result)
}

#[cfg(test)]
mod tests;
