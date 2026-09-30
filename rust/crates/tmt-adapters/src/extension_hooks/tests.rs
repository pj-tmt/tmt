use super::*;
use crate::test_support::TestDirectory;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;

const ID: &str = "11111111-1111-4111-8111-111111111111";
const ROOM: &str = "22222222-2222-4222-8222-222222222222";

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
    let consent = |name: &str| Consent {
        name: name.into(),
        path: PathBuf::from("/x"),
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
    };
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
    fs::write(&path, b"{\"version\":2,\"extensions\":[]}").unwrap();
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
