//! Migration 65 rebuilds `request_attempts` so a request can have a remote route (#4536). The
//! rebuild must lose nothing: every row, every dependent row, every index and trigger.
use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use rusqlite::types::Value;
use std::collections::BTreeMap;

/// The columns `request_attempts` had before 65, in table order.
const OLD_COLUMNS: &str = "attempt_id, request_id, nonce, identity_id, route_kind, server_id,
    socket_path, server_pid, server_start_time, pane_id, pane_pid, wait_active, status,
    preamble_every, inject_preamble, cadence_reserved, prepared_at_ms, sending_at_ms,
    settled_at_ms, wait_released_at_ms, expires_at_ms, response_submitted_at_ms, retention_days,
    retention_expires_at_ms, originator_kind, originator_identity_id, recipient_identity_id,
    message_text, message_bytes, message_expires_at_ms, attention_revision,
    attention_acknowledged_revision, recipient_attention_revision,
    recipient_attention_acknowledged_revision, request_kind, room_id, wake_state, host,
    withdrawn_at_ms, withdrawal_reason, origin";

/// Tables that reference `request_attempts`, and the table itself in its pre-65 columns.
const CHILDREN: [&str; 4] = [
    "request_delivery_policies",
    "request_notifications",
    "request_passthrough",
    "focus_items",
];

const SEED: &str = "
INSERT INTO identities (id, name, canonical_name, created_at, updated_at)
  VALUES ('i1', 'Ana', 'ana', 'now', 'now'), ('i2', 'Bo', 'bo', 'now', 'now');
INSERT INTO request_attempts (
    attempt_id, request_id, route_kind, server_id, socket_path, server_pid, server_start_time,
    pane_id, pane_pid, host, wait_active, status, inject_preamble, cadence_reserved,
    prepared_at_ms, expires_at_ms, retention_expires_at_ms, originator_kind,
    originator_identity_id, message_text, message_bytes, message_expires_at_ms,
    attention_revision, attention_acknowledged_revision
) VALUES ('a-pane', 'r-pane', 'pane', 's1', '/tmp/s', 11, 'start', '%1', 12, 'tmux', 1, 'sent',
    1, 0, 1000, 3601000, 604801000, 'explicit', 'i2', 'to a pane', 9, 604801000, 3, 2);
INSERT INTO request_attempts (
    attempt_id, request_id, route_kind, recipient_identity_id, wait_active, status,
    inject_preamble, cadence_reserved, prepared_at_ms, expires_at_ms, retention_expires_at_ms,
    originator_kind, originator_identity_id, message_text, message_bytes, message_expires_at_ms,
    attention_revision, recipient_attention_revision, wake_state, origin
) VALUES ('a-inbox', 'r-inbox', 'inbox', 'i1', 0, 'queued', 0, 0, 2000, 3602000, 604802000,
    'explicit', 'i2', 'to an inbox', 11, 604802000, 5, 4, 'sent', 'person');
INSERT INTO request_attempts (
    attempt_id, request_id, route_kind, recipient_identity_id, wait_active, status,
    inject_preamble, cadence_reserved, prepared_at_ms, expires_at_ms, retention_expires_at_ms,
    message_text, message_bytes, message_expires_at_ms, request_kind
) VALUES ('a-note', 'r-note', 'inbox', 'i1', 0, 'queued', 0, 0, 3000, 3603000, 604803000,
    'an announcement', 15, 604803000, 'announcement');
INSERT INTO request_responses (
    request_id, attempt_id, route_kind, route_recipient_identity_id, body, body_bytes,
    submitted_at_ms, response_expires_at_ms
) VALUES ('r-inbox', 'a-inbox', 'inbox', 'i1', 'the reply', 9, 2500, 604802500);
INSERT INTO request_delivery_policies (request_id, urgent, kind, automatic, mode)
  VALUES ('r-inbox', 0, 'fyi', 1, 'request'), ('r-note', 1, 'decision', 0, 'passthrough');
INSERT INTO request_notifications (request_id, deadline_ms, timeout_ms, waiter_pid, waiter_start)
  VALUES ('r-pane', 90000, 1000, 77, 'waiter-start');
INSERT INTO request_passthrough (request_id, delivered_at_ms, state)
  VALUES ('r-note', 3100, 'pending');
