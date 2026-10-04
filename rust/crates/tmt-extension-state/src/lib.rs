//! Private extension subtrees beneath a caller-supplied, core-reported data root.
//! Names, error presentation, entropy and key interpretation belong to consumers.
use nix::{
    errno::Errno,
    fcntl::{Flock, FlockArg, OFlag},
    unistd::Uid,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

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
    Flock::lock(file, FlockArg::LockExclusiveNonblock).map_err(|(_, error)| {
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
