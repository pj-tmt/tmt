use super::*;
use crate::test_support::TestDirectory;
use tmt_core::{
    binding::BindingRepository,
    identity::{Identity, Lifetime, create_or_resolve},
};

fn identity(storage: &mut Storage) -> Identity {
    create_or_resolve(storage, "agent", Lifetime::Temporary)
        .unwrap()
        .identity
}

fn retire(storage: &mut Storage, identity: &Identity) {
    storage
        .with_binding_transaction(|rows| rows.retire_identity(identity, false))
        .unwrap();
}

#[test]
fn registration_retirement_reopen_and_delivery_preserve_terminal_state() {
    let directory = TestDirectory::new();
    let path = directory.path.join("hooks.db");
    let mut storage = Storage::open(&path).unwrap();
    let original = identity(&mut storage);
    let hook = IdentityHook::new("office", &original.id, "scope").unwrap();
    for _ in 0..2 {
        assert_eq!(
            storage.register_identity_hook(&hook).unwrap(),
            IdentityHookState::Registered
        );
    }
    assert!(storage.acknowledge_identity_hook(&hook).is_err());
    assert!(storage.record_identity_hook_attempt(&hook).is_err());
    retire(&mut storage, &original);
    storage.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 43);
    assert_eq!(
        storage.pending_identity_hooks("office", 100).unwrap()[0].hook,
        hook
    );
    assert!(storage.record_identity_hook_attempt(&hook).unwrap());
    assert_eq!(
        storage.pending_identity_hooks("office", 1).unwrap()[0].attempt_count,
        1
    );
    assert!(storage.acknowledge_identity_hook(&hook).unwrap());
    assert!(!storage.acknowledge_identity_hook(&hook).unwrap());
    assert!(!storage.record_identity_hook_attempt(&hook).unwrap());
    retire(&mut storage, &original);
    assert_eq!(
        storage.register_identity_hook(&hook).unwrap(),
        IdentityHookState::Delivered
    );
    assert_eq!(storage.count_pending_identity_hooks("office").unwrap(), 0);
    let replacement = identity(&mut storage);
    assert_ne!(replacement.id, original.id);
    let replacement_hook = IdentityHook::new("office", &replacement.id, "scope").unwrap();
    assert_eq!(
        storage.register_identity_hook(&replacement_hook).unwrap(),
        IdentityHookState::Registered
    );
    let late = IdentityHook::new("office", &original.id, "late").unwrap();
    assert_eq!(
        storage.register_identity_hook(&late).unwrap(),
        IdentityHookState::Pending
    );
}

#[test]
fn returned_error_and_queue_failure_roll_back_retirement() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("hooks.db")).unwrap();
    let original = identity(&mut storage);
    let hook = IdentityHook::new("office", &original.id, "scope").unwrap();
    storage.register_identity_hook(&hook).unwrap();
    let result: Result<(), StorageError> = storage.with_binding_transaction(|rows| {
        rows.retire_identity(&original, false)?;
        Err(invalid())
    });
    assert!(result.is_err());
    assert!(
        storage
            .find_active_identity_by_id(&original.id)
            .unwrap()
            .is_some()
    );
    assert_eq!(storage.count_pending_identity_hooks("office").unwrap(), 0);
    storage.connection().unwrap().execute_batch("CREATE TRIGGER reject_hook BEFORE UPDATE ON identity_hooks BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    assert!(
        storage
            .with_binding_transaction(|rows| rows.retire_identity(&original, false))
            .is_err()
    );
    assert!(
        storage
            .find_active_identity_by_id(&original.id)
            .unwrap()
            .is_some()
    );
    storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_hook")
        .unwrap();
    retire(&mut storage, &original);
    assert_eq!(storage.count_pending_identity_hooks("office").unwrap(), 1);
}

#[test]
fn attempts_make_progress_and_invalid_inputs_or_rows_fail() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("hooks.db")).unwrap();
    let original = identity(&mut storage);
    retire(&mut storage, &original);
    let first = IdentityHook::new("office", &original.id, "a").unwrap();
    let second = IdentityHook::new("office", &original.id, "b").unwrap();
    storage.register_identity_hook(&first).unwrap();
    storage.register_identity_hook(&second).unwrap();
    assert_eq!(
        storage.pending_identity_hooks("office", 1).unwrap()[0].hook,
        first
    );
    storage.record_identity_hook_attempt(&first).unwrap();
    assert_eq!(
        storage.pending_identity_hooks("office", 1).unwrap()[0].hook,
        second
    );
    for limit in [0, 101, usize::MAX] {
        assert!(storage.pending_identity_hooks("office", limit).is_err());
    }
    assert!(storage.count_pending_identity_hooks("").is_err());
    let missing =
        IdentityHook::new("office", &uuid::Uuid::new_v4().to_string(), "missing").unwrap();
    assert!(storage.register_identity_hook(&missing).is_err());
    assert!(storage.acknowledge_identity_hook(&missing).is_err());
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identity_hooks SET reference = char(10) WHERE reference = 'a'",
            [],
        )
        .unwrap();
    assert!(storage.pending_identity_hooks("office", 100).is_err());
}

