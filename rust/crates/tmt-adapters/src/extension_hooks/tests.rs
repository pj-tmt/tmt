use super::*;
use crate::test_support::TestDirectory;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;

const ID: &str = "11111111-1111-4111-8111-111111111111";
const ROOM: &str = "22222222-2222-4222-8222-222222222222";

fn write_consents(path: &Path, extensions: &[Consent]) -> Result<(), ExtensionHookError> {
    crate::private_file::replace(path, &encode_consents(extensions)?)
        .map_err(ExtensionHookError::Io)
}

thread_local! {
    static AFTER_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = Default::default();
}

pub(super) fn after_consent_read() {
    let checkpoint = AFTER_READ.with(|slot| slot.borrow_mut().take());
    if let Some(checkpoint) = checkpoint {
        checkpoint();
    }
}

/// Own the thread and release its checkpoint before joining, including on panic.
struct PausedMutation {
    read: std::sync::mpsc::Receiver<()>,
    release: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<Result<(), ExtensionHookError>>>,
}

impl PausedMutation {
    fn start(operation: impl FnOnce() -> Result<(), ExtensionHookError> + Send + 'static) -> Self {
        let (read_tx, read) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            AFTER_READ.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    read_tx.send(()).unwrap();
                    // Dropping the controller also releases a failed scenario.
                    let _ = resume.recv();
                }));
            });
            operation()
        });
        Self {
            read,
            release: Some(release),
            worker: Some(worker),
        }
    }

    fn wait_for_read(&self) {
        self.read.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    fn finish(mut self) {
        self.release.take().unwrap().send(()).unwrap();
        self.worker.take().unwrap().join().unwrap().unwrap();
    }
}

impl Drop for PausedMutation {
    fn drop(&mut self) {
        self.release.take();
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap().unwrap();
            }
        }
    }
}

fn fixture_consent(name: &str) -> Consent {
    Consent {
        name: name.into(),
        path: PathBuf::from("/fixture-not-executed"),
        digest: "0".repeat(64),
        fingerprint: Fingerprint {
            device: 1,
            inode: 2,
            size: 3,
            modified_ns: 4,
            changed_ns: 5,
            uid: 6,
            mode: 0o755,
        },
        protocol: "1".into(),
        capabilities: vec![LIFECYCLE_CAPABILITY.into()],
        consented_at_ms: 7,
    }
}

fn fixture_paths(directory: &TestDirectory) -> ConfigPaths {
    ConfigPaths::resolve(
        &directory.path,
        &directory.path,
        Some(&directory.path),
        None,
    )
}

#[test]
fn concurrent_disables_keep_both_successful_removals() {
    let directory = TestDirectory::new();
    let paths = fixture_paths(&directory);
    write_consents(
        &consent_path(&paths.global_dir),
        &[fixture_consent("a"), fixture_consent("b")],
    )
    .unwrap();
    let first_paths = paths.clone();
    let first = PausedMutation::start(move || {
        assert!(disable(&first_paths, "a")?);
        Ok(())
    });
    first.wait_for_read();
    let second_paths = paths.clone();
    let second = PausedMutation::start(move || {
        assert!(disable(&second_paths, "b")?);
        Ok(())
    });
    second.wait_for_read();
    // Both operations have read [a,b]. Commit a's removal, then b's.
    first.finish();
    second.finish();
    assert!(list_consents(&paths).unwrap().is_empty());
}

