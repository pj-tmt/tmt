//! Consumer-owned SQLite policy, with isolated processes for lock contention.
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::{
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use tmt_extension_state::{Layout, StateError, Store, UpdateError};

#[expect(
    dead_code,
    reason = "The shared worker also supports crash-recovery tests."
)]
mod support;
use support::Worker;

const RETRY: &str = "fixture-adopt --retry";

struct World(PathBuf);
impl World {
    fn new(mode: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-state-adopted-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let world = Self(root.canonicalize().unwrap());
        let layout = world.layout();
        // All filesystem admission happens before opening a SQLite connection.
        drop(layout.file("legacy.db").unwrap());
        let connection = world.connect();
        connection
            .pragma_update(None, "journal_mode", mode)
            .unwrap();
        connection
            .pragma_update(None, "synchronous", "FULL")
            .unwrap();
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        connection
            .pragma_update(None, "max_page_count", 8192)
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE records(id INTEGER PRIMARY KEY); INSERT INTO records VALUES(1);",
            )
            .unwrap();
        world
    }
    fn layout(&self) -> Layout {
        Layout::open(&self.0, "owned", &["legacy.db"]).unwrap()
    }
    fn path(&self) -> PathBuf {
        self.0.join("owned/legacy.db")
    }
    fn connect(&self) -> Connection {
        Connection::open(self.path()).unwrap()
    }
}
impl Drop for World {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn pragma(connection: &Connection, name: &str) -> i64 {
    connection
        .pragma_query_value(None, name, |r| r.get(0))
        .unwrap()
}

#[test]
fn adoption_preserves_legacy_schema_policy_and_once_only_callbacks() {
    for mode in ["DELETE", "WAL"] {
        let world = World::new(mode);
        let connection = world.connect();
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        connection
            .pragma_update(None, "synchronous", "FULL")
            .unwrap();
        connection
            .pragma_update(None, "max_page_count", 8192)
            .unwrap();
        connection.busy_timeout(Duration::ZERO).unwrap();
        let mut store = Store::from_admitted_connection(connection, RETRY).unwrap();
        let mut callbacks = 0;
        store
            .read(|tx| {
                callbacks += 1;
                assert_eq!(pragma(tx, "busy_timeout"), 5000);
                assert_eq!(pragma(tx, "synchronous"), 2);
                assert_eq!(pragma(tx, "foreign_keys"), 0);
                assert_eq!(pragma(tx, "max_page_count"), 8192);
                assert_eq!(pragma(tx, "query_only"), 1);
                let journal: String = tx.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
                assert_eq!(journal, mode.to_lowercase());
                assert!(tx.execute("INSERT INTO records VALUES(9)", []).is_err());
                assert_eq!(
                    tx.query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))?,
                    1
                );
                Ok::<_, rusqlite::Error>(())
            })
            .unwrap();
        store
            .write(|tx| {
                callbacks += 1;
                assert_eq!(pragma(tx, "query_only"), 0);
                tx.execute("INSERT INTO records VALUES(2)", [])?;
                Ok::<_, rusqlite::Error>(())
            })
            .unwrap();
        let rejected = store.write(|tx| {
            callbacks += 1;
            tx.execute("INSERT INTO records VALUES(3)", []).unwrap();
            Err::<(), _>("consumer refusal")
        });
        assert!(matches!(
            rejected,
            Err(UpdateError::Change("consumer refusal"))
        ));
        assert_eq!(callbacks, 3);
        store
            .read(|tx| {
                assert_eq!(pragma(tx, "busy_timeout"), 5000);
                assert_eq!(
                    tx.query_row("SELECT sum(id) FROM records", [], |r| r.get::<_, i64>(0))?,
                    3
                );
                assert_eq!(
                    tx.query_row(
                        "SELECT count(*) FROM sqlite_schema WHERE name='_tmt_state_migrations'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    0
                );
                Ok::<_, rusqlite::Error>(())
            })
            .unwrap();
        drop(store);
        if mode == "DELETE" {
            assert!(!world.path().with_file_name("legacy.db-wal").exists());
        }
        let before = fs::read(world.path()).unwrap();
        let error = Store::open(&world.layout(), "legacy.db", &[], RETRY)
            .err()
            .unwrap();
        assert!(matches!(error, StateError::Migration { .. }));
        assert_eq!(fs::read(world.path()).unwrap(), before);
    }
}

