//! Current-byte updates with one lock and one reserved atomic-rename staging name.
use crate::{Layout, StateError, UpdateError, locking};
use nix::{fcntl::OFlag, unistd::Uid};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

pub struct Document {
    state_root: PathBuf,
    destination: PathBuf,
    limit: u64,
    retry: String,
}
impl Document {
    pub fn new(
        state_root: &Path,
        destination: &Path,
        limit: u64,
        retry: &str,
    ) -> Result<Self, StateError> {
        if !state_root.is_absolute()
            || !destination.is_absolute()
            || destination.file_name().is_none()
            || limit == 0
            || retry.is_empty()
        {
            return Err(StateError::Invalid {
                path: destination.into(),
                reason: "document needs absolute state and destination paths, a byte bound and exact retry",
            });
        }
        let name = destination
            .file_name()
            .expect("constructor checked name")
            .as_bytes();
        let suffix = b".tmt-staging";
        if name.starts_with(b".")
            && name.len() >= suffix.len()
            && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
        {
            return Err(StateError::Invalid {
                path: destination.into(),
                reason: "destination name is reserved for document staging",
            });
        }
        Ok(Self {
            state_root: state_root.into(),
            destination: destination.into(),
            limit,
            retry: retry.into(),
        })
    }

    pub fn read(&self) -> Result<Option<Vec<u8>>, StateError> {
        current(&self.destination, self.limit, false).map(|(bytes, _)| bytes)
    }

    fn destination(&self) -> Result<PathBuf, StateError> {
        let parent = self
            .destination
            .parent()
            .ok_or_else(|| StateError::Invalid {
                path: self.destination.clone(),
                reason: "document has no parent",
            })?;
        let parent = fs::canonicalize(parent)
            .map_err(|e| StateError::io(parent, "resolve document parent", e))?;
        Ok(parent.join(
            self.destination
                .file_name()
                .expect("constructor checked name"),
        ))
    }

    fn acquire(&self, destination: &Path) -> Result<DocumentLock, StateError> {
        let parent = destination.parent().expect("admitted destination");
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(parent)
            .map_err(|e| StateError::io(parent, "open document directory", e))?;
        let metadata = directory
            .metadata()
            .map_err(|e| StateError::io(parent, "admit document directory", e))?;
        if !metadata.is_dir()
            || metadata.uid() != Uid::effective().as_raw()
            || metadata.mode() & 0o022 != 0
        {
            return Err(StateError::Unsafe {
                path: parent.into(),
                reason: "document parent must be owned and not writable by other users",
            });
        }
        let locks = Layout::open(&self.state_root, "document-locks", &[])
            .map_err(|e| StateError::admission(&self.state_root, e))?;
        let path = locks.directory.join(lock_name(&metadata));
        let guard = locking::acquire(&path, true, &self.retry).map_err(|mut error| {
            if let StateError::Timeout { path, .. } = &mut error {
                *path = destination.into();
            }
            error
        })?;
        Ok(DocumentLock {
            _guard: guard,
            _directory: directory,
        })
    }

    /// Explicit recovery is also useful after an uncatchable process death.
    pub fn recover(&self) -> Result<(), StateError> {
        let destination = self.destination()?;
        let _guard = self.acquire(&destination)?;
        cleanup_reserved(&staging_path(&destination))?;
        sync_parent(&destination)
            .map_err(|e| StateError::io(&destination, "sync recovered document directory", e))
    }

