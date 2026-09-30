mod bindings;
mod context;
mod dispatch;
mod errors;
mod host_servers;
mod identities;
mod identity_hooks;
mod identity_metadata;
mod identity_status;
mod migrations;
mod profiles;
mod requests;
mod room;
mod room_roster;
mod storage_cutover;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use rusqlite::Connection;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub use bindings::PurgedSession;
pub use context::{ContextRequests, IdentityContextSnapshot};
pub use dispatch::DispatchError;
pub use errors::{StorageError, StorageErrorCode, classify};
use errors::{classify_io, classify_open, incompatible};
pub use host_servers::HostServerIncarnation;
pub use room::RoomStoreError;
pub use room_roster::{RoomRoster, RosterError, RosterMember};
pub use storage_cutover::StorageCutover;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointMode {
    Passive,
    Truncate,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StorageHealth {
    pub path: PathBuf,
    pub schema_version: u32,
    pub journal_mode: &'static str,
    pub foreign_keys: bool,
    pub busy_timeout_ms: u32,
    pub synchronous: &'static str,
    pub fts5: bool,
}

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const WAL_RETRY_INTERVAL: Duration = Duration::from_millis(10);
const WAL_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(50);

/// One invocation-owned connection. The raw handle never crosses this adapter's
/// boundary. Explicit close reports failures; Connection's RAII remains a safety
/// net for early returns and unwinding, not a replacement for fallible cleanup.
pub struct Storage {
    path: PathBuf,
    connection: Option<Connection>,
    /// Whether this connection records lifecycle evidence for enabled
    /// extension hooks (see `extension_hooks`).
    capturing: bool,
}

impl Storage {
    /// Hooks observe existing state; they must not initialize or upgrade storage.
    /// The supervising hook process supplies the overall deadline, including any
    /// pathological filesystem wait; lock contention is bounded independently.
    pub fn open_hook(path: &Path) -> Result<Self, StorageError> {
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(|error| classify_open(error, "Open existing hook storage", path))?;
        connection
            .busy_timeout(Duration::from_millis(50))
            .map_err(|error| classify(error, "Bound hook storage wait"))?;
        migrations::require_current(&connection)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| classify(error, "Configure hook foreign keys"))?;
        Ok(Self {
            path: path.to_path_buf(),
            connection: Some(connection),
            capturing: false,
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        secure_directory(directory)?;
        let mut connection =
            Connection::open(&path).map_err(|error| classify_open(error, "Open storage", &path))?;
        let prepare = (|| {
            connection
                .pragma_update(None, "foreign_keys", "ON")
                .map_err(|error| classify(error, "Configure foreign keys"))?;
            configure_wal(&connection)?;
            connection
                .pragma_update(None, "synchronous", "NORMAL")
                .map_err(|error| classify(error, "Configure synchronous policy"))?;
            connection.execute_batch("CREATE VIRTUAL TABLE temp._tmt_fts5_check USING fts5(content); DROP TABLE temp._tmt_fts5_check;")
                .map_err(|error| incompatible("The SQLite runtime does not provide FTS5").caused_by(error))?;
            migrations::apply(&mut connection)?;
            secure_files(&path)
        })();
        if let Err(primary) = prepare {
            // Closing must not replace the error that prevented a usable handle.
            let _ = connection.close();
            return Err(primary);
        }
        // Observation is best-effort: a capture that cannot be installed is
        // skipped rather than failing storage.
        let capturing = crate::extension_hooks::should_capture(&path)
            && connection
                .execute_batch(crate::extension_hooks::CAPTURE_SQL)
                .is_ok();
        Ok(Self {
            path,
            connection: Some(connection),
            capturing,
        })
    }

    pub fn health(&self) -> Result<StorageHealth, StorageError> {
        let connection = self.connection()?;
        let policy = connection.query_row(
            "SELECT (SELECT foreign_keys FROM pragma_foreign_keys), (SELECT journal_mode FROM pragma_journal_mode), (SELECT timeout FROM pragma_busy_timeout), (SELECT synchronous FROM pragma_synchronous)",
            [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
        ).map_err(|error| classify(error, "Read storage health"))?;
        if policy != (1, "wal".into(), 5000, 1) {
            return Err(incompatible(
                "The SQLite connection does not match storage policy",
            ));
        }
        let version = connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _migrations",
                [],
                |row| row.get::<_, u32>(0),
            )
            .map_err(|error| classify(error, "Read schema version"))?;
        Ok(StorageHealth {
            path: self.path.clone(),
            schema_version: version,
            journal_mode: "wal",
            foreign_keys: true,
            busy_timeout_ms: 5000,
            synchronous: "normal",
            fts5: true,
        })
    }

    pub fn checkpoint(&self, mode: CheckpointMode) -> Result<(), StorageError> {
        checkpoint(self.connection()?, &self.path, mode)
    }

    pub fn close(&mut self) -> Result<(), StorageError> {
        let Some(connection) = self.connection.take() else {
            return Ok(());
        };
        if self.capturing {
            crate::extension_hooks::drain(&connection);
        }
        let checkpoint_result = checkpoint(&connection, &self.path, CheckpointMode::Passive);
        let close_result = connection
            .close()
            .map_err(|(_connection, error)| classify(error, "Close storage"));
        // Both effects have already run. Preserve the first failure.
        checkpoint_result.and(close_result)
    }

    fn connection(&self) -> Result<&Connection, StorageError> {
        self.connection
            .as_ref()
            .ok_or_else(|| StorageError::new(StorageErrorCode::Closed, "Storage is already closed"))
    }

    fn connection_mut(&mut self) -> Result<&mut Connection, StorageError> {
        self.connection
            .as_mut()
            .ok_or_else(|| StorageError::new(StorageErrorCode::Closed, "Storage is already closed"))
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if self.capturing
            && let Some(connection) = &self.connection
        {
            crate::extension_hooks::drain(connection);
        }
    }
}

fn configure_wal(connection: &Connection) -> Result<(), StorageError> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        // SQLite may skip its busy handler for competing WAL transitions.
        // Bound both its individual waits and our retries by one deadline.
        connection
            .busy_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(WAL_ATTEMPT_TIMEOUT),
            )
            .map_err(|error| classify(error, "Configure busy timeout"))?;
        let mode = connection
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0));
        match mode {
            Ok(mode) if mode == "wal" => {
                return connection
                    .busy_timeout(BUSY_TIMEOUT)
                    .map_err(|error| classify(error, "Configure busy timeout"));
            }
            Ok(_) => return Err(incompatible("The SQLite connection could not enable WAL")),
            Err(error) => {
                let error = classify(error, "Configure WAL");
                let remaining = deadline.saturating_duration_since(Instant::now());
                if error.code != StorageErrorCode::Busy || remaining.is_zero() {
                    return Err(error);
                }
                std::thread::sleep(remaining.min(WAL_RETRY_INTERVAL));
                if Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
    }
}

