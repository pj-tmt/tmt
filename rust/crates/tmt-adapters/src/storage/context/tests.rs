use super::*;
use crate::test_support::TestDirectory;
use std::fs;
use tmt_core::identity::{Lifetime, create_or_resolve};

fn fixture() -> (TestDirectory, std::path::PathBuf, String) {
    let directory = TestDirectory::new();
    let path = directory.path.join("context.db");
    let mut storage = Storage::open(&path).unwrap();
    let identity = create_or_resolve(&mut storage, "Context agent", Lifetime::Saved)
        .unwrap()
        .identity;
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO bindings (id, identity_id, transport, pane_id, server_id,
         socket_path, server_pid, server_start_time, pane_pid, bound_at, last_verified_at)
         VALUES ('binding', ?1, 'tmux', '%1', 'server', '/tmp/context-socket', 10,
                 'start', 11, 'original', 'original')",
            [&identity.id],
        )
        .unwrap();
    storage.close().unwrap();
    (directory, path, identity.id)
}

fn insert_request(connection: &Connection, identity: &str, id: &str) {
    connection
        .execute(
            "INSERT INTO request_attempts (attempt_id, request_id, route_kind,
         wait_active, status, inject_preamble, cadence_reserved, prepared_at_ms,
         expires_at_ms, retention_expires_at_ms, originator_kind,
         originator_identity_id, recipient_identity_id, attention_revision,
         recipient_attention_revision)
         VALUES (?1, ?1, 'inbox', 0, 'queued', 0, 0, 1, 100, 1000,
                 'explicit', ?2, ?2, 1, 1)",
            params![id, identity],
        )
        .unwrap();
}

#[test]
fn absent_storage_is_not_created_and_missing_binding_is_not_reconciled() {
    let directory = TestDirectory::new();
    let absent = directory.path.join("missing/context.db");
    assert!(Storage::context_by_pane(&absent, "%1", "server", 10).is_err());
    assert!(!absent.parent().unwrap().exists());
    let (_directory, path, _) = fixture();
    let before = fs::read(&path).unwrap();
    assert!(
        Storage::context_by_pane(&path, "%2", "server", 10)
            .unwrap()
            .is_none()
    );
    let snapshot = Storage::context_by_pane(&path, "%1", "server", 10)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.entry.identity.lifetime, Lifetime::Saved);
    assert_eq!(snapshot.entry.binding.unwrap().pane_pid, 11);
    assert_eq!(snapshot.requests, ContextRequests::default());
    assert_eq!(snapshot.role, None);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn hook_open_neither_creates_missing_storage_nor_migrates_an_old_schema() {
    let directory = TestDirectory::new();
    let absent = directory.path.join("missing.db");
    assert!(Storage::open_hook(&absent).is_err());
    assert!(!absent.exists());
    let old = directory.path.join("old.db");
    let connection = Connection::open(&old).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE user_data (value TEXT); INSERT INTO user_data VALUES ('keep');",
        )
        .unwrap();
    connection.close().unwrap();
    let before = fs::read(&old).unwrap();
    assert!(Storage::open_hook(&old).is_err());
    assert_eq!(fs::read(&old).unwrap(), before);
    let (_directory, path, _) = fixture();
    let mut current = Storage::open_hook(&path).unwrap();
    assert_eq!(
        current
            .connection()
            .unwrap()
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        50
    );
    current.close().unwrap();
}

