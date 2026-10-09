use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab::{
    keyring::{Keyring, Layout},
    store::{Accepted, Envelope, Fault, Namespace, Store, StreamScope},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-847-state-{}-{}",
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
fn scope() -> StreamScope<'static> {
    StreamScope {
        page: "page",
        epoch: 1,
        stream: "device",
    }
}
fn envelope(seq: u64, namespace: Namespace) -> Envelope<'static> {
    Envelope {
        scope: scope(),
        seq,
        namespace,
        hash: [seq as u8; 32],
        previous: [(seq - 1) as u8; 32],
        bytes: b"opaque-ciphertext",
    }
}
#[test]
fn keyring_is_stable_private_and_no_follow_with_owned_service_lock() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let key = Keyring::open(&layout).unwrap();
    assert_eq!(key.space_id, Keyring::open(&layout).unwrap().space_id);
    assert_eq!(key.space_id.len(), 32);
    assert_eq!(
        fs::metadata(&layout.directory).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(layout.directory.join("owner.key"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!layout.running().unwrap());
    let lock = layout.serve_lock().unwrap();
    assert!(layout.running().unwrap());
    assert!(layout.serve_lock().is_err());
    drop(lock);
    assert!(!layout.running().unwrap());
    fs::set_permissions(
        layout.directory.join("owner.key"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(Keyring::open(&layout).is_err());
    fs::remove_file(layout.directory.join("owner.key")).unwrap();
    symlink(
        fixture.0.join("outside"),
        layout.directory.join("owner.key"),
    )
    .unwrap();
    assert!(Keyring::open(&layout).is_err());
    assert!(!fixture.0.join("outside").exists());
}
#[test]
fn checkpoint_waits_for_pair_then_prunes_both_prefixes_preserving_tail_and_fork_receipts() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let mut store = Store::open(&layout).unwrap();
    store.create_page("page").unwrap();
    store.append(&envelope(1, Namespace::Content)).unwrap();
    assert!(matches!(
        store.append(&envelope(3, Namespace::Content)),
        Err(Fault::Gap)
    ));
    store.append(&envelope(2, Namespace::Own)).unwrap();
    store.append(&envelope(3, Namespace::Content)).unwrap();
    let checkpoint = Envelope {
        hash: [8; 32],
        previous: [2; 32],
        bytes: b"namespace-checkpoint",
        ..envelope(2, Namespace::Content)
    };
    assert_eq!(store.checkpoint(&checkpoint).unwrap(), Accepted::New);
    assert_eq!(store.checkpoint(&checkpoint).unwrap(), Accepted::Replay);
    // An unpaired checkpoint neither removes payloads nor becomes bootstrap.
    assert_eq!(
        store.payload(scope(), 1).unwrap(),
        Some(b"opaque-ciphertext".to_vec())
    );
    let first = store
        .namespace_next(scope(), Namespace::Content, Default::default())
        .unwrap()
        .unwrap();
    assert!(!first.checkpoint);
    assert_eq!(first.cursor.seq, 1);
    store.close().unwrap();
    let mut store = Store::open(&layout).unwrap();
    assert_eq!(
        store.payload(scope(), 2).unwrap(),
        Some(b"opaque-ciphertext".to_vec())
    );
    let own_checkpoint = Envelope {
        namespace: Namespace::Own,
        hash: [9; 32],
        ..checkpoint
    };
    store.checkpoint(&own_checkpoint).unwrap();
    assert_eq!(store.payload(scope(), 1).unwrap(), None);
    assert_eq!(store.payload(scope(), 2).unwrap(), None);
    assert_eq!(
        store.payload(scope(), 3).unwrap(),
        Some(b"opaque-ciphertext".to_vec())
    );
    store.close().unwrap();
    let mut store = Store::open(&layout).unwrap();
    assert_eq!(
        store.append(&envelope(1, Namespace::Content)).unwrap(),
        Accepted::Replay
    );
    let fork = Envelope {
        hash: [9; 32],
        bytes: b"fork",
        ..envelope(1, Namespace::Content)
    };
    assert!(matches!(store.append(&fork), Err(Fault::Conflict)));
    store.close().unwrap();
    let mut store = Store::open(&layout).unwrap();
    assert!(matches!(
        store.append(&envelope(4, Namespace::Content)),
        Err(Fault::Conflict)
    ));
    assert_eq!(
        store.payload(scope(), 3).unwrap(),
        Some(b"opaque-ciphertext".to_vec())
    );
    store.advance_epoch("page", 1).unwrap();
    assert!(matches!(
        store.append(&envelope(1, Namespace::Content)),
        Err(Fault::StaleEpoch)
    ));
    assert!(matches!(store.payload(scope(), 3), Err(Fault::StaleEpoch)));
}
#[test]
fn failed_checkpoint_rolls_back_prune_and_receipt_capacity_never_evicts() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let mut store = Store::open(&layout).unwrap();
    store.create_page("page").unwrap();
    store.append(&envelope(1, Namespace::Content)).unwrap();
    let oracle = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    oracle.execute_batch("CREATE TRIGGER fail_checkpoint BEFORE INSERT ON checkpoints BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    let checkpoint = Envelope {
        hash: [8; 32],
        previous: [1; 32],
        bytes: b"checkpoint",
        ..envelope(1, Namespace::Content)
    };
    assert!(matches!(store.checkpoint(&checkpoint), Err(Fault::Sql(_))));
    assert_eq!(
        store.payload(scope(), 1).unwrap(),
        Some(b"opaque-ciphertext".to_vec())
    );
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM checkpoints", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    oracle.execute_batch("DROP TRIGGER fail_checkpoint;
        WITH RECURSIVE counts(n) AS (SELECT 2 UNION ALL SELECT n+1 FROM counts WHERE n<100000)
        INSERT INTO receipts SELECT 'page','1','device',printf('%020d',n),'own',zeroblob(32),zeroblob(32),NULL FROM counts;").unwrap();
    let full = Envelope {
        seq: 100001,
        previous: [0; 32],
        ..envelope(2, Namespace::Own)
    };
    assert!(matches!(store.append(&full), Err(Fault::Capacity)));
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        100000
    );
    assert_eq!(
        store.append(&envelope(1, Namespace::Content)).unwrap(),
        Accepted::Replay
    );
    let too_big = vec![0; tmt_colab::limits::OBJECT_BYTES + 1];
    assert!(matches!(
        store.append(&Envelope {
            bytes: &too_big,
            ..full
        }),
        Err(Fault::Invalid)
    ));
}

