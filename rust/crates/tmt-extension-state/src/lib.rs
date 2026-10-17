//! Private extension subtrees beneath a caller-supplied, core-reported data root.
//! Names, error presentation, entropy and key interpretation belong to consumers.
//!
//! `Layout` admits an extension-selected private subtree beneath the injected
//! absolute core-reported data root. It preserves existing root permissions and
//! canonicalizes aliases only in that trusted root. Private directories must be
//! owned 0700 directories; allowlisted files must be owned regular 0600 files.
//! File opens use no-follow and nonblocking flags. Read-only lookup and lock probes
//! create nothing. Reads retain the caller's byte bound.
//!
//! `Publication` holds a nonblocking lock through stale temporary admission,
//! cleanup and publication. Only the selected prefix plus 32 lowercase hex digits
//! matches a temporary; unsafe or oversized matches refuse and foreign names remain.
//! A staged file borrows that guard and links create-only after writing and syncing
//! its bytes. The extension retains entropy, key interpretation, error mapping and
//! its existing staging/removal/directory-sync failure ordering. Store schemas,
//! identity derivation and Remote's serve-lock proof remain extension-owned; this
//! leaf discovers no roots and interprets no Core or provider state.
//!
//! ## Shared mutation entries
//!
//! The caller supplies paths, schemas, codecs, byte bounds and the exact retry
//! command. The shared entries own admission, locking and durable publication;
//! they never execute a retry or discover application roots. Existing `Layout`
//! and `Publication` callers keep their present behavior until migrated.
//!
//! `Store::open` opens a caller-owned private SQLite file and private sidecars, enables
//! foreign keys, verifies WAL and uses Core's synchronous NORMAL policy without
//! importing Core's schema. Consumer-supplied ordered migrations run forward-only
//! in an immediate transaction; future or non-prefix history is refused. A write
//! waits at most five seconds for BEGIN IMMEDIATE. Its callback runs once after
//! admission and is never replayed. Busy at COMMIT is the same typed timeout,
//! with an explicit "may not be saved" consequence and the caller's exact retry;
//! there is no silent commit retry. Other SQLite/I/O causes remain distinguishable.
//! Reads set a five-second busy wait; statement BUSY remains a retryable consumer error.
//! `StateError::is_newer` distinguishes newer history from other migration
//! refusals; consumers must not classify errors by display text.
//! Existing database and sidecar files are admitted by lstat without opening extra
//! descriptors whose close could release another live connection's POSIX locks.
//! Concurrent first creation of one store by threads in the same process can still
//! release a sibling connection's POSIX locks through descriptor close. Create the
//! store in one place before sharing it, or serialize first creation in the consumer.
//!
//! `Store::from_admitted_connection` instead takes an already-admitted writable
//! file connection, without migrations or changes to its journal, synchronous,
//! foreign-key or page-limit policy. WAL retains zero-wait COMMIT; other journal
//! modes let COMMIT wait within the same five-second window from the BEGIN
//! attempt, including callback time. Callbacks are never replayed. Query-only
//! read snapshots restore the connection policy and, in rollback journals, delay
//! writers' COMMIT, so keep reads short.
//! The supplied connection must be in autocommit (no transaction or savepoint).
//! Admit the main file before connecting. While a connection lives, never open
//! and close that file or its journal/WAL/shared-memory sidecars: closing any
//! descriptor releases the process's POSIX locks on that file.
//!
//! `Document` updates current bytes under an exclusive lock: admit, read, validate,
//! change once, stage, write, fsync, rename, then fsync the destination directory.
//! One writer lock per destination directory lives under the caller's private
//! TMT state root, keyed by the device and inode of the opened directory, whose
//! handle stays alive through the lock. Distinct documents in that directory
//! deliberately serialize within the same five-second admission. This also
//! serializes path/case aliases without a filesystem-collation rule; no persistent
//! sibling lock is created in a project directory. Existing document permissions
//! are preserved; new documents are 0600. Missing reads create nothing. Unsafe or oversized
//! inputs refuse before the callback or publication. Write-lock admission waits
//! at most five seconds and does not renew its budget.
//!
//! Every destination reserves one sibling staging name, `.<name>.tmt-staging`.
//! Destination names matching that reserved pattern case-insensitively are refused,
//! so a document can never name another document's staging file.
//! Under the same directory lock, remove a previous entry at that exact name only
//! when it is a regular file owned by this user; otherwise refuse with the named
//! cause. Create staging exclusively, never follow a link, and remove it on each
//! normal or error return. No directory sweep or intent registry is introduced.
//! After an uncatchable process death the destination is immediately old or new;
//! the next update or recovery under the same directory lock removes owned staging
//! (including a different case alias on a case-insensitive volume) and
//! leaves the project directory clean. There is no instant-cleanup promise after
//! SIGKILL and no daemon. A failure after rename is reported as uncertain, never
//! as a successful rollback; the original failure keeps precedence over cleanup.
//!
//! `Guard` is fail-fast ownership, using the existing OS-lock mechanism rather
//! than a data-write queue. Contention names the holder PID and acquisition start
//! time when recorded; legacy, unpublished or invalid metadata is explicitly
//! unknown. The record is diagnostic, not process-incarnation evidence, and never
//! authorizes killing a process or unlinking an active lock. Dropping a held guard
//! releases the OS lock without deleting another owner's state.
//!
//! The new entries share one typed timeout carrying the store/path, operation,
//! preserved cause and exact retry. Guard contention is a separate typed outcome.
//! Five seconds bounds write-lock admission, not consumer computation. Callers
//! retain their own CLI codes, formatting and schemas. Process-crash recovery
//! tests prove old/new state; they do not simulate power loss.
use nix::{
    errno::Errno,
    fcntl::{Flock, OFlag},
    unistd::Uid,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

mod document;
mod guard;
mod locking;
mod shared_error;
mod store;
pub use document::Document;
pub use guard::Guard;
pub use shared_error::{Holder, StateError, UpdateError};
pub use store::{Migration, Store};

/// Operation distinctions retain each consumer's existing error interpretation.
#[derive(Debug)]
pub enum Error {
    RootNotAbsolute,
    UnsafeDirectory,
    UnsafeFile,
    InvalidFileName,
    InvalidLength,
    Busy,
    DirectoryMissing(io::Error),
    FileOpen(io::Error),
    ReadOpen(io::Error),
    Io(io::Error),
    Lock(Errno),
}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone)]
pub struct Layout {
    pub directory: PathBuf,
    files: &'static [&'static str],
}
impl Layout {
    pub fn open(
        data_root: &Path,
        subtree: &str,
        files: &'static [&'static str],
    ) -> Result<Self, Error> {
        check_root_and_names(data_root, subtree, files)?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(data_root)?;
        // Only the trusted core-selected root may contain aliases such as macOS /var.
        let data_root = fs::canonicalize(data_root)?;
        let directory = data_root.join(subtree);
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => File::open(&data_root)?.sync_all()?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        Self::admit(directory, files)
    }

    /// Admit without creating the root, subtree or any files.
    pub fn existing(
        data_root: &Path,
        subtree: &str,
        files: &'static [&'static str],
    ) -> Result<Option<Self>, Error> {
        check_root_and_names(data_root, subtree, files)?;
        let data_root = match fs::canonicalize(data_root) {
            Ok(root) => root,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        match Self::admit(data_root.join(subtree), files) {
            Ok(layout) => Ok(Some(layout)),
            Err(Error::DirectoryMissing(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn admit(directory: PathBuf, files: &'static [&'static str]) -> Result<Self, Error> {
        let metadata = fs::symlink_metadata(&directory).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Error::DirectoryMissing(error)
            } else {
                Error::Io(error)
            }
        })?;
        if !metadata.is_dir()
            || metadata.uid() != Uid::effective().as_raw()
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(Error::UnsafeDirectory);
        }
        Ok(Self { directory, files })
    }

    fn path(&self, name: &str) -> Result<PathBuf, Error> {
        if !self.files.contains(&name) {
            return Err(Error::InvalidFileName);
        }
        Ok(self.directory.join(name))
    }

    pub fn file(&self, name: &str) -> Result<File, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(self.path(name)?)
            .map_err(Error::FileOpen)?;
        validate_file(&file)?;
        Ok(file)
    }

    /// Open an admitted private file for reading, without creating it.
    pub fn read_file(&self, name: &str) -> Result<File, Error> {
        read_file(&self.path(name)?)
    }

    /// Read at most the caller's admission bound, without creating a missing file.
    pub fn read(&self, name: &str, limit: u64) -> Result<Vec<u8>, Error> {
        let file = self.read_file(name)?;
        let mut bytes = Vec::new();
        file.take(limit).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub fn lock(&self, name: &str) -> Result<Flock<File>, Error> {
        lock(self.file(name)?)
    }

    /// Probe an existing lock without creating it or retaining a successful lock.
    pub fn running(&self, name: &str) -> Result<bool, Error> {
        let file = match self.read_file(name) {
            Ok(file) => file,
            Err(Error::ReadOpen(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        match lock(file) {
            Ok(_lock) => Ok(false),
            Err(Error::Busy) => Ok(true),
            Err(error) => Err(error),
        }
    }

    pub fn sync(&self) -> io::Result<()> {
        File::open(&self.directory)?.sync_all()
    }

    /// Publication and stale cleanup share a nonblocking lock for their full lifetime.
    pub fn publication(
        &self,
        key_file: &str,
        lock_file: &str,
        temporary_prefix: &str,
        max_bytes: u64,
    ) -> Result<Publication<'_>, Error> {
        let destination = self.path(key_file)?;
        if !single_name(temporary_prefix) {
            return Err(Error::InvalidFileName);
        }
        let guard = self.lock(lock_file)?;
        let publication = Publication {
            layout: self,
            destination,
            temporary_prefix: temporary_prefix.to_owned(),
            _guard: guard,
        };
        publication.cleanup(max_bytes)?;
        Ok(publication)
    }
}

fn single_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(part)) if part == name)
        && components.next().is_none()
}
fn check_root_and_names(root: &Path, subtree: &str, files: &[&str]) -> Result<(), Error> {
    if !root.is_absolute() {
        return Err(Error::RootNotAbsolute);
    }
    if !single_name(subtree) || files.iter().any(|name| !single_name(name)) {
        return Err(Error::InvalidFileName);
    }
    Ok(())
}
fn validate_file(file: &File) -> Result<(), Error> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != Uid::effective().as_raw()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(Error::UnsafeFile);
    }
    Ok(())
}
fn read_file(path: &Path) -> Result<File, Error> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
        .map_err(Error::ReadOpen)?;
    validate_file(&file)?;
    Ok(file)
}
fn lock(file: File) -> Result<Flock<File>, Error> {
    locking::try_file(file).map_err(|(_, error)| {
        if error == Errno::EWOULDBLOCK {
            Error::Busy
        } else {
            Error::Lock(error)
        }
    })
}

