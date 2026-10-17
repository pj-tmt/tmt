use super::*;
use std::os::unix::fs::{DirBuilderExt, symlink};

struct World(PathBuf);
impl World {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ops-state-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn root(&self) -> PathBuf {
        self.0.join("ops")
    }
    fn slot(&self, limit: u64) -> Slot {
        Slot {
            kind: "test",
            key: "one".into(),
            legacy: self.root().join("test/one.json"),
            limit,
        }
    }
    fn legacy(&self, bytes: &[u8]) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.root().join("test"))
            .unwrap();
        fs::write(self.root().join("test/one.json"), bytes).unwrap();
    }
}
impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

type Probe = Box<dyn FnMut(&Path)>;

thread_local! {
    static ARCHIVING: std::cell::RefCell<Option<Probe>> = const { std::cell::RefCell::new(None) };
}

/// Called by the archive step, inside its database lock, so a test can look at that moment.
pub(super) fn archiving(path: &Path) {
    ARCHIVING.with(|probe| {
        if let Some(probe) = probe.borrow_mut().as_mut() {
            probe(path);
        }
    });
}

fn keep(bytes: Option<&[u8]>) -> Result<Option<Vec<u8>>, String> {
    Ok(bytes.map(<[u8]>::to_vec))
}

#[test]
fn an_absent_database_and_legacy_file_read_as_nothing_and_create_nothing() {
    let world = World::new();
    let slot = world.slot(1024);
    assert!(
        Database::open_existing(&world.root(), &slot.legacy)
            .unwrap()
            .is_none()
    );
    assert!(!world.root().exists());
    // The directory alone is not enough either: only a database or a legacy file is.
    fs::DirBuilder::new()
        .mode(0o700)
        .create(world.root())
        .unwrap();
    assert!(
        Database::open_existing(&world.root(), &slot.legacy)
            .unwrap()
            .is_none()
    );
    assert!(!world.root().join("ops.db").exists());
}

