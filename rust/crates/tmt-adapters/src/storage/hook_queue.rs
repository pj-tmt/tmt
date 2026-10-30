//! Durable hook handoff. This database never shares the Core writer lock.
//! Enqueue is the hook's commit point; only finite, event-started flushers drain it.
mod capture;
#[cfg(test)]
mod tests;
mod worker;

use super::{StorageError, StorageErrorCode, classify};
pub use capture::{CaptureEvent, CaptureFollowUp, CaptureLearning, CaptureResult};
use nix::fcntl::Flock;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

pub const ENTRY_LIMIT: usize = 128 * 1024;
pub const QUEUE_SCHEMA_VERSION: u32 = 3;
const READ_IDENTITY: &str = "SELECT sequence,CAST(event_id AS BLOB),CAST(identity_id AS BLOB),CAST(payload AS BLOB) FROM hook_writes WHERE identity_id=?2 AND committed=0 AND ?1=1 ORDER BY sequence LIMIT 1";
const READ_QUEUE: &str = "SELECT sequence,CAST(event_id AS BLOB),CAST(identity_id AS BLOB),CAST(payload AS BLOB) FROM hook_writes WHERE (?1=0 OR committed=0) AND ?2 IS NULL ORDER BY sequence LIMIT 1";
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS hook_writes(sequence INTEGER PRIMARY KEY AUTOINCREMENT,event_id TEXT NOT NULL UNIQUE,identity_id TEXT,provisional_batch TEXT,kind TEXT NOT NULL CHECK(kind='capture'),enqueued_ms INTEGER NOT NULL CHECK(enqueued_ms>=0),committed INTEGER NOT NULL DEFAULT 0 CHECK(committed IN (0,1)),payload TEXT NOT NULL CHECK(length(CAST(payload AS BLOB))<=131072)) STRICT; CREATE INDEX IF NOT EXISTS hook_writes_identity_pending ON hook_writes(identity_id,committed,sequence); CREATE INDEX IF NOT EXISTS hook_writes_provisional_batch ON hook_writes(provisional_batch); CREATE TABLE IF NOT EXISTS invalid_hook_writes(event_id TEXT PRIMARY KEY NOT NULL,quarantined_ms INTEGER NOT NULL) STRICT; CREATE INDEX IF NOT EXISTS invalid_hook_writes_retention ON invalid_hook_writes(quarantined_ms,event_id); CREATE TABLE IF NOT EXISTS provisional_batches(turn_key TEXT PRIMARY KEY NOT NULL,batch_id TEXT NOT NULL,created_ms INTEGER NOT NULL) STRICT; CREATE INDEX IF NOT EXISTS provisional_batches_retention ON provisional_batches(created_ms,turn_key); CREATE TABLE IF NOT EXISTS hook_retention_scan(id INTEGER PRIMARY KEY CHECK(id=1),receipt_ms INTEGER NOT NULL,receipt_id TEXT NOT NULL,batch_ms INTEGER NOT NULL,batch_key TEXT NOT NULL) STRICT; INSERT OR IGNORE INTO hook_retention_scan VALUES(1,-1,'',-1,''); PRAGMA user_version=3;";

/// Publication evidence is separate from whether a finite worker could start.
/// Unknown never authorizes another append or a terminal failure receipt.
#[derive(Debug)]
pub enum CapturePublication {
    Queued,
    Recorded(CaptureResult),
    Unpublished(StorageError),
    Unknown {
        publication: StorageError,
        readback: StorageError,
    },
}

pub struct HookQueue {
    database: PathBuf,
    connection: Connection,
}

struct QueuedRow {
    sequence: i64,
    id: Vec<u8>,
    owner: Option<Vec<u8>>,
    payload: Vec<u8>,
}

