//! Shared mutation entries on disposable state, without a consumer migration.
use std::{
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_extension_state::{Document, Guard, Layout, Migration, Store};

mod support;
use support::Worker;

const RETRY: &str = "fixture-write --retry";
const FILES: &[&str] = &["state.db"];
const MIGRATIONS: &[Migration<'_>] = &[Migration {
    name: "records",
    sql: "CREATE TABLE records(id INTEGER PRIMARY KEY, value TEXT NOT NULL);",
}];

struct World(PathBuf);
impl World {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-state-shared-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn layout(&self) -> Layout {
        Layout::open(&self.0, "owned", FILES).unwrap()
    }
}
impl Drop for World {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn store_commits_once_and_rolls_back_a_rejected_change() {
    let world = World::new();
    let layout = world.layout();
    let mut store = Store::open(&layout, "state.db", MIGRATIONS, RETRY).unwrap();
    store
        .write(|transaction| {
            transaction.execute("INSERT INTO records VALUES(1,'kept')", [])?;
            Ok::<_, rusqlite::Error>(())
        })
        .unwrap();
    let rejected = store.write(|transaction| {
        transaction
            .execute("INSERT INTO records VALUES(2,'rolled back')", [])
            .map_err(io::Error::other)?;
        Err::<(), _>(io::Error::other("consumer refused the change"))
    });
    assert!(rejected.is_err());
    drop(store);
    let database = rusqlite::Connection::open(layout.directory.join("state.db")).unwrap();
    let rows: Vec<(i64, String)> = database
        .prepare("SELECT id,value FROM records ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(rows, [(1, "kept".into())]);
}

#[test]
fn document_changes_current_bytes_and_preserves_them_on_rejection() {
    let world = World::new();
    let state = world.layout();
    let project = world.0.join("project");
    fs::create_dir(&project).unwrap();
    let destination = project.join("settings.json");
    let document = Document::new(&state.directory, &destination, 128, RETRY).unwrap();
    assert!(document.read().unwrap().is_none());
    assert_eq!(fs::read_dir(&project).unwrap().count(), 0);
    document
        .update(|current| {
            assert!(current.is_none());
            Ok::<_, io::Error>(b"first".to_vec())
        })
        .unwrap();
    document
        .update(|current| {
            let mut bytes = current.unwrap().to_vec();
            bytes.extend_from_slice(b"+second");
            Ok::<_, io::Error>(bytes)
        })
        .unwrap();
    let rejected = document.update(|_| Err::<Vec<u8>, _>(io::Error::other("rejected")));
    assert!(rejected.is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"first+second");
    let entries: Vec<_> = fs::read_dir(&project)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, ["settings.json"]);
}

#[test]
fn opening_a_second_store_preserves_the_first_stores_main_file_lock() {
    let world = World::new();
    let layout = world.layout();
    let probe = |held| {
        Worker::spawn(
            &world.0,
            i64::from(std::process::id()),
            "process_worker",
            if held {
                "main-file-lock"
            } else {
                "main-file-unlocked"
            },
        )
        .finish();
    };
    let first = Store::open(&layout, "state.db", MIGRATIONS, RETRY).unwrap();
    probe(true);
    eprintln!("main-file lock verified before opening the second Store");
    let second = Store::open(&layout, "state.db", MIGRATIONS, RETRY).unwrap();
    probe(true);
    drop(second);
    probe(true);
    drop(first);
    probe(false);
}

#[test]
fn guard_collision_names_the_holder_and_exact_retry() {
    let world = World::new();
    let layout = world.layout();
    let path = layout.directory.join("runner.lock");
    let held = Guard::acquire(&path, RETRY).unwrap();
    let collision = Guard::acquire(&path, RETRY).err().unwrap();
    let holder = collision
        .holder()
        .expect("held guard published its identity");
    assert_eq!(holder.pid, std::process::id());
    assert!(holder.acquired_at_ms > 0);
    let message = collision.to_string();
    assert!(message.contains(&std::process::id().to_string()));
    assert!(message.contains(&path.display().to_string()));
    assert!(message.contains(RETRY));
    drop(held);
    assert!(Guard::acquire(&path, RETRY).is_ok());
}
#[test]
fn process_worker() {
    let Some(root) = std::env::var_os("TMT_STATE_WORKER_ROOT") else {
        return;
    };
    let id: i64 = std::env::var("TMT_STATE_WORKER_ID")
        .unwrap()
        .parse()
        .unwrap();
    let root = PathBuf::from(root);
    let layout = Layout::open(&root, "owned", FILES).unwrap();
    let mode = std::env::var("TMT_STATE_WORKER_MODE").unwrap();
    if matches!(mode.as_str(), "main-file-lock" | "main-file-unlocked") {
        // Probe only in a different process: closing a main-file descriptor in
        // the Store process would itself release that process's POSIX locks.
        let file = fs::File::open(layout.directory.join("state.db")).unwrap();
        let mut lock = nix::libc::flock {
            l_start: 0,
            l_len: 0,
            l_pid: 0,
            l_type: nix::libc::F_WRLCK as _,
            l_whence: nix::libc::SEEK_SET as _,
        };
        nix::fcntl::fcntl(&file, nix::fcntl::FcntlArg::F_GETLK(&mut lock)).unwrap();
        assert_eq!(
            lock.l_type != nix::libc::F_UNLCK as _,
            mode == "main-file-lock",
            "the live Stores' main-file lock must remain held until both close"
        );
        if mode == "main-file-lock" {
            assert_eq!(
                i64::from(lock.l_pid),
                id,
                "the Store process holds the lock"
            );
        }
        return;
    }
    if std::env::var("TMT_STATE_WORKER_MODE").unwrap() == "wal-index-lock" {
        // SQLite's "dead man switch" (byte 128 of the WAL index): a connection holds a
        // shared lock on it for as long as it lives, and a process that finds it free
        // concludes it is the first and resets the index. Report who holds it.
        let index = std::fs::File::open(layout.directory.join("state.db-shm")).unwrap();
        let mut lock = nix::libc::flock {
            l_start: 128,
            l_len: 1,
            l_pid: 0,
            l_type: nix::libc::F_WRLCK as _,
            l_whence: nix::libc::SEEK_SET as _,
        };
        nix::fcntl::fcntl(&index, nix::fcntl::FcntlArg::F_GETLK(&mut lock)).unwrap();
        println!(
            "WAL_INDEX_LOCK_HELD={}",
            lock.l_type != nix::libc::F_UNLCK as _
        );
        return;
    }
    let mut store = Store::open(&layout, "state.db", MIGRATIONS, RETRY).unwrap();
    println!("STATE_READY");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "go");
    if std::env::var("TMT_STATE_WORKER_MODE").unwrap() == "held-transaction" {
        store
            .write(|tx| {
                tx.execute("INSERT INTO records VALUES(999,'uncommitted')", [])?;
                println!("STATE_HELD");
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
                let mut wait = String::new();
                std::io::stdin().read_line(&mut wait).unwrap();
                Ok::<_, rusqlite::Error>(())
            })
            .unwrap();
        return;
    }
    store
        .write(|tx| {
            tx.execute(
                "INSERT INTO records VALUES(?1,?2)",
                rusqlite::params![id, format!("writer-{id}")],
            )
        })
        .unwrap();
    if std::env::var("TMT_STATE_WORKER_MODE").unwrap() == "committed" {
        println!("STATE_COMMITTED");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
        let mut wait = String::new();
        std::io::stdin().read_line(&mut wait).unwrap();
        return;
    }
    let document = Document::new(
        &layout.directory,
        &root.join(
            if std::env::var("TMT_STATE_WORKER_MODE").unwrap() == "case-alias" && id % 2 == 1 {
                "project/SETTINGS.JSON"
            } else {
                "project/settings.json"
            },
        ),
        8192,
        RETRY,
    )
    .unwrap();
    document
        .update(|current| {
            let mut next = current.unwrap_or_default().to_vec();
            next.extend_from_slice(format!("writer-{id}\n").as_bytes());
            Ok::<_, io::Error>(next)
        })
        .unwrap();
}

