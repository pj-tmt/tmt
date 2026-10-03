//! Remote private state on real temporary data roots.
use rusqlite::Connection;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_remote::{
    state::{Layout, MachineKey},
    store::Store,
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-1039-state-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn remote(&self) -> PathBuf {
        self.0.join("remote")
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::set_permissions(self.remote(), fs::Permissions::from_mode(0o700));
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn mode(path: &std::path::Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn machine_key_and_identity_are_created_once_and_stable() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    assert_eq!(mode(&root.remote()), 0o700);
    let key = MachineKey::open(&layout).unwrap().public();
    let machine = Store::open(&layout.serve_lock().unwrap())
        .unwrap()
        .machine()
        .unwrap();
    assert_eq!(mode(&root.remote().join("machine.key")), 0o600);
    assert_eq!(mode(&root.remote().join("remote.db")), 0o600);
    let reopened = Layout::open(&root.0).unwrap();
    assert_eq!(MachineKey::open(&reopened).unwrap().public(), key);
    assert_eq!(
        Store::open(&reopened.serve_lock().unwrap())
            .unwrap()
            .machine()
            .unwrap(),
        machine
    );
    assert!(machine.route_prefix.starts_with("/r/") && machine.route_prefix.len() == 35);
    // Distinct roots get distinct identities.
    let other = Root::new();
    let other_layout = Layout::open(&other.0).unwrap();
    assert_ne!(MachineKey::open(&other_layout).unwrap().public(), key);
    let other_machine = Store::open(&other_layout.serve_lock().unwrap())
        .unwrap()
        .machine()
        .unwrap();
    assert_ne!(other_machine.id, machine.id);
    assert_ne!(other_machine.route_prefix, machine.route_prefix);
}

#[test]
fn unsafe_directories_files_and_keys_fail_closed() {
    assert_eq!(
        Layout::open(std::path::Path::new("relative"))
            .err()
            .unwrap()
            .code,
        "REMOTE_ROOT_INVALID"
    );
    let root = Root::new();
    fs::create_dir(root.remote()).unwrap();
    fs::set_permissions(root.remote(), fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(
        Layout::open(&root.0).err().unwrap().code,
        "REMOTE_STATE_UNSAFE"
    );
    fs::set_permissions(root.remote(), fs::Permissions::from_mode(0o700)).unwrap();
    let layout = Layout::open(&root.0).unwrap();
    // A wider key file is refused and left untouched.
    fs::write(root.remote().join("machine.key"), [7; 32]).unwrap();
    fs::set_permissions(
        root.remote().join("machine.key"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        MachineKey::open(&layout).err().unwrap().code,
        "REMOTE_STATE_UNSAFE"
    );
    fs::set_permissions(
        root.remote().join("machine.key"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(MachineKey::open(&layout).is_ok(), "positive control");
    // A wrong-length key is invalid and is not replaced.
    fs::write(root.remote().join("machine.key"), [7; 31]).unwrap();
    assert_eq!(
        MachineKey::open(&layout).err().unwrap().code,
        "REMOTE_KEY_INVALID"
    );
    assert_eq!(
        fs::read(root.remote().join("machine.key")).unwrap(),
        [7; 31]
    );
    // A symlinked state file is never followed.
    let target = root.0.join("elsewhere.db");
    fs::write(&target, b"").unwrap();
    symlink(&target, root.remote().join("remote.db")).unwrap();
    assert_eq!(
        Store::open(&layout.serve_lock().unwrap())
            .err()
            .unwrap()
            .code,
        "REMOTE_STATE_UNSAFE"
    );
    assert!(fs::read(&target).unwrap().is_empty(), "target untouched");
    // A symlinked remote directory is refused.
    let other = Root::new();
    let real = other.0.join("real");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&real, other.remote()).unwrap();
    assert_eq!(
        Layout::open(&other.0).err().unwrap().code,
        "REMOTE_STATE_UNSAFE"
    );
}

#[test]
fn stale_key_temporaries_are_cleaned_and_foreign_files_kept() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let stale = root.remote().join(format!(".machine-{}", "a".repeat(32)));
    fs::write(&stale, [1; 32]).unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o600)).unwrap();
    let foreign = root.remote().join(".machine-not-ours");
    fs::write(&foreign, b"keep").unwrap();
    MachineKey::open(&layout).unwrap();
    assert!(!stale.exists());
    assert!(foreign.exists());
}

#[test]
fn second_serve_lock_on_one_root_refuses() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let held = layout.serve_lock().unwrap();
    assert_eq!(
        layout.serve_lock().err().unwrap().code,
        "REMOTE_ALREADY_SERVING"
    );
    drop(held);
    assert!(layout.serve_lock().is_ok());
}

#[test]
fn only_the_serve_lock_holder_opens_the_database() {
    let root = Root::new();
    let serving = Layout::open(&root.0).unwrap().serve_lock().unwrap();
    let mut store = Store::open(&serving).unwrap();
    // A second opener in any process must first take the same lock, and cannot.
    let other = Layout::open(&root.0).unwrap();
    assert_eq!(
        other.serve_lock().err().unwrap().code,
        "REMOTE_ALREADY_SERVING"
    );
    assert!(store.machine().is_ok());
    drop(store);
    drop(serving);
    assert!(Store::open(&other.serve_lock().unwrap()).is_ok());
}

#[test]
fn schema_history_uses_core_migrations() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    drop(Store::open(&layout.serve_lock().unwrap()).unwrap());
    let db = root.remote().join("remote.db");
    let inspect = Connection::open(&db).unwrap();
    let history: Vec<(i64, String)> = inspect
        .prepare("SELECT version, name FROM _migrations ORDER BY version")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        history,
        [
            (1, "machine".to_owned()),
            (2, "grants".to_owned()),
            (3, "sessions".to_owned()),
            (4, "journal".to_owned())
        ]
    );
    let journal: String = inspect
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "delete");
    drop(inspect);
    // Reopening applies nothing twice.
    drop(Store::open(&layout.serve_lock().unwrap()).unwrap());
    let count: i64 = Connection::open(&db)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 4);
    // A newer build's history refuses instead of being reinterpreted.
    Connection::open(&db)
        .unwrap()
        .execute("INSERT INTO _migrations VALUES (5, 'future', 'now')", [])
        .unwrap();
    assert_eq!(
        Store::open(&layout.serve_lock().unwrap())
            .err()
            .unwrap()
            .code,
        "REMOTE_STATE_UNSUPPORTED"
    );
    // A renamed step refuses as damaged history.
    let damage = Connection::open(&db).unwrap();
    damage
        .execute("DELETE FROM _migrations WHERE version = 5", [])
        .unwrap();
    damage
        .execute(
            "UPDATE _migrations SET name = 'other' WHERE version = 1",
            [],
        )
        .unwrap();
    drop(damage);
    assert_eq!(
        Store::open(&layout.serve_lock().unwrap())
            .err()
            .unwrap()
            .code,
        "REMOTE_STATE_UNAVAILABLE"
    );
}
