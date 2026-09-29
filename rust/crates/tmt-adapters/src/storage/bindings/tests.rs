use std::{error::Error, time::Duration};

use rusqlite::Connection;
use tmt_core::{
    binding::BindingRepository,
    endpoint::ServerEvidence,
    identity::{Lifetime, create_or_resolve},
    limits::MAX_JS_SAFE_INTEGER,
};

use super::super::*;

use super::test_support::{Fixture, pane};

fn server() -> ServerEvidence {
    ServerEvidence {
        host: tmt_core::host::HostKind::Tmux,
        server_id: "123e4567-e89b-42d3-a456-426614174000".into(),
        socket_path: "/tmp/tmux-test.sock".into(),
        server_pid: 41,
        server_start_time: "server-start".into(),
    }
}

fn assert_invalid_pid(error: StorageError) {
    assert_eq!(error.code, StorageErrorCode::Unknown);
    assert!(matches!(
        error.source().unwrap().downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::InvalidQuery)
    ));
}

#[test]
fn binding_pid_boundaries_round_trip_without_conversion_loss() {
    for pid in [1, MAX_JS_SAFE_INTEGER] {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Boundary", Lifetime::Saved)
            .unwrap()
            .identity;
        let mut endpoint = server();
        endpoint.server_pid = pid;
        let binding = storage
            .with_binding_transaction(|records| {
                records.insert_binding(&identity, &endpoint, &pane("%3", pid))
            })
            .unwrap();
        assert_eq!(binding.server.server_pid, pid);
        assert_eq!(binding.pane_pid, pid);
        let observed = storage
            .with_binding_transaction(|records| records.entry_by_id(&identity.id))
            .unwrap()
            .unwrap();
        assert_eq!(observed.binding, Some(binding));
        storage.close().unwrap();
    }
}

#[test]
fn invalid_binding_pid_writes_fail_before_insert_even_if_caller_handles_error() {
    for pid in [0, MAX_JS_SAFE_INTEGER + 1, i64::MAX as u64 + 1, u64::MAX] {
        for server_field in [true, false] {
            let fixture = Fixture::new();
            let mut storage = fixture.open();
            let identity = create_or_resolve(&mut storage, "Invalid", Lifetime::Saved)
                .unwrap()
                .identity;
            let mut endpoint = server();
            let mut target = pane("%3", 99);
            if server_field {
                endpoint.server_pid = pid;
            } else {
                target.pane_pid = pid;
            }
            // Catch inside the transaction: rollback must not conceal an INSERT
            // performed before a failing RETURNING decoder.
            storage
                .with_binding_transaction(|records| {
                    assert_invalid_pid(
                        records
                            .insert_binding(&identity, &endpoint, &target)
                            .unwrap_err(),
                    );
                    Ok::<_, StorageError>(())
                })
                .unwrap();
            let connection = Connection::open(&fixture.database).unwrap();
            let count: i64 = connection
                .query_row("SELECT COUNT(*) FROM bindings", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 0);
            storage.close().unwrap();
        }
    }
}

