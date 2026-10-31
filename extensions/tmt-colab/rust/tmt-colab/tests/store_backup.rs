//! The copy of the owner store taken before a schema migration (#4280): when it is made, who may
//! read it, how many stay, and how a newer store's refusal points back to one.
#[path = "support/store_snapshot.rs"]
mod store_snapshot;
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
/// step adds page_auto_cleaned, so dropping that table and the version is that store.
fn previous_schema(layout: &Layout) -> (PathBuf, u32) {
    let store = layout.directory.join("space.db");
    Store::open(layout).unwrap().close().unwrap();
    let previous = SCHEMA_VERSION - 1;
    Connection::open(&store)
        .unwrap()
        .execute_batch(&format!(
            "DROP TABLE page_auto_cleaned; PRAGMA user_version={previous}"
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
fn physical_conversion_shrinks_a_current_legacy_store_without_a_copy() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    Store::open(&layout).unwrap().close().unwrap();
    let store = layout.directory.join("space.db");
    let connection = Connection::open(&store).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "auto_vacuum", |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    connection
        .execute_batch(
            "PRAGMA auto_vacuum=NONE; VACUUM;
        CREATE TABLE marker(value BLOB); INSERT INTO marker VALUES(zeroblob(1048576));
        DELETE FROM marker; INSERT INTO marker VALUES(X'0102');",
        )
        .unwrap();
    drop(connection);
    let before = fs::read(&store).unwrap();
    Store::open(&layout).unwrap().close().unwrap();
    assert!(names(&layout).is_empty());
    eprintln!(
        "space.db after legacy conversion: {} -> {} bytes",
        before.len(),
        fs::metadata(&store).unwrap().len()
    );
    assert!(fs::metadata(&store).unwrap().len() < before.len() as u64);
    let connection = Connection::open(&store).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "auto_vacuum", |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("SELECT value FROM marker", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        [1, 2]
    );
    assert_eq!(version(&store), SCHEMA_VERSION);
    let unchanged = fs::read(&store).unwrap();
    drop(connection);
    Store::open(&layout).unwrap().close().unwrap();
    assert_eq!(fs::read(&store).unwrap(), unchanged);
    assert!(names(&layout).is_empty());
}

#[test]
fn busy_physical_conversion_keeps_the_store_usable_and_retries_on_next_open() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    Store::open(&layout).unwrap().close().unwrap();
    let path = layout.directory.join("space.db");
    let reader = Connection::open(&path).unwrap();
    reader
        .execute_batch(
            "PRAGMA auto_vacuum=NONE; VACUUM;
        CREATE TABLE marker(value); INSERT INTO marker VALUES('saved');
        BEGIN; SELECT * FROM marker;",
        )
        .unwrap();
    // The migration's no-op transaction can finish, but VACUUM cannot commit with this reader.
    Store::open(&layout).unwrap().close().unwrap();
    assert!(names(&layout).is_empty());
    assert_eq!(
        reader
            .pragma_query_value(None, "auto_vacuum", |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        reader
            .query_row("SELECT value FROM marker", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "saved"
    );
    reader.execute_batch("COMMIT").unwrap();
    drop(reader);
    Store::open(&layout).unwrap().close().unwrap();
    assert!(names(&layout).is_empty());
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .pragma_query_value(None, "auto_vacuum", |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        oracle
            .query_row("SELECT value FROM marker", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "saved"
    );
}

#[test]
fn an_upgrade_snapshots_the_store_first_and_owner_only() {
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
    store_snapshot::assert_snapshot(&copy, &before);
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
    store_snapshot::assert_snapshot(&backup_path(&store, previous), &before);
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
            "DROP TABLE page_auto_cleaned; PRAGMA user_version={previous}; CREATE TABLE marker(x); VACUUM"
        ))
        .unwrap();
    let second = fs::read(&store).unwrap();
    Store::open(&layout).unwrap().close().unwrap();
    store_snapshot::assert_snapshot(&backup_path(&store, previous), &second);
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
fn a_migration_that_fails_partway_keeps_its_copy_and_every_older_one() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let (store, _) = previous_schema(&layout);
    // Keep exercising the two-statement migration 18, even while the newest step is a
    // single ALTER. Its second statement fails after its first one has executed.
    let previous = 17;
    Connection::open(&store)
        .unwrap()
        .execute_batch(
            "DROP TABLE project_pin_choice; DROP TABLE project_pins; PRAGMA user_version=17",
        )
        .unwrap();
    for old in [3, 7] {
        fs::write(backup_path(&store, old), format!("v{old}")).unwrap();
    }
    // The step creates `project_pins`, then `project_pin_choice`: this one is already there, so
    // the step stops after its first statement.
    Connection::open(&store)
        .unwrap()
        .execute_batch("CREATE TABLE project_pin_choice(chosen INTEGER)")
        .unwrap();
    let before = fs::read(&store).unwrap();
    assert!(Store::open(&layout).is_err());
    // Nothing of the step stays, the copy of the schema it was leaving is whole, and no older
    // copy was pruned for a migration that did not happen.
    let connection = Connection::open(&store).unwrap();
    let pins: u32 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='project_pins'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pins, 0);
    assert_eq!(version(&store), previous);
    store_snapshot::assert_snapshot(&backup_path(&store, previous), &before);
    let mut expected = vec![
        "space.db.pre-v3.bak".to_string(),
        format!("space.db.pre-v{previous}.bak"),
        "space.db.pre-v7.bak".to_string(),
    ];
    expected.sort();
    assert_eq!(names(&layout), expected);
}

#[test]
fn the_copy_just_taken_stays_even_when_copies_at_newer_schemas_exist() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let (store, previous) = previous_schema(&layout);
    // Copies from a newer Colab outrank the one this open takes by schema number.
    for newer in [SCHEMA_VERSION + 10, SCHEMA_VERSION + 11] {
        fs::write(backup_path(&store, newer), format!("v{newer}")).unwrap();
    }
    let before = fs::read(&store).unwrap();
    Store::open(&layout).unwrap().close().unwrap();
    store_snapshot::assert_snapshot(&backup_path(&store, previous), &before);
    assert_eq!(
        names(&layout),
        [
            format!("space.db.pre-v{previous}.bak"),
            format!("space.db.pre-v{}.bak", SCHEMA_VERSION + 11),
        ]
    );
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