#[test]
fn acknowledged_work_is_excluded_and_role_is_bounded_without_writes() {
    let (_directory, path, identity) = fixture();
    let connection = Connection::open(&path).unwrap();
    for index in 0..9 {
        insert_request(&connection, &identity, &format!("request-{index}"));
    }
    connection
        .execute(
            "UPDATE request_attempts SET attention_acknowledged_revision = 1,
        recipient_attention_acknowledged_revision = 1",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_attention_identities VALUES (?1, 1, 1)",
            [&identity],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_recipient_attention_identities VALUES (?1, 1, 1)",
            [&identity],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO role_profiles VALUES (?1, ?2, 'original')",
            params![identity, "x\0".repeat(400)],
        )
        .unwrap();
    connection.close().unwrap();
    let before = fs::read(&path).unwrap();
    let snapshot = Storage::context_by_pane(&path, "%1", "server", 10)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.requests.originated, 0);
    assert_eq!(snapshot.requests.incoming, 0);
    assert_eq!(snapshot.role.unwrap(), "x\0".repeat(250));
    assert!(snapshot.role_truncated);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn incompatible_history_is_not_read_or_repaired() {
    for future in [false, true] {
        let (_directory, path, _) = fixture();
        let connection = Connection::open(&path).unwrap();
        if future {
            connection
                .execute(
                    "INSERT INTO _migrations VALUES (999, 'future', 'original')",
                    [],
                )
                .unwrap();
        } else {
            connection.execute("DELETE FROM _migrations WHERE version = (SELECT MAX(version) FROM _migrations)", []).unwrap();
        }
        connection.close().unwrap();
        let before = fs::read(&path).unwrap();
        assert!(Storage::context_by_pane(&path, "%1", "server", 10).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn replies_and_incoming_obey_separate_attention_and_retention_without_cleanup() {
    let (_directory, path, identity) = fixture();
    let connection = Connection::open(&path).unwrap();
    for id in ["pending", "reply", "expired", "failed"] {
        insert_request(&connection, &identity, id);
    }
    connection
        .execute(
            "UPDATE request_attempts SET response_submitted_at_ms = 5
        WHERE request_id = 'reply'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE request_attempts SET retention_expires_at_ms = 10
        WHERE request_id = 'expired'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE request_attempts SET status = 'definitely_failed'
        WHERE request_id = 'failed'",
            [],
        )
        .unwrap();
    connection.close().unwrap();
    let before = fs::read(&path).unwrap();
    let snapshot = Storage::context_by_pane(&path, "%1", "server", 10)
        .unwrap()
        .unwrap();
    // X includes failed delivery attention until acknowledged; expiration excludes one row.
    assert_eq!(snapshot.requests.originated, 3);
    assert_eq!(snapshot.requests.incoming, 2);
    assert_eq!(fs::read(&path).unwrap(), before);
    // The boundary reads a temporary identity too, but never resurrects retired ones.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute("UPDATE identities SET lifetime = 'temporary'", [])
        .unwrap();
    assert_eq!(
        Storage::context_by_pane(&path, "%1", "server", 10)
            .unwrap()
            .unwrap()
            .entry
            .identity
            .lifetime,
        Lifetime::Temporary
    );
    connection
        .execute("UPDATE identities SET retired_at_ms = 5", [])
        .unwrap();
    assert!(
        Storage::context_by_pane(&path, "%1", "server", 10)
            .unwrap()
            .is_none()
    );
}

#[test]
fn many_unread_items_keep_exact_counts_and_independent_bulk_watermarks() {
    let (_directory, path, identity) = fixture();
    let connection = Connection::open(&path).unwrap();
    for revision in 1..=9 {
        let id = format!("request-{revision}");
        insert_request(&connection, &identity, &id);
        connection
            .execute(
                "UPDATE request_attempts SET response_submitted_at_ms = 5,
             attention_revision = ?1, recipient_attention_revision = ?1
             WHERE request_id = ?2",
                params![revision, id],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO request_attention_identities VALUES (?1, 9, 0)",
            [&identity],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_recipient_attention_identities VALUES (?1, 9, 0)",
            [&identity],
        )
        .unwrap();
    connection.close().unwrap();
    let before = fs::read(&path).unwrap();
    let snapshot = Storage::context_by_pane(&path, "%1", "server", 10)
        .unwrap()
        .unwrap();
    for summary in [snapshot.requests.originated, snapshot.requests.incoming] {
        assert_eq!(summary, 9);
    }
    assert_eq!(fs::read(&path).unwrap(), before);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE request_attention_identities SET acknowledged_through = 4",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE request_recipient_attention_identities SET acknowledged_through = 2",
            [],
        )
        .unwrap();
    connection.close().unwrap();
    let before = fs::read(&path).unwrap();
    let snapshot = Storage::context_by_pane(&path, "%1", "server", 10)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.requests.originated, 5);
    assert_eq!(snapshot.requests.incoming, 7);
    assert_eq!(fs::read(&path).unwrap(), before);
}
