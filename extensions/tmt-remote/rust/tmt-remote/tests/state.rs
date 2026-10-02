//! Remote private state on real temporary data roots.
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
    let machine = Store::open(&layout).unwrap().machine().unwrap();
    assert_eq!(mode(&root.remote().join("machine.key")), 0o600);
    assert_eq!(mode(&root.remote().join("remote.db")), 0o600);
    let reopened = Layout::open(&root.0).unwrap();
    assert_eq!(MachineKey::open(&reopened).unwrap().public(), key);
    assert_eq!(Store::open(&reopened).unwrap().machine().unwrap(), machine);
    assert!(machine.route_prefix.starts_with("/r/") && machine.route_prefix.len() == 35);
    // Distinct roots get distinct identities.
    let other = Root::new();
    let other_layout = Layout::open(&other.0).unwrap();
    assert_ne!(MachineKey::open(&other_layout).unwrap().public(), key);
    let other_machine = Store::open(&other_layout).unwrap().machine().unwrap();
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
    fs::remove_file(root.remote().join("remote.db")).ok();
    let target = root.0.join("elsewhere.db");
    fs::write(&target, b"").unwrap();
    symlink(&target, root.remote().join("remote.db")).unwrap();
    assert_eq!(
        Store::open(&layout).err().unwrap().code,
        "REMOTE_STATE_UNSAFE"
    );
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