#[test]
fn eight_processes_preserve_every_store_and_document_mutation_once() {
    concurrent_mutations("mutate");
}

#[test]
fn eight_case_alias_writers_share_one_document_admission() {
    concurrent_mutations("case-alias");
}

fn concurrent_mutations(mode: &str) {
    let world = World::new();
    let project = world.0.join("project");
    fs::create_dir(&project).unwrap();
    if mode == "case-alias" {
        fs::write(project.join("probe"), b"case probe").unwrap();
        match fs::metadata(project.join("PROBE")) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                eprintln!(
                    "case-alias concurrency test skipped: this test volume is case-sensitive"
                );
                return;
            }
            result => {
                result.unwrap();
            }
        }
        fs::remove_file(project.join("probe")).unwrap();
    }
    let mut children = Vec::new();
    for id in 0..8 {
        let mut child = Worker::spawn(&world.0, id, "process_worker", mode);
        child.ready();
        children.push(child);
    }
    for child in &mut children {
        child.release();
    }
    for child in children {
        child.finish();
    }
    let mut store = Store::open(&world.layout(), "state.db", MIGRATIONS, RETRY).unwrap();
    let rows = store
        .read(|tx| {
            tx.prepare("SELECT id,value FROM records ORDER BY id")?
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap();
    assert_eq!(
        rows,
        (0..8)
            .map(|id| (id, format!("writer-{id}")))
            .collect::<Vec<_>>()
    );
    let bytes = fs::read_to_string(world.0.join("project/settings.json")).unwrap();
    let mut lines = bytes.lines().collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(
        lines,
        (0..8).map(|id| format!("writer-{id}")).collect::<Vec<_>>()
    );
    assert_eq!(fs::read_dir(world.0.join("project")).unwrap().count(), 1);
}

#[test]
fn killed_store_writer_rolls_back_and_waiter_gets_one_bounded_timeout() {
    let world = World::new();
    let mut store = Store::open(&world.layout(), "state.db", MIGRATIONS, RETRY).unwrap();
    let mut holder = Worker::spawn(&world.0, 0, "process_worker", "held-transaction");
    holder.ready();
    holder.release();
    holder.until("STATE_HELD");
    let started = std::time::Instant::now();
    let mut invoked = false;
    let error = store
        .write(|_| {
            invoked = true;
            Ok::<_, io::Error>(())
        })
        .unwrap_err();
    let tmt_extension_state::UpdateError::State(error) = error else {
        panic!("state timeout expected")
    };
    assert!(error.is_timeout());
    assert!(!error.may_not_be_saved());
    assert!(!invoked);
    assert!(started.elapsed() >= std::time::Duration::from_secs(4));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    let message = error.to_string();
    assert!(message.contains("state.db") && message.contains(RETRY));
    assert!(std::error::Error::source(&error).is_some());
    holder.kill_and_reap();
    assert_eq!(
        store
            .read(|tx| tx.query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0)))
            .unwrap(),
        0
    );
    store
        .write(|tx| tx.execute("INSERT INTO records VALUES(1,'survives reopen')", []))
        .unwrap();
    drop(store);
    let mut reopened = Store::open(&world.layout(), "state.db", MIGRATIONS, RETRY).unwrap();
    assert_eq!(
        reopened
            .read(|tx| tx.query_row("SELECT value FROM records", [], |r| r.get::<_, String>(0)))
            .unwrap(),
        "survives reopen"
    );
}