#[test]
fn concurrent_enable_and_disable_keep_both_successful_changes() {
    let directory = TestDirectory::new();
    let paths = fixture_paths(&directory);
    write_consents(
        &consent_path(&paths.global_dir),
        &[fixture_consent("a"), fixture_consent("b")],
    )
    .unwrap();
    let bin = directory.path.join("bin");
    fs::create_dir(&bin).unwrap();
    let executable = bin.join("tmt-c");
    tmt_test_support::write_executable(
        &executable,
        b"#!/bin/sh\nprintf 'TMT-HOOKS/1\\nlifecycle_observations_v1\\n'\n",
        0o700,
    )
    .unwrap();
    let first_paths = paths.clone();
    let first = PausedMutation::start(move || {
        enable(&first_paths, "c", bin.as_os_str(), &executable)?;
        Ok(())
    });
    first.wait_for_read();
    let second_paths = paths.clone();
    let second = PausedMutation::start(move || {
        assert!(disable(&second_paths, "a")?);
        Ok(())
    });
    second.wait_for_read();
    first.finish();
    second.finish();
    assert_eq!(
        list_consents(&paths)
            .unwrap()
            .iter()
            .map(|consent| consent.name.as_str())
            .collect::<Vec<_>>(),
        ["b", "c"]
    );
}

#[test]
fn same_name_mutations_use_current_bytes_and_noops_do_not_publish() {
    let directory = TestDirectory::new();
    let paths = fixture_paths(&directory);
    let path = consent_path(&paths.global_dir);
    write_consents(&path, &[fixture_consent("a")]).unwrap();
    let pending_paths = paths.clone();
    let pending = PausedMutation::start(move || {
        assert!(!disable(&pending_paths, "a")?);
        Ok(())
    });
    pending.wait_for_read();
    assert!(disable(&paths, "a").unwrap());
    let inode = fs::metadata(&path).unwrap().ino();
    pending.finish();
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    let mut renewed = fixture_consent("a");
    renewed.consented_at_ms = 99;
    apply_consent(&paths, ConsentChange::Enable(&renewed)).unwrap();
    assert_eq!(list_consents(&paths).unwrap(), [renewed]);
    assert!(disable(&paths, "a").unwrap());
}

#[test]
fn missing_disable_and_failed_probe_create_no_settings_or_lock() {
    let directory = TestDirectory::new();
    let missing = directory.path.join("absent");
    let paths = ConfigPaths::resolve(&missing, &missing, Some(&missing), None);
    assert!(!disable(&paths, "a").unwrap());
    assert!(!missing.exists());
    let executable = directory.path.join("tmt-bad");
    tmt_test_support::write_executable(&executable, b"#!/bin/sh\nprintf 'invalid\\n'\n", 0o700)
        .unwrap();
    let error = enable(&paths, "bad", directory.path.as_os_str(), &executable).unwrap_err();
    assert_eq!(error.code(), "EXTENSION_INCOMPATIBLE");
    assert!(!missing.exists());
}