#[test]
fn owner_identity_matches_independent_rfc8032_and_python_vector() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    // RFC 8032 test 1 seed; expected ID is independent Python hashlib/base32 LP.
    let seed = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    use std::io::Write;
    layout.file("owner.key").unwrap().write_all(&seed).unwrap();
    assert_eq!(
        Keyring::read(&layout).unwrap().space_id,
        "uqvpga22vglwpngpg7jd7zpk5vzjtoud"
    );
    // #829 encoding-vectors.json ownerSeed/ownerPublic/spaceId, also the
    // application-vectors.json header space. These are public fixture keys.
    fs::remove_file(layout.directory.join("owner.key")).unwrap();
    layout
        .file("owner.key")
        .unwrap()
        .write_all(&(0..32).collect::<Vec<u8>>())
        .unwrap();
    let owner = Keyring::read(&layout).unwrap();
    assert_eq!(
        owner
            .owner_public()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8"
    );
    assert_eq!(owner.space_id, "4kph3kmtxo7dinlvoixpw642ozfibd2w");
    assert_eq!(
        owner.space_id,
        tmt_colab_model::crypto::space_id(&owner.owner_public()).unwrap()
    );
}

#[test]
fn listing_layout_lookup_is_read_only_and_missing_root_is_created_only_on_open() {
    let fixture = Fixture::new();
    let absent = fixture.0.join("missing");
    assert!(Layout::existing(&absent).unwrap().is_none());
    assert!(!absent.exists());
    assert!(Layout::existing(std::path::Path::new("relative")).is_err());
    let layout = Layout::open(&absent).unwrap();
    assert_eq!(fs::metadata(&absent).unwrap().mode() & 0o777, 0o700);
    assert!(!layout.running().unwrap());
    assert_eq!(fs::read_dir(&layout.directory).unwrap().count(), 0);
    assert!(Keyring::read(&layout).is_err());
    assert_eq!(fs::read_dir(&layout.directory).unwrap().count(), 0);
}

#[test]
fn shared_layout_preserves_root_permissions_sibling_bytes_and_state_faults() {
    use tmt_colab::keyring::StateFault;
    let fixture = Fixture::new();
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o751)).unwrap();
    let sibling = fixture.0.join("remote");
    fs::create_dir(&sibling).unwrap();
    let machine = sibling.join("machine.key");
    fs::write(&machine, [2; 32]).unwrap();
    let layout = fixture.layout();
    Keyring::open(&layout).unwrap();
    assert_eq!(fs::metadata(&fixture.0).unwrap().mode() & 0o777, 0o751);
    assert_eq!(fs::read(machine).unwrap(), [2; 32]);
    for name in ["../machine.key", "machine.key", "owner.key/"] {
        let error = layout.file(name).err().unwrap();
        assert_eq!(
            error.downcast_ref::<StateFault>(),
            Some(&StateFault::InvalidFileName)
        );
        assert_eq!(error.to_string(), "Invalid private file name.");
    }
    let held = layout.serve_lock().unwrap();
    assert_eq!(
        layout
            .serve_lock()
            .err()
            .unwrap()
            .downcast_ref::<StateFault>(),
        Some(&StateFault::AlreadyServing)
    );
    drop(held);
    assert!(!layout.running().unwrap());
}

