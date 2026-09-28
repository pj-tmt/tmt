//! The Office repository connection and its transaction helper.
//!
//! Core's storage cutover receipt selects the store: before the switch, Office
//! repositories run on the shared core database file, opened through the public
//! core storage entry point so creation, migration and file permissions stay
//! exactly as before; after it they run on `office.db`, activated first if an
//! interrupted switch left it unmarked. Core-owned references are read only
//! through [`CoreReferences`] preflight.

use crate::{
    StorageLayout,
    core_references::{CoreReferences, CoreStore},
    migration::{self, MigrationError},
};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::{path::Path, time::Duration};
use tmt_adapters::storage::{StorageError, StorageErrorCode, classify};

pub struct OfficeStore {
    connection: Option<Connection>,
    references: Box<dyn CoreReferences + Send>,
}

impl OfficeStore {
    /// Opens the store selected by core's cutover receipt.
    pub fn open_configured(layout: &StorageLayout) -> Result<Self, StorageError> {
        let references = CoreStore::open(&layout.source)?;
        if references.storage_cutover()?.is_none() {
            return Self::with_references(&layout.source, Box::new(references));
        }
        migration::ensure_active(layout).map_err(|error| {
            let code = match error {
                MigrationError::Busy => StorageErrorCode::Busy,
                MigrationError::NotWritable(_) => StorageErrorCode::NotWritable,
                _ => StorageErrorCode::Unknown,
            };
            StorageError::new(code, error.to_string())
        })?;
        Self::with_references(&layout.database, Box::new(references))
    }

    /// Opens repositories on one database file, the pre-switch layout. Tests
    /// only: production must honor the receipt through [`Self::open_configured`].
    #[cfg(test)]
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let references = CoreStore::open(path)?;
        Self::with_references(path, Box::new(references))
    }

    pub(crate) fn with_references(
        path: &Path,
        references: Box<dyn CoreReferences + Send>,
    ) -> Result<Self, StorageError> {
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
            references,
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

    /// The Office connection together with the core reference port, for reads
    /// that depend on rows observed inside an Office transaction.
    pub(crate) fn split(&mut self) -> Result<(&mut Connection, &dyn CoreReferences), StorageError> {
        let connection = self.connection.as_mut().ok_or_else(closed)?;
        Ok((connection, self.references.as_ref()))
    }

    /// Core-owned identity and room reads for preflight.
    pub(crate) fn references(&self) -> &dyn CoreReferences {
        self.references.as_ref()
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
    with_immediate_transaction_mapped(storage, operation_name, E::from, operation)
}

/// [`with_immediate_transaction`] for error types that cannot implement
/// `From<StorageError>`, such as the model-owned board error.
pub(crate) fn with_immediate_transaction_mapped<T, E>(
    storage: &mut OfficeStore,
    operation_name: &str,
    storage_error: impl Fn(StorageError) -> E,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
) -> Result<T, E> {
    let connection = storage
        .connection
        .as_mut()
        .ok_or_else(|| storage_error(closed()))?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            storage_error(classify(
                error,
                &format!("Begin {operation_name} transaction"),
            ))
        })?;
    let result = operation(&transaction)?;
    transaction.commit().map_err(|error| {
        storage_error(classify(
            error,
            &format!("Commit {operation_name} transaction"),
        ))
    })?;
    Ok(result)
}

/// Like [`with_immediate_transaction`], and also lends the core reference port
/// for reads that depend on rows observed inside the Office transaction.
pub(crate) fn with_immediate_transaction_and_references<T, E>(
    storage: &mut OfficeStore,
    operation_name: &str,
    operation: impl FnOnce(&Transaction<'_>, &dyn CoreReferences) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<StorageError>,
{
    let (connection, references) = storage.split().map_err(E::from)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            E::from(classify(
                error,
                &format!("Begin {operation_name} transaction"),
            ))
        })?;
    let result = operation(&transaction, references)?;
    transaction.commit().map_err(|error| {
        E::from(classify(
            error,
            &format!("Commit {operation_name} transaction"),
        ))
    })?;
    Ok(result)
}

/// Marks the end of preflight, before the Office transaction begins. Tests
/// install a barrier here to change core state inside the accepted window;
/// non-test builds compile this to nothing.
#[inline]
pub(crate) fn preflight_complete() {
    #[cfg(test)]
    tests::run_barrier();
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    thread_local! {
        static BARRIER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }

    /// Runs `action` once, at the next preflight boundary on this thread.
    pub(crate) fn at_next_preflight(action: impl FnOnce() + 'static) {
        BARRIER.with(|barrier| *barrier.borrow_mut() = Some(Box::new(action)));
    }

    pub(super) fn run_barrier() {
        if let Some(action) = BARRIER.with(|barrier| barrier.borrow_mut().take()) {
            action();
        }
    }
}
