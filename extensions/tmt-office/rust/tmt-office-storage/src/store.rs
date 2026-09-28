//! The Office repository connection and its transaction helper.
//!
//! Until the storage switch, Office repositories run on the shared core
//! database file. Opening goes through the public core storage entry point so
//! creation, migration and file permissions stay exactly as before; Office then
//! owns its own connection with the same connection policy.

use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::{path::Path, time::Duration};
use tmt_adapters::storage::{Storage, StorageError, StorageErrorCode, classify};

pub struct OfficeStore {
    connection: Option<Connection>,
}

impl OfficeStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        Storage::open(path)?.close()?;
        let connection =
            Connection::open(path).map_err(|error| classify(error, "Open Office storage"))?;
        let prepare = (|| {
            connection
                .pragma_update(None, "foreign_keys", "ON")
                .map_err(|error| classify(error, "Configure foreign keys"))?;
            connection
                .busy_timeout(Duration::from_millis(5000))
                .map_err(|error| classify(error, "Configure busy timeout"))?;
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .map_err(|error| classify(error, "Configure WAL"))?;
            connection
                .pragma_update(None, "synchronous", "NORMAL")
                .map_err(|error| classify(error, "Configure synchronous policy"))
        })();
        if let Err(primary) = prepare {
            let _ = connection.close();
            return Err(primary);
        }
        Ok(Self {
            connection: Some(connection),
        })
    }

    pub fn close(&mut self) -> Result<(), StorageError> {
        let Some(connection) = self.connection.take() else {
            return Ok(());
        };
        let checkpoint = connection
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))
            .map_err(|error| classify(error, "Checkpoint Office storage"));
        let close = connection
            .close()
            .map_err(|(_connection, error)| classify(error, "Close Office storage"));
        checkpoint.and(close)
    }

    pub(crate) fn connection(&self) -> Result<&Connection, StorageError> {
        self.connection.as_ref().ok_or_else(closed)
    }

    pub(crate) fn connection_mut(&mut self) -> Result<&mut Connection, StorageError> {
        self.connection.as_mut().ok_or_else(closed)
    }
}

fn closed() -> StorageError {
    StorageError::new(StorageErrorCode::Closed, "Storage is already closed")
}

/// Runs one Office write in an immediate transaction; Office owns its commits.
pub(crate) fn with_immediate_transaction<T, E>(
    storage: &mut OfficeStore,
    operation_name: &str,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<StorageError>,
{
    let connection = storage
        .connection
        .as_mut()
        .ok_or_else(|| E::from(closed()))?;
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