#[test]
fn new_consent_is_private_and_oversized_publication_preserves_previous_bytes() {
    let directory = TestDirectory::new();
    let paths = fixture_paths(&directory);
    let path = consent_path(&paths.global_dir);
    apply_consent(&paths, ConsentChange::Enable(&fixture_consent("a"))).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    let original = fs::read(&path).unwrap();
    let mut oversized = fixture_consent("large");
    oversized.capabilities.push("x".repeat(CONSENT_LIMIT));
    assert_eq!(
        apply_consent(&paths, ConsentChange::Enable(&oversized))
            .unwrap_err()
            .code(),
        "EXTENSION_HOOKS_UNAVAILABLE"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn mutation_requires_private_mode_without_changing_read_only_consent() {
    let directory = TestDirectory::new();
    let paths = fixture_paths(&directory);
    let path = consent_path(&paths.global_dir);
    write_consents(&path, &[fixture_consent("a'b")]).unwrap();
    let original = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(list_consents(&paths).unwrap().len(), 1);
    let error = disable(&paths, "a'b").unwrap_err();
    assert_eq!(error.code(), "EXTENSION_HOOKS_UNAVAILABLE");
    let message = error.to_string();
    assert!(message.contains(&path.display().to_string()));
    assert!(message.contains(&format!("chmod 600 {}", quoted(&path.to_string_lossy()))));
    assert!(message.contains("Retry: tmt extension hooks disable 'a'\\''b'"));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o644);
    assert!(!directory.path.join("document-locks").exists());
}

#[test]
fn invalid_and_unsafe_settings_are_never_replaced() {
    let directory = TestDirectory::new();
    let paths = fixture_paths(&directory);
    let path = consent_path(&paths.global_dir);
    for (bytes, code) in [
        (b"{".to_vec(), "EXTENSION_HOOKS_INVALID"),
        (
            br#"{"version":2,"extensions":[]}"#.to_vec(),
            "EXTENSION_HOOKS_INVALID",
        ),
        (vec![b' '; CONSENT_LIMIT + 1], "EXTENSION_HOOKS_UNAVAILABLE"),
    ] {
        crate::private_file::replace(&path, &bytes).unwrap();
        assert_eq!(disable(&paths, "a").unwrap_err().code(), code);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::remove_file(&path).unwrap();
    let target = directory.path.join("untouched");
    fs::write(&target, b"sentinel").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(disable(&paths, "a").is_err());
    assert_eq!(fs::read(&target).unwrap(), b"sentinel");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(disable(&paths, "a").is_err());
    assert!(path.is_dir());
}

fn consent_lock_path(paths: &ConfigPaths) -> PathBuf {
    let entries = fs::read_dir(paths.global_dir.join("document-locks"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    entries[0].clone()
}

#[test]
fn shared_lock_times_out_without_publication_and_is_reused_after_release() {
    use nix::fcntl::{Flock, FlockArg};
    let directory = TestDirectory::new();
    let paths = fixture_paths(&directory);
    let path = consent_path(&paths.global_dir);
    write_consents(&path, &[fixture_consent("a")]).unwrap();
    // An existing no-op admits the shared lock without replacing the document.
    assert!(!disable(&paths, "absent").unwrap());
    let lock = consent_lock_path(&paths);
    let inode = fs::metadata(&lock).unwrap().ino();
    let held = Flock::lock(fs::File::open(&lock).unwrap(), FlockArg::LockExclusive).unwrap();
    let original = fs::read(&path).unwrap();
    let started = Instant::now();
    let error = disable(&paths, "a").unwrap_err();
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(error.code(), "EXTENSION_HOOKS_UNAVAILABLE");
    let ExtensionHookError::Io(error) = error else {
        unreachable!()
    };
    assert!(
        matches!(error.get_ref().unwrap().downcast_ref::<tmt_extension_state::StateError>(),
        Some(tmt_extension_state::StateError::Timeout { may_not_be_saved: false, retry, .. })
        if retry == "tmt extension hooks disable 'a'")
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    drop(held);
    assert!(disable(&paths, "a").unwrap());
    assert_eq!(fs::metadata(&lock).unwrap().ino(), inode);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert!(
        !path
            .with_file_name(".extension-hooks.json.tmt-staging")
            .exists()
    );
}

#[test]
fn unsafe_lock_and_staging_refuse_without_mutation() {
    let directory = TestDirectory::new();
    let paths = fixture_paths(&directory);
    let path = consent_path(&paths.global_dir);
    write_consents(&path, &[fixture_consent("a")]).unwrap();
    assert!(!disable(&paths, "absent").unwrap());
    let lock = consent_lock_path(&paths);
    fs::remove_file(&lock).unwrap();
    let target = directory.path.join("untouched");
    fs::write(&target, b"sentinel").unwrap();
    std::os::unix::fs::symlink(&target, &lock).unwrap();
    assert!(disable(&paths, "a").is_err());
    fs::remove_file(&lock).unwrap();
    let staging = path.with_file_name(".extension-hooks.json.tmt-staging");
    std::os::unix::fs::symlink(&target, &staging).unwrap();
    assert!(disable(&paths, "a").is_err());
    assert_eq!(list_consents(&paths).unwrap(), [fixture_consent("a")]);
    assert_eq!(fs::read(&target).unwrap(), b"sentinel");
    fs::remove_file(&staging).unwrap();
    fs::write(&staging, b"interrupted stage").unwrap();
    assert!(disable(&paths, "a").unwrap());
    assert!(!staging.exists());
}

fn capturing(path: &Path) -> Connection {
    crate::storage::Storage::open(path)
        .unwrap()
        .close()
        .unwrap();
    let connection = Connection::open(path).unwrap();
    connection.execute_batch(CAPTURE_SQL).unwrap();
    connection
}

#[test]
fn capture_records_only_committed_typed_evidence() {
    let directory = TestDirectory::new();
    let connection = capturing(&directory.path.join("state.db"));
    connection
        .execute_batch(&format!(
            "INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES ('{ID}', 'Secret Name', 'secret name', 't', 't', 'saved');
             UPDATE identities SET name = 'Secret Other', canonical_name = 'secret other' WHERE id = '{ID}';
             UPDATE identities SET name = 'Secret Other' WHERE id = '{ID}';
             UPDATE identities SET retired_at_ms = 5 WHERE id = '{ID}';
             UPDATE identities SET retired_at_ms = 6 WHERE id = '{ID}';
             INSERT INTO office_meeting_rooms (room_id, name, revision) VALUES ('{ROOM}', 'Private room', 1);
             UPDATE office_meeting_rooms SET name = 'Renamed', revision = 2 WHERE room_id = '{ROOM}';
             UPDATE office_meeting_rooms SET retired = 1, revision = 3 WHERE room_id = '{ROOM}';"
        ))
        .unwrap();
    // A rolled-back change leaves no evidence.
    connection
        .execute_batch(
            "BEGIN; INSERT INTO identities (id, name, canonical_name, created_at, updated_at, lifetime) VALUES ('33333333-3333-4333-8333-333333333333', 'Gone', 'gone', 't', 't', 'saved'); ROLLBACK;",
        )
        .unwrap();
    let events = collect(&connection).unwrap();
    assert_eq!(
        events,
        vec![
            serde_json::json!({"kind": "identity.created", "identityId": ID, "lifetime": "saved", "retired": false}),
            serde_json::json!({"kind": "identity.renamed", "identityId": ID, "lifetime": "saved", "retired": false}),
            serde_json::json!({"kind": "identity.retired", "identityId": ID, "lifetime": "saved", "retired": true}),
            serde_json::json!({"kind": "room.created", "roomId": ROOM, "revision": 1, "retired": false}),
            serde_json::json!({"kind": "room.updated", "roomId": ROOM, "revision": 2, "retired": false}),
            serde_json::json!({"kind": "room.retired", "roomId": ROOM, "revision": 3, "retired": true}),
        ]
    );
    // Names and other content never appear in evidence.
    assert!(!serde_json::to_string(&events).unwrap().contains("Secret"));
    assert!(!serde_json::to_string(&events).unwrap().contains("Private"));
    assert_eq!(
        collect(&connection).unwrap(),
        Vec::<serde_json::Value>::new()
    );
}

#[test]
fn capabilities_are_a_versioned_token_set() {
    assert_eq!(
        decode_capabilities(b"TMT-HOOKS/1\nlifecycle_observations_v1\nfuture_v9\n"),
        Some(vec![
            "future_v9".to_owned(),
            "lifecycle_observations_v1".to_owned()
        ])
    );
    for bytes in [
        &b"TMT-HOOKS/2\nlifecycle_observations_v1\n"[..],
        b"TMT-HOOKS/1\n",
        b"TMT-HOOKS/1\nlifecycle_observations_v1",
        b"TMT-HOOKS/1\nlifecycle_observations_v1\nlifecycle_observations_v1\n",
        b"TMT-HOOKS/1\nLifecycle\n",
        b"\xff",
    ] {
        assert_eq!(decode_capabilities(bytes), None, "{bytes:?}");
    }
}

#[test]
fn only_a_privately_owned_executable_is_trusted_and_changes_revoke_trust() {
    let directory = TestDirectory::new();
    let bin = directory.path.join("bin");
    fs::create_dir(&bin).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    let executable = bin.join("tmt-fixture");
    fs::write(&executable, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let metadata = verify_ownership(&executable).unwrap();
    let consent = Consent {
        name: "fixture".into(),
        path: executable.clone(),
        digest: "0".repeat(64),
        fingerprint: Fingerprint::of(&metadata),
        protocol: PROTOCOL_VERSION.into(),
        capabilities: vec![LIFECYCLE_CAPABILITY.into()],
        consented_at_ms: 1,
    };
    assert_eq!(
        verified(vec![consent.clone()], LIFECYCLE_CAPABILITY).len(),
        1
    );
    assert!(verified(vec![consent.clone()], CONTEXT_CAPABILITY).is_empty());

    fs::set_permissions(&executable, fs::Permissions::from_mode(0o775)).unwrap();
    assert!(matches!(
        verify_ownership(&executable),
        Err(ExtensionHookError::Unsafe(_))
    ));
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        verify_ownership(&executable),
        Err(ExtensionHookError::Unsafe(_))
    ));
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        verify_ownership(&executable),
        Err(ExtensionHookError::Unsafe(_))
    ));
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    // Rewriting the file changes its fingerprint, so the old consent lapses.
    fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(verified(vec![consent], LIFECYCLE_CAPABILITY).is_empty());
}

#[test]
fn consents_are_stored_privately_and_disable_removes_one() {
    let directory = TestDirectory::new();
    let path = consent_path(&directory.path);
    assert!(read_consents(&path).unwrap().is_empty());
    assert!(!has_context_consent(&directory.path));
    let consent = fixture_consent;
    write_consents(&path, &[consent("a"), consent("b")]).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(read_consents(&path).unwrap().len(), 2);
    let paths = ConfigPaths::resolve(
        &directory.path,
        &directory.path,
        Some(&directory.path),
        None,
    );
    assert!(disable(&paths, "a").unwrap());
    assert!(!disable(&paths, "a").unwrap());
    assert_eq!(list_consents(&paths).unwrap(), vec![consent("b")]);
    assert!(!has_context_consent(&directory.path));
    let mut context = consent("context");
    context.capabilities = vec![CONTEXT_CAPABILITY.into()];
    write_consents(&path, &[context]).unwrap();
    assert!(has_context_consent(&directory.path));
    fs::write(&path, b"{\"version\":2,\"extensions\":[]}").unwrap();
    assert!(!has_context_consent(&directory.path));
    assert!(matches!(
        read_consents(&path),
        Err(ExtensionHookError::Invalid(_))
    ));
}

#[test]
fn a_summary_is_one_bounded_string_or_nothing() {
    assert_eq!(
        decode_summary(br#"{"summary":"Office: desk"}"#),
        Some("Office: desk".into())
    );
    let longest = "\u{2603}".repeat(SUMMARY_CHARACTER_LIMIT);
    assert_eq!(
        decode_summary(
            serde_json::json!({"summary": longest})
                .to_string()
                .as_bytes()
        ),
        Some(longest)
    );
    for bytes in [
        br#"{"summary":null}"#.to_vec(),
        br#"{"summary":"  "}"#.to_vec(),
        br#"{"summary":"x","extra":1}"#.to_vec(),
        br#"{"summary":7}"#.to_vec(),
        b"not json".to_vec(),
        serde_json::json!({"summary": "x".repeat(SUMMARY_CHARACTER_LIMIT + 1)})
            .to_string()
            .into_bytes(),
    ] {
        assert_eq!(
            decode_summary(&bytes),
            None,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
    }
}

#[test]
fn no_context_consent_means_no_contribution() {
    let directory = TestDirectory::new();
    assert!(
        context_contributions(&directory.path, ID, Instant::now() + CONTEXT_DEADLINE).is_empty()
    );
    assert!(!consent_path(&directory.path).exists());
}