INSERT INTO focus_items (identity_id, request_id, kind, source, created_at_ms)
  VALUES ('i1', 'r-inbox', 'fyi', 'incoming', 2600);
";

pub(super) fn rows(db: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = db.prepare(sql).unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|index| row.get::<_, Value>(index))
                .collect()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Everything 65 must keep byte-for-byte.
struct Snapshot {
    attempts: Vec<Vec<Value>>,
    children: Vec<Vec<Vec<Value>>>,
    responses: Vec<Vec<Value>>,
    /// Name to SQL of the indexes and triggers on `request_attempts`.
    dependents: BTreeMap<String, String>,
    /// Name to SQL of every table that references `request_attempts`.
    child_tables: BTreeMap<String, String>,
}

fn snapshot(db: &Connection) -> Snapshot {
    let text = |sql: &str| -> BTreeMap<String, String> {
        rows(db, sql)
            .into_iter()
            .map(|row| match (&row[0], &row[1]) {
                (Value::Text(name), Value::Text(sql)) => (name.clone(), sql.clone()),
                other => panic!("{other:?}"),
            })
            .collect()
    };
    Snapshot {
        attempts: rows(
            db,
            &format!("SELECT {OLD_COLUMNS} FROM request_attempts ORDER BY attempt_id"),
        ),
        children: CHILDREN
            .iter()
            .map(|table| rows(db, &format!("SELECT * FROM {table} ORDER BY 1, 2")))
            .collect(),
        responses: rows(db, "SELECT * FROM request_responses ORDER BY request_id"),
        dependents: text(
            "SELECT name, sql FROM sqlite_schema WHERE tbl_name = 'request_attempts'
             AND type IN ('index', 'trigger') AND sql IS NOT NULL",
        ),
        child_tables: text(&format!(
            "SELECT name, sql FROM sqlite_schema WHERE type = 'table' AND name IN ({})",
            CHILDREN.map(|table| format!("'{table}'")).join(", ")
        )),
    }
}

fn seeded(name: &str) -> (TestDirectory, std::path::PathBuf) {
    let directory = TestDirectory::new();
    let path = directory.path.join(name);
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 64).unwrap();
    old.execute_batch(SEED).unwrap();
    old.close().unwrap();
    (directory, path)
}

#[test]
fn the_rebuild_keeps_every_row_dependent_row_index_and_trigger() {
    let (_directory, path) = seeded("schema64.db");
    let old = Connection::open(&path).unwrap();
    let before = snapshot(&old);
    assert_eq!(before.attempts.len(), 3);
    assert!(
        before.children.iter().all(|rows| !rows.is_empty()),
        "every child has a row"
    );
    assert_eq!(before.dependents.len(), 18, "15 indexes and 3 triggers");
    drop(old);

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 68);
    storage.close().unwrap();

    let db = Connection::open(&path).unwrap();
    let after = snapshot(&db);
    assert_eq!(after.attempts, before.attempts, "rows are byte-identical");
    assert_eq!(
        after.children, before.children,
        "dependent rows are untouched"
    );
    assert_eq!(after.responses, before.responses);
    assert_eq!(
        after.child_tables, before.child_tables,
        "children still reference the table"
    );
    // Every index and trigger is back by name and SQL; the update trigger also watches the new columns.
    assert_eq!(
        after.dependents.keys().collect::<Vec<_>>(),
        before.dependents.keys().collect::<Vec<_>>()
    );
    for (name, sql) in &before.dependents {
        let rebuilt = &after.dependents[name];
        if name == "request_attempts_advances_change_cursor_on_update" {
            let added = "   OR OLD.remote_ref IS NOT NEW.remote_ref\n   OR OLD.remote_target_id IS NOT NEW.remote_target_id\n   OR OLD.remote_target_name IS NOT NEW.remote_target_name\n   OR OLD.remote_sender_name IS NOT NEW.remote_sender_name\n";
            assert_eq!(&rebuilt.replace(added, ""), sql, "{name}");
            assert_ne!(rebuilt, sql);
        } else {
            assert_eq!(rebuilt, sql, "{name}");
        }
    }
    assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());
    assert_eq!(
        rows(
            &db,
            "SELECT count(*) FROM sqlite_schema WHERE name = 'request_attempts_v65'"
        ),
        vec![vec![Value::Integer(0)]]
    );

    // The children's REFERENCES resolve to the rebuilt table: deleting a request still cascades.
    db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    db.execute(
        "DELETE FROM request_attempts WHERE request_id = 'r-note'",
        [],
    )
    .unwrap();
    for table in ["request_delivery_policies", "request_passthrough"] {
        assert_eq!(
            rows(
                &db,
                &format!("SELECT count(*) FROM {table} WHERE request_id = 'r-note'")
            ),
            vec![vec![Value::Integer(0)]],
            "{table}"
        );
    }
    assert!(
        db.execute(
            "INSERT INTO request_passthrough (request_id, delivered_at_ms, state) VALUES ('nobody', 1, 'pending')",
            []
        )
        .is_err(),
        "the reference is still enforced"
    );
}