#[test]
fn newer_history_is_typed_without_changing_legacy_errors_or_store_bytes() {
    let world = World::new();
    let layout = world.layout();
    let path = layout.directory.join("state.db");
    let future = [
        Migration {
            name: "records",
            sql: MIGRATIONS[0].sql,
        },
        Migration {
            name: "future",
            sql: "ALTER TABLE records ADD COLUMN note TEXT;",
        },
    ];
    let mut store = Store::open(&layout, "state.db", &future, RETRY).unwrap();
    store
        .write(|tx| {
            tx.execute(
                "INSERT INTO records(id,value,note) VALUES(1,'kept','future')",
                [],
            )
        })
        .unwrap();
    drop(store);
    let bytes = fs::read(&path).unwrap();
    let entries = || {
        let mut names: Vec<_> = fs::read_dir(&layout.directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let before = entries();
    let error = Store::open(&layout, "state.db", MIGRATIONS, RETRY)
        .err()
        .unwrap();
    assert!(error.is_newer());
    // Preserve the existing variant/display until consumers adopt the predicate.
    assert!(
        matches!(&error, tmt_extension_state::StateError::Migration { path: actual, reason }
        if actual == &path && *reason == "store is newer than this binary; use the newer binary")
    );
    assert_eq!(
        error.to_string(),
        format!(
            "{}: store is newer than this binary; use the newer binary",
            path.display()
        )
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(entries(), before);

    let non_prefix = [
        Migration {
            name: "different",
            sql: MIGRATIONS[0].sql,
        },
        Migration {
            name: "future",
            sql: future[1].sql,
        },
    ];
    let error = Store::open(&layout, "state.db", &non_prefix, RETRY)
        .err()
        .unwrap();
    assert!(matches!(
        error,
        tmt_extension_state::StateError::Migration { .. }
    ));
    assert!(!error.is_newer());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(entries(), before);
    let mut store = Store::open(&layout, "state.db", &future, RETRY).unwrap();
    assert_eq!(
        store
            .read(
                |tx| tx.query_row("SELECT value,note FROM records", [], |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?
                )))
            )
            .unwrap(),
        ("kept".into(), "future".into())
    );
}

#[test]
fn foreign_history_and_unrelated_errors_are_not_newer() {
    use tmt_extension_state::StateError;
    let world = World::new();
    let layout = world.layout();
    let path = layout.directory.join("state.db");
    drop(layout.file("state.db").unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE foreign_records(value); INSERT INTO foreign_records VALUES(7);",
        )
        .unwrap();
    drop(connection);
    let bytes = fs::read(&path).unwrap();
    let error = Store::open(&layout, "state.db", MIGRATIONS, RETRY)
        .err()
        .unwrap();
    assert!(matches!(error, StateError::Migration { .. }));
    assert!(!error.is_newer());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    for error in [
        StateError::Invalid {
            path: path.clone(),
            reason: "newer than this binary",
        },
        StateError::Unsafe {
            path: path.clone(),
            reason: "unsafe file",
        },
        StateError::Migration {
            path: path.clone(),
            reason: "unrelated newer than this binary text",
        },
        StateError::Io {
            path,
            operation: "read state",
            source: io::Error::other("newer than this binary"),
        },
    ] {
        assert!(!error.is_newer(), "unrelated error: {error}");
    }
}

#[test]
fn migrations_are_forward_only_atomic_and_queries_are_read_only() {
    let world = World::new();
    let layout = world.layout();
    let mut store = Store::open(&layout, "state.db", MIGRATIONS, RETRY).unwrap();
    assert!(
        store
            .read(|tx| tx.execute("INSERT INTO records VALUES(1,'forbidden')", []))
            .is_err()
    );
    store
        .write(|tx| tx.execute("INSERT INTO records VALUES(2,'allowed')", []))
        .unwrap();
    assert_eq!(
        store
            .read(|tx| tx.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0)))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .read(|tx| tx.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0)))
            .unwrap(),
        "wal"
    );
    drop(store);
    assert!(Store::open(&layout, "state.db", &[], RETRY).is_err());
    assert!(
        Store::open(
            &layout,
            "state.db",
            &[Migration {
                name: "changed",
                sql: MIGRATIONS[0].sql
            }],
            RETRY
        )
        .is_err()
    );
    let failed = [
        Migration {
            name: "records",
            sql: MIGRATIONS[0].sql,
        },
        Migration {
            name: "failed",
            sql: "CREATE TABLE leaked(id); THIS IS INVALID;",
        },
    ];
    assert!(Store::open(&layout, "state.db", &failed, RETRY).is_err());
    let mut restored = Store::open(&layout, "state.db", MIGRATIONS, RETRY).unwrap();
    assert_eq!(
        restored
            .read(|tx| tx.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='leaked'",
                [],
                |r| r.get::<_, i64>(0)
            ))
            .unwrap(),
        0
    );
    assert_eq!(
        restored
            .read(
                |tx| tx.query_row("SELECT count(*) FROM _tmt_state_migrations", [], |r| r
                    .get::<_, i64>(0))
            )
            .unwrap(),
        1
    );
}