fn checkpoint(
    connection: &Connection,
    path: &Path,
    mode: CheckpointMode,
) -> Result<(), StorageError> {
    let statement = match mode {
        CheckpointMode::Passive => "PRAGMA wal_checkpoint(PASSIVE)",
        CheckpointMode::Truncate => "PRAGMA wal_checkpoint(TRUNCATE)",
    };
    // A busy result row is not an exception in the reference adapter. Do not
    // reinterpret passive checkpoint contention as a failed committed command.
    connection
        .query_row(statement, [], |row| row.get::<_, i64>(0))
        .map_err(|error| classify(error, "Checkpoint storage"))?;
    secure_files(path)
}

fn secure_directory(path: &Path) -> Result<(), StorageError> {
    let result = (|| {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            if fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o200 == 0) {
                return Err(std::io::Error::from_raw_os_error(nix::libc::EACCES));
            }
            builder.mode(0o700);
            builder.create(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(not(unix))]
        builder.create(path)?;
        Ok::<(), std::io::Error>(())
    })();
    result.map_err(|error| classify_io(error, "Cannot secure storage directory"))
}

fn secure_files(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            match fs::metadata(&file) {
                Ok(_) => fs::set_permissions(&file, fs::Permissions::from_mode(0o600))
                    .map_err(|error| classify_io(error, "Cannot secure storage file"))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(classify_io(error, "Cannot inspect storage file"));
                }
            }
        }
    }
    Ok(())
}