#[test]
fn saved_detach_does_not_enqueue_and_registration_serializes_with_retirement() {
    let directory = TestDirectory::new();
    let path = directory.path.join("hooks.db");
    let mut storage = Storage::open(&path).unwrap();
    let original = create_or_resolve(&mut storage, "agent", Lifetime::Saved)
        .unwrap()
        .identity;
    let hook = IdentityHook::new("office", &original.id, "scope").unwrap();
    storage.register_identity_hook(&hook).unwrap();
    let binding = storage
        .with_binding_transaction(|rows| {
            rows.insert_binding(
                &original,
                &tmt_core::endpoint::ServerEvidence {
                    host: tmt_core::host::HostKind::Tmux,
                    server_id: uuid::Uuid::new_v4().to_string(),
                    socket_path: "/fixture/socket".into(),
                    server_pid: 100,
                    server_start_time: "fixture".into(),
                },
                &tmt_core::endpoint::PaneObservation {
                    id: "%1".into(),
                    target: None,
                    cwd: None,
                    command: "fixture".into(),
                    pane_pid: 101,
                    pane_incarnation: None,
                    suggested_name: None,
                    marker: None,
                },
            )
        })
        .unwrap();
    storage
        .with_binding_transaction(|rows| rows.detach_binding(&binding.id))
        .unwrap();
    assert!(
        storage
            .find_active_identity_by_id(&original.id)
            .unwrap()
            .is_some()
    );
    assert_eq!(storage.count_pending_identity_hooks("office").unwrap(), 0);
    let mut concurrent = Storage::open(&path).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let late = IdentityHook::new("office", &original.id, "late").unwrap();
    let peer_hook = late.clone();
    std::thread::scope(|scope| {
        let peer = scope.spawn(|| {
            barrier.wait();
            concurrent.register_identity_hook(&peer_hook).unwrap();
        });
        barrier.wait();
        retire(&mut storage, &original);
        peer.join().unwrap();
    });
    assert_eq!(
        storage.register_identity_hook(&late).unwrap(),
        IdentityHookState::Pending
    );
    assert_eq!(storage.count_pending_identity_hooks("office").unwrap(), 2);
}

#[test]
fn schema_nine_upgrade_is_atomic_and_preserves_identity() {
    let directory = TestDirectory::new();
    let path = directory.path.join("hooks.db");
    let mut predecessor = Connection::open(&path).unwrap();
    super::super::migrations::seed_hook_predecessor(&mut predecessor);
    let original = Identity {
        id: "11111111-1111-4111-8111-111111111111".into(),
        name: "agent".into(),
        canonical_name: "agent".into(),
        lifetime: Lifetime::Temporary,
        created_at: "created".into(),
        updated_at: "updated".into(),
    };
    predecessor.execute(
        "INSERT INTO identities (id,name,canonical_name,lifetime,created_at,updated_at) VALUES (?,?,?,'temporary',?,?)",
        params![original.id, original.name, original.canonical_name, original.created_at, original.updated_at],
    ).unwrap();
    predecessor.execute_batch("CREATE TRIGGER reject_tenth BEFORE INSERT ON _migrations WHEN NEW.version = 10 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    predecessor.close().unwrap();
    let error = match Storage::open(&path) {
        Ok(_) => panic!("migration should fail"),
        Err(error) => error,
    };
    assert_eq!(error.migration_version, Some(10));
    let observer = Connection::open(&path).unwrap();
    let table_count: i64 = observer
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'identity_hooks'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_count, 0);
    observer.execute_batch("DROP TRIGGER reject_tenth").unwrap();
    observer.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 43);
    assert_eq!(
        storage
            .find_active_identity_by_id(&original.id)
            .unwrap()
            .unwrap(),
        original
    );
    let hook = IdentityHook::new("office", &original.id, "scope").unwrap();
    assert_eq!(
        storage.register_identity_hook(&hook).unwrap(),
        IdentityHookState::Registered
    );
}