#[test]
fn corrupt_binding_pids_fail_every_reader_without_repair_or_retirement() {
    for column in ["server_pid", "pane_pid"] {
        for pid in [-1_i64, 0, MAX_JS_SAFE_INTEGER as i64 + 1, i64::MAX] {
            let fixture = Fixture::new();
            let mut storage = fixture.open();
            let identity = create_or_resolve(&mut storage, "Corrupt", Lifetime::Temporary)
                .unwrap()
                .identity;
            storage
                .with_binding_transaction(|records| {
                    records.insert_binding(&identity, &server(), &pane("%3", 99))
                })
                .unwrap();
            let connection = Connection::open(&fixture.database).unwrap();
            connection
                .execute(&format!("UPDATE bindings SET {column} = ?"), [pid])
                .unwrap();
            storage
                .with_binding_transaction(|records| {
                    assert_invalid_pid(records.entry_by_id(&identity.id).unwrap_err());
                    assert_invalid_pid(
                        records
                            .entry_by_pane(
                                tmt_core::host::HostKind::Tmux,
                                "%3",
                                &server().server_id,
                            )
                            .unwrap_err(),
                    );
                    assert_invalid_pid(records.binding_entries().unwrap_err());
                    Ok::<_, StorageError>(())
                })
                .unwrap();
            let retained: (i64, Option<i64>) = connection.query_row(
                &format!("SELECT b.{column}, i.retired_at_ms FROM bindings b JOIN identities i ON b.identity_id = i.id"),
                [], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(retained, (pid, None));
            storage.close().unwrap();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ApplicationFailure;

impl From<StorageError> for ApplicationFailure {
    fn from(_: StorageError) -> Self {
        Self
    }
}

#[test]
fn joined_entries_are_binary_ordered_and_exclude_retired_identities() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let beta = create_or_resolve(&mut storage, "Beta", Lifetime::Saved)
        .unwrap()
        .identity;
    let alpha = create_or_resolve(&mut storage, "alpha", Lifetime::Saved)
        .unwrap()
        .identity;
    let binding = storage
        .with_binding_transaction(|records| {
            records.insert_binding(&alpha, &server(), &pane("%3", 99))
        })
        .unwrap();

    let entries = storage
        .with_binding_transaction(|records| Ok::<_, StorageError>(records.binding_entries()))
        .unwrap()
        .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.identity.canonical_name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert_eq!(entries[0].binding.as_ref(), Some(&binding));
    assert!(entries[1].binding.is_none());
    let selected = storage
        .with_binding_transaction(|records| records.entry_by_id(&alpha.id))
        .unwrap()
        .unwrap();
    assert_eq!(selected.binding, Some(binding.clone()));

    storage
        .with_binding_transaction(|records| records.retire_identity(&alpha, false))
        .unwrap();
    let remaining = storage
        .with_binding_transaction(|records| Ok::<_, StorageError>(records.binding_entries()))
        .unwrap()
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].identity.id, beta.id);
    assert!(remaining[0].binding.is_none());

    let connection = Connection::open(&fixture.database).unwrap();
    let retired: (Option<i64>, i64) = connection
        .query_row(
            "SELECT retired_at_ms, (SELECT COUNT(*) FROM bindings WHERE identity_id = ?) FROM identities WHERE id = ?",
            [&alpha.id, &alpha.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(retired.0.is_some_and(|value| value > 0));
    assert_eq!(retired.1, 0);
    storage.close().unwrap();
}

#[test]
fn binding_mutations_are_exact_and_content_removal_is_opt_in() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    let other = create_or_resolve(&mut storage, "Other", Lifetime::Saved)
        .unwrap()
        .identity;
    let binding = storage
        .with_binding_transaction(|records| {
            records.insert_binding(&identity, &server(), &pane("%3", 99))
        })
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE bindings SET last_verified_at = 'old' WHERE id = ?",
            [&binding.id],
        )
        .unwrap();

    storage
        .with_binding_transaction(|records| records.touch_binding(&other.id))
        .unwrap();
    assert_eq!(
        storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT last_verified_at FROM bindings WHERE id = ?",
                [&binding.id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "old"
    );
    storage
        .with_binding_transaction(|records| records.touch_binding(&binding.id))
        .unwrap();
    assert_ne!(
        storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT last_verified_at FROM bindings WHERE id = ?",
                [&binding.id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "old"
    );

    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO role_profiles VALUES (?, 'role', 'role-time')",
            [&identity.id],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO identity_preambles VALUES (?, 'preamble', 'preamble-time')",
            [&identity.id],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO identity_metadata VALUES (?, 'project', 'tmt')",
            [&identity.id],
        )
        .unwrap();
    storage
        .with_binding_transaction(|records| records.retire_identity(&identity, false))
        .unwrap();
    let retained_content: (i64, i64, i64) = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM role_profiles WHERE identity_id = ?),
                (SELECT COUNT(*) FROM identity_preambles WHERE identity_id = ?),
                (SELECT COUNT(*) FROM identity_metadata WHERE identity_id = ?)",
            [&identity.id, &identity.id, &identity.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(retained_content, (1, 1, 1));

    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO role_profiles VALUES (?, 'role', 'role-time')",
            [&other.id],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO identity_preambles VALUES (?, 'preamble', 'preamble-time')",
            [&other.id],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO identity_metadata VALUES (?, 'project', 'other')",
            [&other.id],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO preamble_counters VALUES (?, 4, 100)",
            [&other.id],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO request_attention_identities VALUES (?, 2, 1)",
            [&other.id],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO request_attempts (
                attempt_id, request_id, identity_id, server_id, socket_path, server_pid,
                server_start_time, pane_id, pane_pid, wait_active, status, inject_preamble,
                cadence_reserved, prepared_at_ms, expires_at_ms, retention_days,
                retention_expires_at_ms
             ) VALUES ('attempt-other', 'request-other', ?, 'server', '/tmp/socket', 41,
                'server-start', '%3', 99, 0, 'sent', 0, 1, 100, 200, 7, 300)",
            [&other.id],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO request_responses (
                request_id, attempt_id, server_id, socket_path, server_pid,
                server_start_time, pane_id, pane_pid, body, body_bytes, submitted_at_ms,
                response_expires_at_ms
             ) VALUES ('request-other', 'attempt-other', 'server', '/tmp/socket', 41,
                'server-start', '%3', 99, 'response', 8, 150, 500)",
            [],
        )
        .unwrap();
    storage
        .with_binding_transaction(|records| records.retire_identity(&other, true))
        .unwrap();
    let retained_state: (i64, i64, i64, i64, i64, i64, i64) = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM role_profiles WHERE identity_id = ?),
                (SELECT COUNT(*) FROM identity_preambles WHERE identity_id = ?),
                (SELECT COUNT(*) FROM identity_metadata WHERE identity_id = ?),
                (SELECT COUNT(*) FROM preamble_counters WHERE identity_id = ?),
                (SELECT COUNT(*) FROM request_attempts WHERE identity_id = ?),
                (SELECT COUNT(*) FROM request_responses WHERE request_id = 'request-other'),
                (SELECT COUNT(*) FROM request_attention_identities WHERE identity_id = ?)",
            [
                &other.id, &other.id, &other.id, &other.id, &other.id, &other.id,
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(retained_state, (0, 0, 0, 1, 1, 1, 1));

    // A stale exact ID cannot detach another identity's binding.
    let survivor = create_or_resolve(&mut storage, "Survivor", Lifetime::Saved)
        .unwrap()
        .identity;
    let survivor_binding = storage
        .with_binding_transaction(|records| {
            records.insert_binding(&survivor, &server(), &pane("%4", 100))
        })
        .unwrap();
    storage
        .with_binding_transaction(|records| records.detach_binding("stale-binding-id"))
        .unwrap();
    assert_eq!(
        storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT id FROM bindings WHERE identity_id = ?",
                [&survivor.id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        survivor_binding.id
    );
    storage.close().unwrap();
}

#[test]
fn application_failure_rolls_back_binding_writes_and_releases_lock() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Rollback", Lifetime::Temporary)
        .unwrap()
        .identity;
    let result: Result<(), ApplicationFailure> = storage.with_binding_transaction(|records| {
        records.insert_binding(&identity, &server(), &pane("%8", 100))?;
        Err(ApplicationFailure)
    });
    assert_eq!(result, Err(ApplicationFailure));

    let verification = Connection::open(&fixture.database).unwrap();
    assert_eq!(
        verification
            .query_row("SELECT COUNT(*) FROM bindings", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    verification
        .execute_batch("BEGIN IMMEDIATE; COMMIT;")
        .unwrap();
    storage.close().unwrap();
}

#[test]
fn closed_storage_and_immediate_lock_contention_are_reported() {
    let fixture = Fixture::new();
    let mut closed = fixture.open();
    closed.close().unwrap();
    let error: Result<(), StorageError> = closed.with_binding_transaction(|_| Ok(()));
    assert_eq!(error.unwrap_err().code, StorageErrorCode::Closed);

    let mut storage = fixture.open();
    storage
        .connection()
        .unwrap()
        .busy_timeout(Duration::from_millis(25))
        .unwrap();
    let blocker = Connection::open(&fixture.database).unwrap();
    blocker.busy_timeout(Duration::from_millis(25)).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let error: Result<(), StorageError> = storage.with_binding_transaction(|_| Ok(()));
    let error = error.unwrap_err();
    assert_eq!(error.code, StorageErrorCode::Busy);
    assert!(error.retryable);
    assert!(storage.connection().unwrap().is_autocommit());
    blocker.execute_batch("ROLLBACK;").unwrap();
    storage.close().unwrap();
}

#[test]
fn a_binding_stores_its_host_and_an_unknown_stored_host_is_invalid() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Hosted", Lifetime::Saved)
        .unwrap()
        .identity;
    let binding = storage
        .with_binding_transaction(|records| {
            records.insert_binding(&identity, &server(), &pane("%3", 99))
        })
        .unwrap();
    assert_eq!(binding.server.host, tmt_core::host::HostKind::Tmux);
    let connection = Connection::open(&fixture.database).unwrap();
    let transport: String = connection
        .query_row("SELECT transport FROM bindings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(transport, "tmux");
    // Another host's row reads back as that host, and a lookup on one host
    // never selects another host's pane.
    connection
        .execute("UPDATE bindings SET transport = 'herdr'", [])
        .unwrap();
    storage
        .with_binding_transaction(|records| {
            let entry = records.entry_by_id(&identity.id).unwrap().unwrap();
            assert_eq!(
                entry.binding.unwrap().server.host,
                tmt_core::host::HostKind::Herdr
            );
            assert!(
                records
                    .entry_by_pane(tmt_core::host::HostKind::Tmux, "%3", &server().server_id)
                    .unwrap()
                    .is_none(),
                "a tmux lookup never selects another host's pane"
            );
            Ok::<_, StorageError>(())
        })
        .unwrap();
    // A host this build does not know (written past the CHECK by a newer
    // schema or by hand) is invalid, never a guess.
    connection
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON; UPDATE bindings SET transport = 'screen';",
        )
        .unwrap();
    storage
        .with_binding_transaction(|records| {
            assert_invalid_pid(records.entry_by_id(&identity.id).unwrap_err());
            assert_invalid_pid(records.binding_entries().unwrap_err());
            Ok::<_, StorageError>(())
        })
        .unwrap();
    storage.close().unwrap();
}