#[test]
fn bounded_owner_keys_fail_with_the_same_downcastable_fault_and_keep_exact_bytes() {
    use tmt_colab::keyring::StateFault;
    let fixture = Fixture::new();
    let layout = fixture.layout();
    Keyring::open(&layout).unwrap();
    let path = layout.directory.join("owner.key");
    for length in [0, 31, 33, 4096] {
        let original = vec![7; length];
        fs::write(&path, &original).unwrap();
        for error in [
            Keyring::read(&layout).err().unwrap(),
            Keyring::open(&layout).err().unwrap(),
        ] {
            assert_eq!(
                error.downcast_ref::<StateFault>(),
                Some(&StateFault::InvalidOwnerKey)
            );
            assert_eq!(
                error.to_string(),
                "Invalid owner key length; key was not replaced."
            );
        }
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    fs::write(&path, [7; 32]).unwrap();
    assert!(Keyring::open(&layout).is_ok(), "valid key positive control");
    let temporary = layout.directory.join(format!(".owner-{}", "b".repeat(32)));
    fs::write(&temporary, [8; 33]).unwrap();
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        Keyring::open(&layout)
            .err()
            .unwrap()
            .downcast_ref::<StateFault>(),
        Some(&StateFault::InvalidOwnerKey)
    );
    assert_eq!(fs::read(temporary).unwrap(), [8; 33]);
    assert_eq!(fs::read(path).unwrap(), [7; 32]);
}

#[test]
fn no_follow_and_missing_owner_key_errors_remain_raw_io_errors() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let missing = Keyring::read(&layout).err().unwrap();
    assert_eq!(
        missing.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
    let path = layout.directory.join("owner.key");
    let outside = fixture.0.join("outside");
    fs::write(&outside, [9; 32]).unwrap();
    symlink(&outside, &path).unwrap();
    for error in [
        Keyring::read(&layout).err().unwrap(),
        Keyring::open(&layout).err().unwrap(),
    ] {
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(nix::libc::ELOOP)
        );
    }
    assert_eq!(fs::read(outside).unwrap(), [9; 32]);
    assert!(fs::symlink_metadata(path).unwrap().is_symlink());
}

#[test]
fn namespace_conflict_and_byte_capacity_preserve_existing_ciphertext() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let mut store = Store::open(&layout).unwrap();
    store.create_page("page").unwrap();
    store.append(&envelope(1, Namespace::Content)).unwrap();
    assert!(matches!(
        store.append(&envelope(1, Namespace::Own)),
        Err(Fault::Conflict)
    ));
    assert_eq!(
        store.payload(scope(), 1).unwrap(),
        Some(b"opaque-ciphertext".to_vec())
    );
    store.create_page("other").unwrap();
    let other = Envelope {
        scope: StreamScope {
            page: "other",
            ..scope()
        },
        ..envelope(1, Namespace::Content)
    };
    store.append(&other).unwrap();
    let oracle = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    oracle
        .execute(
            "UPDATE receipts SET payload=zeroblob(?) WHERE page='other'",
            [tmt_colab::limits::PAGE_BYTES as i64],
        )
        .unwrap();
    let tail = Envelope {
        seq: 2,
        hash: [2; 32],
        previous: [1; 32],
        ..other
    };
    assert!(matches!(store.append(&tail), Err(Fault::Capacity)));
    assert_eq!(
        oracle
            .query_row(
                "SELECT length(payload) FROM receipts WHERE page='other'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        tmt_colab::limits::PAGE_BYTES as i64
    );
    // Pruning and checkpoint insertion share the capacity transaction, reclaiming its own prefix.
    let checkpoint = Envelope {
        hash: [8; 32],
        previous: [1; 32],
        bytes: b"checkpoint",
        ..other
    };
    assert_eq!(store.checkpoint(&checkpoint).unwrap(), Accepted::New);
    assert!(store.payload(other.scope, 1).unwrap().is_none());
    assert_eq!(store.append(&tail).unwrap(), Accepted::New);
}

