use std::{
    path::PathBuf,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

use crate::test_support::TestDirectory;
use rusqlite::{Connection, params, types::Value};
use tmt_core::identity::{
    CreatedIdentity, IdentityError, IdentityReader, IdentityRepository, Lifetime,
    create_or_resolve, find_by_name,
};
use tmt_core::names::validate_name;

use super::super::*;

struct Fixture {
    _directory: TestDirectory,
    database: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = TestDirectory::new();
        Self {
            database: directory.path.join("state").join("tmux-team.db"),
            _directory: directory,
        }
    }

    fn open(&self) -> Storage {
        Storage::open(&self.database).unwrap()
    }
}

type DependentSnapshot = Vec<(&'static str, Vec<Vec<Value>>)>;

fn snapshot_dependent_rows(connection: &Connection, identity_id: &str) -> DependentSnapshot {
    const TABLES: [(&str, &str); 7] = [
        (
            "role_profiles",
            "SELECT * FROM role_profiles WHERE identity_id = ? ORDER BY identity_id",
        ),
        (
            "identity_preambles",
            "SELECT * FROM identity_preambles WHERE identity_id = ? ORDER BY identity_id",
        ),
        (
            "preamble_counters",
            "SELECT * FROM preamble_counters WHERE identity_id = ? ORDER BY identity_id",
        ),
        (
            "bindings",
            "SELECT * FROM bindings WHERE identity_id = ? ORDER BY identity_id, id",
        ),
        (
            "request_attempts",
            "SELECT * FROM request_attempts WHERE identity_id = ? ORDER BY identity_id, attempt_id",
        ),
        (
            "request_responses",
            "SELECT request_responses.* FROM request_responses
             JOIN request_attempts ON request_attempts.attempt_id = request_responses.attempt_id
             WHERE request_attempts.identity_id = ? ORDER BY request_responses.request_id",
        ),
        (
            "request_attention_identities",
            "SELECT * FROM request_attention_identities WHERE identity_id = ? ORDER BY identity_id",
        ),
    ];
    TABLES
        .into_iter()
        .map(|(table, query)| {
            let mut statement = connection.prepare(query).unwrap();
            let rows = statement
                .query_map([identity_id], |row| {
                    (0..row.as_ref().column_count())
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<Value>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

fn seed_dependents(connection: &Connection, identity_id: &str) {
    let attempt_id = format!("attempt-{identity_id}");
    let request_id = format!("request-{identity_id}");
    connection
        .execute(
            "INSERT INTO role_profiles VALUES (?, 'role body', 'role-time')",
            params![identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO identity_preambles VALUES (?, 'preamble body', 'preamble-time')",
            params![identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO preamble_counters VALUES (?, 12, 123456789)",
            params![identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO bindings (id, identity_id, transport, pane_id, server_id, socket_path, server_pid, server_start_time, pane_pid, bound_at, last_verified_at) VALUES ('binding', ?, 'tmux', '%1', 'server', '/tmp/socket', 11, 'server-time', 12, 'bound-time', 'verified-time')",
            params![identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_attempts (
                attempt_id, request_id, identity_id, server_id, socket_path, server_pid,
                server_start_time, pane_id, pane_pid, wait_active, status, preamble_every,
                inject_preamble, cadence_reserved, prepared_at_ms, expires_at_ms,
                retention_days, retention_expires_at_ms, originator_kind, originator_identity_id,
                recipient_identity_id, message_text, message_bytes, message_expires_at_ms,
                attention_revision, attention_acknowledged_revision
             ) VALUES (?, ?, ?, 'server', '/tmp/socket', 11, 'server-time', '%1', 12, 0,
                'sent', 3, 1, 1, 100, 200, 7, 300, 'explicit', ?, ?,
                'prompt body', 11, 400, 7, 5)",
            params![
                attempt_id,
                request_id,
                identity_id,
                identity_id,
                identity_id,
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_responses (
                request_id, attempt_id, server_id, socket_path, server_pid,
                server_start_time, pane_id, pane_pid, body, body_bytes, submitted_at_ms,
                response_expires_at_ms
             ) VALUES (?, ?, 'server', '/tmp/socket', 11, 'server-time', '%1', 12,
                'response body', 13, 150, 500)",
            params![request_id, attempt_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_attention_identities VALUES (?, 7, 5)",
            params![identity_id],
        )
        .unwrap();
}

fn assert_uuid_v4(value: &str) {
    let uuid = uuid::Uuid::parse_str(value).expect("identity ID must be a UUID");
    assert_eq!(value.len(), 36);
    assert_eq!(uuid.to_string(), value);
    assert_eq!(uuid.get_version_num(), 4);
}

fn assert_utc_millisecond_timestamp(value: &str) {
    assert_eq!(
        value.len(),
        24,
        "timestamp must use UTC milliseconds: {value}"
    );
    for (index, character) in value.chars().enumerate() {
        if [4, 7, 10, 13, 16, 19, 23].contains(&index) {
            continue;
        }
        assert!(
            character.is_ascii_digit(),
            "timestamp must be numeric: {value}"
        );
    }
    assert_eq!(&value[4..5], "-");
    assert_eq!(&value[7..8], "-");
    assert_eq!(&value[10..11], "T");
    assert_eq!(&value[13..14], ":");
    assert_eq!(&value[16..17], ":");
    assert_eq!(&value[19..20], ".");
    assert_eq!(&value[23..24], "Z");
}

#[test]
fn canonical_name_reuse_is_idempotent_and_preserves_the_original_record() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();

    let first = create_or_resolve(&mut storage, "  Alice  ", Lifetime::Temporary).unwrap();
    assert!(first.created);
    assert_uuid_v4(&first.identity.id);
    assert_utc_millisecond_timestamp(&first.identity.created_at);
    assert_utc_millisecond_timestamp(&first.identity.updated_at);
    let second = create_or_resolve(&mut storage, "alice", Lifetime::Temporary).unwrap();
    assert!(!second.created);
    assert_eq!(second.identity, first.identity);
    assert_eq!(
        storage.list_identities().unwrap(),
        vec![first.identity.clone()]
    );
    assert_eq!(
        find_by_name(&storage, " ALICE ").unwrap(),
        Some(first.identity)
    );
    storage.close().unwrap();
}

#[test]
fn promotion_reuses_uuid_without_downgrade_or_rewriting_creation_fields() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();

    let temporary = create_or_resolve(&mut storage, "  Alice  ", Lifetime::Temporary)
        .unwrap()
        .identity;
    seed_dependents(storage.connection().unwrap(), &temporary.id);
    let dependents_before = snapshot_dependent_rows(storage.connection().unwrap(), &temporary.id);
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET created_at = ?, updated_at = ? WHERE id = ?",
            (
                "2000-01-01T00:00:00.000Z",
                "2000-01-02T00:00:00.000Z",
                &temporary.id,
            ),
        )
        .unwrap();

    let saved = create_or_resolve(&mut storage, "alice", Lifetime::Saved).unwrap();
    assert!(!saved.created);
    assert_eq!(saved.identity.id, temporary.id);
    assert_eq!(saved.identity.name, temporary.name);
    assert_eq!(saved.identity.canonical_name, temporary.canonical_name);
    assert_eq!(saved.identity.created_at, "2000-01-01T00:00:00.000Z");
    assert_eq!(saved.identity.lifetime, Lifetime::Saved);
    assert!(saved.identity.updated_at.as_str() > "2000-01-02T00:00:00.000Z");
    assert_eq!(
        snapshot_dependent_rows(storage.connection().unwrap(), &temporary.id),
        dependents_before
    );

    let repeated = create_or_resolve(&mut storage, "ALICE", Lifetime::Temporary).unwrap();
    assert!(!repeated.created);
    assert_eq!(repeated.identity, saved.identity);
    storage.close().unwrap();
}

#[test]
fn saved_identity_cannot_be_downgraded_to_temporary() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();

    let saved = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET created_at = ?, updated_at = ? WHERE id = ?",
            (
                "1999-01-01T00:00:00.000Z",
                "1999-01-02T00:00:00.000Z",
                &saved.id,
            ),
        )
        .unwrap();

    let repeated_saved = create_or_resolve(&mut storage, " alice ", Lifetime::Saved).unwrap();
    assert!(!repeated_saved.created);
    assert_eq!(repeated_saved.identity.id, saved.id);
    assert_eq!(repeated_saved.identity.name, saved.name);
    assert_eq!(repeated_saved.identity.canonical_name, saved.canonical_name);
    assert_eq!(repeated_saved.identity.lifetime, Lifetime::Saved);
    assert_eq!(
        repeated_saved.identity.created_at,
        "1999-01-01T00:00:00.000Z"
    );
    assert_eq!(
        repeated_saved.identity.updated_at,
        "1999-01-02T00:00:00.000Z"
    );

    let reused = create_or_resolve(&mut storage, " alice ", Lifetime::Temporary).unwrap();
    assert!(!reused.created);
    assert_eq!(reused.identity, repeated_saved.identity);
    storage.close().unwrap();
}

#[test]
fn list_is_non_retired_and_deterministically_binary_ordered() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    // Non-English letters distinguish binary ordering from locale collation.
    for name in ["ä", "a2", "a", "é", "aa"] {
        create_or_resolve(&mut storage, name, Lifetime::Temporary).unwrap();
    }

    let listed = storage.list_identities().unwrap();
    let canonical_names = listed
        .iter()
        .map(|identity| identity.canonical_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(canonical_names, ["a", "a2", "aa", "ä", "é"]);
    storage.close().unwrap();
}

#[test]
fn retired_name_is_excluded_and_reuse_gets_a_fresh_uuid_without_dependents() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let original = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    seed_dependents(storage.connection().unwrap(), &original.id);
    let dependents_before = snapshot_dependent_rows(storage.connection().unwrap(), &original.id);
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms = ? WHERE id = ?",
            params![1700000000000_i64, original.id],
        )
        .unwrap();
    assert_eq!(
        snapshot_dependent_rows(storage.connection().unwrap(), &original.id),
        dependents_before
    );

    assert!(storage.find_identity("alice").unwrap().is_none());
    assert!(
        storage
            .list_identities()
            .unwrap()
            .iter()
            .all(|identity| identity.id != original.id)
    );

    let created = create_or_resolve(&mut storage, " alice ", Lifetime::Temporary).unwrap();
    assert!(created.created);
    let replacement = created.identity;
    assert_ne!(replacement.id, original.id);
    assert_eq!(replacement.lifetime, Lifetime::Temporary);
    assert!(
        snapshot_dependent_rows(storage.connection().unwrap(), &replacement.id)
            .iter()
            .all(|(_, rows)| rows.is_empty()),
        "retired identity dependents must not be inherited"
    );
    assert_eq!(
        snapshot_dependent_rows(storage.connection().unwrap(), &original.id),
        dependents_before
    );
    assert_uuid_v4(&replacement.id);
    assert_utc_millisecond_timestamp(&replacement.created_at);
    assert_utc_millisecond_timestamp(&replacement.updated_at);
    storage.close().unwrap();
}

#[test]
fn promotion_failure_rolls_back_lifetime_and_timestamps() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let temporary = create_or_resolve(&mut storage, "Alice", Lifetime::Temporary)
        .unwrap()
        .identity;
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET created_at = ?, updated_at = ? WHERE id = ?",
            (
                "1980-01-01T00:00:00.000Z",
                "1980-01-02T00:00:00.000Z",
                &temporary.id,
            ),
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_identity_promotion
             BEFORE UPDATE OF lifetime ON identities
             WHEN NEW.lifetime = 'saved'
             BEGIN SELECT RAISE(ABORT, 'injected promotion failure'); END;",
        )
        .unwrap();

    let error = create_or_resolve(&mut storage, "alice", Lifetime::Saved).unwrap_err();
    match error {
        IdentityError::Repository(error) => assert_eq!(error.code, StorageErrorCode::Unknown),
        IdentityError::InvalidName(error) => panic!("unexpected invalid name: {error}"),
    }
    let row: (String, String, String) = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT lifetime, created_at, updated_at FROM identities WHERE id = ?",
            [&temporary.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "temporary".into(),
            "1980-01-01T00:00:00.000Z".into(),
            "1980-01-02T00:00:00.000Z".into()
        )
    );
    storage.close().unwrap();
}

#[test]
fn transaction_rolls_back_prior_identity_writes_and_releases_writer_lock() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let temporary = create_or_resolve(&mut storage, "Alice", Lifetime::Temporary)
        .unwrap()
        .identity;
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET created_at = ?, updated_at = ? WHERE id = ?",
            (
                "1970-01-01T00:00:00.000Z",
                "1970-01-02T00:00:00.000Z",
                &temporary.id,
            ),
        )
        .unwrap();
    let rollback_name = validate_name("RollbackNew").unwrap();

    let error: Result<(), StorageError> = storage.with_identity_transaction(|writer| {
        writer.save_identity(&temporary)?;
        writer.insert_identity(&rollback_name, Lifetime::Temporary)?;
        Err(StorageError::new(
            StorageErrorCode::Unknown,
            "injected transaction failure",
        ))
    });
    assert_eq!(error.unwrap_err().code, StorageErrorCode::Unknown);

    let verification = Connection::open(&fixture.database).unwrap();
    verification
        .busy_timeout(Duration::from_millis(100))
        .unwrap();
    let original: (String, String, String) = verification
        .query_row(
            "SELECT lifetime, created_at, updated_at FROM identities WHERE id = ?",
            [&temporary.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        original,
        (
            "temporary".into(),
            "1970-01-01T00:00:00.000Z".into(),
            "1970-01-02T00:00:00.000Z".into()
        )
    );
    let rollback_count: i64 = verification
        .query_row(
            "SELECT COUNT(*) FROM identities WHERE canonical_name = 'rollbacknew'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rollback_count, 0);
    verification
        .execute_batch("BEGIN IMMEDIATE; COMMIT;")
        .unwrap();
    storage.close().unwrap();
}

#[test]
fn commit_contention_rolls_back_creation_and_releases_the_transaction() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    // A rollback-journal reader allows BEGIN IMMEDIATE and INSERT, but blocks
    // COMMIT. This intentionally changed fixture policy isolates the commit
    // failure path; production remains WAL with its five-second busy timeout.
    storage
        .connection()
        .unwrap()
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    storage
        .connection()
        .unwrap()
        .busy_timeout(Duration::from_millis(100))
        .unwrap();
    let reader = Connection::open(&fixture.database).unwrap();
    reader
        .execute_batch("BEGIN; SELECT COUNT(*) FROM identities;")
        .unwrap();

    let error = create_or_resolve(&mut storage, "CommitBlocked", Lifetime::Temporary).unwrap_err();
    let IdentityError::Repository(error) = error else {
        panic!("expected storage contention")
    };
    assert_eq!(error.code, StorageErrorCode::Busy);
    assert_eq!(error.message, "Commit identity transaction failed");
    assert!(error.retryable);
    assert!(storage.connection().unwrap().is_autocommit());
    reader.execute_batch("ROLLBACK;").unwrap();
    let count: i64 = reader
        .query_row("SELECT COUNT(*) FROM identities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    reader.execute_batch("BEGIN IMMEDIATE; COMMIT;").unwrap();
    storage.close().unwrap();
}

#[test]
fn closed_storage_reports_closed_errors_through_reads_and_mutations() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    storage.close().unwrap();

    let mutation = create_or_resolve(&mut storage, "Alice", Lifetime::Temporary).unwrap_err();
    match mutation {
        IdentityError::Repository(error) => assert_eq!(error.code, StorageErrorCode::Closed),
        IdentityError::InvalidName(error) => panic!("unexpected invalid name: {error}"),
    }
    let lookup = find_by_name(&storage, "Alice").unwrap_err();
    match lookup {
        IdentityError::Repository(error) => assert_eq!(error.code, StorageErrorCode::Closed),
        IdentityError::InvalidName(error) => panic!("unexpected invalid name: {error}"),
    }
    assert_eq!(
        storage.list_identities().unwrap_err().code,
        StorageErrorCode::Closed
    );
}

#[test]
fn concurrent_connections_create_once_and_keep_one_active_canonical_row() {
    let fixture = Fixture::new();
    let _initial = fixture.open();
    drop(_initial);

    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(2);
    let start_channels = (0..2)
        .map(|_| mpsc::sync_channel::<()>(1))
        .collect::<Vec<_>>();
    let start_txs = start_channels
        .iter()
        .map(|(sender, _)| sender.clone())
        .collect::<Vec<_>>();
    let (result_tx, result_rx) = mpsc::sync_channel::<Result<(bool, String, Lifetime), String>>(2);

    thread::scope(|scope| {
        for (worker_index, (_, start_rx)) in start_channels.into_iter().enumerate() {
            let ready_tx = ready_tx.clone();
            let result_tx = result_tx.clone();
            let database = fixture.database.clone();
            scope.spawn(move || {
                let mut storage = match Storage::open(database) {
                    Ok(storage) => storage,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                if start_rx.recv_timeout(Duration::from_secs(5)).is_err() {
                    let _ = result_tx.send(Err("timed out waiting for start".into()));
                    return;
                }
                let (name, lifetime) = if worker_index == 0 {
                    ("  Concurrent  ", Lifetime::Temporary)
                } else {
                    ("CONCURRENT", Lifetime::Saved)
                };
                let outcome = create_or_resolve(&mut storage, name, lifetime)
                    .map(|CreatedIdentity { identity, created }| {
                        (created, identity.id, identity.lifetime)
                    })
                    .map_err(|error| error.to_string())
                    .and_then(|outcome| {
                        storage
                            .close()
                            .map(|()| outcome)
                            .map_err(|error| error.to_string())
                    });
                let _ = result_tx.send(outcome);
            });
        }
        drop(ready_tx);
        drop(result_tx);

        let mut ready = 0;
        for _ in 0..2 {
            match ready_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Ok(())) => ready += 1,
                Ok(Err(error)) => panic!("worker failed before synchronization: {error}"),
                Err(RecvTimeoutError::Timeout) => panic!("worker did not become ready"),
                Err(RecvTimeoutError::Disconnected) => panic!("all workers exited early"),
            }
        }
        assert_eq!(ready, 2);
        for start_tx in start_txs {
            start_tx.send(()).unwrap();
        }

        let mut outcomes = Vec::new();
        for _ in 0..ready {
            outcomes.push(
                result_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("worker did not return an outcome")
                    .unwrap(),
            );
        }
        assert_eq!(
            outcomes.iter().filter(|(created, _, _)| *created).count(),
            1
        );
        assert_eq!(outcomes[0].1, outcomes[1].1);
    });

    let verification = Connection::open(&fixture.database).unwrap();
    let active: (i64, i64, String) = verification
        .query_row(
            "SELECT COUNT(*), COUNT(DISTINCT canonical_name), MAX(lifetime)
             FROM identities WHERE retired_at_ms IS NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(active, (1, 1, "saved".into()));
    let canonical: String = verification
        .query_row(
            "SELECT canonical_name FROM identities WHERE retired_at_ms IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(canonical, "concurrent");
}
