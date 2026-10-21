//! The copy of the owner store taken before a schema migration (#4280): when it is made, who may
//! read it, how many stay, and how a newer store's refusal points back to one.
use rusqlite::Connection;
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    keyring::Layout,
    store::{Fault, SCHEMA_VERSION, Store, backup_path},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-4280-backup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn layout(&self) -> Layout {
        Layout::open(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

/// A store left at the schema before the current one, as an older Colab saved it: the current
/// step only adds `page_titles`, so dropping it and the version is that store.
fn previous_schema(layout: &Layout) -> (PathBuf, u32) {
    let store = layout.directory.join("space.db");
    Store::open(layout).unwrap().close().unwrap();
    let previous = SCHEMA_VERSION - 1;
    Connection::open(&store)
        .unwrap()
        .execute_batch(&format!(
            "DROP TABLE page_titles; PRAGMA user_version={previous}"
        ))
        .unwrap();
    (store, previous)
}

fn names(layout: &Layout) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(&layout.directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.contains(".pre-v"))
        .collect();
    found.sort();
    found
}

fn version(store: &Path) -> u32 {
    Connection::open(store)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

#[test]
fn an_upgrade_copies_the_store_first_byte_for_byte_and_owner_only() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let (store, previous) = previous_schema(&layout);
    let before = fs::read(&store).unwrap();
    Store::open(&layout).unwrap().close().unwrap();
    assert_eq!(version(&store), SCHEMA_VERSION);
    let copy = backup_path(&store, previous);
    assert_eq!(
        copy,
        layout
            .directory
            .join(format!("space.db.pre-v{previous}.bak"))
    );
    assert_eq!(fs::read(&copy).unwrap(), before);
    assert_eq!(fs::metadata(&copy).unwrap().mode() & 0o777, 0o600);
    // Nothing else is left beside it: no partial file.
    assert_eq!(names(&layout), [format!("space.db.pre-v{previous}.bak")]);
}

#[test]
fn a_new_store_and_a_current_store_make_no_copy() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    Store::open(&layout).unwrap().close().unwrap();
    Store::open(&layout).unwrap().close().unwrap();
    assert_eq!(names(&layout), Vec::<String>::new());
}

#[test]
fn a_copy_that_cannot_be_written_stops_the_migration_and_leaves_the_store_untouched() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let (store, previous) = previous_schema(&layout);
    // A directory where the partial file goes cannot be removed or created over.
    let blocker = layout
        .directory
        .join(format!("space.db.pre-v{previous}.bak.partial"));
    fs::create_dir(&blocker).unwrap();
    let before = fs::read(&store).unwrap();
    let error = Store::open(&layout).err().unwrap();
    let fault = error.downcast_ref::<Fault>().unwrap();
    assert!(matches!(fault, Fault::Backup(_)), "{fault:?}");
    assert_eq!(fault.code(), Some("COLAB_STORE_BACKUP_FAILED"));
    assert_eq!(
        fault.to_string(),
        "Colab could not save a backup of this space before updating it. Nothing was changed."
    );
    assert_eq!(fs::read(&store).unwrap(), before);
    assert_eq!(version(&store), previous);
    assert!(!backup_path(&store, previous).exists());
    // Once the way is clear the same open migrates and copies.
    fs::remove_dir(&blocker).unwrap();
    Store::open(&layout).unwrap().close().unwrap();
    assert_eq!(version(&store), SCHEMA_VERSION);
    assert_eq!(fs::read(backup_path(&store, previous)).unwrap(), before);
}

#[test]
fn a_second_upgrade_from_the_same_schema_replaces_the_copy() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let (store, previous) = previous_schema(&layout);
    Store::open(&layout).unwrap().close().unwrap();
    // The same schema again, now with different content.
    Connection::open(&store)
        .unwrap()
        .execute_batch(&format!(
            "DROP TABLE page_titles; PRAGMA user_version={previous}; CREATE TABLE marker(x); VACUUM"
        ))
        .unwrap();
    let second = fs::read(&store).unwrap();
    Store::open(&layout).unwrap().close().unwrap();
    assert_eq!(fs::read(backup_path(&store, previous)).unwrap(), second);
    assert_eq!(names(&layout), [format!("space.db.pre-v{previous}.bak")]);
}

#[test]
fn only_the_two_newest_copies_stay_and_other_names_are_left_alone() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let (store, previous) = previous_schema(&layout);
    for old in [3, 7] {
        fs::write(backup_path(&store, old), format!("v{old}")).unwrap();
    }
    // Names `backup_path` never writes are not copies.
    let others = [
        "space.db.pre-v09.bak",
        "space.db.pre-v+5.bak",
        "space.db.pre-v1.bak.partial",
        "other.db.pre-v2.bak",
    ];
    for other in others {
        fs::write(layout.directory.join(other), "x").unwrap();
    }
    Store::open(&layout).unwrap().close().unwrap();
    let mut expected: Vec<String> = others
        .iter()
        .filter(|name| name.contains(".pre-v"))
        .map(|name| name.to_string())
        .collect();
    expected.push(format!("space.db.pre-v{previous}.bak"));
    expected.push("space.db.pre-v7.bak".into());
    expected.sort();
    assert_eq!(names(&layout), expected);
    assert_eq!(fs::read(backup_path(&store, 7)).unwrap(), b"v7");
}

#[test]
fn a_newer_store_names_the_newest_copy_this_build_can_open() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let store = layout.directory.join("space.db");
    Store::open(&layout).unwrap().close().unwrap();
    let newer = SCHEMA_VERSION + 1;
    Connection::open(&store)
        .unwrap()
        .execute_batch(&format!("PRAGMA user_version={newer}"))
        .unwrap();
    // No copy: the refusal is as before.
    let error = Store::open(&layout).err().unwrap();
    let fault = error.downcast_ref::<Fault>().unwrap();
    assert!(matches!(fault, Fault::UnsupportedSchema(version, None) if *version == newer));
    assert_eq!(fault.to_string(), "This space was saved by a newer Colab.");
    assert_eq!(fault.hint(), Some("Run tmt upgrade, then try again."));
    // Copies at a schema this build can open, and one it cannot: the newest openable one is named.
    for schema in [3, SCHEMA_VERSION, newer] {
        fs::write(backup_path(&store, schema), "x").unwrap();
    }
    let error = Store::open(&layout).err().unwrap();
    let fault = error.downcast_ref::<Fault>().unwrap();
    let copy = backup_path(&store, SCHEMA_VERSION);
    assert!(
        matches!(fault, Fault::UnsupportedSchema(_, Some(found)) if *found == copy),
        "{fault:?}"
    );
    assert_eq!(
        fault.to_string(),
        format!(
            "This space was saved by a newer Colab. Run tmt upgrade to open it. To stay on this version instead, stop Colab and replace space.db with the backup from before that update ({}); changes made since then are lost.",
            copy.display()
        )
    );
    assert_eq!(fault.code(), Some("COLAB_STORE_NEWER"));
    // The read-only open names it too, and neither open changed anything.
    let read = Store::read(&layout).err().unwrap();
    assert!(matches!(
        read.downcast_ref::<Fault>(),
        Some(Fault::UnsupportedSchema(_, Some(found))) if *found == copy
    ));
    assert_eq!(version(&store), newer);
}