#[test]
fn symlinked_extension_directory_is_refused_without_outside_writes() {
    let fixture = Fixture::new();
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&outside, fixture.0.join("colab")).unwrap();
    assert!(Layout::existing(&fixture.0).is_err());
    assert!(Layout::open(&fixture.0).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn repeated_compaction_reclaims_unpinned_checkpoints_and_rejects_sequence_rollback() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let mut store = Store::open(&layout).unwrap();
    store.create_page("page").unwrap();
    let checkpoint_bytes = vec![7; tmt_colab::limits::OBJECT_BYTES];
    for seq in 1..=8 {
        store.append(&envelope(seq, Namespace::Content)).unwrap();
        let checkpoint = Envelope {
            hash: [42 + seq as u8; 32],
            previous: [seq as u8; 32],
            bytes: &checkpoint_bytes,
            ..envelope(seq, Namespace::Content)
        };
        store.checkpoint(&checkpoint).unwrap();
        if seq == 1 {
            store
                .pin_checkpoint(scope(), Namespace::Content, 1)
                .unwrap();
        }
    }
    let oracle = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM checkpoints", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        8
    );
    assert_eq!(
        oracle
            .query_row("SELECT sum(length(payload)) FROM checkpoints", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        (2 * tmt_colab::limits::OBJECT_BYTES) as i64
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM checkpoints WHERE payload IS NOT NULL AND pinned=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    // A genuinely new namespace checkpoint may not roll back its highest prefix.
    let own = Envelope {
        hash: [77; 32],
        previous: [8; 32],
        bytes: b"own checkpoint",
        ..envelope(8, Namespace::Own)
    };
    store.checkpoint(&own).unwrap();
    let stale = Envelope {
        seq: 7,
        previous: [7; 32],
        ..own
    };
    assert!(matches!(
        store.checkpoint(&stale),
        Err(Fault::StaleCheckpoint)
    ));
    let old = Envelope {
        hash: [44; 32],
        previous: [2; 32],
        bytes: &checkpoint_bytes,
        ..envelope(2, Namespace::Content)
    };
    assert_eq!(store.checkpoint(&old).unwrap(), Accepted::Replay);
    assert!(matches!(
        store.pin_checkpoint(scope(), Namespace::Content, 2),
        Err(Fault::Gap)
    ));
    // Failed publication restores the prior checkpoint payload as well as update payloads.
    store.append(&envelope(9, Namespace::Content)).unwrap();
    oracle.execute_batch("CREATE TRIGGER fail_new_checkpoint BEFORE INSERT ON checkpoints BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let next = Envelope {
        hash: [99; 32],
        previous: [9; 32],
        bytes: b"next",
        ..envelope(9, Namespace::Content)
    };
    assert!(matches!(store.checkpoint(&next), Err(Fault::Sql(_))));
    assert_eq!(
        store.payload(scope(), 9).unwrap(),
        Some(b"opaque-ciphertext".to_vec())
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM checkpoints WHERE payload IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        3
    );
}

#[test]
fn owner_temporary_cleanup_is_locked_bounded_and_no_follow() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let original = Keyring::open(&layout).unwrap();
    let temporary = layout.directory.join(format!(".owner-{}", "a".repeat(32)));
    fs::write(&temporary, b"partial").unwrap();
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).unwrap();
    let foreign = layout.directory.join(".owner-note");
    fs::write(&foreign, b"keep").unwrap();
    let guard = nix::fcntl::Flock::lock(
        layout.file("keyring.lock").unwrap(),
        nix::fcntl::FlockArg::LockExclusiveNonblock,
    )
    .unwrap();
    assert_eq!(
        Keyring::open(&layout)
            .err()
            .unwrap()
            .downcast_ref::<tmt_colab::keyring::StateFault>(),
        Some(&tmt_colab::keyring::StateFault::KeyringBusy)
    );
    assert!(temporary.exists());
    drop(guard);
    assert_eq!(
        Keyring::open(&layout).unwrap().owner_public(),
        original.owner_public()
    );
    assert!(!temporary.exists());
    assert_eq!(fs::read(foreign).unwrap(), b"keep");
    let outside = fixture.0.join("outside");
    fs::write(&outside, b"keep outside").unwrap();
    symlink(&outside, &temporary).unwrap();
    assert!(Keyring::open(&layout).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"keep outside");
    assert!(fs::symlink_metadata(temporary).unwrap().is_symlink());
}

#[test]
fn too_new_schema_has_a_typed_fault_and_preserves_the_database() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    layout.file("space.db").unwrap();
    let path = layout.directory.join("space.db");
    let oracle = rusqlite::Connection::open(&path).unwrap();
    oracle.execute_batch("PRAGMA user_version=99;").unwrap();
    drop(oracle);
    let before = fs::read(&path).unwrap();
    assert!(matches!(
        Store::open(&layout).err().unwrap().downcast_ref::<Fault>(),
        Some(Fault::UnsupportedSchema(99))
    ));
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn existing_space_keeps_model_id_owner_seed_and_ciphertext_on_reopen() {
    use std::io::Write;
    let fixture = Fixture::new();
    let layout = fixture.layout();
    // Existing L2a space: public fixture seed and the independently frozen ID.
    let seed = (0..32).collect::<Vec<u8>>();
    layout.file("owner.key").unwrap().write_all(&seed).unwrap();
    let mut store = Store::open(&layout).unwrap();
    store.create_page("page").unwrap();
    store.append(&envelope(1, Namespace::Content)).unwrap();
    drop(store);
    let reopened = Keyring::read(&layout).unwrap();
    assert_eq!(reopened.space_id, "4kph3kmtxo7dinlvoixpw642ozfibd2w");
    assert_eq!(
        reopened.space_id,
        tmt_colab_model::crypto::space_id(&reopened.owner_public()).unwrap()
    );
    assert_eq!(fs::read(layout.directory.join("owner.key")).unwrap(), seed);
    let mut store = Store::open(&layout).unwrap();
    assert_eq!(
        store.payload(scope(), 1).unwrap(),
        Some(b"opaque-ciphertext".to_vec())
    );
    assert_eq!(
        store.append(&envelope(1, Namespace::Content)).unwrap(),
        Accepted::Replay
    );
}

