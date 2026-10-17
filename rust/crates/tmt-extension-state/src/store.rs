//! Private, consumer-schema SQLite with one write admission and commit owner.
use crate::{Layout, StateError, UpdateError, locking::WRITE_WAIT};
use nix::fcntl::OFlag;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior, params};
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub struct Migration<'a> {
    pub name: &'a str,
    pub sql: &'a str,
}
pub struct Store {
    path: PathBuf,
    retry: String,
    connection: Connection,
}
impl Store {
    pub fn open(
        layout: &Layout,
        name: &str,
        migrations: &[Migration<'_>],
        retry: &str,
    ) -> Result<Self, StateError> {
        let path = layout
            .path(name)
            .map_err(|e| StateError::admission(&layout.directory, e))?;
        if retry.is_empty()
            || migrations
                .iter()
                .any(|m| m.name.is_empty() || m.sql.is_empty())
            || migrations
                .iter()
                .map(|m| m.name)
                .collect::<BTreeSet<_>>()
                .len()
                != migrations.len()
        {
            return Err(StateError::Invalid {
                path,
                reason: "migrations must be uniquely named and the exact retry must be supplied",
            });
        }
        Layout::admit(layout.directory.clone(), &[])
            .map_err(|e| StateError::admission(&path, e))?;
        drop(
            layout
                .file(name)
                .map_err(|e| StateError::admission(&path, e))?,
        );
        secure_sidecars(&path)?;
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| StateError::sqlite(&path, "open store", e))?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(|e| StateError::sqlite(&path, "enable foreign keys", e))?;
        // Refuse unrecognized/future history before even changing journal mode.
        migration_start(&connection, &path, migrations)?;
        configure_wal(&connection, &path, retry)?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| StateError::sqlite(&path, "configure synchronous policy", e))?;
        let mut store = Self {
            path,
            retry: retry.into(),
            connection,
        };
        store.migrate(migrations)?;
        secure_sidecars(&store.path)?;
        Ok(store)
    }

    fn migrate(&mut self, migrations: &[Migration<'_>]) -> Result<(), StateError> {
        self.connection
            .busy_timeout(WRITE_WAIT)
            .map_err(|e| StateError::sqlite(&self.path, "configure write wait", e))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| lock_error(&self.path, &self.retry, "begin migration", false, e))?;
        // The five-second admission budget ends at BEGIN. In particular, COMMIT
        // must surface Busy rather than silently entering SQLite's busy retry loop.
        transaction
            .busy_timeout(Duration::ZERO)
            .map_err(|e| StateError::sqlite(&self.path, "finish migration admission", e))?;
        let start = migration_start(&transaction, &self.path, migrations)?;
        if start.is_none() {
            transaction.execute_batch(
                "CREATE TABLE _tmt_state_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, sql TEXT NOT NULL);")
                .map_err(|e| StateError::sqlite(&self.path, "create migration history", e))?;
        }
        for (index, migration) in migrations.iter().enumerate().skip(start.unwrap_or(0)) {
            transaction
                .execute_batch(migration.sql)
                .map_err(|e| StateError::sqlite(&self.path, "apply migration", e))?;
            transaction
                .execute(
                    "INSERT INTO _tmt_state_migrations(version,name,sql) VALUES(?,?,?)",
                    params![index as i64 + 1, migration.name, migration.sql],
                )
                .map_err(|e| StateError::sqlite(&self.path, "record migration", e))?;
        }
        transaction
            .commit()
            .map_err(|e| lock_error(&self.path, &self.retry, "commit migration", true, e))
    }

    /// Run the change once, under BEGIN IMMEDIATE, and commit only on success.
    pub fn write<T, E>(
        &mut self,
        change: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, UpdateError<E>> {
        self.connection
            .busy_timeout(WRITE_WAIT)
            .map_err(|e| StateError::sqlite(&self.path, "configure write wait", e))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| lock_error(&self.path, &self.retry, "begin write", false, e))?;
        transaction
            .busy_timeout(Duration::ZERO)
            .map_err(|e| StateError::sqlite(&self.path, "finish write admission", e))?;
        let value = change(&transaction).map_err(UpdateError::Change)?;
        transaction
            .commit()
            .map_err(|e| lock_error(&self.path, &self.retry, "commit write", true, e))?;
        Ok(value)
    }

    /// Read one query-only snapshot. No connection escapes the callback lifetime.
    pub fn read<T, E>(
        &mut self,
        read: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, UpdateError<E>> {
        self.connection
            .pragma_update(None, "query_only", true)
            .map_err(|e| StateError::sqlite(&self.path, "begin query-only snapshot", e))?;
        let result = (|| {
            let transaction = self
                .connection
                .transaction()
                .map_err(|e| StateError::sqlite(&self.path, "begin read snapshot", e))?;
            let value = read(&transaction).map_err(UpdateError::Change)?;
            transaction
                .commit()
                .map_err(|e| StateError::sqlite(&self.path, "finish read snapshot", e))?;
            Ok(value)
        })();
        let restored = self
            .connection
            .pragma_update(None, "query_only", false)
            .map_err(|e| StateError::sqlite(&self.path, "restore connection policy", e));
        match result {
            Err(error) => Err(error),
            Ok(value) => {
                restored?;
                Ok(value)
            }
        }
    }
}
fn migration_start(
    connection: &Connection,
    path: &Path,
    migrations: &[Migration<'_>],
) -> Result<Option<usize>, StateError> {
    let has_history: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='_tmt_state_migrations')", [], |r| r.get(0))
        .map_err(|e| StateError::sqlite(path, "read migration history", e))?;
    if !has_history {
        let existing: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| StateError::sqlite(path, "admit new store", e))?;
        if existing {
            return Err(StateError::Migration {
                path: path.into(),
                reason: "store has no recognized migration history; it was not changed",
            });
        }
        return Ok(None);
    }
    let history: Vec<(i64, String, String)> = {
        let mut statement = connection
            .prepare("SELECT version,name,sql FROM _tmt_state_migrations ORDER BY version")
            .map_err(|e| StateError::sqlite(path, "read migration history", e))?;
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .and_then(|rows| rows.collect())
            .map_err(|e| StateError::sqlite(path, "decode migration history", e))?
    };
    if history.len() > migrations.len() {
        return Err(StateError::Migration {
            path: path.into(),
            reason: "store is newer than this binary; use the newer binary",
        });
    }
    if history
        .iter()
        .enumerate()
        .any(|(index, (version, name, sql))| {
            *version != index as i64 + 1
                || name != migrations[index].name
                || sql != migrations[index].sql
        })
    {
        return Err(StateError::Migration {
            path: path.into(),
            reason: "store migration history is not the declared prefix; it was not changed",
        });
    }
    Ok(Some(history.len()))
}

