//! Shared mutation entries on disposable state, without a consumer migration.
use std::{
    fs,
    io,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_extension_state::{Document, Guard, Layout, Migration, Store};

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
        transaction.execute("INSERT INTO records VALUES(2,'rolled back')", [])?;
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
fn guard_collision_names_the_holder_and_exact_retry() {
    let world = World::new();
    let layout = world.layout();
    let path = layout.directory.join("runner.lock");
    let held = Guard::acquire(&path, RETRY).unwrap();
    let collision = Guard::acquire(&path, RETRY).err().unwrap();
    let holder = collision.holder().expect("held guard published its identity");
    assert_eq!(holder.pid, std::process::id());
    assert!(holder.acquired_at_ms > 0);
    let message = collision.to_string();
    assert!(message.contains(&std::process::id().to_string()));
    assert!(message.contains(&path.display().to_string()));
    assert!(message.contains(RETRY));
    drop(held);
    assert!(Guard::acquire(&path, RETRY).is_ok());
}