/// Holds the publication lock; consumers retain their cleanup/error ordering.
pub struct Publication<'a> {
    layout: &'a Layout,
    destination: PathBuf,
    temporary_prefix: String,
    _guard: Flock<File>,
}
impl Publication<'_> {
    pub fn exists(&self) -> io::Result<bool> {
        self.destination.try_exists()
    }

    fn temporary_path(&self, nonce: &[u8; 16]) -> PathBuf {
        let suffix: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        self.layout
            .directory
            .join(format!("{}{suffix}", self.temporary_prefix))
    }

    pub fn stage(&self, nonce: &[u8; 16]) -> io::Result<Temporary<'_>> {
        let path = self.temporary_path(nonce);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        Ok(Temporary {
            file,
            path,
            destination: &self.destination,
        })
    }

    pub fn discard(&self, nonce: &[u8; 16]) -> io::Result<()> {
        fs::remove_file(self.temporary_path(nonce))
    }

    fn cleanup(&self, max_bytes: u64) -> Result<(), Error> {
        let mut removed = false;
        for entry in fs::read_dir(&self.layout.directory)? {
            let entry = entry?;
            if !entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix(&self.temporary_prefix))
                .is_some_and(|suffix| {
                    suffix.len() == 32
                        && suffix
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
            {
                continue;
            }
            let file = read_file(&entry.path())?;
            if file.metadata()?.len() > max_bytes {
                return Err(Error::InvalidLength);
            }
            fs::remove_file(entry.path())?;
            removed = true;
        }
        if removed {
            self.layout.sync()?;
        }
        Ok(())
    }
}

/// A create-new 0600 file; linking occurs only after all bytes have been synced.
pub struct Temporary<'a> {
    file: File,
    path: PathBuf,
    destination: &'a Path,
}
impl Temporary<'_> {
    pub fn write_and_link(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)?;
        self.file.sync_all()?;
        // Create-only: a concurrent publisher's key wins and ours is discarded.
        match fs::hard_link(&self.path, self.destination) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => Err(error),
            _ => Ok(()),
        }
    }
}
