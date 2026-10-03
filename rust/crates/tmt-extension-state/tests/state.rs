//! Filesystem admission and publication on disposable roots, without crypto or stores.
use std::{
    fs, io,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_extension_state::{Error, Layout};

const FILES: &[&str] = &["seed", "publication.lock", "serve.lock", "state.db"];
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-extension-state-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn layout(&self) -> Layout {
        Layout::open(&self.0, "extension", FILES).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o777
}
fn private_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn lookup_is_read_only_and_open_preserves_the_trusted_root_and_siblings() {
    let root = Root::new();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o751)).unwrap();
    let sibling = root.0.join("core.db");
    fs::write(&sibling, b"untouched core bytes").unwrap();
    assert!(
        Layout::existing(&root.0.join("missing"), "extension", FILES)
            .unwrap()
            .is_none()
    );
    assert!(!root.0.join("missing").exists());
    assert!(
        Layout::existing(&root.0, "extension", FILES)
            .unwrap()
            .is_none()
    );
    let layout = root.layout();
    assert_eq!(mode(&layout.directory), 0o700);
    assert_eq!(mode(&root.0), 0o751);
    assert_eq!(fs::read(sibling).unwrap(), b"untouched core bytes");
    assert!(!layout.running("serve.lock").unwrap());
    assert!(matches!(layout.read("seed", 33), Err(Error::ReadOpen(_))));
    assert_eq!(fs::read_dir(&layout.directory).unwrap().count(), 0);
    assert!(
        Layout::existing(&root.0, "extension", FILES)
            .unwrap()
            .is_some()
    );
    let held = layout.lock("serve.lock").unwrap();
    assert_eq!(mode(&layout.directory.join("serve.lock")), 0o600);
    assert!(layout.running("serve.lock").unwrap());
    assert!(matches!(layout.lock("serve.lock"), Err(Error::Busy)));
    drop(held);
    assert!(!layout.running("serve.lock").unwrap());
}

#[test]
fn aliases_are_allowed_only_in_the_trusted_root_and_private_names_are_bounded() {
    let root = Root::new();
    let alias = root.0.join("alias");
    let real = root.0.join("real");
    fs::create_dir(&real).unwrap();
    symlink(&real, &alias).unwrap();
    let layout = Layout::open(&alias, "extension", FILES).unwrap();
    assert_eq!(
        layout.directory,
        fs::canonicalize(&real).unwrap().join("extension")
    );
    for name in ["../escape", "/absolute", "seed/", "unknown"] {
        assert!(matches!(layout.file(name), Err(Error::InvalidFileName)));
    }
    assert!(matches!(
        Layout::open(Path::new("relative"), "extension", FILES),
        Err(Error::RootNotAbsolute)
    ));
    assert!(matches!(
        Layout::open(&root.0, "../escape", FILES),
        Err(Error::InvalidFileName)
    ));
    let other = Root::new();
    symlink(&layout.directory, other.0.join("extension")).unwrap();
    assert!(matches!(other.layout_result(), Err(Error::UnsafeDirectory)));
    assert!(matches!(
        Layout::existing(&other.0, "extension", FILES),
        Err(Error::UnsafeDirectory)
    ));
}

impl Root {
    fn layout_result(&self) -> Result<Layout, Error> {
        Layout::open(&self.0, "extension", FILES)
    }
}

