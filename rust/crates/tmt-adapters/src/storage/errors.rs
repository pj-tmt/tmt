use std::{error::Error, fmt, fs::OpenOptions, io, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageErrorCode {
    Busy,
    Corrupt,
    Permission,
    NotWritable,
    IncompatibleSchema,
    Migration,
    Unknown,
    Closed,
}

impl StorageErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Corrupt => "corrupt",
            Self::Permission => "permission",
            Self::NotWritable => "not-writable",
            Self::IncompatibleSchema => "incompatible-schema",
            Self::Migration => "migration",
            Self::Unknown => "unknown",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug)]
pub struct StorageError {
    pub code: StorageErrorCode,
    pub message: String,
    pub migration_version: Option<u32>,
    pub retryable: bool,
    cause: Option<Box<dyn Error + Send + Sync>>,
}

impl StorageError {
    pub(crate) fn new(code: StorageErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            migration_version: None,
            retryable: code == StorageErrorCode::Busy,
            cause: None,
        }
    }

    pub(crate) fn caused_by(mut self, cause: impl Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }

    pub(crate) fn migration(version: u32, cause: Self) -> Self {
        let retryable = cause.retryable;
        let mut error = Self::new(
            if cause.code == StorageErrorCode::NotWritable {
                StorageErrorCode::NotWritable
            } else {
                StorageErrorCode::Migration
            },
            format!("Migration {version} failed"),
        )
        .caused_by(cause);
        error.migration_version = Some(version);
        error.retryable = retryable;
        error
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause
            .as_deref()
            .map(|cause| cause as &(dyn Error + 'static))
    }
}

pub(crate) fn classify(error: rusqlite::Error, operation: &str) -> StorageError {
    use rusqlite::ErrorCode;
    let code = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => StorageErrorCode::Busy,
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => StorageErrorCode::Corrupt,
        Some(ErrorCode::ReadOnly | ErrorCode::PermissionDenied) => StorageErrorCode::NotWritable,
        Some(ErrorCode::CannotOpen) => StorageErrorCode::Permission,
        Some(ErrorCode::SystemIoFailure) if wal_permission_failure(&error) => {
            StorageErrorCode::NotWritable
        }
        _ => StorageErrorCode::Unknown,
    };
    StorageError::new(code, format!("{operation} failed")).caused_by(error)
}

/// SQLite's CANTOPEN also means missing paths and malformed files. Only a
/// separate OS permission result can identify the sandbox/read-only case.
pub(crate) fn classify_open(error: rusqlite::Error, operation: &str, path: &Path) -> StorageError {
    let companions = ["-wal", "-shm"].map(|suffix| {
        let mut value = path.as_os_str().to_os_string();
        value.push(suffix);
        std::path::PathBuf::from(value)
    });
    let denied = error.sqlite_error_code() == Some(rusqlite::ErrorCode::CannotOpen)
        && [
            path.to_path_buf(),
            companions[0].clone(),
            companions[1].clone(),
        ]
        .iter()
        .any(|candidate| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(candidate)
                .err()
                .is_some_and(|cause| is_write_denied(&cause))
        });
    if denied {
        StorageError::new(StorageErrorCode::NotWritable, format!("{operation} failed"))
            .caused_by(error)
    } else {
        classify(error, operation)
    }
}

pub(crate) fn classify_io(error: io::Error, operation: &str) -> StorageError {
    let code = if is_write_denied(&error) {
        StorageErrorCode::NotWritable
    } else {
        StorageErrorCode::Permission
    };
    StorageError::new(code, operation).caused_by(error)
}

fn is_write_denied(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == nix::libc::EACCES || code == nix::libc::EPERM || code == nix::libc::EROFS
    )
}

fn wal_permission_failure(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(details, _)
        if matches!(details.extended_code,
            rusqlite::ffi::SQLITE_IOERR_SHMOPEN | rusqlite::ffi::SQLITE_IOERR_SHMSIZE))
}

pub(crate) fn incompatible(message: impl Into<String>) -> StorageError {
    StorageError::new(StorageErrorCode::IncompatibleSchema, message)
}
