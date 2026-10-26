//! Free space inside the Core database (#5036): its read-only page report, the
//! one-time switch to incremental auto-vacuum, and the routine bounded reclaim
//! that keeps it small afterwards.
//!
//! A database created with `auto_vacuum=NONE` keeps every page it ever used;
//! deleted rows only move pages onto the free list. Switching the mode needs one
//! full `VACUUM`, which rewrites the whole file, so it runs only as an explicit
//! maintenance step (`tmt upgrade` runs it) and never while another writer is
//! active. After the switch, writer transactions that already prune retained
//! records also return a few free pages to the filesystem
//! (`reclaim_free_pages`), so no further rewrite is needed.

use super::{StorageError, classify};
use rusqlite::{Connection, ErrorCode, OpenFlags};
use std::{fs, io, path::Path, time::Duration};

/// `PRAGMA auto_vacuum` as SQLite reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoVacuum {
    None,
    Full,
    Incremental,
}

impl AutoVacuum {
    fn from_pragma(value: i64) -> Self {
        match value {
            1 => Self::Full,
            2 => Self::Incremental,
            _ => Self::None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Full => "full",
            Self::Incremental => "incremental",
        }
    }
}

/// The page accounting of one SQLite database, committed state included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabasePages {
    pub page_size: u64,
    pub page_count: u64,
    pub freelist_count: u64,
    pub auto_vacuum: AutoVacuum,
}

impl DatabasePages {
    pub fn free_bytes(&self) -> u64 {
        self.freelist_count.saturating_mul(self.page_size)
    }
}

fn pages(connection: &Connection) -> rusqlite::Result<DatabasePages> {
    let value = |pragma: &str| connection.pragma_query_value(None, pragma, |row| row.get(0));
    let count = |pragma: &str| value(pragma).map(|value: i64| value.unsigned_abs());
    Ok(DatabasePages {
        page_size: count("page_size")?,
        page_count: count("page_count")?,
        freelist_count: count("freelist_count")?,
        auto_vacuum: AutoVacuum::from_pragma(value("auto_vacuum")?),
    })
}

/// `path` as a SQLite URI that reads the file without locks or a log.
fn immutable_uri(path: &Path) -> String {
    let mut uri = String::from("file:");
    for character in path.to_string_lossy().chars() {
        match character {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            other => uri.push(other),
        }
    }
    uri.push_str("?immutable=1");
    uri
}

/// Reads the page accounting of the database at `path` through a read-only
/// connection: no migration, no write transaction, a bounded wait. Without a
/// write-ahead log the file holds every committed page, so it is read as
/// immutable and SQLite creates no `-wal` or `-shm` beside it. A writer that
/// starts during that read can at worst make this one report unreadable.
pub fn observe_pages(path: &Path) -> Result<DatabasePages, StorageError> {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let opened = if Path::new(&wal).exists() {
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    } else {
        Connection::open_with_flags(
            immutable_uri(path),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
    };
    let connection = opened.map_err(|error| classify(error, "Open storage page observation"))?;
    connection
        .busy_timeout(Duration::from_millis(100))
        .map_err(|error| classify(error, "Bound storage page observation"))?;
    let observed = pages(&connection).map_err(|error| classify(error, "Read storage pages"));
    let closed = connection
        .close()
        .map_err(|(_, error)| classify(error, "Close storage page observation"));
    observed.and_then(|pages| closed.map(|()| pages))
}

/// Pages one routine reclaim returns at most: 256 KiB with the default 4 KiB
/// page, small enough to add no noticeable time to the transaction it joins.
const RECLAIM_PAGES: u32 = 64;

/// Returns up to `RECLAIM_PAGES` free pages to the filesystem, inside the
/// caller's write transaction. A database that has not been switched to
/// incremental auto-vacuum, or has no free page, is left alone.
pub(super) fn reclaim_free_pages(connection: &Connection) -> Result<(), StorageError> {
    let pages = pages(connection).map_err(|error| classify(error, "Read free pages"))?;
    if pages.auto_vacuum != AutoVacuum::Incremental || pages.freelist_count == 0 {
        return Ok(());
    }
    // Run through `execute_batch`, the pragma stops at its first step and frees
    // nothing; it is stepped to completion instead.
    (|| {
        let mut statement =
            connection.prepare(&format!("PRAGMA incremental_vacuum({RECLAIM_PAGES})"))?;
        let mut rows = statement.query([])?;
        while rows.next()?.is_some() {}
        Ok(())
    })()
    .map_err(|error| classify(error, "Reclaim free pages"))
}

/// The outcome of the one-time compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compaction {
    /// No Core database exists.
    Absent,
    /// The database already uses incremental auto-vacuum.
    Current,
    /// The database was rewritten with incremental auto-vacuum.
    Compacted { bytes_before: u64, bytes_after: u64 },
    /// Another connection was writing; nothing changed.
    InUse,
    /// Nothing changed, for this reason.
    Refused(String),
}

