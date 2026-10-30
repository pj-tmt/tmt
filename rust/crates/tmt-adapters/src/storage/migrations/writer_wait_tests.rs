//! Waiting for the migration writer follows the winner's progress (#5335).

use super::*;
use crate::test_support::TestDirectory;
use std::{path::PathBuf, sync::mpsc::sync_channel, thread};

const CHAIN: usize = 10;
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

fn waiter(path: &PathBuf) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection.busy_timeout(Duration::from_millis(7)).unwrap();
    connection
}

#[test]
fn a_waiter_follows_committed_progress_past_the_stall_bound() {
    let directory = TestDirectory::new();
    let path = history_database(&directory, 0);
    let waiter = waiter(&path);
    let (advance_tx, advance_rx) = sync_channel(1);
    let (ready_tx, ready_rx) = sync_channel(1);
    let chain_path = path.clone();
    let chain = thread::spawn(move || -> Result<usize, String> {
        let connection = Connection::open(&chain_path).map_err(|error| error.to_string())?;
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| error.to_string())?;
        ready_tx.send(0).map_err(|error| error.to_string())?;
        for version in 1..=CHAIN {
            advance_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            record(&connection, version);
            connection
                .execute_batch("COMMIT")
                .map_err(|error| error.to_string())?;
            if version < CHAIN {
                connection
                    .execute_batch("BEGIN IMMEDIATE")
                    .map_err(|error| error.to_string())?;
            }
            // The waiter is inside its clock callback until this writer is
            // retaken. Its next BEGIN cannot race a fixture scheduling gap.
            ready_tx.send(version).map_err(|error| error.to_string())?;
        }
        Ok(CHAIN)
    });
    let ready = ready_rx.recv_timeout(Duration::from_secs(5));
    let start = Instant::now();
    let mut samples = 0_u32;
    let mut last_sampled = start;
    let mut observed_progress = Vec::new();
    let outcome = ready.as_ref().is_ok_and(|version| *version == 0).then(|| {
        let mut now = || {
            let sample = samples;
            samples += 1;
            if sample.is_multiple_of(2) {
                let progress = advance_tx
                    .send(())
                    .map_err(|error| error.to_string())
                    .and_then(|()| {
                        ready_rx
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(|error| error.to_string())
                    });
                observed_progress.push(progress);
            }
            // A progress observation jumps beyond the previous stall deadline.
            // Then hold the same writer for a no-progress probe half a stall
            // later. That probe succeeds only if the deadline really reset.
            let mut offset = STALL.saturating_mul(sample.div_ceil(2) * 2);
            if sample != 0 && sample.is_multiple_of(2) {
                offset += STALL / 2;
            }
            last_sampled = start + offset;
            last_sampled
        };
        wait_for_migration_writer(&waiter, CHAIN, STALL, "Acquire migration lock", &mut now)
            .and_then(|taken| match taken {
                Some(transaction) => validate_history(&transaction),
                None => recorded_through(&waiter).map(|version| version.unwrap_or(0)),
            })
    });
    // Closing the command lane also releases a producer after an early error.
    // Join it before asserting or allowing its private directory to be removed.
    drop(advance_tx);
    let joined = chain.join();
    assert_eq!(ready.unwrap(), 0);
    assert_eq!(outcome.unwrap().unwrap(), CHAIN);
    assert_eq!(joined.unwrap().unwrap(), CHAIN);
    assert_eq!(
        observed_progress,
        (1..=CHAIN).map(Ok::<_, String>).collect::<Vec<_>>()
    );
    assert_eq!(samples as usize, (CHAIN - 1) * 2 + 1);
    assert!(last_sampled.saturating_duration_since(start) > STALL);
    assert_eq!(recorded_through(&waiter).unwrap(), Some(CHAIN));
}

fn assert_busy_timeout_restored(connection: &Connection) {
    let restored: i64 = connection
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
    assert_busy_timeout_restored(&waiter);
    let waited = begun.elapsed();
    assert!(waited >= STALL, "{waited:?}");
    assert!(waited < Duration::from_secs(3), "{waited:?}");

    holder.execute_batch("ROLLBACK").unwrap();
    let taken = take_migration_writer(&waiter, CHAIN, STALL, "Acquire migration lock").unwrap();
    assert!(taken.is_some(), "the free writer is taken");
    drop(taken);
    assert_busy_timeout_restored(&waiter);
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
    assert_busy_timeout_restored(&waiter);
}

