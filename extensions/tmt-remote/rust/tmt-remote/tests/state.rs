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
    assert!(machine.route_prefix.starts_with("/r/") && machine.route_prefix.len() == 19);
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
fn shared_layout_preserves_root_permissions_sibling_bytes_and_private_names() {
    let root = Root::new();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o751)).unwrap();
    let sibling = root.0.join("colab");
    fs::create_dir(&sibling).unwrap();
    let owner = sibling.join("owner.key");
    fs::write(&owner, [2; 32]).unwrap();
    let layout = Layout::open(&root.0).unwrap();
    MachineKey::open(&layout).unwrap();
    assert_eq!(mode(&root.0), 0o751);
    assert_eq!(fs::read(owner).unwrap(), [2; 32]);
    for name in ["../owner.key", "owner.key", "machine.key/"] {
        let error = layout.file(name).err().unwrap();
        assert_eq!(error.code, "REMOTE_STATE_NAME_INVALID");
        assert_eq!(error.message, "Invalid private file name.");
    }
}

#[test]
fn publication_contention_preserves_stale_bytes_and_releases_for_cleanup() {
    use nix::fcntl::{Flock, FlockArg};
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let stale = root.remote().join(format!(".machine-{}", "b".repeat(32)));
    fs::write(&stale, b"partial").unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o600)).unwrap();
    let held = Flock::lock(
        layout.file("key.lock").unwrap(),
        FlockArg::LockExclusiveNonblock,
    )
    .unwrap();
    let error = MachineKey::open(&layout).err().unwrap();
    assert_eq!(error.code, "REMOTE_KEY_BUSY");
    assert_eq!(
        error.message,
        "Machine key publication is already in progress."
    );
    assert_eq!(fs::read(&stale).unwrap(), b"partial");
    assert!(!root.remote().join("machine.key").exists());
    drop(held);
    MachineKey::open(&layout).unwrap();
    assert!(!stale.exists());
}

#[test]
fn bounded_keys_and_unsafe_temporaries_keep_original_bytes_and_error_mapping() {
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    MachineKey::open(&layout).unwrap();
    let key = root.remote().join("machine.key");
    for length in [0, 31, 33, 4096] {
        let original = vec![7; length];
        fs::write(&key, &original).unwrap();
        let error = MachineKey::open(&layout).err().unwrap();
        assert_eq!(error.code, "REMOTE_KEY_INVALID");
        assert_eq!(
            error.message,
            "Invalid machine key length; the key was not replaced."
        );
        assert_eq!(fs::read(&key).unwrap(), original);
    }
    fs::write(&key, [7; 32]).unwrap();
    assert!(
        MachineKey::open(&layout).is_ok(),
        "valid key positive control"
    );
    let stale = root.remote().join(format!(".machine-{}", "c".repeat(32)));
    fs::write(&stale, [8; 33]).unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        MachineKey::open(&layout).err().unwrap().code,
        "REMOTE_KEY_INVALID"
    );
    assert_eq!(fs::read(&stale).unwrap(), [8; 33]);
    fs::remove_file(&stale).unwrap();
    symlink(&key, &stale).unwrap();
    let error = MachineKey::open(&layout).err().unwrap();
    assert_eq!(error.code, "REMOTE_STATE_UNSAFE");
    assert_eq!(
        error.message,
        "Remote state files must be owned regular 0600 files."
    );
    assert!(fs::symlink_metadata(&stale).unwrap().is_symlink());
    assert_eq!(fs::read(&key).unwrap(), [7; 32]);
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
            (4, "journal".to_owned()),
            (5, "door_port".to_owned()),
            (6, "short_route_prefix".to_owned())
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
    assert_eq!(count, 6);
    // A newer build's history refuses instead of being reinterpreted.
    Connection::open(&db)
        .unwrap()
        .execute("INSERT INTO _migrations VALUES (7, 'future', 'now')", [])
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
        .execute("DELETE FROM _migrations WHERE version = 7", [])
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

