//! `LocalFs` on real temporary data roots: the shared conformance suite plus the
//! private-tree, ledger and lease checks only the local adapter has. Crash
//! windows and races at exact durable milestones live beside the algorithms in
//! `src/objects/local/tests.rs`.
use crate::conformance::{self, Adapter, CHUNK, Session, bytes, io, receipt, spec, upload};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use tmt_remote::{
    objects::{
        BackendError, BeginResult, Clock, ExtensionId, LocalFs, NamespaceId, ObjectBackend, Quotas,
        TransferState, Usage,
    },
    state::{Layout, Serving},
};

pub struct Root(pub PathBuf);
impl Root {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = PathBuf::from(format!(
            "/tmp/t1850-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn remote(&self) -> PathBuf {
        self.0.join("remote")
    }
    pub fn ledger(&self) -> PathBuf {
        self.remote().join("objects.db")
    }
    pub fn tree(&self, extension: &str) -> PathBuf {
        self.0.join(extension).join("objects")
    }
    pub fn serving(&self) -> Serving {
        Layout::open(&self.0).unwrap().serve_lock().unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        // Restore searchable modes so an interrupted unsafe-mode test still cleans up.
        let _ = std::process::Command::new("chmod")
            .args(["-R", "u+rwX"])
            .arg(&self.0)
            .status();
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn clock(now: &Arc<AtomicU64>) -> Clock {
    let now = now.clone();
    Arc::new(move || now.load(Ordering::SeqCst))
}
pub fn open<'a>(
    serving: &'a Serving,
    quotas: Quotas,
    now: &Arc<AtomicU64>,
) -> Result<LocalFs<'a>, tmt_remote::error::RemoteError> {
    LocalFs::open(serving, quotas, clock(now), &io())
}
fn handle<'a>(fs: &'a LocalFs<'_>, extension: &str) -> impl ObjectBackend + 'a {
    fs.handle(ExtensionId::new(extension).unwrap())
}
pub fn final_path(root: &Root, extension: &str, namespace: u8, object: u8) -> PathBuf {
    root.tree(extension)
        .join("blobs")
        .join(hex(&[namespace; 32]))
        .join(hex(&[object; 32]))
}
pub fn staging_path(root: &Root, extension: &str, intent: u8) -> PathBuf {
    root.tree(extension)
        .join("staging")
        .join(hex(&[intent; 32]))
}

struct LocalSession<'a> {
    serving: &'a Serving,
    quotas: Quotas,
    now: Arc<AtomicU64>,
    fs: Option<LocalFs<'a>>,
}
impl Session for LocalSession<'_> {
    fn backend(&self, extension: &str) -> Box<dyn ObjectBackend + '_> {
        Box::new(
            self.fs
                .as_ref()
                .unwrap()
                .handle(ExtensionId::new(extension).unwrap()),
        )
    }
    fn advance(&self, milliseconds: u64) {
        self.now.fetch_add(milliseconds, Ordering::SeqCst);
    }
    fn restart(&mut self) {
        self.fs = None;
        self.fs = Some(open(self.serving, self.quotas, &self.now).unwrap());
    }
    fn installation(&self) -> Usage {
        self.fs.as_ref().unwrap().installation_usage(&io()).unwrap()
    }
}
pub struct LocalAdapter;
impl Adapter for LocalAdapter {
    fn run(&self, quotas: Quotas, scenario: &mut dyn FnMut(&mut dyn Session)) {
        let root = Root::new();
        let serving = root.serving();
        let now = Arc::new(AtomicU64::new(1_000_000_000));
        let mut session = LocalSession {
            serving: &serving,
            quotas,
            fs: Some(open(&serving, quotas, &now).unwrap()),
            now,
        };
        scenario(&mut session);
    }
}
mod suite {
    use super::LocalAdapter;
    crate::conformance!(LocalAdapter);
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}
/// Every entry under `path` with its mode, length and SHA-256, for equality before and after.
fn snapshot(path: &Path) -> Vec<(PathBuf, u32, String)> {
    let mut out = Vec::new();
    let mut stack = vec![path.to_owned()];
    while let Some(next) = stack.pop() {
        let metadata = fs::symlink_metadata(&next).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        if metadata.is_dir() {
            for entry in fs::read_dir(&next).unwrap() {
                stack.push(entry.unwrap().path());
            }
            out.push((next, mode, "directory".into()));
        } else if metadata.is_file() {
            let body = fs::read(&next).unwrap();
            let digest = Sha256::digest(&body);
            out.push((next, mode, format!("{}:{}", body.len(), hex(&digest))));
        } else {
            out.push((next, mode, "non-regular".into()));
        }
    }
    out.sort();
    out
}

