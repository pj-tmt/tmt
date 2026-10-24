//! The Ops state database: one private SQLite file under the Ops subtree that holds the
//! cron jobs and every room's checklist as versioned documents (`kind`, `key`, bytes).
//!
//! Admission, the write queue (five seconds, then a typed busy refusal), crash-atomic
//! commits and the newer-than-this-binary refusal belong to `tmt-extension-state`. This
//! owner adds what Ops alone knows: the document slots, the first-use import of the files
//! these documents replaced, and the archive of an imported file. The callers keep their
//! codecs and error codes; they receive a [`Refusal`] and decide how to present it.
//!
//! ## Import and archive
//!
//! A slot names the legacy file its document used to live in. The first access after an
//! upgrade finds the row absent and the file present, hands the file's bytes to the same
//! change closure as any stored bytes (so the caller's decoder validates them), and inserts
//! them inside the write transaction. A corrupt file therefore refuses and changes nothing.
//! Only after that commit is the file renamed to `<name>.migrated-<YYYY-MM-DD>`
//! (`-<n>` when taken), under the database write lock so archivers never overlap. The
//! rename never overwrites and nothing is ever deleted: if a crash lands between commit
//! and rename, the next access sees the row and the file, trusts the row and finishes the
//! rename.
use rusqlite::{OptionalExtension, Transaction, params};
use std::{
    fs,
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use tmt_extension_state::{
    Error as LayoutError, Layout, Migration, StateError, Store, UpdateError,
};

const NAME: &str = "ops.db";
const FILES: &[&str] = &[NAME];
/// The one step a busy refusal offers; Ops never retries a change on its own.
pub const BUSY_RETRY: &str = "Run the same command again.";
const MIGRATIONS: &[Migration<'static>] = &[
    Migration {
        name: "documents",
        sql: "CREATE TABLE documents(kind TEXT NOT NULL, key TEXT NOT NULL, body BLOB NOT NULL, PRIMARY KEY(kind, key)) WITHOUT ROWID;",
    },
    // The checklist mutation log (#4882). `checklist_key` is the document's slot key. `id` never
    // repeats (AUTOINCREMENT), so it is a cursor that survives pruning. The floor says what a
    // list lost: `legacy` for mutations made before the log existed (every list that already
    // has a document), `pruned_through` for the newest id retention removed.
    Migration {
        name: "checklist_events",
        sql: "CREATE TABLE checklist_events(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                checklist_key TEXT NOT NULL,
                room_id TEXT NOT NULL,
                member_id TEXT,
                checklist_id TEXT NOT NULL,
                item_id TEXT,
                actor_id TEXT NOT NULL,
                at_ms INTEGER NOT NULL,
                action TEXT NOT NULL CHECK(action IN ('add','edit','assign','unassign','complete','reopen','archive','restore','delete','reorder')),
                revision INTEGER NOT NULL
            );
            CREATE INDEX checklist_events_by_list ON checklist_events(checklist_key, id);
            CREATE INDEX checklist_events_by_room ON checklist_events(room_id, id);
            CREATE TABLE checklist_event_floor(
                checklist_key TEXT PRIMARY KEY,
                legacy INTEGER NOT NULL CHECK(legacy IN (0, 1)),
                pruned_through INTEGER NOT NULL DEFAULT 0
            ) WITHOUT ROWID;
            INSERT INTO checklist_event_floor(checklist_key, legacy) SELECT key, 1 FROM documents WHERE kind = 'checklist';",
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Another Ops process held the write queue for its whole wait.
    Busy,
    /// A newer Ops wrote this database; this binary must not touch it.
    Newer,
    /// Anything else: unsafe path, SQLite or I/O error, an unreadable legacy file.
    Failed,
}

/// Why the database could not serve a request. `message` is complete without the retry
/// hint, which the caller attaches when `cause` is [`Cause::Busy`].
#[derive(Debug)]
pub struct Refusal {
    pub cause: Cause,
    pub message: String,
    /// The change may already be committed (a busy commit); the caller must say so.
    pub may_not_be_saved: bool,
}
impl Refusal {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            cause: Cause::Failed,
            message: message.into(),
            may_not_be_saved: false,
        }
    }
}
impl From<StateError> for Refusal {
    fn from(error: StateError) -> Self {
        match &error {
            StateError::Timeout {
                operation,
                may_not_be_saved,
                ..
            } => Self {
                cause: Cause::Busy,
                message: format!(
                    "Another Ops process still holds the state database ({operation} gave up after 5 s), {}.",
                    if *may_not_be_saved {
                        "so the change may not be saved"
                    } else {
                        "so nothing was saved"
                    }
                ),
                may_not_be_saved: *may_not_be_saved,
            },
            StateError::Migration { path, .. } if error.is_newer() => Self {
                cause: Cause::Newer,
                message: format!(
                    "{} was written by a newer Ops and was not changed. Upgrade Ops (`tmt extension upgrade ops` or `tmt upgrade`), then run the command again.",
                    path.display()
                ),
                may_not_be_saved: false,
            },
            _ => Self {
                cause: Cause::Failed,
                message: error.to_string(),
                may_not_be_saved: error.may_not_be_saved(),
            },
        }
    }
}