fn secure_sidecars(path: &Path) -> Result<(), StateError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let sidecar = PathBuf::from(name);
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(&sidecar)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(StateError::io(&sidecar, "admit SQLite sidecar", e)),
        };
        super::validate_file(&file).map_err(|e| StateError::admission(&sidecar, e))?;
    }
    Ok(())
}
fn busy(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(e, _) if matches!(e.code,
        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
}
fn lock_error(
    path: &Path,
    retry: &str,
    operation: &'static str,
    uncertain: bool,
    error: rusqlite::Error,
) -> StateError {
    if busy(&error) {
        StateError::Timeout {
            path: path.into(),
            operation,
            retry: retry.into(),
            may_not_be_saved: uncertain,
            source: error.into(),
        }
    } else {
        StateError::sqlite(path, operation, error)
    }
}
fn configure_wal(connection: &Connection, path: &Path, retry: &str) -> Result<(), StateError> {
    let deadline = Instant::now() + WRITE_WAIT;
    loop {
        connection
            .busy_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            )
            .map_err(|e| StateError::sqlite(path, "configure WAL wait", e))?;
        match connection
            .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))
        {
            Ok(mode) if mode == "wal" => {
                return connection
                    .busy_timeout(WRITE_WAIT)
                    .map_err(|e| StateError::sqlite(path, "configure write wait", e));
            }
            Ok(_) => {
                return Err(StateError::Invalid {
                    path: path.into(),
                    reason: "SQLite did not enable WAL",
                });
            }
            Err(error) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if !busy(&error) || remaining.is_zero() {
                    return Err(lock_error(path, retry, "configure WAL", false, error));
                }
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
                if Instant::now() >= deadline {
                    return Err(lock_error(path, retry, "configure WAL", false, error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commit_busy_is_typed_uncertain_and_other_errors_keep_their_cause() {
        let path = Path::new("/private/state.db");
        for code in [rusqlite::ffi::SQLITE_BUSY, rusqlite::ffi::SQLITE_LOCKED] {
            let error = lock_error(
                path,
                "retry-exact",
                "commit write",
                true,
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(code),
                    Some("held".into()),
                ),
            );
            assert!(error.is_timeout());
            assert!(error.may_not_be_saved());
            assert!(error.to_string().contains("may not be saved"));
            assert!(error.to_string().contains("retry-exact"));
            assert!(std::error::Error::source(&error).is_some());
        }
        let error = lock_error(
            path,
            "retry-exact",
            "commit write",
            true,
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                None,
            ),
        );
        assert!(!error.is_timeout());
        assert!(matches!(error, StateError::Sqlite { .. }));
    }
}