#[test]
fn the_payload_tree_is_private_extension_scoped_and_leaves_no_staging() {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let local = open(&serving, Quotas::contract(), &now).unwrap();
    let alpha = handle(&local, "alpha");
    let payload = bytes(CHUNK + 9, 1);
    let sp = spec(1, 2, 3, &payload);
    assert!(
        !root.0.join("alpha").exists(),
        "nothing is created before the first write"
    );
    alpha.begin(&sp, &io()).unwrap();
    assert!(
        !root.0.join("alpha").exists(),
        "an adoption alone creates no payload tree"
    );
    upload(&alpha, &sp, &payload);
    let key = final_path(&root, "alpha", 2, 3);
    assert_eq!(fs::read(&key).unwrap(), payload);
    assert_eq!(mode(&key), 0o600);
    assert_eq!(
        fs::metadata(&key).unwrap().nlink(),
        1,
        "the staging name is gone"
    );
    assert!(!staging_path(&root, "alpha", 1).exists());
    for dir in [
        root.0.join("alpha"),
        root.tree("alpha"),
        root.tree("alpha").join("staging"),
        root.tree("alpha").join("blobs"),
        key.parent().unwrap().to_owned(),
    ] {
        assert_eq!(mode(&dir), 0o700, "{dir:?}");
        assert_eq!(
            fs::metadata(&dir).unwrap().uid(),
            fs::metadata(&key).unwrap().uid()
        );
    }
    assert_eq!(mode(&root.ledger()), 0o600);
    assert!(
        !root.0.join("beta").exists(),
        "another extension has no tree"
    );
    // The ledger is the only metadata: no sidecar or stray file remains.
    let names: Vec<_> = fs::read_dir(root.remote())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        names.iter().all(|n| !n.starts_with("objects.db-")),
        "{names:?}"
    );
}

#[test]
fn status_stat_and_usage_never_write() {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let local = open(&serving, Quotas::contract(), &now).unwrap();
    let alpha = handle(&local, "alpha");
    let payload = bytes(CHUNK + 1, 2);
    let (done, stale) = (spec(1, 1, 1, &payload), spec(2, 1, 2, &payload));
    upload(&alpha, &done, &payload);
    alpha.begin(&stale, &io()).unwrap();
    alpha
        .append(stale.intent, 0, &payload[..CHUNK], &io())
        .unwrap();
    now.fetch_add(2 * conformance::DAY, Ordering::SeqCst);
    let before = snapshot(&root.0);
    assert_eq!(
        alpha.status(stale.intent, &io()).unwrap().state,
        TransferState::Expired
    );
    alpha.status(done.intent, &io()).unwrap();
    alpha
        .status(conformance::spec(9, 9, 9, b"").intent, &io())
        .unwrap();
    alpha.stat(done.key, &io()).unwrap();
    alpha.usage(None, &io()).unwrap();
    assert_eq!(
        snapshot(&root.0),
        before,
        "observation expires, trims, publishes and reconciles nothing"
    );
}

/// An outside directory and file that no attack may read through or modify.
struct Canary {
    directory: PathBuf,
    file: PathBuf,
}
impl Canary {
    fn new(root: &Root) -> Self {
        let directory = root.0.join("outside");
        fs::create_dir(&directory).unwrap();
        let file = directory.join("secret");
        fs::write(&file, b"outside bytes").unwrap();
        // Valid-looking targets (owned, 0700/0600): only no-follow stops a link to them.
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        Self { directory, file }
    }
    fn untouched(&self) {
        assert_eq!(fs::read(&self.file).unwrap(), b"outside bytes");
        let names: Vec<_> = fs::read_dir(&self.directory)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["secret"], "nothing was created outside the tree");
    }
}

#[test]
fn symlinks_and_unsafe_modes_at_every_level_fail_closed_without_touching_outside() {
    let payload = bytes(2 * CHUNK, 3);
    let (committed, staged) = (spec(1, 1, 1, &payload), spec(2, 1, 2, &payload));
    // `refuses_at_restart`: readiness reconciles the staged original through this path.
    struct Level {
        name: &'static str,
        locate: fn(&Root) -> PathBuf,
        directory: bool,
        refuses_at_restart: bool,
    }
    let level = |name, locate, directory, refuses_at_restart| Level {
        name,
        locate,
        directory,
        refuses_at_restart,
    };
    let levels = [
        level(
            "extension",
            (|r| r.0.join("alpha")) as fn(&Root) -> PathBuf,
            true,
            true,
        ),
        level("objects", |r| r.tree("alpha"), true, true),
        level("staging", |r| r.tree("alpha").join("staging"), true, true),
        level("blobs", |r| r.tree("alpha").join("blobs"), true, false),
        level(
            "namespace",
            |r| final_path(r, "alpha", 1, 1).parent().unwrap().to_owned(),
            true,
            false,
        ),
        level("payload", |r| final_path(r, "alpha", 1, 1), false, false),
        level(
            "staged payload",
            |r| staging_path(r, "alpha", 2),
            false,
            true,
        ),
    ];
    for Level {
        name,
        locate,
        directory,
        refuses_at_restart,
    } in levels
    {
        for attack in ["symlink", "mode"] {
            let root = Root::new();
            let canary = Canary::new(&root);
            let serving = root.serving();
            let now = Arc::new(AtomicU64::new(1_000_000_000));
            let local = open(&serving, Quotas::contract(), &now).unwrap();
            let alpha = handle(&local, "alpha");
            upload(&alpha, &committed, &payload);
            alpha.begin(&staged, &io()).unwrap();
            alpha
                .append(staged.intent, 0, &payload[..CHUNK], &io())
                .unwrap();
            let target = locate(&root);
            match attack {
                "symlink" => {
                    let moved = target.with_extension("moved");
                    fs::rename(&target, &moved).unwrap();
                    symlink(
                        if directory {
                            &canary.directory
                        } else {
                            &canary.file
                        },
                        &target,
                    )
                    .unwrap();
                }
                _ => fs::set_permissions(
                    &target,
                    fs::Permissions::from_mode(if directory { 0o755 } else { 0o644 }),
                )
                .unwrap(),
            }
            let context = format!("{name} {attack}");
            // Reads and writes that traverse the attacked path all refuse.
            let traverses_committed = !matches!(name, "staging" | "staged payload");
            if traverses_committed {
                assert_eq!(
                    alpha.read(committed.key, 0, 8, &io()),
                    Err(BackendError::Unavailable),
                    "{context}"
                );
            }
            if !matches!(name, "blobs" | "namespace" | "payload") {
                assert_eq!(
                    alpha.append(staged.intent, 1, &payload[CHUNK..], &io()),
                    Err(BackendError::Unavailable),
                    "{context}"
                );
            }
            canary.untouched();
            assert_eq!(
                alpha.status(staged.intent, &io()).unwrap().state,
                TransferState::Pending(conformance::pending(&staged, 1, CHUNK as u64)),
                "{context}: the charged original is retained, never reset"
            );
            // Readiness never adopts a fresh tree over damage: it either refuses (the
            // staged original lives behind the damaged path) or keeps refusing reads.
            drop(alpha);
            drop(local);
            match open(&serving, Quotas::contract(), &now) {
                Err(_) => assert!(refuses_at_restart, "{context}: unexpected refusal"),
                Ok(opened) => {
                    assert!(!refuses_at_restart, "{context}: damage went unnoticed");
                    assert_eq!(
                        handle(&opened, "alpha").read(committed.key, 0, 8, &io()),
                        Err(BackendError::Unavailable),
                        "{context} after restart"
                    );
                }
            }
            canary.untouched();
        }
    }
}