/// Either the state database refused, or the caller's own change did.
#[derive(Debug)]
pub enum StateFailure<E> {
    Refused(Refusal),
    Change(E),
}
impl<E> From<Refusal> for StateFailure<E> {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

/// One document: its identity in the database and the legacy file it replaced.
pub struct Slot {
    pub kind: &'static str,
    pub key: String,
    pub legacy: PathBuf,
    /// Largest document the owner admits; also bounds the legacy read.
    pub limit: u64,
}

/// Why a write closure stopped; mapped to a [`StateFailure`] once the transaction is gone.
enum Step<E> {
    Sql(rusqlite::Error),
    Legacy(String),
    Oversized(u64),
    Change(E),
}
impl<E> From<rusqlite::Error> for Step<E> {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

pub struct Database {
    store: Store,
}

fn layout_refusal(error: LayoutError) -> Refusal {
    Refusal::failed(match error {
        LayoutError::UnsafeDirectory => {
            "The Ops state directory must be a directory you own with mode 0700.".to_owned()
        }
        LayoutError::UnsafeFile => {
            "The Ops state database must be a regular file you own with mode 0600.".to_owned()
        }
        other => format!("The Ops state directory is unavailable: {other:?}."),
    })
}
fn split(extension_root: &Path) -> Result<(&Path, &str), Refusal> {
    match (
        extension_root.parent(),
        extension_root.file_name().and_then(|name| name.to_str()),
    ) {
        (Some(data), Some(subtree)) if extension_root.is_absolute() => Ok((data, subtree)),
        _ => Err(Refusal::failed(
            "The selected Ops state root must be an absolute path.",
        )),
    }
}
fn io_refusal(path: &Path, what: &str, error: impl std::fmt::Display) -> Refusal {
    Refusal::failed(format!("{}: {what}: {error}", path.display()))
}

impl Database {
    /// Open for a change, creating the directory and database on first use.
    pub fn open(extension_root: &Path) -> Result<Self, Refusal> {
        let (data, subtree) = split(extension_root)?;
        let layout = Layout::open(data, subtree, FILES).map_err(layout_refusal)?;
        Self::on(&layout)
    }

    /// Open for a read. Returns `None`, creating nothing, when neither the database nor
    /// the slot's legacy file exists.
    pub fn open_existing(extension_root: &Path, legacy: &Path) -> Result<Option<Self>, Refusal> {
        let (data, subtree) = split(extension_root)?;
        let Some(layout) = Layout::existing(data, subtree, FILES).map_err(layout_refusal)? else {
            return Ok(None);
        };
        match layout.read_file(NAME) {
            Ok(_) => Self::on(&layout).map(Some),
            Err(LayoutError::ReadOpen(error)) if error.kind() == io::ErrorKind::NotFound => {
                if exists(legacy)? {
                    Self::on(&layout).map(Some)
                } else {
                    Ok(None)
                }
            }
            Err(error) => Err(layout_refusal(error)),
        }
    }

    fn on(layout: &Layout) -> Result<Self, Refusal> {
        Store::open(layout, NAME, MIGRATIONS, BUSY_RETRY)
            .map(|store| Self { store })
            .map_err(Refusal::from)
    }

    /// Decode the slot's document with `decode`, importing a legacy file on first use.
    pub fn read<T, E>(
        &mut self,
        slot: &Slot,
        decode: impl FnOnce(Option<&[u8]>) -> Result<T, E>,
    ) -> Result<T, StateFailure<E>> {
        let stored = self
            .store
            .read(|transaction| select(transaction, slot))
            .map_err(sql_refusal)?;
        if stored.is_none() && exists(&slot.legacy)? {
            return self.update(
                slot,
                |bytes| decode(bytes).map(|value| (None, value)),
                |_, _| Ok(()),
            );
        }
        let value = decode(stored.as_deref()).map_err(StateFailure::Change)?;
        if stored.is_some() {
            self.archive(&slot.legacy);
        }
        Ok(value)
    }