/// Bytes the database occupies on disk: the file plus its write-ahead log.
fn occupied(path: &Path) -> io::Result<u64> {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let wal = match fs::metadata(&wal) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    Ok(fs::metadata(path)?.len() + wal)
}

fn available_bytes(directory: &Path) -> io::Result<u64> {
    let stat = nix::sys::statvfs::statvfs(directory).map_err(io::Error::from)?;
    // The field types differ by platform (u32 or u64); both widen losslessly.
    #[allow(clippy::unnecessary_cast)]
    let (blocks, size) = (stat.blocks_available() as u64, stat.fragment_size() as u64);
    Ok(blocks.saturating_mul(size))
}

/// Switches the Core database at `path` to incremental auto-vacuum with one
/// `VACUUM`, only when no other connection is writing and the filesystem has
/// room for the rewrite. Every refusal leaves the database unchanged.
pub fn compact_database(path: &Path) -> io::Result<Compaction> {
    compact_with(path, available_bytes)
}

fn busy(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// `available` reports the free bytes of a directory's filesystem; tests
/// inject it so the space check holds on any host.
fn compact_with(
    path: &Path,
    available: impl FnOnce(&Path) -> io::Result<u64>,
) -> io::Result<Compaction> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Compaction::Absent),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Ok(Compaction::Refused(
            "the database path is not a regular file".into(),
        ));
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(io::Error::other)?;
    // Never wait for another writer: a busy database is refused, not queued.
    connection
        .busy_timeout(Duration::ZERO)
        .map_err(io::Error::other)?;
    let outcome = compact_open(path, &connection, available);
    let closed = connection
        .close()
        .map_err(|(_, error)| io::Error::other(error));
    outcome.and_then(|outcome| closed.map(|()| outcome))
}

