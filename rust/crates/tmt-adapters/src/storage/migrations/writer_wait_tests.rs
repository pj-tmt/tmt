//! Waiting for the migration writer follows the winner's progress (#5335).

use super::*;
use crate::test_support::TestDirectory;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

// Each version outlasts the 100 ms writer slice, so some slices see no
// progress, yet takes a fifth of the stall bound, leaving 800 ms of slack for a
// loaded host; the whole chain outlasts the stall bound twice.
const CHAIN: usize = 10;
const STEP: Duration = Duration::from_millis(200);
const STALL: Duration = Duration::from_millis(1000);

fn history_database(directory: &TestDirectory, recorded: usize) -> PathBuf {
    let path = directory.path.join("history.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .unwrap();
    connection
        .execute_batch("CREATE TABLE _migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL)")
        .unwrap();
    for version in 1..=recorded {
        record(&connection, version);
    }
    path
}

fn record(connection: &Connection, version: usize) {
    connection
        .execute(
            "INSERT INTO _migrations (version, name, applied_at) VALUES (?, ?, 'now')",
            params![version as i64, MIGRATIONS[version - 1].name],
        )
        .unwrap();
}

/// The opener that migrates: one write transaction per version, taken again
/// at once after each commit, so a polling waiter never finds the lock free.
fn run_chain(path: PathBuf, started: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let connection = Connection::open(&path).unwrap();
        for version in 1..=CHAIN {
            connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            started.store(true, Ordering::SeqCst);
            thread::sleep(STEP);
            record(&connection, version);
            connection.execute_batch("COMMIT").unwrap();
        }
    })
}

fn waiter(path: &PathBuf) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection.busy_timeout(Duration::from_millis(7)).unwrap();
    connection
}

#[test]
fn a_waiter_behind_a_chain_that_keeps_advancing_finishes_with_the_complete_history() {
    let directory = TestDirectory::new();
    let path = history_database(&directory, 0);
    let started = Arc::new(AtomicBool::new(false));
    let winner = run_chain(path.clone(), Arc::clone(&started));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the chain never started");
        thread::sleep(Duration::from_millis(1));
    }

    let waiter = waiter(&path);
    let begun = Instant::now();
    // Either the history was seen complete, or the freed writer was taken
    // after the last version; the caller finds nothing to apply in both.
    let taken = take_migration_writer(&waiter, CHAIN, STALL, "Acquire migration lock").unwrap();
    if let Some(transaction) = &taken {
        assert_eq!(validate_history(transaction).unwrap(), CHAIN);
    }
    drop(taken);
    assert!(
        begun.elapsed() > STALL,
        "the chain must outlast the stall bound for this to prove progress is followed"
    );
    winner.join().unwrap();
    assert_eq!(recorded_through(&waiter).unwrap(), Some(CHAIN));
    let restored: i64 = waiter
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .unwrap();
    assert_eq!(restored, crate::storage::BUSY_TIMEOUT.as_millis() as i64);
}

#[test]
fn a_writer_that_adds_no_version_still_ends_the_wait_in_busy_after_the_stall_bound() {
    let directory = TestDirectory::new();
    let path = history_database(&directory, 0);
    let holder = Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();

    let waiter = waiter(&path);
    let begun = Instant::now();
    match take_migration_writer(&waiter, CHAIN, STALL, "Acquire migration lock") {
        Err(error) => assert_eq!(error.code, StorageErrorCode::Busy),
        Ok(_) => panic!("a stuck writer must end the wait"),
    }
    let waited = begun.elapsed();
    assert!(waited >= STALL, "{waited:?}");
    assert!(waited < Duration::from_secs(3), "{waited:?}");

    holder.execute_batch("ROLLBACK").unwrap();
    let taken = take_migration_writer(&waiter, CHAIN, STALL, "Acquire migration lock").unwrap();
    assert!(taken.is_some(), "the free writer is taken");
}

#[test]
fn a_complete_history_returns_without_waiting_for_the_writer() {
    let directory = TestDirectory::new();
    let path = history_database(&directory, CHAIN);
    let holder = Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();

    let waiter = waiter(&path);
    let begun = Instant::now();
    let taken = take_migration_writer(&waiter, CHAIN, STALL, "Acquire migration lock").unwrap();
    assert!(taken.is_none());
    assert!(begun.elapsed() < Duration::from_secs(2));
}
