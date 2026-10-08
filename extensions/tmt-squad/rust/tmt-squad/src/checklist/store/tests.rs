use super::*;
use crate::checklist::{
    model::{self, InventoryExpectation, Mutation},
    test_support::*,
};
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, Barrier},
};

fn insert(store: &Store) -> Result<(), Error> {
    store.update(
        |d| model::apply(d, &create(ITEM, InventoryExpectation::Absent, None), None).map(|_| ()),
        || Ok(()),
    )
}

#[test]
fn absence_reads_are_inert_and_corruption_is_not_empty() {
    let f = Fixture::new();
    let store = Store::new(&f.root, id(ROOM), "ops").unwrap();
    assert!(store.read().unwrap().is_none());
    assert!(!f.root.join("ops").exists());
    insert(&store).unwrap();
    for bytes in [b"".as_slice(), b"{broken", b"{\"version\":2}"] {
        fs::write(f.directory().join("items.json"), bytes).unwrap();
        assert_eq!(store.read().unwrap_err().code, Code::StorageError);
        assert_eq!(insert(&store).unwrap_err().code, Code::StorageError);
        assert_eq!(f.bytes(), bytes);
    }
}

#[test]
fn concurrent_first_create_and_lock_contention_preserve_one_inventory() {
    let f = Fixture::new();
    let barrier = Arc::new(Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let root = f.root.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = Store::new(&root, id(ROOM), "ops").unwrap();
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
    let store = Store::new(&f.root, id(ROOM), "ops").unwrap();
    let d = store.read().unwrap().unwrap();
    assert_eq!(d.items.len(), 1);
    assert_eq!(d.inventory_revision, 1);
    let lock = Flock::lock(
        File::open(f.directory().join("items.lock")).unwrap(),
        FlockArg::LockExclusiveNonblock,
    )
    .unwrap();
    assert_eq!(store.read().unwrap_err().code, Code::StorageError);
    assert_eq!(insert(&store).unwrap_err().code, Code::StorageError);
    drop(lock);
    assert!(store.read().unwrap().is_some());
}

#[test]
fn unsafe_files_and_directories_refuse_without_following_or_repair() {
    let f = Fixture::new();
    let store = Store::new(&f.root, id(ROOM), "ops").unwrap();
    let outside = f.root.join("foreign");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, f.root.join("ops")).unwrap();
    assert_eq!(store.read().unwrap_err().code, Code::StorageError);
    assert_eq!(insert(&store).unwrap_err().code, Code::StorageError);
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
    fs::remove_file(f.root.join("ops")).unwrap();
    insert(&store).unwrap();
    let before = f.bytes();
    let foreign = f.root.join("foreign.txt");
    fs::write(&foreign, b"keep").unwrap();
    symlink(&foreign, f.directory().join("items.tmp")).unwrap();
    let request = mutate(ITEM, 1, Mutation::Complete);
    assert_eq!(
        store
            .update(|d| model::apply(d, &request, None).map(|_| ()), || Ok(()))
            .unwrap_err()
            .code,
        Code::StorageError
    );
    assert_eq!(f.bytes(), before);
    assert_eq!(fs::read(foreign).unwrap(), b"keep");
    fs::remove_file(f.directory().join("items.tmp")).unwrap();
    fs::remove_file(f.directory().join("items.lock")).unwrap();
    assert_eq!(store.read().unwrap_err().code, Code::StorageError);
    assert_eq!(insert(&store).unwrap_err().code, Code::StorageError);
    assert!(!f.directory().join("items.lock").exists());
}

