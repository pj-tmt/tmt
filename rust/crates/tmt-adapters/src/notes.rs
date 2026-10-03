//! Owner-local Markdown notebook initialization and bounded reads for saved identities.

use crate::config::ConfigPaths;
use nix::{
    fcntl::{OFlag, openat},
    sys::stat::Mode,
};
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tmt_core::identity::NotesIdentityId;

/// Read-side bound only; this never truncates or limits the agent's Markdown file.
pub const NOTEBOOK_READ_LIMIT: usize = 1_048_576;

#[derive(Debug, PartialEq, Eq)]
pub struct Notebook {
    pub identity_id: String,
    pub name: String,
    pub content: String,
}

pub fn value(notebook: &Notebook) -> serde_json::Value {
    serde_json::json!({"identityId": notebook.identity_id, "name": notebook.name, "content": notebook.content})
}

pub fn encode(notebook: &Notebook) -> Vec<u8> {
    serde_json::to_vec(&value(notebook)).expect("notebook resource")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookError {
    InvalidIdentity,
    IdentityNotFound,
    SavedIdentityRequired,
    Missing,
    TooLarge,
    InvalidText,
    Unavailable,
}

impl NotebookError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidIdentity => "NOTEBOOK_INVALID_IDENTITY",
            Self::IdentityNotFound => "NOTEBOOK_IDENTITY_NOT_FOUND",
            Self::SavedIdentityRequired => "NOTEBOOK_SAVED_IDENTITY_REQUIRED",
            Self::Missing => "NOTEBOOK_NOT_FOUND",
            Self::TooLarge => "NOTEBOOK_TOO_LARGE",
            Self::InvalidText => "NOTEBOOK_INVALID_TEXT",
            Self::Unavailable => "NOTEBOOK_UNAVAILABLE",
        }
    }
}

/// The browser selects an identity, never a path. Reading does not initialize notes.
pub fn read(paths: &ConfigPaths, identity_id: &str) -> Result<Notebook, NotebookError> {
    read_with_open_error(paths, identity_id).map_err(|error| match error {
        NotebookReadError::Open(_) => NotebookError::Unavailable,
        NotebookReadError::Notebook(error) => error,
    })
}

pub(crate) enum NotebookReadError {
    Open(crate::storage::StorageError),
    Notebook(NotebookError),
}

impl From<NotebookError> for NotebookReadError {
    fn from(error: NotebookError) -> Self {
        Self::Notebook(error)
    }
}

pub(crate) fn read_with_open_error(
    paths: &ConfigPaths,
    identity_id: &str,
) -> Result<Notebook, NotebookReadError> {
    if !tmt_core::dispatch::canonical_id(identity_id) {
        return Err(NotebookError::InvalidIdentity.into());
    }
    let mut storage =
        crate::storage::Storage::open(&paths.database).map_err(NotebookReadError::Open)?;
    let pending = storage.find_active_identity_by_id(identity_id);
    let closed = storage.close();
    let identity = pending
        .map_err(|_| NotebookError::Unavailable)?
        .ok_or(NotebookError::IdentityNotFound)?;
    closed.map_err(|_| NotebookError::Unavailable)?;
    let notes_id = NotesIdentityId::try_from(&identity).map_err(|error| match error {
        tmt_core::identity::NotesIdentityError::SavedIdentityRequired => {
            NotebookError::SavedIdentityRequired
        }
        tmt_core::identity::NotesIdentityError::InvalidIdentityId => NotebookError::InvalidIdentity,
    })?;
    let bytes = read_existing(paths, &notes_id).map_err(|error| match error {
        crate::bounded_file::FileReadError::TooLarge => NotebookError::TooLarge,
        crate::bounded_file::FileReadError::Io(error)
            if error.kind() == io::ErrorKind::NotFound =>
        {
            NotebookError::Missing
        }
        crate::bounded_file::FileReadError::Io(_) => NotebookError::Unavailable,
    })?;
    Ok(Notebook {
        identity_id: identity.id,
        name: identity.name,
        content: String::from_utf8(bytes).map_err(|_| NotebookError::InvalidText)?,
    })
}

fn read_existing(
    paths: &ConfigPaths,
    identity_id: &NotesIdentityId,
) -> Result<Vec<u8>, crate::bounded_file::FileReadError> {
    use crate::bounded_file::{FileReadError, read_opened};
    read_opened(
        open_existing(paths, identity_id).map_err(FileReadError::Io)?,
        NOTEBOOK_READ_LIMIT,
    )
}

/// Project an existing saved-identity notebook without opening storage, creating
/// directories or reading its contents. The caller owns active-identity lookup.
pub fn existing_path(
    paths: &ConfigPaths,
    identity_id: &NotesIdentityId,
) -> io::Result<Option<PathBuf>> {
    let file = match open_existing(paths, identity_id) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Notes path is not a regular file."));
    }
    Ok(Some(paths.notes_layout(identity_id)?.2))
}

fn open_existing(paths: &ConfigPaths, identity_id: &NotesIdentityId) -> io::Result<File> {
    let (root, _, file) = paths.notes_layout(identity_id)?;
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_DIRECTORY).bits())
        .open(&root)?;
    let relative = file.strip_prefix(root).map_err(io::Error::other)?;
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::other("Invalid notes path component."));
        };
        let flags = OFlag::O_RDONLY
            | OFlag::O_NOFOLLOW
            | OFlag::O_CLOEXEC
            | OFlag::O_NONBLOCK
            | if components.peek().is_some() {
                OFlag::O_DIRECTORY
            } else {
                OFlag::empty()
            };
        directory = File::from(openat(&directory, Path::new(name), flags, Mode::empty())?);
    }
    Ok(directory)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotesPath {
    pub path: PathBuf,
    pub created: bool,
}

pub fn initialize(paths: &ConfigPaths, identity_id: &NotesIdentityId) -> io::Result<NotesPath> {
    let (global_dir, identity_dir, notes_file) = paths.notes_layout(identity_id)?;
    require_directory(&global_dir)?;
    create_private_directory(&global_dir.join("notes"))?;
    create_private_directory(&identity_dir)?;

    let created = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(OFlag::O_NOFOLLOW.bits())
        .open(&notes_file)
    {
        Ok(file) => {
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.sync_all()?;
            true
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            require_regular_file(&notes_file)?;
            false
        }
        Err(error) => return Err(error),
    };
    Ok(NotesPath {
        path: notes_file,
        created,
    })
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.mode(0o700);
    match builder.create(path) {
        Ok(()) => {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => require_directory(path),
        Err(error) => Err(error),
    }
}

fn require_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} is not a directory",
            path.display()
        )))
    }
}

fn require_regular_file(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} is not a regular file",
            path.display()
        )))
    }
}

#[cfg(test)]
mod tests;