#[test]
fn a_forced_gap_after_version_four_returns_an_owned_partial_writer() {
    let directory = TestDirectory::new();
    let path = history_database(&directory, 0);
    let waiter = waiter(&path);
    let (ready_tx, ready_rx) = sync_channel(1);
    let (resume_tx, resume_rx) = sync_channel(1);
    let chain_path = path.clone();
    let chain = thread::spawn(move || {
        let connection = Connection::open(&chain_path).unwrap();
        for version in 1..=CHAIN {
            connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            record(&connection, version);
            connection.execute_batch("COMMIT").unwrap();
            if version == 4 {
                ready_tx.send(()).unwrap();
                // The next BEGIN cannot happen before the waiter observes
                // this committed prefix. No scheduling delay is assumed.
                resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
    });
    let ready = ready_rx.recv_timeout(Duration::from_secs(5));
    let observed = ready.is_ok().then(|| {
        take_migration_writer(&waiter, CHAIN, STALL, "Acquire migration lock").and_then(|taken| {
            match taken {
                Some(transaction) => validate_history(&transaction).map(|version| (true, version)),
                None => recorded_through(&waiter).map(|version| (false, version.unwrap_or(0))),
            }
        })
    });
    // Release and join before any assertion, including a setup failure. The
    // temporary directory cannot disappear while its producer is still live.
    let released = resume_tx.send(());
    let joined = chain.join();
    assert!(
        ready.is_ok(),
        "the committed-prefix signal was not received"
    );
    released.unwrap();
    joined.unwrap();
    assert_eq!(recorded_through(&waiter).unwrap(), Some(CHAIN));
    assert_eq!(
        observed.unwrap().unwrap(),
        (true, 4),
        "writer acquisition may precede migration completion"
    );
}

#[test]
fn the_waiting_migrator_completes_the_schema_before_the_paused_opener_resumes() {
    let directory = TestDirectory::new();
    let path = history_database(&directory, 0);
    let mut waiter = waiter(&path);
    set_foreign_keys(&waiter, true).unwrap();
    let (ready_tx, ready_rx) = sync_channel(1);
    let (resume_tx, resume_rx) = sync_channel(1);
    let chain_path = path.clone();
    let chain = thread::spawn(move || {
        let mut connection = Connection::open(&chain_path).unwrap();
        set_foreign_keys(&connection, true).unwrap();
        // This opener applies real SQL, not only synthetic history records.
        apply_through(&mut connection, 4).unwrap();
        ready_tx.send(()).unwrap();
        resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // The original opener must revalidate and skip the second one's work.
        apply_through(&mut connection, CHAIN).unwrap();
    });
    let schema = |connection: &Connection| -> rusqlite::Result<Vec<(String, String)>> {
        let mut query=connection.prepare("SELECT name,sql FROM sqlite_schema WHERE type IN ('table','index','trigger') AND name NOT LIKE 'sqlite_%' ORDER BY type,name")?;
        query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect()
    };
    let ready = ready_rx.recv_timeout(Duration::from_secs(5));
    let prefix_before_waiter = validate_history(&waiter);
    let returned = ready
        .is_ok()
        .then(|| apply_through(&mut waiter, CHAIN).and_then(|()| validate_history(&waiter)));
    // Both observations are made before permitting the first opener to resume.
    let schema_at_return = schema(&waiter);
    let released = resume_tx.send(());
    let joined = chain.join();
    assert!(ready.is_ok(), "the real four-version prefix was not ready");
    released.unwrap();
    joined.unwrap();
    assert_eq!(prefix_before_waiter.unwrap(), 4);
    assert_eq!(
        returned.unwrap().unwrap(),
        CHAIN,
        "the waiter itself must return a complete schema"
    );
    let expected = fresh_through(CHAIN).unwrap();
    assert_eq!(schema_at_return.unwrap(), schema(&expected).unwrap());
    assert_eq!(validate_history(&waiter).unwrap(), CHAIN);
    assert_eq!(schema(&waiter).unwrap(), schema(&expected).unwrap());
}