#[test]
fn document_preserves_modes_bounds_and_foreign_entries() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let world = World::new();
    let project = world.0.join("project");
    fs::create_dir(&project).unwrap();
    let target = project.join("settings");
    let document = Document::new(&world.0.join("state"), &target, 8, RETRY).unwrap();
    assert!(document.read().unwrap().is_none());
    assert!(!world.0.join("state").exists());
    document
        .update(|_| Ok::<_, io::Error>(b"initial".to_vec()))
        .unwrap();
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    document
        .update(|_| Ok::<_, io::Error>(b"changed".to_vec()))
        .unwrap();
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(document.update(|_| Ok::<_, io::Error>(vec![0; 9])).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"changed");
    fs::write(&target, b"too large for this bound").unwrap();
    let mut oversized_called = false;
    assert!(
        document
            .update(|_| {
                oversized_called = true;
                Ok::<_, io::Error>(vec![])
            })
            .is_err()
    );
    assert!(!oversized_called);
    assert!(document.read().is_err());
    fs::write(&target, b"changed").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(document.update(|_| Ok::<_, io::Error>(vec![])).is_err());
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o666
    );
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    let stage = project.join(".settings.tmt-staging");
    symlink(&target, &stage).unwrap();
    let mut invoked = false;
    assert!(
        document
            .update(|_| {
                invoked = true;
                Ok::<_, io::Error>(vec![])
            })
            .is_err()
    );
    assert!(!invoked);
    assert!(
        fs::symlink_metadata(&stage)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_file(&stage).unwrap();
    fs::remove_file(&target).unwrap();
    symlink(project.join("missing"), &target).unwrap();
    assert!(document.read().is_err());
    assert!(document.update(|_| Ok::<_, io::Error>(vec![])).is_err());
    assert_eq!(fs::read_dir(&project).unwrap().count(), 1);
}