#[test]
fn file_admission_refuses_modes_symlinks_and_nonregular_files_without_following() {
    let root = Root::new();
    let layout = root.layout();
    let seed = layout.directory.join("seed");
    private_file(&seed, &[9; 4096]);
    assert_eq!(layout.read("seed", 33).unwrap(), [9; 33]);
    fs::set_permissions(&seed, fs::Permissions::from_mode(0o640)).unwrap();
    assert!(matches!(layout.read("seed", 33), Err(Error::UnsafeFile)));
    assert!(matches!(layout.file("seed"), Err(Error::UnsafeFile)));
    assert_eq!(fs::read(&seed).unwrap(), [9; 4096]);
    fs::remove_file(&seed).unwrap();
    let outside = root.0.join("outside");
    private_file(&outside, b"outside bytes");
    symlink(&outside, &seed).unwrap();
    assert!(matches!(layout.read("seed", 33), Err(Error::ReadOpen(_))));
    assert!(matches!(layout.file("seed"), Err(Error::FileOpen(_))));
    assert_eq!(fs::read(outside).unwrap(), b"outside bytes");
    fs::remove_file(&seed).unwrap();
    nix::unistd::mkfifo(
        &seed,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    assert!(matches!(layout.read("seed", 33), Err(Error::UnsafeFile)));
    assert!(matches!(layout.file("seed"), Err(Error::UnsafeFile)));
}

#[test]
fn publication_is_create_only_and_cleanup_cannot_run_while_the_lock_is_held() {
    let root = Root::new();
    let layout = root.layout();
    let publication = layout
        .publication("seed", "publication.lock", ".seed-", 32)
        .unwrap();
    let nonce = [0x12; 16];
    let temporary = layout.directory.join(format!(".seed-{}", "12".repeat(16)));
    let mut staged = publication.stage(&nonce).unwrap();
    assert_eq!(mode(&temporary), 0o600);
    assert!(matches!(
        layout.publication("seed", "publication.lock", ".seed-", 32),
        Err(Error::Busy)
    ));
    assert!(temporary.exists());
    staged.write_and_link(&[3; 32]).unwrap();
    drop(staged);
    publication.discard(&nonce).unwrap();
    layout.sync().unwrap();
    assert_eq!(fs::read(layout.directory.join("seed")).unwrap(), [3; 32]);
    let mut losing = publication.stage(&nonce).unwrap();
    losing.write_and_link(&[4; 32]).unwrap();
    drop(losing);
    publication.discard(&nonce).unwrap();
    layout.sync().unwrap();
    assert_eq!(fs::read(layout.directory.join("seed")).unwrap(), [3; 32]);
    assert!(!temporary.exists());
    assert_eq!(mode(&layout.directory.join("seed")), 0o600);
    // Failure to stage leaves the caller in control of cleanup and error precedence.
    private_file(&temporary, b"collision");
    assert_eq!(
        publication.stage(&nonce).err().unwrap().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&temporary).unwrap(), b"collision");
    publication.discard(&nonce).unwrap();
    layout.sync().unwrap();
    drop(publication);
    assert!(
        layout
            .publication("seed", "publication.lock", ".seed-", 32)
            .is_ok()
    );
}

#[test]
fn stale_cleanup_preserves_foreign_names_and_refuses_unsafe_or_oversized_matches() {
    let root = Root::new();
    let layout = root.layout();
    let stale = layout.directory.join(format!(".seed-{}", "a".repeat(32)));
    private_file(&stale, b"partial");
    let foreign = layout.directory.join(".seed-note");
    fs::write(&foreign, b"keep").unwrap();
    drop(
        layout
            .publication("seed", "publication.lock", ".seed-", 32)
            .unwrap(),
    );
    assert!(!stale.exists());
    assert_eq!(fs::read(&foreign).unwrap(), b"keep");
    private_file(&stale, &[0; 33]);
    assert!(matches!(
        layout.publication("seed", "publication.lock", ".seed-", 32),
        Err(Error::InvalidLength)
    ));
    assert_eq!(fs::read(&stale).unwrap(), [0; 33]);
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        layout.publication("seed", "publication.lock", ".seed-", 32),
        Err(Error::UnsafeFile)
    ));
    fs::remove_file(&stale).unwrap();
    symlink(&foreign, &stale).unwrap();
    assert!(matches!(
        layout.publication("seed", "publication.lock", ".seed-", 32),
        Err(Error::ReadOpen(_))
    ));
    assert!(fs::symlink_metadata(&stale).unwrap().is_symlink());
    assert_eq!(fs::read(foreign).unwrap(), b"keep");
    fs::remove_file(&stale).unwrap();
    // A rejected temporary must release the publication lock for the next admission.
    assert!(
        layout
            .publication("seed", "publication.lock", ".seed-", 32)
            .is_ok()
    );
}
