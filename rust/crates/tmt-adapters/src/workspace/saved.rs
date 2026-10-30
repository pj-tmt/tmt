//! Named immutable recovery documents share the automatic publisher's lock.

use super::{Recovery, Store, generations};
use crate::{bounded_file, config::ConfigPaths, file_lock};
use std::{
    fmt, fs, io,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use tmt_core::{
    content_digest::sha256,
    workspace::{
        MAX_BYTES, MAX_SAVED_SNAPSHOTS, SnapshotName, SnapshotSelector, WorkspaceSnapshot,
        generations::WorkspaceGeneration,
    },
};

#[derive(Debug)]
pub enum Error {
    NotFound,
    Exists,
    Limit,
    Changed,
    Busy,
    Unavailable(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("The selected workspace snapshot was not found."),
            Self::Exists => f.write_str("That workspace snapshot name already exists."),
            Self::Limit => write!(
                f,
                "Each workspace store permits at most {MAX_SAVED_SNAPSHOTS} named snapshots; remove one first."
            ),
            Self::Changed => {
                f.write_str("The workspace snapshot changed; preview it again before restoring.")
            }
            Self::Busy => {
                f.write_str("Workspace publication is busy; try again after it completes.")
            }
            Self::Unavailable(error) => write!(f, "Workspace snapshot unavailable: {error}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(error) => Some(error),
            _ => None,
        }
    }
}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::Unavailable(error)
        }
    }
}

#[derive(Debug)]
pub struct Document<D = WorkspaceSnapshot> {
    pub snapshot: D,
    pub digest: String,
    pub generation: Option<WorkspaceGeneration>,
    pub protected: bool,
    pub requires_confirmation: bool,
    pub baseline: Option<WorkspaceGeneration>,
    bytes: Vec<u8>,
}

fn destination(directory: &Path, name: Option<&SnapshotName>) -> PathBuf {
    directory.join(name.map_or_else(
        || "latest.json".to_owned(),
        |name| format!("saved-{}.json", name.as_str()),
    ))
}

/// Reads do not create directories. Refuse a foreign or symlinked workspace root.
fn directory(paths: &ConfigPaths, store: Store<'_>) -> Result<PathBuf, Error> {
    let directory = store.directory(paths);
    for path in [paths.global_dir.join("workspace"), directory.clone()] {
        let metadata = fs::symlink_metadata(&path)?;
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_dir()
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(Error::Unavailable(io::Error::other(
                "Workspace directory must be private and owned.",
            )));
        }
    }
    Ok(directory)
}

/// The tmux sockets that have a recovery store, read from each store's newest document. A
/// store's directory name is a digest, so its socket is known only from what it holds. This is
/// advisory: an unreadable store is left out, and at most 256 directories are examined.
pub fn socket_stores(paths: &ConfigPaths) -> Vec<String> {
    let root = paths.global_dir.join("workspace");
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut sockets = entries
        .take(256)
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            tmt_core::content_digest::is_sha256(&name).then_some(name)
        })
        .filter_map(|name| {
            let directory = root.join(&name);
            let bytes = match generations::read(&directory).ok().flatten() {
                Some(catalogue) => {
                    generations::acquire::<WorkspaceSnapshot>(&directory, catalogue.latest()?)
                        .ok()?
                }
                None => {
                    bounded_file::read_no_follow(&destination(&directory, None), MAX_BYTES).ok()?
                }
            };
            let socket = <WorkspaceSnapshot as Recovery>::decode(&bytes)
                .ok()?
                .server
                .socket;
            (sha256(socket.as_bytes()) == name).then_some(socket)
        })
        .collect::<Vec<_>>();
    sockets.sort();
    sockets
}

/// The digest and topology come from one acquisition. Restore never reopens it.
pub fn acquire<D: Recovery>(
    paths: &ConfigPaths,
    store: Store<'_>,
    name: Option<&SnapshotName>,
    expect: Option<&str>,
) -> Result<Document<D>, Error> {
    let directory = directory(paths, store)?;
    let bytes = bounded_file::read_no_follow(&destination(&directory, name), MAX_BYTES).map_err(
        |error| match error {
            bounded_file::FileReadError::Io(error) => Error::from(error),
            error => Error::Unavailable(io::Error::other(error)),
        },
    )?;
    document(bytes, store, expect)
}