#[test]
fn namespace_read_pages_resolve_checkpoint_tail_and_reject_pruned_or_substituted_cursors() {
    use tmt_colab::store::NamespaceCursor;
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let mut store = Store::open(&layout).unwrap();
    store.create_page("page").unwrap();
    store.append(&envelope(1, Namespace::Content)).unwrap();
    store.append(&envelope(2, Namespace::Own)).unwrap();
    store.append(&envelope(3, Namespace::Content)).unwrap();
    let checkpoint = Envelope {
        hash: [8; 32],
        previous: [2; 32],
        bytes: b"checkpoint",
        ..envelope(2, Namespace::Content)
    };
    store.checkpoint(&checkpoint).unwrap();
    store
        .checkpoint(&Envelope {
            namespace: Namespace::Own,
            hash: [9; 32],
            ..checkpoint
        })
        .unwrap();
    assert_eq!(
        store.namespaces("page", 1).unwrap(),
        vec![
            ("device".into(), Namespace::Content),
            ("device".into(), Namespace::Own)
        ]
    );
    let first = store
        .namespace_next(scope(), Namespace::Content, NamespaceCursor::default())
        .unwrap()
        .unwrap();
    assert!(first.checkpoint);
    assert_eq!(first.bytes, b"checkpoint");
    let tail = store
        .namespace_next(scope(), Namespace::Content, first.cursor)
        .unwrap()
        .unwrap();
    assert!(!tail.checkpoint);
    assert_eq!(tail.cursor.seq, 3);
    assert!(
        store
            .namespace_next(scope(), Namespace::Content, tail.cursor)
            .unwrap()
            .is_none()
    );
    for (ns, cursor) in [
        (
            Namespace::Content,
            NamespaceCursor {
                seq: 1,
                hash: [1; 32],
            },
        ),
        (Namespace::Own, first.cursor),
        (
            Namespace::Content,
            NamespaceCursor {
                seq: 3,
                hash: [9; 32],
            },
        ),
    ] {
        assert!(matches!(
            store.namespace_next(scope(), ns, cursor),
            Err(Fault::ResyncRequired)
        ));
    }
    let own = store
        .namespace_next(scope(), Namespace::Own, NamespaceCursor::default())
        .unwrap()
        .unwrap();
    assert_eq!(own.cursor.seq, 2);
    assert!(own.checkpoint);
    assert_eq!(own.cursor.hash, [9; 32]);
    let oracle = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    oracle
        .execute(
            "UPDATE receipts SET payload=zeroblob(?) WHERE seq=?",
            rusqlite::params![
                (tmt_colab::limits::OBJECT_BYTES + 1) as i64,
                "00000000000000000003"
            ],
        )
        .unwrap();
    assert!(matches!(
        store.namespace_next(scope(), Namespace::Content, first.cursor),
        Err(Fault::Capacity)
    ));
    store.advance_epoch("page", 1).unwrap();
    assert!(matches!(
        store.namespace_next(scope(), Namespace::Content, first.cursor),
        Err(Fault::StaleEpoch)
    ));
}