#[test]
fn an_unpublished_legacy_guard_holder_is_honestly_unknown() {
    let world = World::new();
    let layout = Layout::open(&world.0, "owned", &["legacy.lock"]).unwrap();
    let _held = layout.lock("legacy.lock").unwrap();
    let path = layout.directory.join("legacy.lock");
    let collision = Guard::acquire(&path, RETRY).err().unwrap();
    assert!(collision.holder().is_none());
    assert!(collision.to_string().contains("holder unknown"));
    assert!(path.exists());
}

#[test]
fn a_failed_commit_rolls_back_without_replaying_the_consumer() {
    let world = World::new();
    let migrations = [Migration {
        name: "references",
        sql: "CREATE TABLE parent(id INTEGER PRIMARY KEY); CREATE TABLE child(parent_id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);",
    }];
    let mut store = Store::open(&world.layout(), "state.db", &migrations, RETRY).unwrap();
    let mut calls = 0;
    let failed = store.write(|tx| {
        calls += 1;
        assert_eq!(
            tx.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))?,
            0
        );
        tx.execute("INSERT INTO child VALUES(1)", [])
    });
    assert!(matches!(
        failed,
        Err(tmt_extension_state::UpdateError::State(
            tmt_extension_state::StateError::Sqlite { .. }
        ))
    ));
    assert_eq!(calls, 1);
    assert_eq!(
        store
            .read(|tx| tx.query_row("SELECT count(*) FROM child", [], |r| r.get::<_, i64>(0)))
            .unwrap(),
        0
    );
    store
        .write(|tx| tx.execute("INSERT INTO parent VALUES(1)", []))
        .unwrap();
    store
        .write(|tx| tx.execute("INSERT INTO child VALUES(1)", []))
        .unwrap();
}

#[test]
fn store_database_and_sidecars_are_private_and_unsafe_inputs_are_not_repaired() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let world = World::new();
    let layout = world.layout();
    let store = Store::open(&layout, "state.db", MIGRATIONS, RETRY).unwrap();
    let database = layout.directory.join("state.db");
    for suffix in ["", "-wal", "-shm"] {
        let path = layout.directory.join(format!("state.db{suffix}"));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    drop(store);
    fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Store::open(&layout, "state.db", MIGRATIONS, RETRY).is_err());
    assert_eq!(
        fs::metadata(&database).unwrap().permissions().mode() & 0o777,
        0o644
    );
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    let foreign = world.0.join("foreign");
    fs::write(&foreign, b"unchanged").unwrap();
    fs::set_permissions(&foreign, fs::Permissions::from_mode(0o600)).unwrap();
    let sidecar = layout.directory.join("state.db-wal");
    symlink(&foreign, &sidecar).unwrap();
    let error = Store::open(&layout, "state.db", MIGRATIONS, RETRY)
        .err()
        .unwrap();
    let tmt_extension_state::StateError::Io {
        path,
        operation,
        source,
    } = error
    else {
        panic!("the sidecar link must fail at no-follow admission, not later SQLite decoding");
    };
    assert_eq!(path, sidecar);
    assert_eq!(operation, "admit SQLite sidecar");
    assert_eq!(source.raw_os_error(), Some(nix::errno::Errno::ELOOP as i32));
    assert_eq!(fs::read(foreign).unwrap(), b"unchanged");
}