fn document<D: Recovery>(
    bytes: Vec<u8>,
    store: Store<'_>,
    expect: Option<&str>,
) -> Result<Document<D>, Error> {
    let snapshot = D::decode(&bytes)?;
    if !snapshot.belongs_to(store) {
        return Err(Error::Unavailable(io::Error::new(
            io::ErrorKind::InvalidData,
            "Workspace socket differs from its selection.",
        )));
    }
    let digest = sha256(&bytes);
    if expect.is_some_and(|expected| expected != digest) {
        return Err(Error::Changed);
    }
    Ok(Document {
        snapshot,
        digest,
        generation: None,
        protected: false,
        requires_confirmation: false,
        baseline: None,
        bytes,
    })
}

/// One immutable acquisition is retained through preview/restore; the index
/// chooses recovery input, never authorizes a binding or a provider launch.
pub fn acquire_selected<D: Recovery>(
    paths: &ConfigPaths,
    store: Store<'_>,
    selector: Option<&SnapshotSelector>,
    expect: Option<&str>,
) -> Result<Document<D>, Error> {
    if let Some(SnapshotSelector::Named(name)) = selector {
        let mut document = acquire::<D>(paths, store, Some(name), expect)?;
        if let Some(catalogue) = generations::read(&directory(paths, store)?)?
            && let Some(protected) = catalogue.protected()
        {
            let (panes, agents) = document.snapshot.counts();
            document.baseline = Some(protected.clone());
            document.requires_confirmation = panes < protected.panes || agents < protected.agents;
        }
        if expect.is_some() {
            document.requires_confirmation = false;
        }
        return Ok(document);
    }
    let directory = directory(paths, store)?;
    let Some(catalogue) = generations::read(&directory)? else {
        return if selector.is_some() {
            Err(Error::NotFound)
        } else {
            acquire(paths, store, None, expect)
        };
    };
    let selected = match selector {
        Some(SnapshotSelector::Generation(id)) => catalogue.get(id).ok_or(Error::NotFound)?,
        None => catalogue
            .preferred()
            .or_else(|| catalogue.latest())
            .ok_or(Error::NotFound)?,
        Some(SnapshotSelector::Named(_)) => unreachable!("handled above"),
    };
    let mut document = document(
        generations::acquire::<D>(&directory, selected)?,
        store,
        expect,
    )?;
    document.protected = catalogue.protected.as_ref() == Some(&selected.id);
    document.requires_confirmation = catalogue
        .protected()
        .is_some_and(|previous| selected.smaller_than(previous))
        && !(selector.is_some() && expect.is_some());
    document.generation = Some(selected.clone());
    document.baseline = catalogue.protected().cloned();
    Ok(document)
}

pub fn accept_generation(
    paths: &ConfigPaths,
    store: Store<'_>,
    selected: &tmt_core::workspace::generations::GenerationId,
    expected_protected: Option<&tmt_core::workspace::generations::GenerationId>,
) -> Result<bool, Error> {
    let directory = directory(paths, store)?;
    let _lock = lock(&directory)?;
    generations::accept(&directory, selected, expected_protected).map_err(Error::from)
}

