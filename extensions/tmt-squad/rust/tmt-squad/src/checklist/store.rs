//! One stable lock and atomic document publication per canonical room UUID.
use super::{
    Code, Error,
    model::{Document, Id},
};
use nix::{
    fcntl::{Flock, FlockArg, OFlag},
    unistd::getuid,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Implementation resource limits, independent of individual admitted fields.
const LIMIT: u64 = 64 * 1024 * 1024;

pub(super) struct Store {
    root: PathBuf,
    room_id: Id,
    subtree: &'static str,
}
impl Store {
    pub(super) fn new(root: &Path, room_id: Id, subtree: &'static str) -> Result<Self, Error> {
        if !root.is_absolute() {
            return Err(Error::storage(
                "storage.root must return an absolute dataRoot.",
            ));
        }
        // Aliases are allowed only in the trusted public root. Extension children
        // are checked separately and never followed as directory symlinks.
        let root = match fs::canonicalize(root) {
            Ok(root) => root,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => root.into(),
            Err(e) => return Err(Error::storage(e)),
        };
        Ok(Self {
            root,
            room_id,
            subtree,
        })
    }
    pub(super) fn room_id(&self) -> &Id {
        &self.room_id
    }
    fn directory(&self) -> PathBuf {
        self.root
            .join(self.subtree)
            .join("checklist")
            .join(self.room_id.as_str())
    }

    fn directories(&self, create: bool) -> Result<bool, Error> {
        match fs::metadata(&self.root) {
            Ok(m) if m.is_dir() => {}
            Err(e) if !create && e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            _ => {
                return Err(Error::storage(
                    "The public storage root is unavailable or not a directory.",
                ));
            }
        }
        let mut path = self.root.clone();
        for component in [self.subtree, "checklist", self.room_id.as_str()] {
            let parent = path.clone();
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(m) => directory(&m)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if !create {
                        return Ok(false);
                    }
                    match fs::DirBuilder::new().mode(0o700).create(&path) {
                        Ok(()) => File::open(&parent)
                            .and_then(|f| f.sync_all())
                            .map_err(Error::storage)?,
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(e) => return Err(Error::storage(e)),
                    }
                    directory(&fs::symlink_metadata(&path).map_err(Error::storage)?)?;
                }
                Err(e) => return Err(Error::storage(e)),
            }
        }
        Ok(true)
    }
    fn open(&self, name: &str, write: bool, create: bool) -> Result<Option<File>, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(write)
            .create(create)
            .truncate(false)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(self.directory().join(name));
        match file {
            Ok(file) => {
                regular(&file.metadata().map_err(Error::storage)?)?;
                Ok(Some(file))
            }
            Err(e) if !create && e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::storage(e)),
        }
    }
    fn lock(&self, create: bool) -> Result<Option<Flock<File>>, Error> {
        if !self.directories(create)? {
            return Ok(None);
        }
        // An existing document without its stable lock is unsafe, even on write;
        // do not invent another lock that could split concurrent publications.
        if self.open("items.lock", false, false)?.is_none()
            && self.open("items.json", false, false)?.is_some()
        {
            return Err(Error::storage(
                "Checklist document exists without its stable lock.",
            ));
        }
        let Some(file) = self.open("items.lock", create, create)? else {
            return Ok(None);
        };
        Flock::lock(
            file,
            if create {
                FlockArg::LockExclusiveNonblock
            } else {
                FlockArg::LockSharedNonblock
            },
        )
        .map(Some)
        .map_err(|(_, e)| Error::storage(format!("Checklist lock unavailable: {e}")))
    }
    fn read_locked(&self) -> Result<Option<Document>, Error> {
        let Some(file) = self.open("items.json", false, false)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(Error::storage)?;
        if bytes.len() as u64 > LIMIT {
            return Err(Error::storage(
                "Checklist document exceeds the 64 MiB implementation limit.",
            ));
        }
        let value = serde_json::from_slice(&bytes).map_err(Error::storage)?;
        let document = Document::decode(&value)?;
        if document.room_id != self.room_id {
            return Err(Error::storage(
                "Checklist document belongs to another room UUID.",
            ));
        }
        Ok(Some(document))
    }
    pub(super) fn read(&self) -> Result<Option<Document>, Error> {
        let Some(_lock) = self.lock(false)? else {
            return Ok(None);
        };
        self.read_locked()
    }
    pub(super) fn update<T>(
        &self,
        change: impl FnOnce(&mut Option<Document>) -> Result<T, Error>,
        before_publish: impl FnOnce() -> Result<(), Error>,
    ) -> Result<T, Error> {
        let _lock = self
            .lock(true)?
            .expect("create acquires a stable lock or fails");
        let mut document = self.read_locked()?;
        let before = document.clone();
        let result = change(&mut document)?;
        if document == before {
            before_publish()?;
            return Ok(result);
        }
        let document =
            document.ok_or_else(|| Error::storage("Checklist documents cannot be removed."))?;
        document.validate()?;
        if document.room_id != self.room_id {
            return Err(Error::storage("Candidate room UUID changed."));
        }
        let bytes = serde_json::to_vec(&document.encode()).map_err(Error::storage)?;
        if bytes.len() as u64 > LIMIT {
            return Err(Error::storage(
                "Checklist document exceeds the 64 MiB implementation limit.",
            ));
        }
        let permissions = self
            .open("items.json", false, false)?
            .map(|f| f.metadata().map(|m| m.permissions()))
            .transpose()
            .map_err(Error::storage)?;
        let temporary = self.directory().join("items.tmp");
        if let Some(file) = self.open("items.tmp", false, false)? {
            if file.metadata().map_err(Error::storage)?.len() > LIMIT {
                return Err(Error::storage("Unsafe oversized checklist temporary."));
            }
            fs::remove_file(&temporary).map_err(Error::storage)?;
        }
        let mut created = false;
        let staged = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
                .open(&temporary)
                .map_err(Error::storage)?;
            created = true;
            if let Some(permissions) = permissions {
                file.set_permissions(permissions).map_err(Error::storage)?;
            }
            file.write_all(&bytes).map_err(Error::storage)?;
            #[cfg(test)]
            test_stage(Stage::FileSync, &self.directory()).map_err(Error::storage)?;
            file.sync_all().map_err(Error::storage)?;
            before_publish()?;
            #[cfg(test)]
            test_stage(Stage::Rename, &self.directory()).map_err(Error::storage)?;
            fs::rename(&temporary, self.directory().join("items.json")).map_err(Error::storage)
        })();
        if let Err(error) = staged {
            if !created {
                return Err(error);
            }
            // Only the exclusively created staging path is removed. Preserve
            // unsafe/foreign files rather than cleaning them by analogy.
            let cleanup = fs::remove_file(&temporary);
            return Err(match cleanup {
                Ok(()) => error,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => error,
                Err(e) => Error::storage(format!("{error}; temporary cleanup failed: {e}")),
            });
        }
        let durable = (|| {
            #[cfg(test)]
            test_stage(Stage::DirectorySync, &self.directory())?;
            File::open(self.directory())?.sync_all()?;
            #[cfg(test)]
            test_stage(Stage::Acknowledgement, &self.directory())?;
            Ok::<_, std::io::Error>(())
        })();
        durable.map_err(|e| Error::new(Code::OutcomeUnknown, format!("Checklist replacement may have committed; original operation outcome remains unknown: {e}")))?;
        Ok(result)
    }
}
fn directory(metadata: &fs::Metadata) -> Result<(), Error> {
    if !metadata.is_dir() || metadata.uid() != getuid().as_raw() {
        return Err(Error::storage(
            "Checklist path is not an owned real directory.",
        ));
    }
    Ok(())
}
fn regular(metadata: &fs::Metadata) -> Result<(), Error> {
    if !metadata.is_file() || metadata.uid() != getuid().as_raw() || metadata.nlink() != 1 {
        return Err(Error::storage(
            "Checklist path is not an owned regular file with one link.",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    FileSync,
    Rename,
    DirectorySync,
    Acknowledgement,
}
#[cfg(test)]
type Fault = Box<dyn FnMut(Stage, &Path) -> std::io::Result<()>>;
#[cfg(test)]
thread_local! { static FAULT: std::cell::RefCell<Option<Fault>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
fn test_stage(stage: Stage, directory: &Path) -> std::io::Result<()> {
    FAULT.with_borrow_mut(|fault| match fault {
        Some(fault) => fault(stage, directory),
        None => Ok(()),
    })
}
#[cfg(test)]
pub(super) fn with_fault<T>(fault: Fault, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<Fault>);
    impl Drop for Reset {
        fn drop(&mut self) {
            FAULT.with_borrow_mut(|f| *f = self.0.take());
        }
    }
    let _reset = Reset(FAULT.replace(Some(fault)));
    run()
}
#[cfg(test)]
mod tests;