#[test]
fn a_hard_link_alias_of_a_committed_payload_is_refused() {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let local = open(&serving, Quotas::contract(), &now).unwrap();
    let alpha = handle(&local, "alpha");
    let payload = bytes(64, 4);
    let sp = spec(1, 1, 1, &payload);
    upload(&alpha, &sp, &payload);
    assert_eq!(alpha.read(sp.key, 0, 8, &io()).unwrap().bytes, payload[..8]);
    fs::hard_link(final_path(&root, "alpha", 1, 1), root.0.join("alias")).unwrap();
    assert_eq!(
        alpha.read(sp.key, 0, 8, &io()),
        Err(BackendError::Unavailable)
    );
    assert_eq!(fs::read(root.0.join("alias")).unwrap(), payload);
}

#[test]
fn a_foreign_entry_in_a_namespace_keeps_removal_unsettled_and_charged() {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let local = open(&serving, Quotas::contract(), &now).unwrap();
    let alpha = handle(&local, "alpha");
    let payload = bytes(64, 5);
    let sp = spec(1, 1, 1, &payload);
    upload(&alpha, &sp, &payload);
    let directory = final_path(&root, "alpha", 1, 1)
        .parent()
        .unwrap()
        .to_owned();
    fs::write(directory.join("foreign"), b"not ours").unwrap();
    let charged = alpha.usage(None, &io()).unwrap().charged_bytes;
    assert_eq!(
        alpha.remove_namespace(NamespaceId([1; 32]), &io()),
        Err(BackendError::Unavailable)
    );
    assert_eq!(fs::read(directory.join("foreign")).unwrap(), b"not ours");
    assert_eq!(
        alpha.stat(sp.key, &io()),
        Err(BackendError::Missing),
        "the fence is durable"
    );
    assert_eq!(alpha.usage(None, &io()).unwrap().charged_bytes, charged);
    drop(alpha);
    drop(local);
    assert!(
        open(&serving, Quotas::contract(), &now).is_err(),
        "an unsettled removal refuses readiness instead of resetting"
    );
    fs::remove_file(directory.join("foreign")).unwrap();
    let local = open(&serving, Quotas::contract(), &now).unwrap();
    let alpha = handle(&local, "alpha");
    assert_eq!(
        alpha.usage(None, &io()).unwrap().charged_bytes,
        conformance::RECORD + conformance::FENCE + conformance::TREE
    );
    assert!(!directory.exists());
}

#[test]
fn the_backend_cannot_be_reached_without_the_serve_lease() {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let local = open(&serving, Quotas::contract(), &now).unwrap();
    let again = Layout::open(&root.0).unwrap().serve_lock();
    assert!(matches!(again, Err(error) if error.code == "REMOTE_ALREADY_SERVING"));
    drop(local);
    assert!(
        open(&serving, Quotas::contract(), &now).is_ok(),
        "the proof is reusable by its holder"
    );
}

#[test]
fn the_maximum_payload_round_trips_and_larger_inputs_are_refused_before_any_effect() {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let local = open(&serving, Quotas::contract(), &now).unwrap();
    let alpha = handle(&local, "alpha");
    let mut over = spec(1, 1, 1, b"x");
    over.payload_bytes = tmt_remote::limits::OBJECT_PAYLOAD_BYTES + 1;
    assert_eq!(alpha.begin(&over, &io()), Err(BackendError::Invalid));
    let mut wide = spec(2, 1, 2, b"x");
    wide.binding = vec![0; tmt_remote::limits::OBJECT_BINDING_BYTES + 1];
    assert_eq!(alpha.begin(&wide, &io()), Err(BackendError::Invalid));
    assert_eq!(alpha.usage(None, &io()).unwrap(), Usage::default());
    let payload = bytes(tmt_remote::limits::OBJECT_PAYLOAD_BYTES as usize, 6);
    let sp = spec(3, 1, 3, &payload);
    assert_eq!(upload(&alpha, &sp, &payload), receipt(&sp));
    let last = alpha
        .read(sp.key, payload.len() as u64 - 5, 100, &io())
        .unwrap();
    assert_eq!(last.bytes, payload[payload.len() - 5..]);
    assert_eq!(
        alpha.begin(&sp, &io()),
        Ok(BeginResult::Committed(receipt(&sp)))
    );
}