/// Frozen pre-change schema copied from 180295209; expected preservation is literal.
fn schema5(root: &Root, prefix: &str) -> Layout {
    let layout = Layout::open(&root.0).unwrap();
    drop(layout.file("remote.db").unwrap());
    let connection = Connection::open(root.remote().join("remote.db")).unwrap();
    connection
        .execute_batch(include_str!("fixtures/schema5.sql"))
        .unwrap();
    connection
        .execute(
            "INSERT INTO machine VALUES (1, '00000000-0000-4000-8000-000000000001', ?1)",
            [prefix],
        )
        .unwrap();
    connection.execute_batch("INSERT INTO door_port VALUES (1, 12345);
        INSERT INTO grants VALUES ('00000000-0000-4000-8000-000000000002', zeroblob(32), 'browser',
        'http://127.0.0.1:12345', 'Legacy browser', 'all', 'agents.read talk', 'direct', 1, NULL, 1, 0);").unwrap();
    layout
}
#[test]
fn schema6_changes_only_the_prefix_once_and_preserves_origin_bound_grants() {
    let root = Root::new();
    let old_prefix = "/r/0123456789abcdef0123456789abcdef";
    let layout = schema5(&root, old_prefix);
    let public = MachineKey::open(&layout).unwrap().public();
    let serving = layout.serve_lock().unwrap();
    let before = fs::read(root.remote().join("remote.db")).unwrap();
    assert_eq!(Store::stopped_port(&serving).unwrap(), Some(12345));
    assert_eq!(fs::read(root.remote().join("remote.db")).unwrap(), before);
    let mut migrated = Store::open(&serving).unwrap();
    let machine = migrated.machine().unwrap();
    assert_eq!(machine.id, "00000000-0000-4000-8000-000000000001");
    assert!(tmt_remote::canonical::route_prefix(&machine.route_prefix));
    assert_ne!(machine.route_prefix, old_prefix);
    assert_eq!(migrated.remembered_port().unwrap(), Some(12345));
    let grants = migrated.grants().unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].client_id, "00000000-0000-4000-8000-000000000002");
    assert_eq!(grants[0].origin, "http://127.0.0.1:12345");
    assert_eq!(grants[0].name, "Legacy browser");
    assert_eq!(grants[0].revision, 1);
    assert!(!grants[0].disabled);
    assert_eq!(grants[0].public_key, [0; 32]);
    drop(migrated);
    let mut again = Store::open(&serving).unwrap();
    assert_eq!(again.machine().unwrap(), machine);
    assert_eq!(again.grants().unwrap(), grants);
    assert_eq!(MachineKey::open(&layout).unwrap().public(), public);
}
#[test]
fn a_damaged_legacy_prefix_refuses_without_recording_the_migration() {
    let root = Root::new();
    let layout = schema5(&root, "/r/invalid");
    let serving = layout.serve_lock().unwrap();
    assert_eq!(
        Store::open(&serving).err().unwrap().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    let inspect = Connection::open(root.remote().join("remote.db")).unwrap();
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        inspect
            .query_row("SELECT route_prefix FROM machine", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "/r/invalid"
    );
}

#[test]
fn stopped_port_reads_are_noncreating_and_legacy_state_is_not_migrated() {
    let root = Root::new();
    assert!(Layout::existing(&root.0).unwrap().is_none());
    assert!(!root.remote().exists());
    let layout = Layout::open(&root.0).unwrap();
    assert!(layout.existing_serve_lock().unwrap().is_none());
    assert_eq!(fs::read_dir(root.remote()).unwrap().count(), 0);
    let serving = layout.serve_lock().unwrap();
    assert_eq!(Store::stopped_port(&serving).unwrap(), None);
    assert!(!root.remote().join("remote.db").exists());
    let store = Store::open(&serving).unwrap();
    assert!(store.remember_port(0).is_err());
    store.remember_port(12345).unwrap();
    assert_eq!(store.remembered_port().unwrap(), Some(12345));
    drop(store);
    drop(serving);
    let db = root.remote().join("remote.db");
    let before = fs::read(&db).unwrap();
    let stopped = layout.existing_serve_lock().unwrap().unwrap();
    assert_eq!(Store::stopped_port(&stopped).unwrap(), Some(12345));
    assert_eq!(fs::read(&db).unwrap(), before);
    drop(stopped);
    // Model the previously shipped schema, preserving its exact migration names.
    let old = Connection::open(&db).unwrap();
    old.execute_batch("DROP TABLE door_port; DELETE FROM _migrations WHERE version >= 5")
        .unwrap();
    drop(old);
    let before = fs::read(&db).unwrap();
    let stopped = layout.existing_serve_lock().unwrap().unwrap();
    assert_eq!(Store::stopped_port(&stopped).unwrap(), None);
    assert_eq!(
        fs::read(&db).unwrap(),
        before,
        "stopped status migrated old state"
    );
    drop(stopped);
    let serving = layout.serve_lock().unwrap();
    let upgraded = Store::open(&serving).unwrap();
    assert_eq!(upgraded.remembered_port().unwrap(), None);
    drop(upgraded);
    drop(serving);
    let invalid = Connection::open(&db).unwrap();
    invalid
        .execute_batch("PRAGMA ignore_check_constraints=ON; INSERT INTO door_port VALUES (1, 0)")
        .unwrap();
    drop(invalid);
    let stopped = layout.existing_serve_lock().unwrap().unwrap();
    assert_eq!(
        Store::stopped_port(&stopped).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    drop(stopped);
    fs::write(&db, b"damaged database").unwrap();
    let stopped = layout.existing_serve_lock().unwrap().unwrap();
    assert_eq!(
        Store::stopped_port(&stopped).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(fs::read(&db).unwrap(), b"damaged database");
}

#[test]
fn stop_wait_is_bounded_and_holds_the_released_lease_for_confirmation() {
    use std::time::{Duration, Instant};
    let root = Root::new();
    let layout = Layout::open(&root.0).unwrap();
    let running = layout.serve_lock().unwrap();
    let before = fs::read(root.remote().join("serve.lock")).unwrap();
    let start = Instant::now();
    assert_eq!(
        layout
            .wait_for_release(start + Duration::from_millis(20))
            .err()
            .unwrap()
            .code,
        "REMOTE_STOP_TIMEOUT"
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(fs::read(root.remote().join("serve.lock")).unwrap(), before);
    drop(running);
    let confirmed = layout.wait_for_release(Instant::now()).unwrap().unwrap();
    assert_eq!(
        layout.serve_lock().err().unwrap().code,
        "REMOTE_ALREADY_SERVING"
    );
    drop(confirmed);
    assert!(layout.serve_lock().is_ok());
}