#[test]
fn adoption_refuses_memory_temporary_read_only_and_missing_retry() {
    for connection in [
        Connection::open_in_memory().unwrap(),
        Connection::open("").unwrap(),
    ] {
        assert!(matches!(
            Store::from_admitted_connection(connection, RETRY),
            Err(StateError::Invalid { .. })
        ));
    }
    let world = World::new("DELETE");
    let connection =
        Connection::open_with_flags(world.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(
        matches!(Store::from_admitted_connection(connection, RETRY), Err(StateError::Invalid { path, .. }) if path == world.path())
    );
    assert!(
        matches!(Store::from_admitted_connection(world.connect(), ""), Err(StateError::Invalid { path, .. }) if path == world.path())
    );
}

#[test]
fn adoption_refuses_open_transactions_without_committing_or_retaining_locks() {
    let mut refusals = Vec::new();
    for mode in ["DELETE", "WAL"] {
        for begin in ["BEGIN", "BEGIN IMMEDIATE", "SAVEPOINT consumer"] {
            let world = World::new(mode);
            let connection = world.connect();
            connection.execute_batch(begin).unwrap();
            connection
                .execute("INSERT INTO records VALUES(2)", [])
                .unwrap();
            assert!(!connection.is_autocommit());

            let result = Store::from_admitted_connection(connection, RETRY);
            refusals.push((
                mode,
                begin,
                matches!(
                    &result,
                    Err(StateError::Invalid { path, .. }) if *path == world.path()
                ),
            ));
            // The constructor consumes the connection. Ordinary drop cleanup
            // must roll back its uncommitted row and release its writer lock.
            drop(result);
            let observer = world.connect();
            observer.busy_timeout(Duration::ZERO).unwrap();
            assert_eq!(
                observer
                    .query_row("SELECT sum(id) FROM records", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                1,
                "{mode}: {begin} must not commit its pending row"
            );
            observer
                .execute("INSERT INTO records VALUES(3)", [])
                .unwrap();
        }
    }
    assert!(
        refusals.iter().all(|(_, _, refused)| *refused),
        "each open transaction must be refused with Invalid: {refusals:?}"
    );
}

#[test]
fn read_restores_the_consumers_query_only_policy_on_success_and_rejection() {
    for query_only in [false, true] {
        let world = World::new("DELETE");
        let connection = world.connect();
        connection
            .pragma_update(None, "query_only", query_only)
            .unwrap();
        let mut store = Store::from_admitted_connection(connection, RETRY).unwrap();
        for reject in [false, true] {
            let result = store.read(|tx| {
                assert_eq!(pragma(tx, "query_only"), 1);
                if reject { Err("rejected read") } else { Ok(()) }
            });
            assert_eq!(result.is_err(), reject);
            let mut callbacks = 0;
            let result = store.write(|_| {
                callbacks += 1;
                Ok::<_, rusqlite::Error>(())
            });
            assert_eq!(result.is_err(), query_only);
            assert_eq!(callbacks, usize::from(!query_only));
        }
    }
}

#[test]
fn rollback_commit_waits_for_a_reader_but_wal_does_not() {
    for mode in ["DELETE", "WAL"] {
        let world = World::new(mode);
        // Open this descriptor BEFORE the reader, and close it only AFTER the
        // reader. Otherwise the test itself would release SQLite's POSIX locks.
        let probe = fs::File::open(world.path()).unwrap();
        let mut reader = world.connect();
        let snapshot = reader.transaction().unwrap();
        snapshot
            .query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
            .unwrap();
        let mut worker = Worker::spawn(&world.0, 0, "process_worker", "commit-success");
        worker.ready();
        worker.release();
        if mode == "DELETE" {
            wait_for_pending_lock(&probe);
            drop(snapshot);
            worker.finish();
        } else {
            // The read snapshot stays alive through the successful WAL commit.
            worker.finish();
            drop(snapshot);
        }
        drop(reader);
        drop(probe);
        assert_eq!(
            world
                .connect()
                .query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}

fn wait_for_pending_lock(file: &fs::File) {
    // SQLite's default pending byte is held exclusively while a rollback writer
    // is waiting for readers at COMMIT. F_GETLK ignores this process's own locks.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut lock = nix::libc::flock {
            l_start: 0x4000_0000,
            l_len: 1,
            l_pid: 0,
            l_type: nix::libc::F_WRLCK as _,
            l_whence: nix::libc::SEEK_SET as _,
        };
        nix::fcntl::fcntl(file, nix::fcntl::FcntlArg::F_GETLK(&mut lock)).unwrap();
        if lock.l_type == nix::libc::F_WRLCK as _ && lock.l_pid != std::process::id() as i32 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "writer never reached pending COMMIT"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn rollback_commit_uses_the_remaining_window_and_reports_uncertain_timeout() {
    let world = World::new("DELETE");
    let mut reader = world.connect();
    let snapshot = reader.transaction().unwrap();
    snapshot
        .query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
        .unwrap();
    let mut worker = Worker::spawn(&world.0, 0, "process_worker", "commit-timeout");
    worker.ready();
    worker.release();
    worker.finish();
    drop(snapshot);
    assert_eq!(
        reader
            .query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn adopted_begin_waits_five_seconds_without_running_the_callback() {
    for mode in ["DELETE", "WAL"] {
        let world = World::new(mode);
        let mut writer = world.connect();
        let transaction = writer
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let mut worker = Worker::spawn(&world.0, 0, "process_worker", "begin-timeout");
        worker.ready();
        worker.release();
        worker.finish();
        drop(transaction);
    }
}

#[test]
fn process_worker() {
    let Some(root) = std::env::var_os("TMT_STATE_WORKER_ROOT") else {
        return;
    };
    let mode = std::env::var("TMT_STATE_WORKER_MODE").unwrap();
    let connection = Connection::open(PathBuf::from(root).join("owned/legacy.db")).unwrap();
    let journal: String = connection
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    let mut store = Store::from_admitted_connection(connection, RETRY).unwrap();
    println!("STATE_READY");
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let start = Instant::now();
    let mut callbacks = 0;
    let result = store.write(|tx| {
        callbacks += 1;
        tx.execute("INSERT INTO records VALUES(2)", [])?;
        if mode == "commit-timeout" {
            // Deliberately consume part of the transaction's time allowance;
            // this is workload, not a readiness delay or a lock-release guess.
            std::thread::sleep(Duration::from_secs(2));
        }
        Ok::<_, rusqlite::Error>(())
    });
    if mode == "commit-success" {
        result.unwrap();
        assert_eq!(callbacks, 1);
        if journal == "wal" {
            store
                .read(|tx| {
                    assert_eq!(pragma(tx, "busy_timeout"), 5000);
                    Ok::<_, rusqlite::Error>(())
                })
                .unwrap();
        }
    } else {
        let Err(UpdateError::State(StateError::Timeout {
            path,
            operation,
            retry,
            may_not_be_saved,
            ..
        })) = result
        else {
            panic!("expected typed timeout, got {result:?}");
        };
        assert!(path.ends_with("owned/legacy.db"));
        assert_eq!(retry, RETRY);
        assert_eq!(
            operation,
            if mode == "begin-timeout" {
                "begin write"
            } else {
                "commit write"
            }
        );
        assert_eq!(may_not_be_saved, mode == "commit-timeout");
        assert_eq!(callbacks, usize::from(mode == "commit-timeout"));
        assert!(start.elapsed() >= Duration::from_millis(4700));
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the callback must not renew the window"
        );
        if mode == "commit-timeout" {
            store
                .read(|tx| {
                    assert_eq!(pragma(tx, "busy_timeout"), 5000);
                    Ok::<_, rusqlite::Error>(())
                })
                .unwrap();
        }
    }
}