    pub fn update<E>(
        &self,
        change: impl FnOnce(Option<&[u8]>) -> Result<Vec<u8>, E>,
    ) -> Result<(), UpdateError<E>> {
        let destination = self.destination()?;
        let _guard = self.acquire(&destination)?;
        let temporary = staging_path(&destination);
        cleanup_reserved(&temporary)?;
        let (bytes, permissions) = current(&destination, self.limit, true)?;
        let next = change(bytes.as_deref()).map_err(UpdateError::Change)?;
        if next.len() as u64 > self.limit {
            return Err(StateError::Invalid {
                path: destination,
                reason: "updated document exceeds its byte bound",
            }
            .into());
        }
        if bytes.as_deref() == Some(next.as_slice()) {
            return sync_parent(&destination)
                .map_err(|e| StateError::io(&destination, "sync document directory", e).into());
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(&temporary)
            .map_err(|e| StateError::io(&temporary, "create document staging", e))?;
        let identity = match file.metadata() {
            Ok(identity) => identity,
            Err(source) => {
                let primary = StateError::io(&temporary, "admit document staging", source);
                return Err(match cleanup_reserved(&temporary) {
                    Ok(()) => primary,
                    Err(error) => StateError::Cleanup {
                        primary: Box::new(primary),
                        source: io::Error::other(error),
                    },
                }
                .into());
            }
        };
        let mut stage = Staging {
            file,
            path: temporary,
            device: identity.dev(),
            inode: identity.ino(),
        };
        #[cfg(test)]
        tests::checkpoint("created");
        let result = (|| {
            stage
                .file
                .write_all(&next)
                .map_err(|e| StateError::io(&destination, "write document staging", e))?;
            #[cfg(test)]
            tests::checkpoint("written");
            if let Some(permissions) = permissions {
                stage.file.set_permissions(permissions).map_err(|e| {
                    StateError::io(&destination, "preserve document permissions", e)
                })?;
            }
            stage
                .file
                .sync_all()
                .map_err(|e| StateError::io(&destination, "sync document staging", e))?;
            #[cfg(test)]
            tests::checkpoint("synced");
            fs::rename(&stage.path, &destination)
                .map_err(|e| StateError::io(&destination, "publish document", e))?;
            #[cfg(test)]
            tests::checkpoint("published");
            let synced = sync_parent(&destination);
            #[cfg(test)]
            let synced = tests::directory_sync_result(synced);
            synced.map_err(|source| StateError::Uncertain {
                path: destination.clone(),
                operation: "sync published document directory",
                retry: self.retry.clone(),
                source,
            })
        })();
        let cleanup = stage.cleanup();
        match (result, cleanup) {
            (Err(primary), Err(source)) => Err(StateError::Cleanup {
                primary: Box::new(primary),
                source,
            }
            .into()),
            (Err(error), _) => Err(error.into()),
            (Ok(()), Err(error)) => {
                Err(StateError::io(&destination, "remove document staging", error).into())
            }
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}
fn current(
    path: &Path,
    limit: u64,
    writing: bool,
) -> Result<(Option<Vec<u8>>, Option<Permissions>), StateError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((None, None)),
        Err(e) => return Err(StateError::io(path, "read document", e)),
    };
    let metadata = file
        .metadata()
        .map_err(|e| StateError::io(path, "admit document", e))?;
    if !metadata.is_file()
        || metadata.uid() != Uid::effective().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return Err(StateError::Unsafe {
            path: path.into(),
            reason: "document must be an owned regular file not writable by other users",
        });
    }
    if writing && metadata.mode() & 0o200 == 0 {
        return Err(StateError::Unsafe {
            path: path.into(),
            reason: "document owner has no write permission",
        });
    }
    let permissions = metadata.permissions();
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| StateError::io(path, "read document", e))?;
    if bytes.len() as u64 > limit {
        return Err(StateError::Invalid {
            path: path.into(),
            reason: "document exceeds its byte bound",
        });
    }
    Ok((Some(bytes), Some(permissions)))
}
fn staging_path(destination: &Path) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(destination.file_name().expect("admitted destination"));
    name.push(".tmt-staging");
    destination.with_file_name(name)
}
fn cleanup_reserved(path: &Path) -> Result<(), StateError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(StateError::io(path, "admit reserved staging", e)),
    };
    if !metadata.is_file() || metadata.uid() != Uid::effective().as_raw() {
        return Err(StateError::Unsafe {
            path: path.into(),
            reason: "reserved document staging is not an owned regular file",
        });
    }
    fs::remove_file(path).map_err(|e| StateError::io(path, "recover reserved document staging", e))
}
fn sync_parent(destination: &Path) -> io::Result<()> {
    File::open(destination.parent().expect("admitted destination"))?.sync_all()
}

/// One opened directory identity admits all writers, including case aliases.
/// Retaining the directory handle prevents inode reuse while its lock is held.
fn lock_name(directory: &std::fs::Metadata) -> String {
    format!("{:016x}-{:016x}.lock", directory.dev(), directory.ino())
}
struct DocumentLock {
    _guard: nix::fcntl::Flock<File>,
    _directory: File,
}

struct Staging {
    file: File,
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl Staging {
    fn cleanup(&self) -> io::Result<()> {
        match fs::symlink_metadata(&self.path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
            Ok(metadata)
                if metadata.is_file()
                    && metadata.dev() == self.device
                    && metadata.ino() == self.inode =>
            {
                fs::remove_file(&self.path)
            }
            Ok(_) => Err(io::Error::other(
                "reserved staging no longer names this writer's file",
            )),
        }
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