fn compact_open(
    path: &Path,
    connection: &Connection,
    available: impl FnOnce(&Path) -> io::Result<u64>,
) -> io::Result<Compaction> {
    let refused_by = |error: rusqlite::Error| -> io::Result<Compaction> {
        if busy(&error) {
            Ok(Compaction::InUse)
        } else if error.sqlite_error_code() == Some(ErrorCode::NotADatabase) {
            Ok(Compaction::Refused(
                "the file is not a SQLite database".into(),
            ))
        } else {
            Err(io::Error::other(error))
        }
    };
    let before = match pages(connection) {
        Ok(pages) => pages,
        Err(error) => return refused_by(error),
    };
    if before.auto_vacuum == AutoVacuum::Incremental {
        return Ok(Compaction::Current);
    }
    let bytes_before = occupied(path)?;
    // The rewrite needs room for a second copy of the database (SQLite's
    // temporary copy) and a write-ahead log of about the same size.
    let needed = bytes_before.saturating_mul(2);
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let free = available(directory)?;
    if free < needed {
        return Ok(Compaction::Refused(format!(
            "compaction needs {needed} free bytes on the filesystem of {} and {free} are available",
            directory.display()
        )));
    }
    // An immediate transaction proves that no other connection is writing now.
    // VACUUM cannot run inside a transaction, so the probe ends first; VACUUM
    // takes the write lock itself with the same zero wait, so a writer that
    // starts in between makes it refuse, and a refused VACUUM changes nothing.
    if let Err(error) = connection.execute_batch("BEGIN IMMEDIATE; COMMIT;") {
        return refused_by(error);
    }
    connection
        .pragma_update(None, "auto_vacuum", "INCREMENTAL")
        .map_err(io::Error::other)?;
    if let Err(error) = connection.execute_batch("VACUUM") {
        return refused_by(error);
    }
    // Copy the rewritten pages into the file and truncate the log. A reader
    // that still holds an older snapshot only defers this to a later
    // checkpoint; the busy result is a row, not an error.
    connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(io::Error::other)?;
    let after = pages(connection).map_err(io::Error::other)?;
    if after.auto_vacuum != AutoVacuum::Incremental {
        return Ok(Compaction::Refused(format!(
            "SQLite kept auto_vacuum at {}",
            after.auto_vacuum.as_str()
        )));
    }
    Ok(Compaction::Compacted {
        bytes_before,
        bytes_after: occupied(path)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{storage::Storage, test_support::TestDirectory};
    use std::path::PathBuf;

    const FILLER_ROWS: i64 = 4_000;

    /// A real Core database with many freed pages: rows of a real table are
    /// written, then most are deleted.
    fn freed_store(directory: &Path) -> PathBuf {
        let path = directory.join("tmt.db");
        let mut storage = Storage::open(&path).unwrap();
        let connection = storage.connection().unwrap();
        connection
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)
                 INSERT INTO exchange_flow
                   (sender_identity_id, recipient_identity_id, kind, bucket_start_ms, count, bytes)
                 SELECT hex(zeroblob(500)) || i, 'recipient', 'ack', i * 1800000, 1, 0 FROM n",
                [FILLER_ROWS],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM exchange_flow WHERE bucket_start_ms > 100 * 1800000",
                [],
            )
            .unwrap();
        storage.close().unwrap();
        path
    }

    fn rows(path: &Path) -> i64 {
        Connection::open(path)
            .unwrap()
            .query_row("SELECT count(*) FROM exchange_flow", [], |row| row.get(0))
            .unwrap()
    }

    fn integrity(path: &Path) -> String {
        Connection::open(path)
            .unwrap()
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap()
    }

    fn plenty(_: &Path) -> io::Result<u64> {
        Ok(u64::MAX)
    }

    /// The database's bytes and its log's, so "nothing changed" covers both.
    fn snapshot(path: &Path) -> (Vec<u8>, Option<Vec<u8>>) {
        let mut wal = path.as_os_str().to_os_string();
        wal.push("-wal");
        (fs::read(path).unwrap(), fs::read(PathBuf::from(wal)).ok())
    }

    #[test]
    fn compaction_shrinks_a_database_with_freed_pages_and_keeps_its_data() {
        let directory = TestDirectory::new();
        let path = freed_store(&directory.path);
        let before = observe_pages(&path).unwrap();
        assert_eq!(before.auto_vacuum, AutoVacuum::None);
        assert!(before.freelist_count > 500, "{before:?}");

        let outcome = compact_with(&path, plenty).unwrap();

        let Compaction::Compacted {
            bytes_before,
            bytes_after,
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert!(
            bytes_after * 4 < bytes_before,
            "{bytes_before} -> {bytes_after}"
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), bytes_after);
        let after = observe_pages(&path).unwrap();
        assert_eq!(after.auto_vacuum, AutoVacuum::Incremental);
        assert_eq!(after.freelist_count, 0);
        assert_eq!(rows(&path), 100);
        assert_eq!(integrity(&path), "ok");
        // The real open path still accepts the compacted store.
        let mut reopened = Storage::open(&path).unwrap();
        reopened.health().unwrap();
        reopened.close().unwrap();
        assert_eq!(compact_with(&path, plenty).unwrap(), Compaction::Current);
    }

    #[test]
    fn an_active_writer_refuses_compaction_and_nothing_changes() {
        let directory = TestDirectory::new();
        let path = freed_store(&directory.path);
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let bytes = snapshot(&path);

        assert_eq!(compact_with(&path, plenty).unwrap(), Compaction::InUse);

        assert_eq!(snapshot(&path), bytes);
        writer.execute_batch("ROLLBACK").unwrap();
        drop(writer);
        assert_eq!(observe_pages(&path).unwrap().auto_vacuum, AutoVacuum::None);
        // An idle connection is not a writer: compaction then succeeds.
        let idle = Connection::open(&path).unwrap();
        assert!(matches!(
            compact_with(&path, plenty).unwrap(),
            Compaction::Compacted { .. }
        ));
        drop(idle);
    }

    #[test]
    fn too_little_free_space_refuses_compaction_and_nothing_changes() {
        let directory = TestDirectory::new();
        let path = freed_store(&directory.path);
        let bytes = snapshot(&path);
        let needed = occupied(&path).unwrap() * 2;
        let mut asked = None;

        let outcome = compact_with(&path, |directory| {
            asked = Some(directory.to_path_buf());
            Ok(needed - 1)
        })
        .unwrap();

        let Compaction::Refused(reason) = outcome else {
            panic!("{outcome:?}");
        };
        assert!(
            reason.contains(&format!("needs {needed} free bytes")),
            "{reason}"
        );
        assert_eq!(asked.as_deref(), Some(directory.path.as_path()));
        assert_eq!(snapshot(&path), bytes);
        assert_eq!(observe_pages(&path).unwrap().auto_vacuum, AutoVacuum::None);
        // Exactly the needed space is enough.
        assert!(matches!(
            compact_with(&path, |_| Ok(needed)).unwrap(),
            Compaction::Compacted { .. }
        ));
    }

    #[test]
    fn a_missing_database_is_absent_and_a_foreign_file_is_refused_unchanged() {
        let directory = TestDirectory::new();
        let path = directory.path.join("tmt.db");
        assert_eq!(compact_with(&path, plenty).unwrap(), Compaction::Absent);
        assert!(!path.exists(), "compaction never creates a database");
        fs::write(&path, b"this is not a database at all").unwrap();
        assert_eq!(
            compact_with(&path, plenty).unwrap(),
            Compaction::Refused("the file is not a SQLite database".into())
        );
        assert_eq!(fs::read(&path).unwrap(), b"this is not a database at all");
    }

    #[test]
    fn page_observation_is_read_only() {
        let directory = TestDirectory::new();
        let path = freed_store(&directory.path);
        let bytes = snapshot(&path);
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        // A writer does not block the read-only observation.
        let pages = observe_pages(&path).unwrap();
        writer.execute_batch("ROLLBACK").unwrap();
        assert_eq!(pages.free_bytes(), pages.freelist_count * pages.page_size);
        assert_eq!(snapshot(&path).0, bytes.0);
        assert!(observe_pages(&directory.path.join("missing.db")).is_err());
        assert!(!directory.path.join("missing.db").exists());
    }

    #[test]
    fn routine_reclaim_runs_only_after_the_switch_and_is_bounded() {
        let directory = TestDirectory::new();
        let path = freed_store(&directory.path);
        let connection = Connection::open(&path).unwrap();
        let free = |connection: &Connection| pages(connection).unwrap().freelist_count;
        let before = free(&connection);
        reclaim_free_pages(&connection).unwrap();
        assert_eq!(free(&connection), before, "auto_vacuum NONE is left alone");
        drop(connection);

        assert!(matches!(
            compact_with(&path, plenty).unwrap(),
            Compaction::Compacted { .. }
        ));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "DELETE FROM exchange_flow WHERE bucket_start_ms > 10 * 1800000",
                [],
            )
            .unwrap();
        let freed = free(&connection);
        assert!(freed > u64::from(RECLAIM_PAGES), "{freed}");
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        reclaim_free_pages(&connection).unwrap();
        connection.execute_batch("COMMIT").unwrap();
        assert_eq!(free(&connection), freed - u64::from(RECLAIM_PAGES));
        assert_eq!(rows(&path), 10);
        assert_eq!(integrity(&path), "ok");
    }
}