#[test]
fn existing_store_admission_refuses_links_directories_and_fifos_without_repair() {
    use std::os::unix::fs::{MetadataExt, symlink};
    use tmt_extension_state::StateError;
    let world = World::new();
    let layout = world.layout();
    let path = layout.directory.join("state.db");
    let foreign = world.0.join("foreign");
    fs::write(&foreign, b"unchanged").unwrap();
    for kind in ["link", "directory", "fifo"] {
        match kind {
            "link" => symlink(&foreign, &path).unwrap(),
            "directory" => fs::create_dir(&path).unwrap(),
            "fifo" => nix::unistd::mkfifo(
                &path,
                nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
            )
            .unwrap(),
            _ => unreachable!(),
        }
        let before = fs::symlink_metadata(&path).unwrap();
        let error = Store::open(&layout, "state.db", MIGRATIONS, RETRY)
            .err()
            .unwrap();
        if kind == "link" {
            let StateError::Io {
                path: refused,
                operation,
                source,
            } = error
            else {
                panic!("a link must retain the no-follow refusal");
            };
            assert_eq!(refused, path);
            assert_eq!(operation, "admit state path");
            assert_eq!(source.raw_os_error(), Some(nix::errno::Errno::ELOOP as i32));
            assert!(
                fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        } else {
            assert!(matches!(error, StateError::Unsafe { path: refused, .. } if refused == path));
        }
        let after = fs::symlink_metadata(&path).unwrap();
        assert_eq!(
            (after.ino(), after.mode(), after.uid()),
            (before.ino(), before.mode(), before.uid())
        );
        assert_eq!(fs::read(&foreign).unwrap(), b"unchanged");
        assert_eq!(fs::read_dir(&layout.directory).unwrap().count(), 1);
        if kind == "directory" {
            fs::remove_dir(&path).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
    }
}

#[test]
fn a_kill_after_store_commit_keeps_the_complete_record() {
    let world = World::new();
    let mut child = Worker::spawn(&world.0, 7, "process_worker", "committed");
    child.ready();
    child.release();
    child.until("STATE_COMMITTED");
    child.kill_and_reap();
    let mut store = Store::open(&world.layout(), "state.db", MIGRATIONS, RETRY).unwrap();
    let rows = store
        .read(|tx| {
            tx.query_row("SELECT id,value FROM records", [], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
        })
        .unwrap();
    assert_eq!(rows, (7, "writer-7".to_owned()));
}

#[test]
fn an_unrecognized_database_is_refused_before_journal_or_schema_changes() {
    for table in ["foreign_data", "sqlitex_data"] {
        let world = World::new();
        let layout = world.layout();
        drop(layout.file("state.db").unwrap());
        let path = layout.directory.join("state.db");
        let foreign = rusqlite::Connection::open(&path).unwrap();
        foreign
            .execute_batch(&format!(
                "CREATE TABLE {table}(value TEXT); INSERT INTO {table} VALUES('keep');"
            ))
            .unwrap();
        drop(foreign);
        let before = fs::read(&path).unwrap();
        let error = match Store::open(&layout, "state.db", MIGRATIONS, RETRY) {
            Ok(_) => panic!("unrecognized table {table} was admitted"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            tmt_extension_state::StateError::Migration { .. }
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(layout.directory).unwrap().count(), 1);
    }
}

#[test]
fn concurrent_first_opens_of_a_new_store_all_succeed_and_migrate_once() {
    for round in 0..20 {
        let world = World::new();
        let layout = world.layout();
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (layout, start) = (layout.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    Store::open(&layout, "state.db", MIGRATIONS, RETRY).map(drop)
                })
            })
            .collect();
        for thread in threads {
            thread
                .join()
                .unwrap()
                .unwrap_or_else(|error| panic!("round {round}: {error}"));
        }
        let history: i64 = rusqlite::Connection::open(layout.directory.join("state.db"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM _tmt_state_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(history, 1, "round {round}");
    }
}

#[test]
fn opening_a_store_keeps_its_wal_index_lock_so_another_process_cannot_reset_the_index() {
    let world = World::new();
    let layout = world.layout();
    let mut store = Store::open(&layout, "state.db", MIGRATIONS, RETRY).unwrap();
    store
        .write(|tx| tx.execute("INSERT INTO records VALUES(1,'first')", []))
        .unwrap();
    // Admitting SQLite's sidecars by opening and closing them would release this process's
    // POSIX locks on them (closing any descriptor does), and with them the lock below.
    let mut probe = Worker::spawn(&world.0, 2, "process_worker", "wal-index-lock");
    probe.until("WAL_INDEX_LOCK_HELD=true");
    probe.finish();
    drop(store);
}
