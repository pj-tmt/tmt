//! Renames the Core database from its former basename (`tmux-team.db`, #2292)
//! without copying it and without two writers.
//!
//! SQLite itself proves that nothing else holds the database: an exclusive
//! lock cannot be taken while any other connection is open, idle WAL
//! connections included. While that lock is held the database becomes a single
//! file (no `-wal`, `-shm` or `-journal`), gains its new name through a hard
//! link, and the old name is replaced atomically by a relative link to the new
//! one, so no name is ever missing. Old binaries and a rollback release open
//! the old name, follow the link and share the new name's journals.

use crate::config::{DATABASE, FORMER_DATABASE};
use rusqlite::{Connection, ErrorCode, OpenFlags};
use std::{
    fs::{self, File, Metadata},
    io,
    os::unix::fs::{MetadataExt, symlink},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseRename {
    /// No Core database under either name.
    Absent,
    /// The database already has its current name; a compatibility link may remain.
    Current,
    /// This call gave the database its current name.
    Renamed,
    /// The database keeps its former name.
    Kept(Kept),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kept {
    /// Another process holds the database or its journals; stop it and retry.
    InUse,
    /// The directory is not in a state this rename may change, for this reason.
    Refused(String),
}

fn refused(reason: impl Into<String>) -> io::Result<DatabaseRename> {
    Ok(DatabaseRename::Kept(Kept::Refused(reason.into())))
}

fn lstat(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn sidecars(directory: &Path, basename: &str) -> Vec<PathBuf> {
    ["-wal", "-shm", "-journal"]
        .map(|suffix| directory.join(format!("{basename}{suffix}")))
        .into()
}

fn journal_of(database: &Path) -> PathBuf {
    let mut journal = database.as_os_str().to_os_string();
    journal.push("-journal");
    journal.into()
}

/// Takes the exclusive lock, then leaves the database as one file.
fn fence(database: &Path) -> io::Result<Result<Connection, Kept>> {
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(io::Error::other)?;
    connection
        .busy_timeout(Duration::ZERO)
        .map_err(io::Error::other)?;
    let held = (|| {
        connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        connection.execute_batch("BEGIN EXCLUSIVE; COMMIT;")?;
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        connection.pragma_update_and_check(None, "journal_mode", "DELETE", |row| {
            row.get::<_, String>(0)
        })
    })();
    match held {
        Ok(mode) if mode.eq_ignore_ascii_case("delete") => Ok(Ok(connection)),
        Ok(mode) => Ok(Err(Kept::Refused(format!(
            "the database stayed in {mode} journal mode"
        )))),
        Err(error) => match error.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Ok(Err(Kept::InUse)),
            Some(ErrorCode::NotADatabase) => Ok(Err(Kept::Refused(
                "the file is not a SQLite database".into(),
            ))),
            _ => Err(io::Error::other(error)),
        },
    }
}

/// Replaces `former`, a name of the database, with a relative link to `current`.
fn link_former(directory: &Path, former: &Path) -> io::Result<()> {
    let temporary = directory.join(format!(".{FORMER_DATABASE}.link-{}", Uuid::new_v4()));
    symlink(DATABASE, &temporary)?;
    fs::rename(&temporary, former).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

/// Renames the database in `directory` when nothing holds it. The caller owns
/// when this runs (`tmt upgrade`); a held database keeps its name, unchanged.
pub fn rename_database(directory: &Path) -> io::Result<DatabaseRename> {
    let former = directory.join(FORMER_DATABASE);
    let current = directory.join(DATABASE);
    let (former_meta, current_meta) = (lstat(&former)?, lstat(&current)?);
    // The state to act on: the file to fence and whether the new name exists.
    let (file, linked) = match (&former_meta, &current_meta) {
        (None, None) => return Ok(DatabaseRename::Absent),
        (None, Some(_)) => return Ok(DatabaseRename::Current),
        (Some(old), None) if old.file_type().is_file() => (&former, false),
        (Some(_), None) => return refused("the former database name is not a regular file"),
        (Some(old), Some(new)) if old.file_type().is_symlink() => {
            return if new.file_type().is_file() && fs::read_link(&former)? == Path::new(DATABASE) {
                Ok(DatabaseRename::Current)
            } else {
                refused("the former database name is a link this rename did not create")
            };
        }
        // An interrupted rename left both names on one file.
        (Some(old), Some(new))
            if old.file_type().is_file()
                && new.file_type().is_file()
                && old.dev() == new.dev()
                && old.ino() == new.ino() =>
        {
            (&current, true)
        }
        (Some(_), Some(_)) => return refused("both database names exist as separate files"),
    };
    if !linked {
        for stray in sidecars(directory, DATABASE) {
            if lstat(&stray)?.is_some() {
                return refused(format!("{} is left over", stray.display()));
            }
        }
    }
    let connection = match fence(file)? {
        Ok(connection) => connection,
        Err(kept) => return Ok(DatabaseRename::Kept(kept)),
    };
    // The fenced connection keeps its own journal, named after the path it
    // opened, until it closes; every other sidecar must be gone by now.
    let own_journal = journal_of(file);
    for basename in [FORMER_DATABASE, DATABASE] {
        for stray in sidecars(directory, basename) {
            if stray != own_journal && lstat(&stray)?.is_some() {
                return refused(format!("{} is left over", stray.display()));
            }
        }
    }
    if !linked {
        fs::hard_link(&former, &current)?;
    }
    if let Err(error) = link_former(directory, &former) {
        if !linked {
            // Nothing changed for readers: the old name is still the file.
            let _ = fs::remove_file(&current);
        }
        return Err(error);
    }
    File::open(directory)?.sync_all()?;
    drop(connection);
    Ok(DatabaseRename::Renamed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::ConfigPaths, storage::Storage, test_support::TestDirectory};
    use std::os::unix::fs::PermissionsExt;

    /// The path every reader would open in `directory`.
    fn resolved(directory: &Path) -> PathBuf {
        ConfigPaths::resolve(directory, directory, Some(directory), None).database
    }

    /// A store with a row, closed, under the former name.
    fn former_store(directory: &Path) -> PathBuf {
        let path = directory.join(FORMER_DATABASE);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES ('kept');",
            )
            .unwrap();
        drop(connection);
        path
    }

    fn marker(path: &Path) -> String {
        Connection::open(path)
            .unwrap()
            .query_row("SELECT value FROM marker", [], |row| row.get(0))
            .unwrap()
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_database_nothing_holds_is_renamed_to_one_file_with_a_link_at_the_old_name() {
        let directory = TestDirectory::new();
        let former = former_store(&directory.path);
        fs::set_permissions(&former, fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Renamed
        );

        assert_eq!(names(&directory.path), [DATABASE, FORMER_DATABASE]);
        let current = directory.path.join(DATABASE);
        assert!(
            fs::symlink_metadata(&current)
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_eq!(fs::read_link(&former).unwrap(), Path::new(DATABASE));
        assert_eq!(
            fs::metadata(&current).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(marker(&current), "kept");
        assert_eq!(resolved(&directory.path), current);
        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Current
        );
    }

    #[test]
    fn the_last_released_open_paths_work_through_the_link_and_share_the_new_journals() {
        let directory = TestDirectory::new();
        drop(Storage::open(directory.path.join(FORMER_DATABASE)).unwrap());
        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Renamed
        );
        let former = directory.path.join(FORMER_DATABASE);

        // The same calls the released binary makes with `ConfigPaths.database`:
        // default flags with create (no NOFOLLOW), the hook's read-write open and
        // the read-only schema observation.
        let held = Storage::open(&former).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        drop(Storage::open_hook(&former, deadline).unwrap());
        let version = Storage::application_schema(&former).unwrap();
        assert_eq!(version, Storage::compiled_schema().version);
        assert_eq!(
            names(&directory.path),
            [DATABASE, "tmt.db-shm", "tmt.db-wal", FORMER_DATABASE],
            "journals live under the new name; none is created beside the link"
        );
        drop(held);
        // The current binary opens the new name and sees the same store.
        let current = Storage::open(directory.path.join(DATABASE)).unwrap();
        assert_eq!(
            current.health().unwrap().schema_version,
            Storage::compiled_schema().version
        );
    }

    #[test]
    fn a_held_database_keeps_its_name_and_bytes() {
        let directory = TestDirectory::new();
        let former = former_store(&directory.path);
        let holder = Connection::open(&former).unwrap();
        holder.pragma_update(None, "journal_mode", "WAL").unwrap();
        let before = names(&directory.path);
        let bytes = fs::read(&former).unwrap();

        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Kept(Kept::InUse)
        );

        assert_eq!(names(&directory.path), before);
        assert!(fs::symlink_metadata(&former).unwrap().file_type().is_file());
        assert_eq!(fs::read(&former).unwrap(), bytes);
        assert_eq!(resolved(&directory.path), former);
        drop(holder);
        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Renamed
        );
    }

    #[test]
    fn leftovers_foreign_files_and_a_second_database_refuse_without_changes() {
        for case in ["leftover", "separate", "foreign-link", "not-a-database"] {
            let directory = TestDirectory::new();
            let former = former_store(&directory.path);
            match case {
                "leftover" => fs::write(directory.path.join("tmt.db-wal"), b"x").unwrap(),
                "separate" => fs::write(directory.path.join(DATABASE), b"other").unwrap(),
                "foreign-link" => {
                    fs::remove_file(&former).unwrap();
                    fs::write(directory.path.join("elsewhere.db"), b"x").unwrap();
                    symlink("elsewhere.db", &former).unwrap();
                }
                _ => fs::write(&former, b"this is not a database at all").unwrap(),
            }
            let before = names(&directory.path);

            let outcome = rename_database(&directory.path).unwrap();

            assert!(
                matches!(outcome, DatabaseRename::Kept(Kept::Refused(_))),
                "{case}: {outcome:?}"
            );
            assert_eq!(names(&directory.path), before, "{case}");
            if case != "foreign-link" {
                assert!(!directory.path.join("tmt.db").exists() || case == "separate");
            }
        }
    }

    #[test]
    fn an_interrupted_rename_is_finished_only_when_nothing_holds_the_file() {
        let directory = TestDirectory::new();
        let former = former_store(&directory.path);
        let current = directory.path.join(DATABASE);
        fs::hard_link(&former, &current).unwrap();
        let holder = Connection::open(&current).unwrap();
        holder.pragma_update(None, "journal_mode", "WAL").unwrap();
        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Kept(Kept::InUse)
        );
        assert!(fs::symlink_metadata(&former).unwrap().file_type().is_file());
        drop(holder);

        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Renamed
        );
        assert_eq!(fs::read_link(&former).unwrap(), Path::new(DATABASE));
        assert_eq!(marker(&current), "kept");
    }

    #[test]
    fn a_failure_after_the_fence_leaves_only_the_former_name() {
        let directory = TestDirectory::new();
        let former = former_store(&directory.path);
        // The directory refuses new entries, so the new name cannot be created.
        fs::set_permissions(&directory.path, fs::Permissions::from_mode(0o500)).unwrap();

        let outcome = rename_database(&directory.path);

        fs::set_permissions(&directory.path, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(outcome.is_err() || matches!(outcome, Ok(DatabaseRename::Kept(_))));
        assert!(!directory.path.join(DATABASE).exists());
        assert!(fs::symlink_metadata(&former).unwrap().file_type().is_file());
        assert_eq!(marker(&former), "kept");
        assert_eq!(resolved(&directory.path), former);
    }

    #[test]
    fn no_database_is_absent_and_the_new_name_alone_is_current() {
        let directory = TestDirectory::new();
        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Absent
        );
        assert_eq!(resolved(&directory.path), directory.path.join(DATABASE));
        fs::write(directory.path.join(DATABASE), b"x").unwrap();
        assert_eq!(
            rename_database(&directory.path).unwrap(),
            DatabaseRename::Current
        );
    }
}
