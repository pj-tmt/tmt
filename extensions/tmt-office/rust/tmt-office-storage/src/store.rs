//! The Office repository connection and its transaction helper.
//!
//! An activated `office.db` is the store; installs without user Office data
//! switch to it on first open, and installs with user data keep the legacy
//! store on the shared core file until migrated. Core-owned references are
//! read only through [`CoreReferences`] preflight, over the invoking `tmt`.

use crate::{
    StorageLayout,
    core_references::CoreReferences,
    migration::{self, MigrationError},
};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::{path::Path, time::Duration};
use tmt_adapters::storage::{StorageError, StorageErrorCode, classify};

pub struct OfficeStore {
    connection: Option<Connection>,
    references: Box<dyn CoreReferences + Send>,
    /// Whether this is `office.db`, which alone holds Office-local tables.
    office_database: bool,
}

impl OfficeStore {
    /// Opens the store for this configuration.
    ///
    /// An activated `office.db` is authoritative (an interrupted switch is
    /// settled first, as #402's recovery decides). Without one, an install
    /// whose shared Office tables hold no user data switches now through the
    /// normal coordinator, so core records the receipt and fences the shared
    /// tables. An install with user Office data keeps the legacy store on the
    /// shared core file until the user runs `tmt office storage migrate`.
    pub fn open_configured(layout: &StorageLayout) -> Result<Self, StorageError> {
        Self::open_configured_with(layout, default_references(layout)?)
    }

    pub(crate) fn open_configured_with(
        layout: &StorageLayout,
        references: Box<dyn CoreReferences + Send>,
    ) -> Result<Self, StorageError> {
        if !select_office_database(layout)? {
            // Legacy store: the documented exception until migration, retired
            // with the migration coordinator.
            return Self::with_references(&layout.source, references);
        }
        let mut store = Self::with_references(&layout.database, references)?;
        store.office_database = true;
        crate::schema::upgrade_existing(store.connection()?)
            .map_err(|error| classify(error, "Upgrade Office storage"))?
            .map_err(|message| StorageError::new(StorageErrorCode::IncompatibleSchema, message))?;
        // Point-of-use reads treat Office-recorded retirements like core's.
        let markers = crate::retirement::Markers::load(store.connection()?)
            .map_err(|error| classify(error, "Read Office retirement markers"))?;
        let core = std::mem::replace(&mut store.references, Box::new(crate::retirement::Unset));
        store.references = Box::new(crate::retirement::MarkedReferences::new(core, markers));
        Ok(store)
    }

    /// Opens repositories on one database file, the pre-switch layout. Tests
    /// only: production must honor the receipt through [`Self::open_configured`].
    #[cfg(test)]
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let references = crate::core_references::CoreStore::open(path)?;
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
            office_database: false,
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

    /// Replaces the reference port of an opened store, for fault injection.
    #[cfg(test)]
    pub(crate) fn replace_references(&mut self, references: Box<dyn CoreReferences + Send>) {
        self.references = references;
    }

    pub(crate) fn is_office_database(&self) -> bool {
        self.office_database
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

/// Production reaches core through the invoking `tmt`; tests use the
/// in-process implementation over a disposable core file.
#[cfg(not(any(test, feature = "in-process-core")))]
fn default_references(
    _layout: &StorageLayout,
) -> Result<Box<dyn CoreReferences + Send>, StorageError> {
    Ok(Box::new(
        crate::core_references::ProcessReferences::discover()?,
    ))
}

#[cfg(any(test, feature = "in-process-core"))]
fn default_references(
    layout: &StorageLayout,
) -> Result<Box<dyn CoreReferences + Send>, StorageError> {
    crate::core_references::in_process::references(layout)
}

fn migration_error(error: MigrationError) -> StorageError {
    let code = match error {
        MigrationError::Busy => StorageErrorCode::Busy,
        MigrationError::NotWritable(_) => StorageErrorCode::NotWritable,
        _ => StorageErrorCode::Unknown,
    };
    StorageError::new(code, error.to_string())
}

/// Whether `office.db` is the store. A recorded switch whose storage needs
/// recovery is an error, never a silent return to the shared file.
fn select_office_database(layout: &StorageLayout) -> Result<bool, StorageError> {
    if layout.database.exists() {
        match migration::ensure_active(layout) {
            Ok(()) => return Ok(true),
            // An undecided switch reverted: fall through to the fresh check.
            Err(MigrationError::State(_)) if !layout.database.exists() => {}
            Err(error) => return Err(migration_error(error)),
        }
    }
    match migration::switch_fresh(layout) {
        Ok(_) => {}
        Err(error @ MigrationError::RecoveryRequired { .. }) => {
            return Err(migration_error(error));
        }
        // Anything else (user data raced in, an older core without the fence,
        // a busy lock) leaves the legacy store in use; nothing was decided.
        Err(_) => {}
    }
    if layout.database.exists() {
        migration::ensure_active(layout).map_err(migration_error)?;
        return Ok(true);
    }
    Ok(false)
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