#[test]
fn checkpoint_pair_requires_same_head_and_failed_partner_preserves_the_previous_pair() {
    let fixture = Fixture::new();
    let layout = fixture.layout();
    let mut store = Store::open(&layout).unwrap();
    store.create_page("page").unwrap();
    for seq in 1..=4 {
        store
            .append(&envelope(
                seq,
                if seq % 2 == 1 {
                    Namespace::Content
                } else {
                    Namespace::Own
                },
            ))
            .unwrap();
    }
    let cp = |seq, ns| Envelope {
        hash: [40 + seq as u8 + if ns == Namespace::Own { 10 } else { 0 }; 32],
        previous: [seq as u8; 32],
        bytes: b"checkpoint",
        ..envelope(seq, ns)
    };
    store.checkpoint(&cp(2, Namespace::Content)).unwrap();
    // A checkpoint in the other namespace at a different head is not a pair.
    store.checkpoint(&cp(1, Namespace::Own)).unwrap();
    assert!(store.payload(scope(), 1).unwrap().is_some());
    assert!(store.payload(scope(), 2).unwrap().is_some());
    store.checkpoint(&cp(2, Namespace::Own)).unwrap();
    assert!(store.payload(scope(), 1).unwrap().is_none());
    assert!(store.payload(scope(), 2).unwrap().is_none());
    store
        .pin_checkpoint(scope(), Namespace::Content, 2)
        .unwrap();
    store.checkpoint(&cp(4, Namespace::Content)).unwrap();
    // Retain the old pair until the newer prefix is covered in both namespaces.
    for ns in [Namespace::Content, Namespace::Own] {
        let first = store
            .namespace_next(scope(), ns, Default::default())
            .unwrap()
            .unwrap();
        assert!(first.checkpoint);
        assert_eq!(first.cursor.seq, 2);
    }
    let db = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_partner BEFORE INSERT ON checkpoints WHEN NEW.namespace='own' AND NEW.seq='00000000000000000004' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(matches!(
        store.checkpoint(&cp(4, Namespace::Own)),
        Err(Fault::Sql(_))
    ));
    assert!(store.payload(scope(), 3).unwrap().is_some());
    assert!(store.payload(scope(), 4).unwrap().is_some());
    db.execute_batch("DROP TRIGGER fail_partner").unwrap();
    store.close().unwrap();
    let mut store = Store::open(&layout).unwrap();
    store.checkpoint(&cp(4, Namespace::Own)).unwrap();
    assert!(store.payload(scope(), 3).unwrap().is_none());
    assert!(store.payload(scope(), 4).unwrap().is_none());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM checkpoints WHERE payload IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
    for ns in [Namespace::Content, Namespace::Own] {
        assert_eq!(
            store
                .namespace_next(scope(), ns, Default::default())
                .unwrap()
                .unwrap()
                .cursor
                .seq,
            4
        );
    }
    // Pinned authority-cut checkpoints remain readable after a newer pair prunes.
    store
        .resolve_cursor(
            scope(),
            Namespace::Content,
            tmt_colab::store::NamespaceCursor {
                seq: 2,
                hash: cp(2, Namespace::Content).hash,
            },
        )
        .unwrap();
}