#[test]
fn a_remote_route_has_exactly_its_own_columns_and_nothing_else_does() {
    let (_directory, path) = seeded("remote64.db");
    let mut storage = Storage::open(&path).unwrap();
    storage.close().unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    let cursor = || -> i64 {
        db.query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let insert = |id: &str, route: &str, columns: &str, values: &str| {
        db.execute(
            &format!(
                "INSERT INTO request_attempts (attempt_id, request_id, route_kind, wait_active,
                 status, inject_preamble, cadence_reserved, prepared_at_ms, expires_at_ms,
                 retention_expires_at_ms, originator_kind, originator_identity_id {columns})
                 VALUES ('a-{id}', 'r-{id}', '{route}', 0, 'queued', 0, 0, 5000, 3605000,
                 604805000, 'explicit', 'i1' {values})"
            ),
            [],
        )
    };
    let before = cursor();
    insert(
        "remote",
        "remote",
        ", remote_ref, remote_target_id, remote_target_name, remote_sender_name",
        ", 'bob', '7f0c2f2e-2b1a-4c1e-9d2a-0a1b2c3d4e5f', 'reviewer', 'ana'",
    )
    .unwrap();
    insert(
        "remote-name",
        "remote",
        ", remote_ref, remote_target_name, remote_sender_name",
        ", 'bob', 'reviewer', 'ana'",
    )
    .unwrap();
    assert_eq!(cursor(), before + 2, "an insert advances the change cursor");
    db.execute(
        "UPDATE request_attempts SET remote_target_name = 'lead' WHERE request_id = 'r-remote'",
        [],
    )
    .unwrap();
    assert_eq!(
        cursor(),
        before + 3,
        "a change to a route column advances it too"
    );

    let invalid: [(&str, &str, &str, &str); 11] = [
        (
            "no-ref",
            "remote",
            ", remote_target_name, remote_sender_name",
            ", 'reviewer', 'ana'",
        ),
        (
            "no-target",
            "remote",
            ", remote_ref, remote_sender_name",
            ", 'bob', 'ana'",
        ),
        (
            "no-sender",
            "remote",
            ", remote_ref, remote_target_name",
            ", 'bob', 'reviewer'",
        ),
        (
            "empty-ref",
            "remote",
            ", remote_ref, remote_target_name, remote_sender_name",
            ", '', 'reviewer', 'ana'",
        ),
        (
            "long-name",
            "remote",
            ", remote_ref, remote_target_name, remote_sender_name",
            format!(", 'bob', '{}', 'ana'", "x".repeat(129)).leak(),
        ),
        (
            "short-id",
            "remote",
            ", remote_ref, remote_target_id, remote_target_name, remote_sender_name",
            ", 'bob', 'not-a-uuid', 'reviewer', 'ana'",
        ),
        (
            "uppercase-id",
            "remote",
            ", remote_ref, remote_target_id, remote_target_name, remote_sender_name",
            ", 'bob', '7F0C2F2E-2B1A-4C1E-9D2A-0A1B2C3D4E5F', 'reviewer', 'ana'",
        ),
        (
            "non-hex-id",
            "remote",
            ", remote_ref, remote_target_id, remote_target_name, remote_sender_name",
            ", 'bob', 'zzzzzzzz-2b1a-4c1e-9d2a-0a1b2c3d4e5f', 'reviewer', 'ana'",
        ),
        (
            "with-recipient",
            "remote",
            ", recipient_identity_id, remote_ref, remote_target_name, remote_sender_name",
            ", 'i2', 'bob', 'reviewer', 'ana'",
        ),
        (
            "with-pane",
            "remote",
            ", server_id, socket_path, server_pid, server_start_time, pane_id, pane_pid, remote_ref, remote_target_name, remote_sender_name",
            ", 's', '/s', 1, 't', '%1', 2, 'bob', 'reviewer', 'ana'",
        ),
        (
            "inbox-with-remote",
            "inbox",
            ", recipient_identity_id, remote_ref",
            ", 'i2', 'bob'",
        ),
    ];
    for (id, route, columns, values) in invalid {
        assert!(insert(id, route, columns, values).is_err(), "{id}");
    }
    // A remote route needs a confirmed sender: an anonymous (unknown) originator is refused, a
    // verified one is as good as an explicit one.
    let anonymous = |kind: &str, identity: &str| {
        db.execute(
            &format!(
                "INSERT INTO request_attempts (attempt_id, request_id, route_kind, wait_active,
                 status, inject_preamble, cadence_reserved, prepared_at_ms, expires_at_ms,
                 retention_expires_at_ms, originator_kind, originator_identity_id,
                 remote_ref, remote_target_name, remote_sender_name)
                 VALUES ('a-{kind}', 'r-{kind}', 'remote', 0, 'queued', 0, 0, 5000, 3605000,
                 604805000, '{kind}', {identity}, 'bob', 'reviewer', 'ana')"
            ),
            [],
        )
    };
    assert!(anonymous("unknown", "NULL").is_err());
    anonymous("verified", "'i1'").unwrap();
    // Neither does a pane route carry one, and an announcement is never remote.
    assert!(
        insert(
            "pane-with-remote",
            "pane",
            ", server_id, socket_path, server_pid, server_start_time, pane_id, pane_pid, remote_target_id",
            ", 's', '/s', 1, 't', '%1', 2, '7f0c2f2e-2b1a-4c1e-9d2a-0a1b2c3d4e5f'"
        )
        .is_err()
    );
    assert!(
        insert(
            "remote-note",
            "remote",
            ", request_kind, remote_ref, remote_target_name, remote_sender_name",
            ", 'announcement', 'bob', 'reviewer', 'ana'"
        )
        .is_err()
    );
    assert_eq!(
        rows(
            &db,
            "SELECT count(*) FROM request_attempts WHERE route_kind = 'remote'"
        ),
        vec![vec![Value::Integer(3)]]
    );
}