#[test]
fn the_archive_is_named_with_the_date_and_never_replaces_an_earlier_one() {
    let world = World::new();
    let slot = world.slot(1024);
    world.legacy(b"first");
    let mut database = Database::open(&world.root()).unwrap();
    assert_eq!(
        database.read(&slot, keep).map_err(|_| ()).unwrap(),
        Some(b"first".to_vec())
    );
    let archived: Vec<_> = fs::read_dir(world.root().join("test"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(archived.len(), 1, "{archived:?}");
    let date = archived[0].strip_prefix("one.json.migrated-").unwrap();
    assert_eq!(date.len(), 10, "{date}");
    assert!(
        date.bytes()
            .enumerate()
            .all(|(index, byte)| if matches!(index, 4 | 7) {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }),
        "{date}"
    );
    world.legacy(b"second");
    database.read(&slot, keep).map_err(|_| ()).unwrap();
    let mut archived: Vec<_> = fs::read_dir(world.root().join("test"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    archived.sort();
    assert_eq!(archived.len(), 2, "{archived:?}");
    assert!(archived[1].ends_with(&format!("{date}-1")), "{archived:?}");
}

#[test]
fn a_legacy_file_over_its_limit_or_behind_a_link_is_refused_and_stays() {
    let world = World::new();
    let slot = world.slot(8);
    world.legacy(b"0123456789");
    let mut database = Database::open(&world.root()).unwrap();
    let Err(StateFailure::Refused(refusal)) = database.read(&slot, keep) else {
        panic!("an oversized legacy file must be refused");
    };
    assert_eq!(refusal.cause, Cause::Failed);
    assert!(refusal.message.contains("exceeds"), "{}", refusal.message);
    assert!(world.slot(8).legacy.exists());

    fs::remove_file(&slot.legacy).unwrap();
    let foreign = world.0.join("foreign");
    fs::write(&foreign, b"tiny").unwrap();
    symlink(&foreign, &slot.legacy).unwrap();
    let Err(StateFailure::Refused(refusal)) = database.read(&slot, keep) else {
        panic!("a linked legacy file must be refused");
    };
    assert_eq!(refusal.cause, Cause::Failed);
    assert_eq!(fs::read(&foreign).unwrap(), b"tiny");
    let rows: i64 = rusqlite::Connection::open(world.root().join("ops.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn a_document_over_its_limit_is_refused_and_the_stored_one_stays() {
    let world = World::new();
    let slot = world.slot(8);
    let mut database = Database::open(&world.root()).unwrap();
    database
        .update(
            &slot,
            |_| Ok::<_, String>((Some(b"small".to_vec()), ())),
            |()| Ok(()),
        )
        .map_err(|_| ())
        .unwrap();
    let Err(StateFailure::Refused(refusal)) = database.update(
        &slot,
        |_| Ok::<_, String>((Some(b"far too large".to_vec()), ())),
        |()| Ok(()),
    ) else {
        panic!("an oversized document must be refused");
    };
    assert!(refusal.message.contains("limit"), "{}", refusal.message);
    assert_eq!(
        database.read(&slot, keep).map_err(|_| ()).unwrap(),
        Some(b"small".to_vec())
    );
}

#[test]
fn only_a_newer_history_is_reported_as_newer() {
    let world = World::new();
    let slot = world.slot(1024);
    drop(Database::open(&world.root()).unwrap());
    let connection = rusqlite::Connection::open(world.root().join("ops.db")).unwrap();
    connection
        .execute(
            "INSERT INTO _tmt_state_migrations(version, name, sql) VALUES(2, 'later', 'SELECT 1')",
            [],
        )
        .unwrap();
    let Err(refusal) = Database::open(&world.root()).map(|_| ()) else {
        panic!("a newer database must be refused");
    };
    assert_eq!(refusal.cause, Cause::Newer);
    assert!(
        refusal.message.contains("Upgrade Ops"),
        "{}",
        refusal.message
    );

    // History that is not a prefix of this binary's is something else, not "newer".
    connection
        .execute("DELETE FROM _tmt_state_migrations WHERE version = 2", [])
        .unwrap();
    connection
        .execute("UPDATE _tmt_state_migrations SET name = 'other'", [])
        .unwrap();
    let Err(refusal) = Database::open(&world.root()).map(|_| ()) else {
        panic!("a foreign history must be refused");
    };
    assert_eq!(refusal.cause, Cause::Failed);
    assert!(Database::open_existing(&world.root(), &slot.legacy).is_err());
}

#[test]
fn an_archive_runs_only_while_it_holds_the_database_write_lock() {
    let world = World::new();
    let slot = world.slot(1024);
    world.legacy(b"first");
    let mut database = Database::open(&world.root()).unwrap();
    let database_file = world.root().join("ops.db");
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = seen.clone();
    ARCHIVING.with(|probe| {
        *probe.borrow_mut() = Some(Box::new(move |path| {
            // Another writer cannot start while the archive step runs, so archivers
            // never overlap in one directory.
            let other = rusqlite::Connection::open(&database_file).unwrap();
            other.busy_timeout(std::time::Duration::ZERO).unwrap();
            let refused = other.execute_batch("BEGIN IMMEDIATE").is_err();
            observed.borrow_mut().push((path.to_owned(), refused));
        }));
    });
    database.read(&slot, keep).map_err(|_| ()).unwrap();
    // A leftover that appears later is archived again, under the lock again.
    world.legacy(b"second");
    database.read(&slot, keep).map_err(|_| ()).unwrap();
    ARCHIVING.with(|probe| *probe.borrow_mut() = None);
    assert_eq!(
        *seen.borrow(),
        [(slot.legacy.clone(), true), (slot.legacy.clone(), true)]
    );
    // Reading with nothing to archive takes no write lock at all.
    database.read(&slot, keep).map_err(|_| ()).unwrap();
    assert_eq!(seen.borrow().len(), 2);
}

#[test]
fn a_newer_database_is_told_by_the_leaf_not_by_its_display_text() {
    let lookalike = StateError::Migration {
        path: PathBuf::from("/data/ops/ops.db"),
        reason: "store is newer than this binary, or so a message says",
    };
    assert_eq!(Refusal::from(lookalike).cause, Cause::Failed);
}