/// Enumerate only owned filenames; bound even a directory filled with foreign files.
fn names(directory: &Path) -> Result<Vec<SnapshotName>, Error> {
    let mut names = Vec::new();
    for (index, entry) in fs::read_dir(directory)?.enumerate() {
        if index >= 256 {
            return Err(Error::Unavailable(io::Error::other(
                "Workspace directory exceeds the inventory bound.",
            )));
        }
        let entry = entry?;
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        let Some(name) = filename
            .strip_prefix("saved-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let name: SnapshotName = name
            .parse()
            .map_err(|message| Error::Unavailable(io::Error::other(message)))?;
        names.push(name);
        if names.len() > MAX_SAVED_SNAPSHOTS {
            return Err(Error::Limit);
        }
    }
    names.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    Ok(names)
}

#[derive(Debug)]
pub struct SnapshotEntry {
    pub name: Option<SnapshotName>,
    pub digest: String,
    pub captured_at_ms: u64,
    pub generation: Option<WorkspaceGeneration>,
    pub protected: bool,
    pub panes: usize,
    pub agents: usize,
}

pub fn list_snapshots<D: Recovery>(
    paths: &ConfigPaths,
    store: Store<'_>,
) -> Result<Vec<SnapshotEntry>, Error> {
    let directory = match directory(paths, store) {
        Ok(path) => path,
        Err(Error::NotFound) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut selected = vec![None];
    selected.extend(names(&directory)?.into_iter().map(Some));
    let mut entries = Vec::new();
    if let Some(catalogue) = generations::read(&directory)? {
        for generation in catalogue.entries.iter().rev() {
            let document: Document<D> = document(
                generations::acquire::<D>(&directory, generation)?,
                store,
                None,
            )?;
            entries.push(SnapshotEntry {
                name: None,
                digest: document.digest,
                captured_at_ms: generation.captured_at_ms,
                generation: Some(generation.clone()),
                protected: catalogue.protected.as_ref() == Some(&generation.id),
                panes: generation.panes,
                agents: generation.agents,
            });
        }
        selected.remove(0);
    }
    for name in selected {
        let document = match acquire::<D>(paths, store, name.as_ref(), None) {
            Ok(document) => document,
            // Concurrent explicit removal is also absence, never a fake snapshot.
            Err(Error::NotFound) => continue,
            Err(error) => return Err(error),
        };
        let (panes, agents) = document.snapshot.counts();
        entries.push(SnapshotEntry {
            name,
            digest: document.digest,
            captured_at_ms: document.snapshot.captured_at_ms(),
            generation: None,
            protected: false,
            panes,
            agents,
        });
    }
    Ok(entries)
}

fn lock(directory: &Path) -> Result<nix::fcntl::Flock<fs::File>, Error> {
    file_lock::exclusive(&directory.join("publication.lock")).map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            Error::Busy
        } else {
            Error::from(error)
        }
    })
}

pub fn save<D: Recovery>(
    paths: &ConfigPaths,
    store: Store<'_>,
    name: &SnapshotName,
) -> Result<SnapshotEntry, Error> {
    let directory = directory(paths, store)?;
    let _lock = lock(&directory)?;
    let destination = destination(&directory, Some(name));
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(Error::Exists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    if names(&directory)?.len() >= MAX_SAVED_SNAPSHOTS {
        return Err(Error::Limit);
    }
    let document = acquire::<D>(paths, store, None, None)?;
    publish_new(&destination, &document.bytes)?;
    let (panes, agents) = document.snapshot.counts();
    Ok(SnapshotEntry {
        name: Some(name.clone()),
        digest: document.digest,
        captured_at_ms: document.snapshot.captured_at_ms(),
        generation: None,
        protected: false,
        panes,
        agents,
    })
}

/// Link a complete synced stage atomically, refusing even an unexpected destination.
/// The stable lock serializes normal writers; hard_link is the no-clobber boundary.
pub(super) fn publish_new(destination: &Path, bytes: &[u8]) -> Result<(), Error> {
    let directory = destination
        .parent()
        .expect("workspace destination has a parent");
    let staging = directory.join(format!(
        ".{}.{}.tmp",
        destination
            .file_name()
            .expect("workspace filename")
            .to_string_lossy(),
        std::process::id()
    ));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&staging)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::hard_link(&staging, destination).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                Error::Exists
            } else {
                error.into()
            }
        })?;
        fs::remove_file(&staging)?;
        fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}

pub fn remove<D: Recovery>(
    paths: &ConfigPaths,
    store: Store<'_>,
    name: &SnapshotName,
) -> Result<(), Error> {
    let directory = directory(paths, store)?;
    let _lock = lock(&directory)?;
    // Refuse symlinks, malformed or foreign documents before deleting anything.
    acquire::<D>(paths, store, Some(name), None)?;
    fs::remove_file(destination(&directory, Some(name)))?;
    fs::File::open(&directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;