#[test]
fn the_charge_constants_and_extension_names_are_pinned() {
    use tmt_remote::limits;
    assert_eq!(
        (
            limits::OBJECT_BLOCK_BYTES,
            limits::OBJECT_RECORD_BYTES,
            limits::OBJECT_FENCE_BYTES,
            limits::OBJECT_LEDGER_BASE_BYTES
        ),
        (4096, 8192, 4096 + 512, 256 * 1024)
    );
    assert_eq!(
        (
            limits::OBJECT_CHUNK_BYTES,
            limits::OBJECT_PAYLOAD_BYTES,
            limits::OBJECT_BINDING_BYTES
        ),
        (32 * 1024, 12 * 1024 * 1024, 2048)
    );
    assert_eq!(limits::OBJECT_TREE_BASE_BYTES, 5 * 4096);
    assert_eq!(limits::OBJECT_STAGING.as_secs(), 24 * 60 * 60);
    for bad in [
        "",
        "remote",
        "Alpha",
        "a/b",
        "..",
        "a b",
        &"x".repeat(33),
        "9lives",
    ] {
        assert!(ExtensionId::new(bad).is_err(), "{bad:?}");
    }
    assert!(ExtensionId::new("colab").is_ok());
}

#[test]
fn contract_quotas_are_the_accepted_bounds() {
    let q = Quotas::contract();
    let mib = 1024 * 1024;
    assert_eq!(
        (q.namespace_bytes, q.extension_bytes, q.installation_bytes),
        (64 * mib, 512 * mib, 1024 * mib)
    );
    assert_eq!(
        (
            q.namespace_entries,
            q.extension_entries,
            q.installation_entries
        ),
        (1024, 8192, 32768)
    );
    assert_eq!(
        (
            q.active_intents,
            q.retained_extension,
            q.retained_installation
        ),
        (32, 16384, 65536)
    );
}

/// Open once so the ledger exists, then return its path with the lease held.
fn ledger_fixture() -> (Root, Serving, Arc<AtomicU64>) {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    {
        let local = open(&serving, Quotas::contract(), &now).unwrap();
        upload(&handle(&local, "alpha"), &spec(1, 1, 1, b"abc"), b"abc");
    }
    (root, serving, now)
}
fn refusal(serving: &Serving, now: &Arc<AtomicU64>) -> String {
    match open(serving, Quotas::contract(), now) {
        Ok(_) => panic!("a damaged ledger must refuse readiness"),
        Err(error) => error.code,
    }
}

#[test]
fn a_damaged_or_unsafe_ledger_refuses_and_is_never_reset() {
    let (root, serving, now) = ledger_fixture();
    let ledger = root.ledger();
    let pristine = fs::read(&ledger).unwrap();
    // Garbage and truncation.
    fs::write(&ledger, b"this is not a database").unwrap();
    assert_ne!(refusal(&serving, &now), "");
    assert_eq!(
        fs::read(&ledger).unwrap(),
        b"this is not a database",
        "not reset"
    );
    fs::write(&ledger, &pristine[..10]).unwrap();
    assert_ne!(refusal(&serving, &now), "");
    assert_eq!(fs::read(&ledger).unwrap(), &pristine[..10]);
    fs::write(&ledger, &pristine).unwrap();
    // Wrong mode and symlink.
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNSAFE");
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o600)).unwrap();
    let outside = root.0.join("outside.db");
    fs::rename(&ledger, &outside).unwrap();
    symlink(&outside, &ledger).unwrap();
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNSAFE");
    fs::remove_file(&ledger).unwrap();
    fs::rename(&outside, &ledger).unwrap();
    // Newer, extra object, altered schema, inconsistent row.
    let raw = |sql: &str| {
        Connection::open(&ledger)
            .unwrap()
            .execute_batch(sql)
            .unwrap()
    };
    raw("PRAGMA user_version=2");
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNSUPPORTED");
    raw("PRAGMA user_version=1; CREATE TABLE stray(x)");
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNAVAILABLE");
    raw("DROP TABLE stray; DROP TRIGGER intents_retained");
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNAVAILABLE");
    raw(
        "CREATE TRIGGER intents_retained BEFORE DELETE ON intents BEGIN SELECT RAISE(ABORT,'retained original intent'); END",
    );
    raw("UPDATE intents SET next_index=9");
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNAVAILABLE");
    raw("UPDATE intents SET next_index=1");
    raw("UPDATE intents SET payload_charged=0");
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNAVAILABLE");
    raw("UPDATE intents SET payload_charged=4096");
    assert!(
        open(&serving, Quotas::contract(), &now).is_ok(),
        "the repaired ledger opens"
    );
}