#[test]
fn owned_modes_are_private_and_existing_permissions_survive_replacement() {
    let f = Fixture::new();
    let store = Store::new(&f.root, id(ROOM), "ops").unwrap();
    insert(&store).unwrap();
    for path in [
        f.root.join("ops"),
        f.root.join("ops/checklist"),
        f.directory(),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    for name in ["items.json", "items.lock"] {
        assert_eq!(
            fs::metadata(f.directory().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    fs::set_permissions(
        f.directory().join("items.json"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    store
        .update(
            |d| model::apply(d, &mutate(ITEM, 1, Mutation::Complete), None).map(|_| ()),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        fs::metadata(f.directory().join("items.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
}

#[test]
fn fault_ordering_preserves_prepublication_bytes_and_unknown_committed_bytes() {
    for stage in [
        Stage::FileSync,
        Stage::Rename,
        Stage::DirectorySync,
        Stage::Acknowledgement,
    ] {
        let f = Fixture::new();
        let store = Store::new(&f.root, id(ROOM), "ops").unwrap();
        insert(&store).unwrap();
        let before = f.bytes();
        let error = with_fault(
            Box::new(move |at, _| {
                if at == stage {
                    return Err(std::io::Error::other("injected publication I/O failure"));
                }
                Ok(())
            }),
            || {
                store.update(
                    |d| model::apply(d, &mutate(ITEM, 1, Mutation::Complete), None).map(|_| ()),
                    || Ok(()),
                )
            },
        )
        .unwrap_err();
        let published = matches!(stage, Stage::DirectorySync | Stage::Acknowledgement);
        assert_eq!(
            error.code,
            if published {
                Code::OutcomeUnknown
            } else {
                Code::StorageError
            }
        );
        let actual = f.literal();
        if published {
            assert_eq!(actual["items"][0]["revision"], 2);
            assert_eq!(actual["items"][0]["completion"], "complete");
        } else {
            assert_eq!(f.bytes(), before);
        }
        assert!(!f.directory().join("items.tmp").exists());
        let reread = Store::new(&f.root, id(ROOM), "ops")
            .unwrap()
            .read()
            .unwrap()
            .unwrap();
        assert_eq!(reread.items[0].revision, if published { 2 } else { 1 });
        // A new operation succeeds after the failed lock owner's return.
        store
            .update(
                |d| {
                    model::apply(
                        d,
                        &mutate(ITEM, reread.items[0].revision, Mutation::Archive),
                        None,
                    )
                    .map(|_| ())
                },
                || Ok(()),
            )
            .unwrap();
        if published {
            assert_eq!(error.code, Code::OutcomeUnknown);
        }
    }
}

#[test]
fn actual_rename_refusal_and_locked_read_failure_leave_bytes_unchanged() {
    let f = Fixture::new();
    let store = Store::new(&f.root, id(ROOM), "ops").unwrap();
    insert(&store).unwrap();
    let before = f.bytes();
    let error = with_fault(
        Box::new(|stage, path| {
            if stage == Stage::Rename {
                fs::remove_file(path.join("items.tmp"))?;
            }
            Ok(())
        }),
        || {
            store.update(
                |d| model::apply(d, &mutate(ITEM, 1, Mutation::Complete), None).map(|_| ()),
                || Ok(()),
            )
        },
    )
    .unwrap_err();
    assert_eq!(error.code, Code::StorageError);
    assert_eq!(f.bytes(), before);
    assert!(!f.directory().join("items.tmp").exists());
    assert!(store.read().unwrap().is_some());
    // An actual no-follow read refusal is not an empty projection.
    fs::rename(f.directory().join("items.json"), f.root.join("saved.json")).unwrap();
    symlink(f.root.join("saved.json"), f.directory().join("items.json")).unwrap();
    assert_eq!(store.read().unwrap_err().code, Code::StorageError);
    assert_eq!(fs::read(f.root.join("saved.json")).unwrap(), before);
}

#[test]
fn actual_postrename_directory_open_failure_is_unknown_with_committed_content() {
    let f = Fixture::new();
    let store = Store::new(&f.root, id(ROOM), "ops").unwrap();
    insert(&store).unwrap();
    let moved = f.root.join("moved-checklist");
    let target = moved.clone();
    let error = with_fault(
        Box::new(move |stage, directory| {
            if stage == Stage::DirectorySync {
                fs::rename(directory, &target)?;
            }
            Ok(())
        }),
        || {
            store.update(
                |d| model::apply(d, &mutate(ITEM, 1, Mutation::Complete), None).map(|_| ()),
                || Ok(()),
            )
        },
    )
    .unwrap_err();
    assert_eq!(error.code, Code::OutcomeUnknown);
    let committed: serde_json::Value =
        serde_json::from_slice(&fs::read(moved.join("items.json")).unwrap()).unwrap();
    assert_eq!(committed["items"][0]["revision"], 2);
    assert_eq!(committed["items"][0]["completion"], "complete");
    assert!(!moved.join("items.tmp").exists());
    fs::rename(moved, f.directory()).unwrap();
    assert_eq!(store.read().unwrap().unwrap().items[0].revision, 2);
    assert_eq!(error.code, Code::OutcomeUnknown);
}

#[test]
fn competing_writes_and_independent_items_use_the_current_locked_document() {
    let f = Fixture::new();
    let store = Store::new(&f.root, id(ROOM), "ops").unwrap();
    insert(&store).unwrap();
    store
        .update(
            |d| {
                model::apply(
                    d,
                    &create(OTHER, InventoryExpectation::Revision(1), None),
                    None,
                )
                .map(|_| ())
            },
            || Ok(()),
        )
        .unwrap();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let root = f.root.clone();
    let writer = std::thread::spawn(move || {
        let store = Store::new(&root, id(ROOM), "ops").unwrap();
        store.update(
            |d| {
                model::apply(d, &mutate(ITEM, 1, Mutation::Complete), None)?;
                locked_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
                Ok(())
            },
            || Ok(()),
        )
    });
    locked_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let before = f.bytes();
    assert_eq!(
        store
            .update(
                |d| model::apply(d, &mutate(ITEM, 1, Mutation::Archive), None).map(|_| ()),
                || Ok(())
            )
            .unwrap_err()
            .code,
        Code::StorageError
    );
    assert_eq!(f.bytes(), before);
    release_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
    assert_eq!(
        store
            .update(
                |d| model::apply(d, &mutate(ITEM, 1, Mutation::Archive), None).map(|_| ()),
                || Ok(())
            )
            .unwrap_err()
            .code,
        Code::Conflict
    );
    let refreshed = store.read().unwrap().unwrap();
    assert_eq!(refreshed.items[0].revision, 2);
    assert_eq!(refreshed.items[1].revision, 1);
    store
        .update(
            |d| {
                model::apply(
                    d,
                    &mutate(OTHER, refreshed.items[1].revision, Mutation::Complete),
                    None,
                )
                .map(|_| ())
            },
            || Ok(()),
        )
        .unwrap();
    let restarted = Store::new(&f.root, id(ROOM), "ops")
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