#[test]
fn content_time_is_atomic_new_only_and_nondecreasing_across_restart() {
    use std::sync::{Arc, atomic::AtomicU64};
    let f = Fixture::new();
    let layout = f.layout();
    let time = Arc::new(AtomicU64::new(1000));
    let clock = time.clone();
    let mut store = Store::open(&layout)
        .unwrap()
        .with_clock(move || Ok(clock.load(Ordering::SeqCst)));
    store.create_page("page").unwrap();
    let db = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    let updated = || {
        db.query_row(
            "SELECT last_update_at_ms FROM pages WHERE page='page'",
            [],
            |r| r.get::<_, Option<i64>>(0),
        )
        .unwrap()
    };
    assert_eq!(updated(), None);
    assert_eq!(
        store.append(&envelope(1, Namespace::Content)).unwrap(),
        Accepted::New
    );
    assert_eq!(updated(), Some(1000));
    time.store(5000, Ordering::SeqCst);
    assert_eq!(
        store.append(&envelope(1, Namespace::Content)).unwrap(),
        Accepted::Replay
    );
    store.append(&envelope(2, Namespace::Own)).unwrap();
    assert!(matches!(
        store.append(&envelope(4, Namespace::Content)),
        Err(Fault::Gap)
    ));
    assert_eq!(updated(), Some(1000));
    time.store(500, Ordering::SeqCst);
    store.append(&envelope(3, Namespace::Content)).unwrap();
    assert_eq!(updated(), Some(1000));
    time.store(6000, Ordering::SeqCst);
    store.append(&envelope(4, Namespace::Content)).unwrap();
    assert_eq!(updated(), Some(6000));
    let checkpoint = Envelope {
        hash: [9; 32],
        previous: [4; 32],
        ..envelope(4, Namespace::Content)
    };
    store.checkpoint(&checkpoint).unwrap();
    assert_eq!(updated(), Some(6000));
    store.close().unwrap();
    let mut reopened = Store::open(&layout)
        .unwrap()
        .with_clock(|| Err(Fault::Invalid));
    assert_eq!(
        reopened.append(&envelope(4, Namespace::Content)).unwrap(),
        Accepted::Replay
    );
    assert_eq!(updated(), Some(6000));
    assert!(reopened.append(&envelope(5, Namespace::Content)).is_err());
    assert!(reopened.payload(scope(), 5).unwrap().is_none());
    assert_eq!(updated(), Some(6000));
}
#[test]
fn timestamp_write_failure_rolls_back_content_receipt_and_stream() {
    let f = Fixture::new();
    let layout = f.layout();
    let mut store = Store::open(&layout).unwrap().with_clock(|| Ok(1000));
    store.create_page("page").unwrap();
    let db = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_time BEFORE UPDATE OF last_update_at_ms ON pages BEGIN SELECT RAISE(ABORT,'refused time'); END;").unwrap();
    assert!(store.append(&envelope(1, Namespace::Content)).is_err());
    for table in ["receipts", "streams"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row("SELECT last_update_at_ms FROM pages", [], |r| r
            .get::<_, Option<i64>>(0))
            .unwrap(),
        None
    );
    db.execute_batch("DROP TRIGGER refuse_time").unwrap();
    store.append(&envelope(1, Namespace::Content)).unwrap();
    assert_eq!(
        db.query_row("SELECT last_update_at_ms FROM pages", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1000
    );
}
#[test]
fn schema_four_migration_does_not_backfill_or_let_replay_establish_a_time() {
    let f = Fixture::new();
    let layout = f.layout();
    let mut store = Store::open(&layout).unwrap();
    store.create_page("page").unwrap();
    store.append(&envelope(1, Namespace::Content)).unwrap();
    store.close().unwrap();
    let db = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    let before: (Vec<u8>, Vec<u8>, Vec<u8>) = db
        .query_row("SELECT hash,digest,payload FROM receipts", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    db.execute_batch("ALTER TABLE owner_operations RENAME TO current_owner_operations;
        CREATE TABLE owner_operations(id TEXT PRIMARY KEY, digest BLOB NOT NULL, outcome BLOB NOT NULL);
        INSERT INTO owner_operations(id,digest,outcome) SELECT id,digest,outcome FROM current_owner_operations;
        DROP TABLE current_owner_operations;
        ALTER TABLE pages DROP COLUMN last_update_at_ms; PRAGMA user_version=4")
        .unwrap();
    let raw = fs::read(layout.directory.join("space.db")).unwrap();
    assert!(matches!(
        Store::write_existing(&layout)
            .err()
            .unwrap()
            .downcast_ref::<Fault>(),
        Some(Fault::OutdatedSchema(4))
    ));
    let read = Store::read(&layout).unwrap();
    assert!(matches!(
        read.require_current_schema(),
        Err(Fault::OutdatedSchema(4))
    ));
    read.close().unwrap();
    assert_eq!(fs::read(layout.directory.join("space.db")).unwrap(), raw);
    let mut migrated = Store::open(&layout).unwrap().with_clock(|| Ok(3000));
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        6
    );
    let after: (Vec<u8>, Vec<u8>, Vec<u8>) = db
        .query_row("SELECT hash,digest,payload FROM receipts", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(
        migrated.append(&envelope(1, Namespace::Content)).unwrap(),
        Accepted::Replay
    );
    assert_eq!(
        db.query_row("SELECT last_update_at_ms FROM pages", [], |r| r
            .get::<_, Option<i64>>(0))
            .unwrap(),
        None
    );
    migrated.append(&envelope(2, Namespace::Content)).unwrap();
    migrated.close().unwrap();
    Store::open(&layout).unwrap().close().unwrap();
    assert_eq!(
        db.query_row("SELECT last_update_at_ms FROM pages", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3000
    );
}

// Independent quota oracle includes a rejected original identity from a retained other epoch.
fn scoped_rejection(db: &rusqlite::Connection, layout: &Layout) -> usize {
    use tmt_colab::publication::{JobKey, Outcome, Rejection};
    let key = JobKey {
        operation_id: "40000000-0000-4000-8000-000000000001".into(),
        job_digest: tmt_colab_model::values::encode_binary(&[7; 32]),
        space_id: Keyring::open(layout).unwrap().space_id,
        page_id: "10000000-0000-4000-8000-000000000001".into(),
        original_epoch: "2".into(),
        stream_id: "30000000-0000-4000-8000-000000000001".into(),
    };
    let outcome = Outcome::Rejected {
        key: key.clone(),
        code: Rejection::Capacity,
    }
    .to_json(&key, None)
    .unwrap();
    db.execute("INSERT INTO owner_operations(id,digest,outcome,publication_kind,space,page,original_epoch,original_stream) VALUES(?,?,?,'content',?,?,?,?)",
        rusqlite::params![key.operation_id,[7u8;32].as_slice(),outcome,key.space_id,key.page_id,key.original_epoch,key.stream_id]).unwrap();
    // Compute from individual stored values in Rust, rather than repeating the production aggregate.
    db.query_row("SELECT id,digest,outcome,publication_kind,space,page,original_epoch,original_stream FROM owner_operations WHERE publication_kind IS NOT NULL",[],|r| {
        Ok((0..8).map(|i|match r.get_ref(i).unwrap() {rusqlite::types::ValueRef::Text(b)|rusqlite::types::ValueRef::Blob(b)=>b.len(),_=>panic!("non-byte quota value")}).sum())
    }).unwrap()
}
fn publication_scope() -> StreamScope<'static> {
    StreamScope {
        page: "10000000-0000-4000-8000-000000000001",
        epoch: 1,
        stream: "30000000-0000-4000-8000-000000000001",
    }
}
#[test]
fn native_publication_scoped_bytes_charge_ordinary_append_at_exact_boundary() {
    let f = Fixture::new();
    let layout = f.layout();
    let mut store = Store::open(&layout).unwrap();
    let scope = publication_scope();
    store.create_page(scope.page).unwrap();
    let first = Envelope {
        scope,
        ..envelope(1, Namespace::Own)
    };
    store.append(&first).unwrap();
    let db = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    let outcome_bytes = scoped_rejection(&db, &layout);
    let next = Envelope {
        scope,
        ..envelope(2, Namespace::Content)
    };
    let filler =
        tmt_colab::limits::PAGE_BYTES - first.bytes.len() - outcome_bytes - next.bytes.len();
    db.execute("INSERT INTO checkpoints(page,epoch,stream,namespace,seq,hash,digest,head,payload,pinned) VALUES(?,'1',?,'own','00000000000000000001',X'01',X'02',X'03',zeroblob(?),0)",rusqlite::params![scope.page,scope.stream,filler as i64]).unwrap();
    assert_eq!(store.append(&next).unwrap(), Accepted::New);
    let oracle_total = filler + first.bytes.len() + next.bytes.len() + outcome_bytes;
    assert_eq!(oracle_total, tmt_colab::limits::PAGE_BYTES);
    assert!(matches!(
        store.append(&Envelope {
            scope,
            ..envelope(3, Namespace::Own)
        }),
        Err(Fault::Capacity)
    ));
    assert!(store.payload(scope, 3).unwrap().is_none());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM owner_operations WHERE publication_kind IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn native_publication_scoped_count_charges_pruned_receipts_and_other_epoch() {
    let f = Fixture::new();
    let layout = f.layout();
    let mut store = Store::open(&layout).unwrap();
    let scope = publication_scope();
    store.create_page(scope.page).unwrap();
    store
        .append(&Envelope {
            scope,
            ..envelope(1, Namespace::Own)
        })
        .unwrap();
    let db = rusqlite::Connection::open(layout.directory.join("space.db")).unwrap();
    scoped_rejection(&db, &layout);
    db.execute_batch(&format!("WITH RECURSIVE n(x) AS(SELECT 2 UNION ALL SELECT x+1 FROM n WHERE x<{})
        INSERT INTO receipts(page,epoch,stream,seq,namespace,hash,digest,payload) SELECT '{}','1','{}',printf('%020d',x),'own',zeroblob(32),zeroblob(32),NULL FROM n;",tmt_colab::limits::PAGE_RECEIPTS-1,scope.page,scope.stream)).unwrap();
    let counts:(i64,i64)=db.query_row("SELECT (SELECT count(*) FROM receipts),(SELECT count(*) FROM owner_operations WHERE publication_kind IS NOT NULL)",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(counts, (99999, 1));
    assert!(matches!(
        store.append(&Envelope {
            scope,
            namespace: Namespace::Own,
            seq: 100000,
            hash: [0; 32],
            previous: [0; 32],
            bytes: b"new opaque content"
        }),
        Err(Fault::Capacity)
    ));
    assert!(store.payload(scope, 100000).unwrap().is_none());
    assert_eq!(
        db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        99999
    );
}

#[test]
fn ack_identity_survives_checkpoint_nine_while_catchup_requires_payload() {
    use tmt_colab::store::NamespaceCursor;
    let fixture = Fixture::new();
    let mut store = Store::open(&fixture.layout()).unwrap();
    store.create_page("page").unwrap();
    for seq in 1..=9 {
        store.append(&envelope(seq, Namespace::Content)).unwrap();
    }
    store
        .checkpoint(&Envelope {
            hash: [10; 32],
            previous: [9; 32],
            bytes: b"checkpoint-nine",
            ..envelope(9, Namespace::Content)
        })
        .unwrap();
    let old = NamespaceCursor {
        seq: 1,
        hash: [1; 32],
    };
    store
        .resolve_ack_cursor(scope(), Namespace::Content, old)
        .unwrap();
    assert!(matches!(
        store.resolve_cursor(scope(), Namespace::Content, old),
        Err(Fault::ResyncRequired)
    ));
    for (namespace, cursor) in [
        (
            Namespace::Content,
            NamespaceCursor {
                seq: 10,
                hash: [10; 32],
            },
        ),
        (
            Namespace::Content,
            NamespaceCursor {
                seq: 1,
                hash: [2; 32],
            },
        ),
        (Namespace::Own, old),
    ] {
        assert!(matches!(
            store.resolve_ack_cursor(scope(), namespace, cursor),
            Err(Fault::ResyncRequired)
        ));
    }
    store
        .resolve_ack_cursor(
            scope(),
            Namespace::Content,
            NamespaceCursor {
                seq: 9,
                hash: [10; 32],
            },
        )
        .unwrap();
    store.advance_epoch("page", 1).unwrap();
    assert!(matches!(
        store.resolve_ack_cursor(scope(), Namespace::Content, old),
        Err(Fault::StaleEpoch)
    ));
}