/// Metadata-only admission: opening and closing a second descriptor for a live
/// SQLite inode would release this process's POSIX locks, including SHM locks.
fn admit_file(path: &Path) -> io::Result<bool> {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if !m.is_file() || m.uid() != nix::unistd::geteuid().as_raw() || m.mode() & 0o777 != 0o600 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Hook queue files must be private regular files",
        ));
    }
    Ok(true)
}
fn create_database(path: &Path) -> io::Result<()> {
    if !admit_file(path)? {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => drop(file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                admit_file(path)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl HookQueue {
    /// Called from request housekeeping while the Core writer is held. The
    /// zero-wait queue writer stays held until that Core transaction commits,
    /// so a queued event can never lose its replay receipt during cleanup.
    pub(super) fn begin_retention(
        main: &Connection,
        now_ms: u64,
        limit: u64,
    ) -> Result<Option<Self>, StorageError> {
        const RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1000;
        let Some(cutoff) = now_ms.checked_sub(RETENTION_MS) else {
            return Ok(None);
        };
        let path: String = main
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| classify(e, "Read hook retention path"))?;
        if path.is_empty() || !Self::exists(Path::new(&path)) {
            return Ok(None);
        }
        let queue = match Self::open(Path::new(&path), Duration::ZERO) {
            Ok(queue) => queue,
            Err(_) => return Ok(None),
        };
        if queue.connection.execute_batch("BEGIN IMMEDIATE").is_err() {
            return Ok(None);
        }
        let cutoff = cutoff.min(i64::MAX as u64) as i64;
        let limit = limit.min(100) as i64;
        let (receipt_ms, receipt_id, batch_ms, batch_key): (i64,String,i64,String) = queue.connection.query_row(
            "SELECT receipt_ms,receipt_id,batch_ms,batch_key FROM hook_retention_scan WHERE id=1",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
            .map_err(|e| classify(e,"Read hook retention cursor"))?;
        let receipts = {
            let mut statement = main.prepare("SELECT committed_ms,event_id FROM hook_capture_receipts WHERE committed_ms<=?1 AND (committed_ms,event_id)>(?2,?3) ORDER BY committed_ms,event_id LIMIT ?4")
                .map_err(|e| classify(e,"Read old hook receipts"))?;
            let rows = statement
                .query_map(params![cutoff, receipt_ms, receipt_id, limit], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| classify(e, "Read old hook receipts"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| classify(e, "Read old hook receipts"))?
        };
        let receipt_next = if receipts.len() == limit as usize {
            receipts.last().cloned().unwrap_or((-1, String::new()))
        } else {
            (-1, String::new())
        };
        for (_, id) in receipts {
            let queued: bool = queue
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM hook_writes WHERE event_id=?)",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(|e| classify(e, "Protect queued hook receipt"))?;
            if !queued {
                main.execute("DELETE FROM hook_capture_receipts WHERE event_id=?", [&id])
                    .map_err(|e| classify(e, "Prune hook receipt"))?;
            }
        }
        queue.connection.execute("DELETE FROM invalid_hook_writes WHERE event_id IN (SELECT event_id FROM invalid_hook_writes WHERE quarantined_ms<=?1 ORDER BY quarantined_ms,event_id LIMIT ?2)",params![cutoff,limit])
            .map_err(|e| classify(e,"Prune invalid hook receipts"))?;
        let reservations = {
            let mut statement = queue.connection.prepare("SELECT created_ms,turn_key,batch_id FROM provisional_batches WHERE created_ms<=?1 AND (created_ms,turn_key)>(?2,?3) ORDER BY created_ms,turn_key LIMIT ?4")
                .map_err(|e| classify(e,"Read old provisional batches"))?;
            let rows = statement
                .query_map(params![cutoff, batch_ms, batch_key, limit], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| classify(e, "Read old provisional batches"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| classify(e, "Read old provisional batches"))?
        };
        let batch_next = if reservations.len() == limit as usize {
            reservations
                .last()
                .map(|(at, key, _)| (*at, key.clone()))
                .unwrap_or((-1, String::new()))
        } else {
            (-1, String::new())
        };
        for (_, key, batch) in reservations {
            let finished: bool = main.query_row("SELECT NOT EXISTS(SELECT 1 FROM capture_batches WHERE id=? AND closed_ms IS NULL)",[&batch],|r|r.get(0))
                .map_err(|e| classify(e,"Check finished provisional batch"))?;
            let queued: bool = queue
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM hook_writes WHERE provisional_batch=?)",
                    [&batch],
                    |r| r.get(0),
                )
                .map_err(|e| classify(e, "Protect queued provisional batch"))?;
            if finished && !queued {
                queue
                    .connection
                    .execute("DELETE FROM provisional_batches WHERE turn_key=?", [key])
                    .map_err(|e| classify(e, "Prune finished provisional batch"))?;
            }
        }
        queue.connection.execute("UPDATE hook_retention_scan SET receipt_ms=?1,receipt_id=?2,batch_ms=?3,batch_key=?4 WHERE id=1",params![receipt_next.0,receipt_next.1,batch_next.0,batch_next.1])
            .map_err(|e| classify(e,"Advance hook retention cursor"))?;
        Ok(Some(queue))
    }
    pub(super) fn finish_retention(self) -> Result<(), StorageError> {
        self.connection
            .execute_batch("COMMIT")
            .map_err(|e| classify(e, "Commit hook queue retention"))
    }
    pub fn path(database: &Path) -> PathBuf {
        database.with_file_name("hook-writes.db")
    }
    pub fn exists(database: &Path) -> bool {
        fs::symlink_metadata(Self::path(database)).is_ok()
    }
    pub fn open(database: &Path, wait: Duration) -> Result<Self, StorageError> {
        Self::connect(database, wait, false)
    }
    /// Called only by Core migration while its initialization writer is held.
    /// Main's new schema is not published until the queue is fully provisioned.
    pub(super) fn initialize(main: &Connection) -> Result<(), StorageError> {
        if main.is_autocommit() {
            return Err(invalid(
                "Queue initialization requires the Core migration writer",
            ));
        }
        let path: String = main
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| classify(e, "Read Core initialization path"))?;
        if path.is_empty() {
            return Ok(());
        } // In-memory reference schemas have no hook store.
        Self::connect(Path::new(&path), Duration::from_secs(5), true).map(drop)
    }
    /// A restored current Core store may lack its queue. Ordinary Core opens
    /// complete that initialization under the same writer; hooks never do.
    pub(super) fn initialize_missing(main: &mut Connection) -> Result<(), StorageError> {
        let path: String = main
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| classify(e, "Read Core queue initialization path"))?;
        if path.is_empty() || Self::exists(Path::new(&path)) {
            return Ok(());
        }
        let tx = main
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| classify(e, "Acquire Core queue initialization writer"))?;
        if !Self::exists(Path::new(&path)) {
            Self::initialize(&tx)?;
        }
        tx.commit()
            .map_err(|e| classify(e, "Publish Core queue initialization"))
    }
    fn connect(database: &Path, wait: Duration, initialize: bool) -> Result<Self, StorageError> {
        let selected = Self::path(database);
        let directory = selected
            .parent()
            .ok_or_else(|| invalid("No hook queue directory"))?;
        // Core selects the trusted root; admit its real directory as the shared
        // state leaf does, including macOS aliases and owner-selected root links.
        let directory = fs::canonicalize(directory)
            .map_err(|e| super::errors::classify_io(e, "Resolve hook queue root"))?;
        let path = directory.join("hook-writes.db");
        if initialize {
            crate::private_file::ensure_private_directory(&directory)
                .map_err(|e| super::errors::classify_io(e, "Admit hook queue directory"))?;
            create_database(&path)
                .map_err(|e| super::errors::classify_io(e, "Admit hook queue database"))?;
        } else {
            if !crate::private_file::directory_is_private(&directory) {
                return Err(unavailable("Hook queue is not initialized"));
            }
            if !admit_file(&path)
                .map_err(|e| super::errors::classify_io(e, "Admit hook queue database"))?
            {
                return Err(unavailable("Hook queue is not initialized"));
            }
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut file = path.as_os_str().to_os_string();
            file.push(suffix);
            admit_file(Path::new(&file))
                .map_err(|e| super::errors::classify_io(e, "Admit hook queue sidecar"))?;
        }
        let mut connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| classify(e, "Open hook queue"))?;
        connection
            .busy_timeout(wait)
            .map_err(|e| classify(e, "Bound hook queue writer wait"))?;
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| classify(e, "Read hook queue schema"))?;
        if (initialize && version != 0 && version != QUEUE_SCHEMA_VERSION)
            || (!initialize && version != QUEUE_SCHEMA_VERSION)
        {
            return Err(unavailable("Hook queue schema is not compatible"));
        }
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|e| classify(e, "Make hook enqueue durable"))?;
        if version == 0 {
            let foreign: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| classify(e, "Admit empty hook queue schema"))?;
            if foreign {
                return Err(invalid("Unrecognized hook queue schema"));
            }
            let mode: String = connection
                .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
                .map_err(|e| classify(e, "Enable hook queue WAL during Core initialization"))?;
            if mode != "wal" {
                return Err(invalid("Hook queue requires WAL"));
            }
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| classify(e, "Begin hook queue initialization"))?;
            let current: u32 = tx
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(|e| classify(e, "Recheck hook queue schema"))?;
            if current == 0 {
                let tables: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%')",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|e| classify(e, "Admit empty hook queue schema"))?;
                if tables {
                    return Err(invalid("Unrecognized hook queue schema"));
                }
                tx.execute_batch(SCHEMA)
                    .map_err(|e| classify(e, "Initialize hook queue"))?;
            } else if current != QUEUE_SCHEMA_VERSION {
                return Err(invalid("Unsupported hook queue schema"));
            }
            tx.commit()
                .map_err(|e| classify(e, "Commit hook queue initialization"))?;
            File::open(&directory)
                .and_then(|f| f.sync_all())
                .map_err(|e| super::errors::classify_io(e, "Sync hook queue directory"))?;
        }
        // A matching version alone is not a compatible queue: damaged or
        // foreign tables must select the documented direct fallback.
        if connection.prepare("SELECT sequence,event_id,identity_id,provisional_batch,kind,enqueued_ms,committed,payload FROM hook_writes LIMIT 0").is_err()
            || connection.prepare("SELECT turn_key,batch_id,created_ms FROM provisional_batches LIMIT 0").is_err()
            || connection.prepare("SELECT event_id,quarantined_ms FROM invalid_hook_writes LIMIT 0").is_err()
            || connection.prepare("SELECT receipt_ms,receipt_id,batch_ms,batch_key FROM hook_retention_scan LIMIT 0").is_err()
        {
            return Err(unavailable("Hook queue schema is not compatible"));
        }
        Ok(Self {
            database: database.to_owned(),
            connection,
        })
    }
    pub fn enqueue(&self, event: &CaptureEvent) -> Result<(), StorageError> {
        let mut event = event.clone();
        if !event.valid() {
            return Err(invalid("Invalid hook capture event"));
        }
        let tx =
            rusqlite::Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
                .map_err(|e| classify(e, "Begin hook queue append"))?;
        if let Some(key) = event.provisional_key() {
            let candidate = event
                .provisional_batch()
                .ok_or_else(|| invalid("Missing provisional batch"))?;
            tx.execute(
                "INSERT INTO provisional_batches(turn_key,batch_id,created_ms) VALUES(?,?,?) ON CONFLICT(turn_key) DO NOTHING",
                params![key, candidate, crate::request_runtime::wall_time_ms() as i64],
            ).map_err(|e| classify(e, "Reserve provisional batch"))?;
            let batch: String = tx
                .query_row(
                    "SELECT batch_id FROM provisional_batches WHERE turn_key=?",
                    [&key],
                    |r| r.get(0),
                )
                .map_err(|e| classify(e, "Read provisional batch"))?;
            event.set_provisional_batch(batch);
        }
        let payload =
            serde_json::to_string(&event).map_err(|_| invalid("Invalid hook capture event"))?;
        if payload.len() > ENTRY_LIMIT || !event.valid() {
            return Err(invalid("Invalid or oversized hook capture event"));
        }
        tx.execute(
            "INSERT INTO hook_writes(event_id,identity_id,provisional_batch,kind,payload,enqueued_ms) VALUES(?,?,?,'capture',?,?)",
            params![
                event.id(),
                event.identity(),
                event.provisional_batch(),
                payload,
                crate::request_runtime::wall_time_ms() as i64
            ],
        )
        .map_err(|e| classify(e, "Enqueue hook capture"))?;
        tx.commit()
            .map_err(|e| classify(e, "Commit hook queue append"))
    }
    /// Reconcile an append error only after its transaction has finished.
    pub fn enqueue_observed(
        &self,
        event: &CaptureEvent,
        deadline: std::time::Instant,
    ) -> CapturePublication {
        if std::time::Instant::now() >= deadline {
            return CapturePublication::Unpublished(StorageError::new(
                StorageErrorCode::Busy,
                "Hook publication budget expired",
            ));
        }
        self.observe_enqueue_result(event, self.enqueue(event), deadline)
    }

    fn observe_enqueue_result(
        &self,
        event: &CaptureEvent,
        result: Result<(), StorageError>,
        deadline: std::time::Instant,
    ) -> CapturePublication {
        let Err(publication) = result else {
            return CapturePublication::Queued;
        };
        let readback = (|| {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or_else(|| {
                    StorageError::new(
                        StorageErrorCode::Busy,
                        "Hook publication readback budget expired",
                    )
                })?;
            self.connection
                .busy_timeout(remaining.min(Duration::from_millis(500)))
                .map_err(|error| classify(error, "Bound hook publication readback"))?;
            let queued: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM hook_writes WHERE event_id=?)",
                    [event.id()],
                    |row| row.get(0),
                )
                .map_err(|error| classify(error, "Read back hook publication"))?;
            if queued {
                return Ok(Some(CapturePublication::Queued));
            }
            // A flusher commits the main receipt before removing its queue row.
            // Queue absence alone therefore cannot prove non-publication.
            Ok(
                capture::read_publication_receipt(&self.database, event.id(), deadline)?
                    .map(CapturePublication::Recorded),
            )
        })();
        match readback {
            Ok(Some(outcome)) => outcome,
            Ok(None) => CapturePublication::Unpublished(publication),
            Err(readback) => CapturePublication::Unknown {
                publication,
                readback,
            },
        }
    }

    /// One stable lock inode for all flushers. A contender exits without waiting;
    /// the current drainer owns every committed row until its empty handoff.
    pub fn claim(&self) -> io::Result<Flock<File>> {
        crate::file_lock::exclusive(&self.database.with_file_name("hook-writes.lock"))
    }
    /// A write boundary may recover a crashed worker. Do not launch a contender
    /// when a current drainer already owns the rows. The empty handoff serializes
    /// producers across the same lock, so this notification cannot lose a row.
    pub fn start_flusher(&self, deadline: std::time::Instant) -> io::Result<()> {
        if crate::core_executable::declared().is_none() {
            return Ok(());
        }
        let guard = match self.claim() {
            Ok(guard) => guard,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => {
                let _ = worker::note_claim_failure(&self.database, &error, deadline);
                return Err(error);
            }
        };
        drop(guard);
        worker::start(&self.database, deadline)
    }
    pub fn next(&self) -> Result<Option<CaptureEvent>, StorageError> {
        self.read_next(false, None)
    }
    pub fn has_entries(&self) -> Result<bool, StorageError> {
        self.connection
            .query_row("SELECT EXISTS(SELECT 1 FROM hook_writes)", [], |r| r.get(0))
            .map_err(|e| classify(e, "Check hook queue work"))
    }
    pub fn next_uncommitted(&self) -> Result<Option<CaptureEvent>, StorageError> {
        self.read_next(true, None)
    }
    pub fn next_uncommitted_for_identity(
        &self,
        identity: &str,
    ) -> Result<Option<CaptureEvent>, StorageError> {
        self.read_next(true, Some(identity))
    }
    pub fn mark_committed(&self, event: &CaptureEvent) -> Result<(), StorageError> {
        self.connection
            .execute(
                "UPDATE hook_writes SET committed=1 WHERE event_id=?",
                [event.id()],
            )
            .map_err(|e| classify(e, "Mark queued capture committed"))?;
        Ok(())
    }
    fn read_next(
        &self,
        uncommitted: bool,
        identity: Option<&str>,
    ) -> Result<Option<CaptureEvent>, StorageError> {
        loop {
            let sql = if identity.is_some() {
                READ_IDENTITY
            } else {
                READ_QUEUE
            };
            let row: Option<QueuedRow> = self
                .connection
                .query_row(sql, params![uncommitted, identity], |r| {
                    Ok(QueuedRow {
                        sequence: r.get(0)?,
                        id: r.get(1)?,
                        owner: r.get(2)?,
                        payload: r.get(3)?,
                    })
                })
                .optional()
                .map_err(|e| classify(e, "Read next hook capture"))?;
            let Some(QueuedRow {
                sequence,
                id,
                owner,
                payload,
            }) = row
            else {
                return Ok(None);
            };
            let event = (payload.len() <= ENTRY_LIMIT)
                .then(|| serde_json::from_slice::<CaptureEvent>(&payload).ok())
                .flatten()
                .filter(|event| {
                    event.id().as_bytes() == id
                        && event.valid()
                        && event.identity().map(str::as_bytes) == owner.as_deref()
                });
            if let Some(event) = event {
                return Ok(Some(event));
            }
            // A bad row must not poison the global queue. The failure receipt
            // and removal are one queue transaction; no untrusted payload is kept.
            let tx = rusqlite::Transaction::new_unchecked(
                &self.connection,
                TransactionBehavior::Immediate,
            )
            .map_err(|e| classify(e, "Begin invalid hook quarantine"))?;
            tx.execute("INSERT OR IGNORE INTO invalid_hook_writes(event_id,quarantined_ms) SELECT event_id,?1 FROM hook_writes WHERE sequence=?2", params![crate::request_runtime::wall_time_ms() as i64,sequence])
                .map_err(|e| classify(e,"Record invalid hook capture"))?;
            tx.execute("DELETE FROM hook_writes WHERE sequence=?", [sequence])
                .map_err(|e| classify(e, "Quarantine invalid hook capture"))?;
            tx.commit()
                .map_err(|e| classify(e, "Commit invalid hook quarantine"))?;
        }
    }
    pub fn acknowledge(&self, event: &CaptureEvent) -> Result<(), StorageError> {
        self.connection
            .execute("DELETE FROM hook_writes WHERE event_id=?", [event.id()])
            .map_err(|e| classify(e, "Acknowledge committed hook capture"))?;
        Ok(())
    }
    /// Serialize the empty check with producers, then release the flusher lock
    /// BEFORE releasing the queue writer. An append either precedes this check
    /// (we keep draining) or follows the unlock (its spawned flusher can claim).
    /// No polling, sleep or extra resident worker closes this lost-wakeup gap.
    pub fn finish_if_empty(
        &mut self,
        guard: &mut Option<Flock<File>>,
    ) -> Result<bool, StorageError> {
        self.finish_if_empty_with(guard, || {})
    }
    fn finish_if_empty_with(
        &mut self,
        guard: &mut Option<Flock<File>>,
        at_empty: impl FnOnce(),
    ) -> Result<bool, StorageError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| classify(e, "Begin hook queue empty handoff"))?;
        let empty: bool = tx
            .query_row("SELECT NOT EXISTS(SELECT 1 FROM hook_writes)", [], |r| {
                r.get(0)
            })
            .map_err(|e| classify(e, "Check hook queue empty"))?;
        if empty {
            at_empty();
            drop(guard.take());
        }
        tx.commit()
            .map_err(|e| classify(e, "Finish hook queue empty handoff"))?;
        Ok(empty)
    }
}

fn invalid(message: &str) -> StorageError {
    StorageError::new(StorageErrorCode::Corrupt, message)
}

fn unavailable(message: &str) -> StorageError {
    StorageError::new(StorageErrorCode::IncompatibleSchema, message)
}
