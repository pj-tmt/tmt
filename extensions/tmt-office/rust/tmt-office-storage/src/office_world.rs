//! One transaction-scoped owner for the installation's lazily created world.

mod layout;
mod legacy;
#[cfg(test)]
mod tests;
pub use layout::{LocalWorldSnapshot, WorldStoreError};

use rusqlite::{OptionalExtension, Transaction, params};
use tmt_adapters::storage::{StorageError, classify};

pub(super) fn ensure_world(
    transaction: &Transaction<'_>,
    timestamp: impl FnOnce() -> Result<i64, StorageError>,
) -> Result<String, StorageError> {
    if let Some(id) = transaction
        .query_row(
            "SELECT id FROM office_local_worlds WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| classify(error, "Read local Office world"))?
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    transaction
        .execute(
            "INSERT INTO office_local_worlds (singleton, id, created_at_ms) VALUES (1, ?, ?)",
            params![id, timestamp()?],
        )
        .map_err(|error| classify(error, "Create local Office world"))?;
    Ok(id)
}