#[test]
fn derived_sqlite_files_are_admitted_like_the_database() {
    let (root, serving, now) = ledger_fixture();
    let journal = root.remote().join("objects.db-journal");
    let outside = root.0.join("outside-journal");
    fs::write(&outside, b"outside").unwrap();
    symlink(&outside, &journal).unwrap();
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNSAFE");
    fs::remove_file(&journal).unwrap();
    fs::write(&journal, b"").unwrap();
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNSAFE");
    fs::remove_file(&journal).unwrap();
    for sidecar in ["objects.db-wal", "objects.db-shm"] {
        let path = root.remote().join(sidecar);
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(refusal(&serving, &now), "REMOTE_STATE_UNSAFE", "{sidecar}");
        fs::remove_file(&path).unwrap();
    }
    assert_eq!(fs::read(&outside).unwrap(), b"outside");
    assert!(open(&serving, Quotas::contract(), &now).is_ok());
}

#[test]
fn originals_and_fences_are_immutable_and_never_deleted() {
    let (root, serving, now) = ledger_fixture();
    let local = open(&serving, Quotas::contract(), &now).unwrap();
    handle(&local, "alpha")
        .remove_namespace(NamespaceId([1; 32]), &io())
        .unwrap();
    let connection = Connection::open(root.ledger()).unwrap();
    for sql in [
        "UPDATE intents SET payload_bytes=1",
        "UPDATE intents SET payload_sha256=zeroblob(32)",
        "UPDATE intents SET binding=x''",
        "UPDATE intents SET expires_ms=expires_ms+1",
        "UPDATE intents SET intent=zeroblob(32)",
        "DELETE FROM intents",
        "DELETE FROM namespaces",
    ] {
        assert!(connection.execute(sql, []).is_err(), "{sql}");
    }
}

