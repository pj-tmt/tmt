use super::*;
use crate::checklist::{
    model::{self, InventoryExpectation, Mutation},
    test_support::*,
};
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt, symlink},
    sync::{Arc, Barrier, mpsc},
    time::{Duration, Instant},
};

fn insert(store: &Store) -> Result<(), Error> {
    store.update(
        |d| {
            model::apply(
                d,
                &create(ITEM, InventoryExpectation::Absent, None),
                None,
                &stamp(),
            )
            .map(|outcome| ((), outcome.event))
        },
        || Ok(()),
    )
}
fn complete(store: &Store, item: &str, revision: u64) -> Result<(), Error> {
    store.update(
        |d| {
            model::apply(
                d,
                &mutate(item, revision, Mutation::Complete),
                None,
                &stamp(),
            )
            .map(|outcome| ((), outcome.event))
        },
        || Ok(()),
    )
}
fn store(f: &Fixture) -> Store {
    Store::new(&f.root, id(ROOM), None, "ops").unwrap()
}
/// Ops creates its state directories private; so does the fixture for a legacy tree.
fn legacy_directory(f: &Fixture) {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(f.directory())
        .unwrap();
}
/// Archived copies of the legacy document, sorted by name.
fn archived(f: &Fixture) -> Vec<PathBuf> {
    let mut found: Vec<_> = fs::read_dir(f.directory())
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("items.json.migrated-")
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}
fn item(n: u64) -> String {
    format!("{n:08x}-0000-4000-8000-000000000000")
}

#[test]
fn absence_reads_are_inert_and_corruption_is_not_empty() {
    let f = Fixture::new();
    let store = store(&f);
    assert!(store.read().unwrap().is_none());
    assert!(!f.root.join("ops").exists());
    insert(&store).unwrap();
    for bytes in [b"".as_slice(), b"{broken", b"{\"version\":2}"] {
        f.overwrite(bytes);
        assert_eq!(store.read().unwrap_err().code, Code::StorageError);
        assert_eq!(insert(&store).unwrap_err().code, Code::StorageError);
        assert_eq!(f.bytes(), bytes);
    }
}

#[test]
fn concurrent_first_create_preserves_one_inventory() {
    let f = Fixture::new();
    let barrier = Arc::new(Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let root = f.root.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = Store::new(&root, id(ROOM), None, "ops").unwrap();
                barrier.wait();
                insert(&store)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let error = results.into_iter().find_map(Result::err).unwrap();
    assert!(matches!(error.code, Code::Conflict | Code::StorageError));
    let d = store(&f).read().unwrap().unwrap();
    assert_eq!(d.items.len(), 1);
    assert_eq!(d.inventory_revision, 1);
}

#[test]
fn a_read_and_a_write_report_busy_after_the_budget_without_changing_bytes() {
    let f = Fixture::new();
    let store = store(&f);
    insert(&store).unwrap();
    let before = f.bytes();
    // Another process holding the database's write transaction for longer than the budget.
    let holder = f.database();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = Instant::now();
    let read = std::thread::spawn({
        let root = f.root.clone();
        move || Store::new(&root, id(ROOM), None, "ops").unwrap().read()
    });
    let write = std::thread::spawn({
        let root = f.root.clone();
        move || complete(&Store::new(&root, id(ROOM), None, "ops").unwrap(), ITEM, 1)
    });
    let (read, write) = (read.join().unwrap(), write.join().unwrap());
    assert!(
        start.elapsed() >= Duration::from_secs(4),
        "failed before the budget: {:?}",
        start.elapsed()
    );
    assert_eq!(read.unwrap_err().code, Code::StorageError);
    let error = write.unwrap_err();
    assert_eq!(error.code, Code::StorageError);
    assert!(
        error.message.contains("nothing was saved")
            && error.message.ends_with("Run the same command again."),
        "{}",
        error.message
    );
    assert_eq!(f.bytes(), before);
    holder.execute_batch("ROLLBACK").unwrap();
    drop(holder);
    complete(&store, ITEM, 1).unwrap();
}

#[test]
fn unsafe_directories_and_databases_refuse_without_following_or_repair() {
    let f = Fixture::new();
    let store = store(&f);
    let outside = f.root.join("foreign");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, f.root.join("ops")).unwrap();
    assert_eq!(store.read().unwrap_err().code, Code::StorageError);
    assert_eq!(insert(&store).unwrap_err().code, Code::StorageError);
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
    fs::remove_file(f.root.join("ops")).unwrap();
    insert(&store).unwrap();
    let database = f.root.join("ops/ops.db");
    let copy = f.root.join("foreign.db");
    fs::copy(&database, &copy).unwrap();
    let before = fs::read(&copy).unwrap();
    fs::remove_file(&database).unwrap();
    symlink(&copy, &database).unwrap();
    assert_eq!(store.read().unwrap_err().code, Code::StorageError);
    assert_eq!(
        complete(&store, ITEM, 1).unwrap_err().code,
        Code::StorageError
    );
    assert_eq!(fs::read(&copy).unwrap(), before);
}

