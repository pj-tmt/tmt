//! Opening a database that is already current takes no write lock (#5019).

use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn current_database(directory: &TestDirectory) -> PathBuf {
    let path = directory.path.join("current.db");
    Storage::open(&path).unwrap().close().unwrap();
    path
}

fn hold_write_lock(path: &PathBuf) -> Connection {
    let writer = Connection::open(path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer
}

#[test]
fn a_current_database_needs_no_write_lock_to_apply() {
    let directory = TestDirectory::new();
    let path = current_database(&directory);
    let _writer = hold_write_lock(&path);

    let mut reader = Connection::open(&path).unwrap();
    reader.busy_timeout(Duration::ZERO).unwrap();
    apply(&mut reader).unwrap();
}

#[test]
fn opening_a_current_database_does_not_wait_for_a_writer() {
    let directory = TestDirectory::new();
    let path = current_database(&directory);
    let _writer = hold_write_lock(&path);

    let started = Instant::now();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(
        storage.health().unwrap().schema_version,
        MIGRATIONS.len() as u32
    );
    storage.close().unwrap();
    // The storage busy timeout is 5 s: a write lock would have used all of it.
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_database_behind_is_still_migrated_when_opened() {
    let directory = TestDirectory::new();
    // Migration 84 admits the hook queue directory beside the database, which must be
    // owner-only; a database two migrations behind now replays it.
    owner_only(&directory.path);
    let path = directory.path.join("behind.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, MIGRATIONS.len() - 2).unwrap();
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(
        storage.health().unwrap().schema_version,
        MIGRATIONS.len() as u32
    );
    storage.close().unwrap();
}

#[test]
fn racing_openers_of_a_database_behind_apply_each_migration_once() {
    let directory = TestDirectory::new();
    // Migration 84 admits the hook queue directory beside the database, which must be
    // owner-only; a database two migrations behind now replays it.
    owner_only(&directory.path);
    let path = directory.path.join("behind.db");
    let mut old = Connection::open(&path).unwrap();
    old.pragma_update(None, "journal_mode", "WAL").unwrap();
    apply_through(&mut old, MIGRATIONS.len() - 2).unwrap();
    old.close().unwrap();

    let barrier = std::sync::Barrier::new(8);
    let versions = std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let mut storage = Storage::open(&path)?;
                    let version = storage.health()?.schema_version;
                    storage.close()?;
                    Ok::<_, StorageError>(version)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        versions
            .iter()
            .all(|version| *version == MIGRATIONS.len() as u32)
    );

    let oracle = Connection::open(&path).unwrap();
    let applied: i64 = oracle
        .query_row("SELECT COUNT(*) FROM _migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(applied, MIGRATIONS.len() as i64);
}

#[test]
fn a_newer_schema_is_still_refused_without_a_write() {
    let directory = TestDirectory::new();
    let path = current_database(&directory);
    let setup = Connection::open(&path).unwrap();
    setup
        .execute(
            "INSERT INTO _migrations (version, name, applied_at) VALUES (?, 'future', 't')",
            [MIGRATIONS.len() as i64 + 1],
        )
        .unwrap();
    setup.close().unwrap();
    let _writer = hold_write_lock(&path);

    let mut reader = Connection::open(&path).unwrap();
    reader.busy_timeout(Duration::ZERO).unwrap();
    let error = apply(&mut reader).unwrap_err();
    assert!(error.is_newer_schema());
}

#[test]
fn a_foreign_migration_table_is_still_refused() {
    let directory = TestDirectory::new();
    let path = directory.path.join("foreign.db");
    let mut foreign = Connection::open(&path).unwrap();
    foreign
        .execute_batch("CREATE TABLE _migrations (version TEXT, note TEXT)")
        .unwrap();
    let error = apply(&mut foreign).unwrap_err();
    assert_eq!(error.code, StorageErrorCode::IncompatibleSchema);
}

fn owner_only(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