#[test]
fn the_charge_formula_bounds_the_ledger_footprint() {
    let (root, serving, now) = ledger_fixture();
    drop(open(&serving, Quotas::contract(), &now).unwrap());
    let mut connection = Connection::open(root.ledger()).unwrap();
    let before: i64 = connection
        .query_row("SELECT COUNT(*) FROM intents", [], |r| r.get(0))
        .unwrap();
    // Maximal rows with distinct keys and namespaces: the worst case for pages and indexes.
    let rows = 4000_i64;
    {
        let tx = connection.transaction().unwrap();
        for n in 0..rows {
            let id = |fill: u8| {
                let mut id = vec![fill; 32];
                id[..8].copy_from_slice(&(n + 1000).to_be_bytes());
                id
            };
            tx.execute(
                "INSERT INTO namespaces VALUES (?1,?2,'open')",
                rusqlite::params!["a-maximal-extension-name-32-chars", id(1)],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO intents VALUES (?1,?2,?3,?4,?5,4096,?6,1,2,'committed',1,4096,4096,1,0)",
                rusqlite::params![
                    "a-maximal-extension-name-32-chars",
                    id(2),
                    id(1),
                    id(3),
                    id(4),
                    vec![9u8; tmt_remote::limits::OBJECT_BINDING_BYTES]
                ],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let pages: i64 = connection
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let page: i64 = connection
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap();
    let footprint = (pages * page) as u64;
    let namespaces = (rows + 1) as u64; // the fixture's one plus the inserted
    let charged = tmt_remote::limits::OBJECT_LEDGER_BASE_BYTES
        + (rows + before) as u64 * tmt_remote::limits::OBJECT_RECORD_BYTES
        + namespaces * tmt_remote::limits::OBJECT_FENCE_BYTES;
    assert!(
        footprint <= charged,
        "ledger file {footprint} B exceeds its charge {charged} B"
    );
    // The retained records themselves, not the base, carry the bound.
    let per_row = (footprint - 65_536) as f64 / rows as f64;
    assert!(
        per_row < tmt_remote::limits::OBJECT_RECORD_BYTES as f64 * 0.75,
        "{per_row}"
    );
}

/// Physical bytes of an extension's payload tree under a conservative filesystem
/// model: each file's length rounded up to a 4 KiB block (a hard-linked inode once),
/// each directory a 4 KiB block, and 128 bytes per directory entry.
fn tree_footprint(root: &Path) -> u64 {
    let mut seen = std::collections::HashSet::new();
    let mut total = 0;
    let mut stack = vec![root.to_owned()];
    while let Some(next) = stack.pop() {
        let metadata = fs::symlink_metadata(&next).unwrap();
        if metadata.is_dir() {
            total += 4096;
            for entry in fs::read_dir(&next).unwrap() {
                total += 128;
                stack.push(entry.unwrap().path());
            }
        } else if seen.insert(metadata.ino()) {
            total += metadata.len().div_ceil(4096) * 4096;
        }
    }
    total
}

/// Builds one kind of retained state through the interface (and, for damage, the ledger).
type Population = fn(&dyn ObjectBackend, &Arc<AtomicU64>, &Root);

/// Quotas that never bind, so a population can be as large as the test needs.
fn roomy() -> Quotas {
    Quotas {
        active_intents: 1000,
        ..Quotas::contract()
    }
}
/// An original under a numbered namespace and object, with the largest binding.
fn numbered(
    intent: u16,
    namespace: u16,
    length: usize,
) -> (tmt_remote::objects::BeginSpec, Vec<u8>) {
    use tmt_remote::objects::{BlobKey, IntentId, OpaqueKey};
    let id = |fill: u8, n: u16| {
        let mut id = [fill; 32];
        id[..2].copy_from_slice(&n.to_be_bytes());
        id
    };
    let payload = bytes(length, 7);
    let mut sp = spec(1, 1, 1, &payload);
    sp.intent = IntentId(id(1, intent));
    sp.key = BlobKey {
        namespace: NamespaceId(id(2, namespace)),
        object: OpaqueKey(id(3, intent)),
    };
    sp.binding = vec![9; tmt_remote::limits::OBJECT_BINDING_BYTES];
    (sp, payload)
}
/// The empty ledger, measured on its own root.
fn empty_ledger_bytes() -> u64 {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    drop(open(&serving, Quotas::contract(), &now).unwrap());
    fs::metadata(root.ledger()).unwrap().len()
}
/// The conservative physical footprint (payload tree, each distinct inode once, plus
/// ledger rows beyond the empty ledger) against the formula's charge (without the
/// base, which the journal test bounds separately).
fn physical_vs_charge(root: &Root, local: &LocalFs<'_>, empty: u64) -> (u64, u64) {
    use tmt_remote::limits;
    let usage = local.installation_usage(&io()).unwrap();
    let ledger = fs::metadata(root.ledger()).unwrap().len();
    let tree = root.0.join("alpha");
    let tree = if tree.exists() {
        tree_footprint(&tree)
    } else {
        0
    };
    (
        tree + ledger.saturating_sub(empty),
        usage.charged_bytes - limits::OBJECT_LEDGER_BASE_BYTES,
    )
}
/// Build a population on a fresh root, then compare physical footprint with charge.
fn physical_and_charged(
    populate: impl FnOnce(&dyn ObjectBackend, &Arc<AtomicU64>, &Root),
) -> (u64, u64) {
    let empty = empty_ledger_bytes();
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let local = open(&serving, roomy(), &now).unwrap();
    let alpha = handle(&local, "alpha");
    populate(&alpha, &now, &root);
    physical_vs_charge(&root, &local, empty)
}

#[test]
fn the_charge_formula_bounds_the_physical_footprint_of_every_retained_state() {
    let populations: [(&str, Population); 6] = [
        // The densest case for directories: one namespace (a directory block) per object.
        ("one namespace per tiny object", |alpha, _, _| {
            for n in 0..150 {
                let (sp, payload) = numbered(n + 1, 1000 + n, 1);
                upload(alpha, &sp, &payload);
            }
        }),
        (
            "block-straddling payloads, one namespace each",
            |alpha, _, _| {
                for (n, length) in [4096usize, 4097, 33_000]
                    .into_iter()
                    .cycle()
                    .take(90)
                    .enumerate()
                {
                    let (sp, payload) = numbered(n as u16 + 1, 1000 + n as u16, length);
                    upload(alpha, &sp, &payload);
                }
            },
        ),
        // The densest case for ledger rows and directory entries.
        (
            "one namespace, empty and one-byte objects",
            |alpha, _, _| {
                for n in 0..400u16 {
                    let (sp, payload) = numbered(n + 1, 1, usize::from(n % 2));
                    upload(alpha, &sp, &payload);
                }
            },
        ),
        ("pending staging with parts written", |alpha, _, _| {
            for n in 0..40 {
                let (sp, payload) = numbered(n + 1, 2000 + n, 2 * CHUNK + 5);
                alpha.begin(&sp, &io()).unwrap();
                alpha
                    .append(sp.intent, 0, &payload[..CHUNK], &io())
                    .unwrap();
            }
        }),
        // Tombstones keep only their record and fence.
        (
            "discarded, expired and removed tombstones",
            |alpha, now, _| {
                let mut closing = Vec::new();
                for n in 0..60u16 {
                    let (sp, _) = numbered(n + 1, 3000 + n, 100);
                    alpha.begin(&sp, &io()).unwrap();
                    closing.push(sp);
                }
                for sp in closing.drain(..30) {
                    alpha.discard(sp.intent, &io()).unwrap();
                }
                now.fetch_add(2 * conformance::DAY, Ordering::SeqCst);
                for sp in &closing {
                    alpha.begin(sp, &io()).unwrap();
                }
                for n in 0..30u16 {
                    let (sp, payload) = numbered(100 + n, 4000 + n, 50);
                    upload(alpha, &sp, &payload);
                    alpha.remove_namespace(sp.key.namespace, &io()).unwrap();
                }
            },
        ),
        // Unknown originals stay charged with their bytes.
        ("unknown originals", |alpha, _, root| {
            for n in 0..40u16 {
                let (sp, payload) = numbered(n + 1, 5000 + n, 4097);
                upload(alpha, &sp, &payload);
            }
            Connection::open(root.ledger())
                .unwrap()
                .execute("UPDATE intents SET phase='unknown'", [])
                .unwrap();
        }),
    ];
    for (name, populate) in populations {
        let (physical, charged) = physical_and_charged(populate);
        eprintln!("{name}: physical {physical} B <= charged {charged} B");
        assert!(
            physical <= charged,
            "{name}: physical {physical} B exceeds the formula's charge {charged} B"
        );
    }
}

#[test]
fn readiness_honors_a_spent_budget_before_any_effect() {
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };
    let cancelled = AtomicBool::new(true);
    let never = AtomicBool::new(false);
    let spent = [
        tmt_remote::objects::IoBudget {
            deadline: Instant::now() + Duration::from_secs(60),
            cancelled: &cancelled,
        },
        tmt_remote::objects::IoBudget {
            deadline: Instant::now(),
            cancelled: &never,
        },
    ];
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    for budget in &spent {
        // A fresh root: nothing is created.
        let root = Root::new();
        let serving = root.serving();
        assert!(LocalFs::open(&serving, Quotas::contract(), clock(&now), budget).is_err());
        assert!(!root.ledger().exists(), "a spent budget creates no ledger");
        // An existing root: nothing changes, and reconcile refuses too.
        drop(open(&serving, Quotas::contract(), &now).unwrap());
        let before = snapshot(&root.0);
        assert!(LocalFs::open(&serving, Quotas::contract(), clock(&now), budget).is_err());
        let local = open(&serving, Quotas::contract(), &now).unwrap();
        assert!(matches!(
            local.reconcile(budget),
            Err(BackendError::Cancelled | BackendError::Deadline)
        ));
        assert_eq!(snapshot(&root.0), before);
    }
}

const MIB: usize = 1024 * 1024;

/// Out-of-band fixture corruption, not something `LocalFs` creates: commit an
/// original normally, rewind it to an adopted commit with no receipt, let `shape`
/// arrange the destination and staging names, then reopen so readiness reconciles
/// it. `check` sees the reopened backend and the ledger.
fn damaged_commit(
    payload: &[u8],
    quotas: Quotas,
    shape: impl FnOnce(&Path, &Path),
    check: impl FnOnce(&Root, &LocalFs<'_>, &Serving),
) {
    let root = Root::new();
    let serving = root.serving();
    let now = Arc::new(AtomicU64::new(1_000_000_000));
    let sp = spec(1, 1, 1, payload);
    {
        let local = open(&serving, quotas, &now).unwrap();
        upload(&handle(&local, "alpha"), &sp, payload);
    }
    Connection::open(root.ledger())
        .unwrap()
        .execute("UPDATE intents SET phase='committing',staged=1", [])
        .unwrap();
    fs::create_dir_all(root.tree("alpha").join("staging")).unwrap();
    shape(
        &final_path(&root, "alpha", 1, 1),
        &staging_path(&root, "alpha", 1),
    );
    let local = open(&serving, quotas, &now).unwrap();
    check(&root, &local, &serving);
}
fn write_private(path: &Path, bytes: &[u8]) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new();
    file.write(true).create_new(true).mode(0o600);
    file.open(path).unwrap().write_all(bytes).unwrap();
}
fn phase_and_charge(root: &Root) -> (String, i64) {
    Connection::open(root.ledger())
        .unwrap()
        .query_row("SELECT phase,payload_charged FROM intents", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap()
}

/// What the retained bodies occupy, from fixture metadata alone (never the production
/// measurement): each distinct inode once, at the larger of its length and its allocated
/// blocks, each rounded up to 4 KiB. A filesystem may allocate beyond the length (for
/// example speculative preallocation), so an expectation built from the logical lengths
/// would hold on one filesystem only. The text names every inode for a failing run.
fn retained_bodies(paths: &[PathBuf]) -> (i64, String) {
    let round = |bytes: u64| bytes.div_ceil(4096) * 4096;
    let (mut seen, mut total, mut notes) = (Vec::new(), 0u64, String::new());
    for path in paths {
        let metadata = fs::metadata(path).unwrap();
        if seen.contains(&(metadata.dev(), metadata.ino())) {
            continue;
        }
        seen.push((metadata.dev(), metadata.ino()));
        let allocated = round(metadata.blocks() * 512).max(round(metadata.len()));
        total += allocated;
        notes += &format!(
            " [{}: len {} blocks {} -> {allocated}]",
            path.display(),
            metadata.len(),
            metadata.blocks()
        );
    }
    (total as i64, notes)
}
/// The `unknown` charge: the reservation, or the retained bodies when they exceed it.
fn unknown_charge(reservation: u64, paths: &[PathBuf]) -> (i64, String) {
    let (retained, notes) = retained_bodies(paths);
    ((reservation as i64).max(retained), notes)
}

#[test]
fn an_unknown_original_is_charged_for_every_distinct_body_it_retains() {
    use tmt_remote::{limits, objects::Limit};
    let payload = bytes(MIB, 8);
    let mut wrong = payload.clone();
    wrong[17] ^= 0xff;
    let empty = empty_ledger_bytes();
    let reservation = conformance::round(MIB);
    // A differing destination of the same length beside the staged body: two inodes.
    // The extension limit holds exactly the original reservation plus one tiny adoption.
    let one_object = reservation + conformance::RECORD + conformance::FENCE + conformance::TREE;
    let tight = Quotas {
        extension_bytes: one_object
            + conformance::RECORD
            + conformance::round(1)
            + conformance::FENCE,
        ..Quotas::contract()
    };
    damaged_commit(
        &payload,
        tight,
        |dest, staging| {
            fs::write(dest, &wrong).unwrap();
            write_private(staging, &payload);
        },
        |root, local, _| {
            let alpha = handle(local, "alpha");
            let bodies = [
                final_path(root, "alpha", 1, 1),
                staging_path(root, "alpha", 1),
            ];
            let (expected, notes) = unknown_charge(reservation, &bodies);
            assert_eq!(
                phase_and_charge(root),
                ("unknown".into(), expected),
                "two distinct bodies:{notes}"
            );
            assert!(
                expected >= 2 * reservation as i64,
                "both bodies are charged"
            );
            assert_eq!(
                alpha
                    .status(spec(1, 1, 1, &payload).intent, &io())
                    .unwrap()
                    .state,
                TransferState::Unknown
            );
            assert_eq!(
                fs::read(final_path(root, "alpha", 1, 1)).unwrap(),
                wrong,
                "never overwritten"
            );
            assert_eq!(
                fs::read(staging_path(root, "alpha", 1)).unwrap(),
                payload,
                "never unlinked"
            );
            let (physical, charged) = physical_vs_charge(root, local, empty);
            assert!(
                physical <= charged,
                "two distinct bodies: {physical} B vs charge {charged} B"
            );
            // The measured overage refuses adoption in the limit it exceeds...
            assert_eq!(
                alpha.begin(&spec(9, 9, 9, b"x"), &io()),
                Err(BackendError::Capacity(Limit::ExtensionBytes))
            );
            // ...and not in another extension, which is not full.
            assert!(
                handle(local, "beta")
                    .begin(&spec(9, 9, 9, b"x"), &io())
                    .is_ok()
            );
        },
    );
    // A destination larger than the intent is charged at its real size.
    damaged_commit(
        &payload,
        Quotas::contract(),
        |dest, staging| {
            fs::write(dest, vec![7; 3 * MIB]).unwrap();
            write_private(staging, &payload);
        },
        |root, local, _| {
            let bodies = [
                final_path(root, "alpha", 1, 1),
                staging_path(root, "alpha", 1),
            ];
            let (expected, notes) = unknown_charge(reservation, &bodies);
            assert_eq!(
                phase_and_charge(root),
                ("unknown".into(), expected),
                "the larger destination is charged at its real size:{notes}"
            );
            assert!(expected >= (reservation + conformance::round(3 * MIB)) as i64);
            let (physical, charged) = physical_vs_charge(root, local, empty);
            assert!(physical <= charged, "{physical} B vs {charged} B");
        },
    );
    // A sparse destination allocates less than its length: it is still charged by its
    // length, on every filesystem (the larger of length and allocation).
    damaged_commit(
        &payload,
        Quotas::contract(),
        |dest, staging| {
            use std::os::unix::fs::OpenOptionsExt;
            let hole = dest.with_extension("sparse");
            let sparse = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&hole)
                .unwrap();
            sparse.set_len(3 * MIB as u64).unwrap();
            fs::rename(&hole, dest).unwrap();
            write_private(staging, &payload);
        },
        |root, _, _| {
            let bodies = [
                final_path(root, "alpha", 1, 1),
                staging_path(root, "alpha", 1),
            ];
            let (expected, notes) = unknown_charge(reservation, &bodies);
            assert_eq!(
                phase_and_charge(root),
                ("unknown".into(), expected),
                "sparse destination:{notes}"
            );
            assert!(
                expected >= (reservation + conformance::round(3 * MIB)) as i64,
                "the length is the floor of a sparse body's charge"
            );
        },
    );
    // Identical bytes in a different inode are not one allocation: content equality
    // cannot establish a shared name.
    damaged_commit(
        &payload,
        Quotas::contract(),
        |_, staging| write_private(staging, &payload),
        |root, local, _| {
            let bodies = [
                final_path(root, "alpha", 1, 1),
                staging_path(root, "alpha", 1),
            ];
            let (expected, notes) = unknown_charge(reservation, &bodies);
            assert_eq!(
                phase_and_charge(root),
                ("unknown".into(), expected),
                "identical bytes, two inodes:{notes}"
            );
            assert!(
                expected >= 2 * reservation as i64,
                "both bodies are charged"
            );
            let (physical, charged) = physical_vs_charge(root, local, empty);
            assert!(physical <= charged, "{physical} B vs {charged} B");
        },
    );
    // A destination without its staging name is not proof of an ordinary recovery.
    damaged_commit(
        &payload,
        Quotas::contract(),
        |_, _| {},
        |root, local, _| {
            let (expected, notes) = unknown_charge(reservation, &[final_path(root, "alpha", 1, 1)]);
            assert_eq!(
                phase_and_charge(root),
                ("unknown".into(), expected),
                "one body:{notes}"
            );
            assert_eq!(fs::read(final_path(root, "alpha", 1, 1)).unwrap(), payload);
            let (physical, charged) = physical_vs_charge(root, local, empty);
            assert!(physical <= charged);
            assert_eq!(
                handle(local, "alpha").commit(spec(1, 1, 1, &payload).intent, &io()),
                Err(BackendError::Unavailable)
            );
        },
    );
    // The legitimate crash window: the staging name and the destination are one inode.
    damaged_commit(
        &payload,
        Quotas::contract(),
        |dest, staging| fs::hard_link(dest, staging).unwrap(),
        |root, local, _| {
            assert_eq!(
                phase_and_charge(root),
                ("committed".into(), reservation as i64)
            );
            assert!(!staging_path(root, "alpha", 1).exists());
            assert_eq!(
                fs::metadata(final_path(root, "alpha", 1, 1))
                    .unwrap()
                    .nlink(),
                1
            );
            let (physical, charged) = physical_vs_charge(root, local, empty);
            assert!(physical <= charged);
        },
    );
    // The same with the destination absent: an ordinary create-only completion.
    damaged_commit(
        &payload,
        Quotas::contract(),
        |dest, staging| {
            fs::rename(dest, staging).unwrap();
        },
        |root, _, _| {
            assert_eq!(
                phase_and_charge(root),
                ("committed".into(), reservation as i64)
            );
        },
    );
    let _ = limits::OBJECT_CHUNK_BYTES;
}