#[test]
fn a_newer_database_refuses_with_an_upgrade_hint_and_changes_nothing() {
    let f = Fixture::new();
    let store = store(&f);
    insert(&store).unwrap();
    let before = f.bytes();
    f.database()
        .execute(
            "INSERT INTO _tmt_state_migrations(version, name, sql) SELECT MAX(version) + 1, 'later', 'SELECT 1' FROM _tmt_state_migrations",
            [],
        )
        .unwrap();
    for error in [
        store.read().unwrap_err(),
        complete(&store, ITEM, 1).unwrap_err(),
    ] {
        assert_eq!(error.code, Code::StorageError);
        assert!(error.message.contains("Upgrade Ops"), "{}", error.message);
    }
    assert_eq!(f.bytes(), before);
}

#[test]
fn the_directory_and_database_are_private_and_no_checklist_files_are_created() {
    let f = Fixture::new();
    let store = store(&f);
    insert(&store).unwrap();
    assert_eq!(
        fs::metadata(f.root.join("ops"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(f.root.join("ops/ops.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!f.root.join("ops/checklist").exists());
}

#[test]
fn a_refused_publication_and_an_unacknowledged_commit_keep_their_bytes_and_their_names() {
    let f = Fixture::new();
    let store = store(&f);
    insert(&store).unwrap();
    let before = f.bytes();
    // The caller's own last check refuses: nothing is saved.
    let error = store
        .update(
            |d| {
                model::apply(d, &mutate(ITEM, 1, Mutation::Complete), None, &stamp())
                    .map(|outcome| ((), outcome.event))
            },
            || Err(Error::new(Code::Forbidden, "privilege changed")),
        )
        .unwrap_err();
    assert_eq!(error.code, Code::Forbidden);
    assert_eq!(f.bytes(), before);
    // An I/O failure before the commit is a plain storage error and saves nothing.
    let error = with_fault(
        Box::new(|stage| {
            if stage == Stage::BeforePublish {
                return Err(std::io::Error::other("injected failure before commit"));
            }
            Ok(())
        }),
        || complete(&store, ITEM, 1),
    )
    .unwrap_err();
    assert_eq!(error.code, Code::StorageError);
    assert_eq!(f.bytes(), before);
    // A commit whose acknowledgement never arrived is unknown, and the bytes are committed.
    let error = with_fault(
        Box::new(|stage| {
            if stage == Stage::AfterCommit {
                return Err(std::io::Error::other("commit not acknowledged"));
            }
            Ok(())
        }),
        || complete(&store, ITEM, 1),
    )
    .unwrap_err();
    assert_eq!(error.code, Code::OutcomeUnknown);
    assert_eq!(f.literal()["items"][0]["revision"], 2);
    assert_eq!(f.literal()["items"][0]["completion"], "complete");
    let reread = Store::new(&f.root, id(ROOM), None, "ops")
        .unwrap()
        .read()
        .unwrap()
        .unwrap();
    assert_eq!(reread.items[0].revision, 2);
    // A busy commit reported by the database is the same unknown outcome.
    let unknown = refused(Refusal {
        cause: Cause::Busy,
        message: "Another Ops process still holds the state database.".into(),
        may_not_be_saved: true,
    });
    assert_eq!(unknown.code, Code::OutcomeUnknown);
    assert!(
        unknown
            .message
            .contains("original operation outcome remains unknown")
    );
    // A new operation succeeds afterwards.
    store
        .update(
            |d| {
                model::apply(d, &mutate(ITEM, 2, Mutation::Archive), None, &stamp())
                    .map(|outcome| ((), outcome.event))
            },
            || Ok(()),
        )
        .unwrap();
}

#[test]
fn competing_writes_queue_and_use_the_current_document() {
    let f = Fixture::new();
    let store = store(&f);
    insert(&store).unwrap();
    store
        .update(
            |d| {
                model::apply(
                    d,
                    &create(OTHER, InventoryExpectation::Revision(1), None),
                    None,
                    &stamp(),
                )
                .map(|outcome| ((), outcome.event))
            },
            || Ok(()),
        )
        .unwrap();
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let root = f.root.clone();
    let writer = std::thread::spawn(move || {
        let store = Store::new(&root, id(ROOM), None, "ops").unwrap();
        store.update(
            |d| {
                let outcome =
                    model::apply(d, &mutate(ITEM, 1, Mutation::Complete), None, &stamp())?;
                locked_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                Ok(((), outcome.event))
            },
            || Ok(()),
        )
    });
    locked_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let before = f.bytes();
    let (done_tx, done_rx) = mpsc::channel();
    let root = f.root.clone();
    let competitor = std::thread::spawn(move || {
        let store = Store::new(&root, id(ROOM), None, "ops").unwrap();
        done_tx
            .send(store.update(
                |d| {
                    model::apply(d, &mutate(ITEM, 1, Mutation::Archive), None, &stamp())
                        .map(|outcome| ((), outcome.event))
                },
                || Ok(()),
            ))
            .unwrap();
    });
    // The second writer waits for the first; an immediate refusal would complete the channel.
    assert!(matches!(
        done_rx.recv_timeout(Duration::from_millis(200)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(f.bytes(), before);
    release_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
    // It then sees the first writer's revision and refuses its own stale one.
    assert_eq!(
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap_err()
            .code,
        Code::Conflict
    );
    competitor.join().unwrap();
    let refreshed = store.read().unwrap().unwrap();
    assert_eq!(refreshed.items[0].revision, 2);
    assert_eq!(refreshed.items[1].revision, 1);
    complete(&store, OTHER, refreshed.items[1].revision).unwrap();
    let restarted = Store::new(&f.root, id(ROOM), None, "ops")
        .unwrap()
        .read()
        .unwrap()
        .unwrap();
    assert!(
        restarted
            .items
            .iter()
            .all(|i| i.revision == 2 && i.completion == model::Completion::Complete)
    );
    assert_eq!(restarted.inventory_revision, 2);
}

#[test]
fn eight_parallel_edits_to_different_items_all_land() {
    let f = Fixture::new();
    let store = store(&f);
    for n in 1..=8u64 {
        let id_text = item(n);
        store
            .update(
                |d| {
                    let inventory = if n == 1 {
                        InventoryExpectation::Absent
                    } else {
                        InventoryExpectation::Revision(n - 1)
                    };
                    model::apply(d, &create(&id_text, inventory, None), None, &stamp())
                        .map(|outcome| ((), outcome.event))
                },
                || Ok(()),
            )
            .unwrap();
    }
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (1..=8u64)
        .map(|n| {
            let (root, barrier) = (f.root.clone(), barrier.clone());
            std::thread::spawn(move || {
                let store = Store::new(&root, id(ROOM), None, "ops").unwrap();
                barrier.wait();
                complete(&store, &item(n), 1)
            })
        })
        .collect();
    for thread in threads {
        thread
            .join()
            .unwrap()
            .expect("every parallel edit is saved");
    }
    let stored = store.read().unwrap().unwrap();
    assert_eq!(stored.items.len(), 8);
    assert!(
        stored
            .items
            .iter()
            .all(|i| i.revision == 2 && i.completion == model::Completion::Complete)
    );
}

#[test]
fn the_first_use_after_an_upgrade_imports_items_json_and_keeps_the_old_file_renamed() {
    let f = Fixture::new();
    let seeded = {
        let store = store(&f);
        insert(&store).unwrap();
        complete(&store, ITEM, 1).unwrap();
        f.bytes()
    };
    // Rewind to the previous release: the document lives in items.json and no database exists.
    for name in ["ops.db", "ops.db-wal", "ops.db-shm"] {
        let _ = fs::remove_file(f.root.join("ops").join(name));
    }
    legacy_directory(&f);
    fs::write(f.directory().join("items.json"), &seeded).unwrap();
    let store = store(&f);
    let read = store.read().unwrap().unwrap();
    assert_eq!(read.items[0].revision, 2);
    assert_eq!(f.bytes(), seeded);
    assert!(!f.directory().join("items.json").exists());
    let archive = archived(&f);
    assert_eq!(archive.len(), 1, "{archive:?}");
    assert_eq!(fs::read(&archive[0]).unwrap(), seeded);
    // The import carried the document's revisions: the next edit continues from them.
    store
        .update(
            |d| {
                model::apply(d, &mutate(ITEM, 2, Mutation::Archive), None, &stamp())
                    .map(|outcome| ((), outcome.event))
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(archived(&f), archive);
}

#[test]
fn a_version_1_document_is_read_as_stored_and_the_first_write_stores_version_2() {
    let f = Fixture::new();
    let v1 = serde_json::json!({"version":1,"roomId":ROOM,"checklistId":CHECKLIST,"inventoryRevision":1,
        "items":[{"id":ITEM,"revision":1,"title":"Before provenance","body":null,"reference":null,
            "assignee":null,"completion":"open","archived":false}],"deleted":[]});
    legacy_directory(&f);
    let seeded = serde_json::to_vec(&v1).unwrap();
    fs::write(f.directory().join("items.json"), &seeded).unwrap();
    let store = store(&f);
    // Reads decode it with every stamp unknown and write nothing.
    let read = store.read().unwrap().unwrap();
    assert_eq!(
        (&read.items[0].added, &read.items[0].last_changed),
        (&None, &None)
    );
    assert_eq!(f.bytes(), seeded);
    // The first write is the migration: version 2, the old item unknown until it changes.
    store
        .update(
            |d| {
                model::apply(d, &mutate(ITEM, 1, Mutation::Complete), None, &stamp())
                    .map(|outcome| ((), outcome.event))
            },
            || Ok(()),
        )
        .unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&f.bytes()).unwrap();
    assert_eq!(stored["version"], 2);
    let item = &stored["items"][0];
    assert_eq!(item["addedBy"], serde_json::Value::Null);
    assert_eq!(item["lastChangedBy"], LEAD);
    assert_eq!(item["lastChangedAtMs"], 1_000);
    assert_eq!(
        store.read().unwrap().unwrap().items[0].last_changed,
        Some(stamp())
    );
}

#[test]
fn a_corrupt_or_foreign_legacy_document_refuses_and_stays_where_it_is() {
    let f = Fixture::new();
    legacy_directory(&f);
    let path = f.directory().join("items.json");
    let other_room = {
        // A well-formed document that belongs to another room.
        let other = Fixture::new();
        let store = Store::new(&other.root, id(OTHER), None, "ops").unwrap();
        store
            .update(
                |d| {
                    let mut request = create(ITEM, InventoryExpectation::Absent, None);
                    request.room_id = id(OTHER);
                    model::apply(d, &request, None, &stamp()).map(|outcome| ((), outcome.event))
                },
                || Ok(()),
            )
            .unwrap();
        other
            .database()
            .query_row("SELECT body FROM documents", [], |row| {
                row.get::<_, Vec<u8>>(0)
            })
            .unwrap()
    };
    for bytes in [b"not json".to_vec(), other_room] {
        fs::write(&path, &bytes).unwrap();
        let store = store(&f);
        assert_eq!(store.read().unwrap_err().code, Code::StorageError);
        assert_eq!(insert(&store).unwrap_err().code, Code::StorageError);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(archived(&f).is_empty());
        let rows: i64 = f
            .database()
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }
}

#[test]
fn a_crash_between_the_commit_and_the_rename_finishes_the_rename_and_trusts_the_store() {
    let f = Fixture::new();
    let store = store(&f);
    insert(&store).unwrap();
    let committed = f.bytes();
    // The row exists and the old file does too, even holding different bytes.
    legacy_directory(&f);
    fs::write(f.directory().join("items.json"), b"stale old document").unwrap();
    assert_eq!(store.read().unwrap().unwrap().items.len(), 1);
    assert_eq!(f.bytes(), committed);
    assert!(!f.directory().join("items.json").exists());
    let archive = archived(&f);
    assert_eq!(archive.len(), 1);
    assert_eq!(fs::read(&archive[0]).unwrap(), b"stale old document");
}

#[test]
fn eight_processes_importing_at_once_leave_one_document_and_one_archive() {
    let f = Fixture::new();
    let seeded = {
        let store = store(&f);
        insert(&store).unwrap();
        let bytes = f.bytes();
        for name in ["ops.db", "ops.db-wal", "ops.db-shm"] {
            let _ = fs::remove_file(f.root.join("ops").join(name));
        }
        bytes
    };
    legacy_directory(&f);
    fs::write(f.directory().join("items.json"), &seeded).unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|n| {
            let (root, barrier) = (f.root.clone(), barrier.clone());
            std::thread::spawn(move || {
                let store = Store::new(&root, id(ROOM), None, "ops").unwrap();
                barrier.wait();
                if n % 2 == 0 {
                    store.read().map(|d| d.unwrap().items.len())
                } else {
                    complete(&store, ITEM, 1).map(|()| 1)
                }
            })
        })
        .collect();
    let mut completions = 0;
    for thread in threads {
        match thread.join().unwrap() {
            Ok(_) => completions += 1,
            Err(error) => assert_eq!(error.code, Code::Conflict, "{error}"),
        }
    }
    assert!(completions >= 4);
    let archive = archived(&f);
    assert_eq!(archive.len(), 1, "{archive:?}");
    assert_eq!(fs::read(&archive[0]).unwrap(), seeded);
    let stored = store(&f).read().unwrap().unwrap();
    assert_eq!(stored.items.len(), 1);
    assert_eq!(stored.items[0].revision, 2);
}