    /// Read through one query-only snapshot, for rows that live beside the documents.
    pub fn snapshot<T>(
        &mut self,
        read: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, Refusal> {
        self.store.read(read).map_err(sql_refusal)
    }

    /// Replace the slot's document. `change` sees the current bytes (or the legacy file's
    /// on first use) and returns the new bytes, `None` for no change, and its result.
    /// `before_commit` then runs inside the transaction, with the transaction: its refusal
    /// rolls the change back, and what it writes commits with the document or not at all.
    pub fn update<T, E>(
        &mut self,
        slot: &Slot,
        change: impl FnOnce(Option<&[u8]>) -> Result<(Option<Vec<u8>>, T), E>,
        before_commit: impl FnOnce(&T, &Transaction<'_>) -> Result<(), E>,
    ) -> Result<T, StateFailure<E>> {
        let outcome = self.store.write(|transaction| {
            let stored = select(transaction, slot)?;
            let (current, imported) = match stored {
                Some(bytes) => (Some(bytes), false),
                None => match legacy(slot).map_err(Step::Legacy)? {
                    Some(bytes) => (Some(bytes), true),
                    None => (None, false),
                },
            };
            let (next, value) = change(current.as_deref()).map_err(Step::Change)?;
            let write = match (next, imported) {
                (Some(bytes), _) => Some(bytes),
                (None, true) => current,
                (None, false) => None,
            };
            if let Some(bytes) = write {
                if bytes.len() as u64 > slot.limit {
                    return Err(Step::Oversized(slot.limit));
                }
                transaction.execute(
                    "INSERT INTO documents(kind, key, body) VALUES(?1, ?2, ?3) ON CONFLICT(kind, key) DO UPDATE SET body = excluded.body",
                    params![slot.kind, slot.key, bytes],
                )?;
            }
            before_commit(&value, transaction).map_err(Step::Change)?;
            Ok(value)
        });
        let value = match outcome {
            Ok(value) => value,
            Err(UpdateError::State(error)) => return Err(Refusal::from(error).into()),
            Err(UpdateError::Change(Step::Change(error))) => {
                return Err(StateFailure::Change(error));
            }
            Err(UpdateError::Change(Step::Sql(error))) => {
                return Err(Refusal::failed(format!("Ops state database: {error}")).into());
            }
            Err(UpdateError::Change(Step::Legacy(message))) => {
                return Err(Refusal::failed(message).into());
            }
            Err(UpdateError::Change(Step::Oversized(limit))) => {
                return Err(Refusal::failed(format!(
                    "The document would exceed its {} MiB limit.",
                    limit / (1024 * 1024)
                ))
                .into());
            }
        };
        self.archive(&slot.legacy);
        Ok(value)
    }

    /// Archive an imported legacy file under the database write lock. Archivers must not
    /// overlap: two of them can both hard-link the file and both unlink its name (macOS
    /// reports both unlinks as successful), leaving two archives of one generation. Best
    /// effort: a busy database or a failed rename leaves the file for the next access.
    fn archive(&mut self, path: &Path) {
        if fs::symlink_metadata(path).is_err() {
            return;
        }
        let _ = self.store.write(|_| {
            #[cfg(test)]
            tests::archiving(path);
            let _ = try_archive(path);
            Ok::<(), rusqlite::Error>(())
        });
    }
}

fn sql_refusal(error: UpdateError<rusqlite::Error>) -> Refusal {
    match error {
        UpdateError::State(error) => error.into(),
        UpdateError::Change(error) => Refusal::failed(format!("Ops state database: {error}")),
    }
}

fn select(transaction: &Transaction<'_>, slot: &Slot) -> rusqlite::Result<Option<Vec<u8>>> {
    transaction
        .query_row(
            "SELECT body FROM documents WHERE kind = ?1 AND key = ?2",
            params![slot.kind, slot.key],
            |row| row.get(0),
        )
        .optional()
}

fn exists(path: &Path) -> Result<bool, Refusal> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_refusal(path, "inspect the legacy file", error)),
    }
}

/// The legacy file's bytes, or `None` when it is absent. Refuses anything but an owned
/// regular file, never follows a link and never reads past the slot's limit.
fn legacy(slot: &Slot) -> Result<Option<Vec<u8>>, String> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags((nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK).bits())
        .open(&slot.legacy)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "{}: cannot read the file this document replaced: {error}",
                slot.legacy.display()
            ));
        }
    };
    let metadata = file.metadata().map_err(|error| {
        format!(
            "{}: cannot inspect the file: {error}",
            slot.legacy.display()
        )
    })?;
    if !metadata.is_file() || metadata.uid() != nix::unistd::getuid().as_raw() {
        return Err(format!(
            "{}: the file this document replaced is not an owned regular file.",
            slot.legacy.display()
        ));
    }
    let mut bytes = Vec::new();
    file.take(slot.limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}: cannot read the file: {error}", slot.legacy.display()))?;
    if bytes.len() as u64 > slot.limit {
        return Err(format!(
            "{}: the file this document replaced exceeds {} MiB.",
            slot.legacy.display(),
            slot.limit / (1024 * 1024)
        ));
    }
    Ok(Some(bytes))
}

/// Rename an imported file to `<name>.migrated-<date>` without overwriting anything.
fn try_archive(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    let (Some(directory), Some(name)) = (path.parent(), path.file_name()) else {
        return Ok(());
    };
    let date = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%Y-%m-%d")
        .to_string();
    let base = format!("{}.migrated-{date}", name.to_string_lossy());
    for attempt in 0u32.. {
        let archived = directory.join(if attempt == 0 {
            base.clone()
        } else {
            format!("{base}-{attempt}")
        });
        // A hard link fails when the name exists, which a rename would silently replace.
        match fs::hard_link(path, &archived) {
            Ok(()) => {
                return match fs::remove_file(path) {
                    Ok(()) => fs::File::open(directory)?.sync_all(),
                    // Something outside the database lock moved it first; ours is a duplicate.
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        fs::remove_file(&archived)
                    }
                    Err(error) => Err(error),
                };
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
