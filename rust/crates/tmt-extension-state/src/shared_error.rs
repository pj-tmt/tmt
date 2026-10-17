//! Errors shared by the new state entries; legacy filesystem errors stay stable.
use std::{
    error::Error as StdError,
    fmt, io,
    path::{Path, PathBuf},
};

pub(crate) const NEWER_STORE_REASON: &str = "store is newer than this binary; use the newer binary";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub acquired_at_ms: u64,
}

#[derive(Debug)]
pub enum StateError {
    Timeout {
        path: PathBuf,
        operation: &'static str,
        retry: String,
        may_not_be_saved: bool,
        source: Box<dyn StdError + Send + Sync>,
    },
    GuardBusy {
        path: PathBuf,
        holder: Option<Holder>,
        retry: String,
    },
    Unsafe {
        path: PathBuf,
        reason: &'static str,
    },
    Invalid {
        path: PathBuf,
        reason: &'static str,
    },
    Migration {
        path: PathBuf,
        reason: &'static str,
    },
    Io {
        path: PathBuf,
        operation: &'static str,
        source: io::Error,
    },
    Sqlite {
        path: PathBuf,
        operation: &'static str,
        source: rusqlite::Error,
    },
    Uncertain {
        path: PathBuf,
        operation: &'static str,
        retry: String,
        source: io::Error,
    },
    Cleanup {
        primary: Box<StateError>,
        source: io::Error,
    },
}
impl StateError {
    /// Distinguish future migration history without parsing display text.
    pub fn is_newer(&self) -> bool {
        matches!(self, Self::Migration { reason, .. } if *reason == NEWER_STORE_REASON)
    }

    pub fn holder(&self) -> Option<Holder> {
        match self {
            Self::GuardBusy { holder, .. } => *holder,
            _ => None,
        }
    }
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }
    pub fn may_not_be_saved(&self) -> bool {
        match self {
            Self::Timeout {
                may_not_be_saved: true,
                ..
            }
            | Self::Uncertain { .. } => true,
            Self::Cleanup { primary, .. } => primary.may_not_be_saved(),
            _ => false,
        }
    }
    pub(crate) fn io(path: &Path, operation: &'static str, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            operation,
            source,
        }
    }
    pub(crate) fn sqlite(path: &Path, operation: &'static str, source: rusqlite::Error) -> Self {
        Self::Sqlite {
            path: path.into(),
            operation,
            source,
        }
    }
    pub(crate) fn admission(path: &Path, error: super::Error) -> Self {
        match error {
            super::Error::Io(source)
            | super::Error::FileOpen(source)
            | super::Error::ReadOpen(source)
            | super::Error::DirectoryMissing(source) => Self::io(path, "admit state path", source),
            super::Error::Lock(source) => Self::io(
                path,
                "lock state",
                io::Error::from_raw_os_error(source as i32),
            ),
            super::Error::RootNotAbsolute | super::Error::InvalidFileName => Self::Invalid {
                path: path.into(),
                reason: "an absolute admitted state path is required",
            },
            super::Error::InvalidLength => Self::Invalid {
                path: path.into(),
                reason: "state exceeds its byte bound",
            },
            super::Error::UnsafeDirectory | super::Error::UnsafeFile => Self::Unsafe {
                path: path.into(),
                reason: "state must have the required owner, mode and regular-file type",
            },
            super::Error::Busy => Self::io(
                path,
                "lock state",
                io::Error::from(io::ErrorKind::WouldBlock),
            ),
        }
    }
}
impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout {
                path,
                operation,
                retry,
                may_not_be_saved,
                source,
            } => write!(
                f,
                "{}: {operation} timed out: {source}; {}. Retry: {retry}",
                path.display(),
                if *may_not_be_saved {
                    "the change may not be saved"
                } else {
                    "the change was not started"
                }
            ),
            Self::GuardBusy {
                path,
                holder,
                retry,
            } => {
                write!(f, "{}: guard is held; ", path.display())?;
                match holder {
                    Some(holder) => write!(
                        f,
                        "recorded holder pid {}, acquired at {} ms",
                        holder.pid, holder.acquired_at_ms
                    )?,
                    None => f.write_str("holder unknown")?,
                }
                write!(f, ". Retry: {retry}")
            }
            Self::Unsafe { path, reason }
            | Self::Invalid { path, reason }
            | Self::Migration { path, reason } => write!(f, "{}: {reason}", path.display()),
            Self::Io {
                path,
                operation,
                source,
            } => write!(f, "{}: {operation}: {source}", path.display()),
            Self::Sqlite {
                path,
                operation,
                source,
            } => write!(f, "{}: {operation}: {source}", path.display()),
            Self::Uncertain {
                path,
                operation,
                retry,
                source,
            } => write!(
                f,
                "{}: {operation}: {source}; new bytes may already be saved. Retry: {retry}",
                path.display()
            ),
            Self::Cleanup { primary, source } => {
                write!(f, "{primary}; staging cleanup also failed: {source}")
            }
        }
    }
}
impl StdError for StateError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Timeout { source, .. } => Some(source.as_ref()),
            Self::Io { source, .. } => Some(source),
            Self::Sqlite { source, .. } => Some(source),
            Self::Uncertain { source, .. } => Some(source),
            Self::Cleanup { primary, .. } => Some(primary.as_ref()),
            _ => None,
        }
    }
}

/// Consumer validation errors retain their type and always roll the change back.
#[derive(Debug)]
pub enum UpdateError<E> {
    State(StateError),
    Change(E),
}
impl<E> From<StateError> for UpdateError<E> {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}
impl<E: fmt::Display> fmt::Display for UpdateError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => error.fmt(f),
            Self::Change(error) => error.fmt(f),
        }
    }
}
impl<E: StdError + 'static> StdError for UpdateError<E> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::State(error) => Some(error),
            Self::Change(error) => Some(error),
        }
    }
}