#[test]
fn a_failed_rebuild_changes_nothing() {
    let (_directory, path) = seeded("failing64.db");
    let old = Connection::open(&path).unwrap();
    let before = snapshot(&old);
    let schema = rows(
        &old,
        "SELECT type, name, sql FROM sqlite_schema ORDER BY type, name",
    );
    // The migration's last step records it; refusing that record fails the whole rebuild.
    old.execute_batch("CREATE TRIGGER reject_65 BEFORE INSERT ON _migrations WHEN NEW.version=65 BEGIN SELECT RAISE(ABORT,'injected remote route migration failure'); END;").unwrap();
    drop(old);
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(65)
    );

    let db = Connection::open(&path).unwrap();
    db.execute_batch("DROP TRIGGER reject_65").unwrap();
    let after = snapshot(&db);
    assert_eq!(after.attempts, before.attempts);
    assert_eq!(after.children, before.children);
    assert_eq!(after.responses, before.responses);
    assert_eq!(after.dependents, before.dependents);
    assert_eq!(
        rows(
            &db,
            "SELECT type, name, sql FROM sqlite_schema ORDER BY type, name"
        ),
        schema,
        "no leftover table, index or trigger"
    );
    assert_eq!(
        rows(&db, "SELECT max(version) FROM _migrations"),
        vec![vec![Value::Integer(64)]]
    );
    assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());

    // The same database then migrates, and nothing about the failed attempt lingers.
    drop(db);
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 68);
    storage.close().unwrap();
    assert_eq!(
        snapshot(&Connection::open(&path).unwrap()).attempts,
        before.attempts
    );
}
