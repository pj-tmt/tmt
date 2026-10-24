//! Migration 66 rebuilds `request_responses` so a final can answer a remote request (#4536). The
//! rebuild must lose nothing, and a remote final must hold no pane endpoint, recipient or host.
use super::remote_route_tests::rows;
use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use rusqlite::types::Value;
use std::collections::BTreeMap;

const SEED: &str = "
INSERT INTO identities (id, name, canonical_name, created_at, updated_at)
  VALUES ('i1', 'Ana', 'ana', 'now', 'now');
INSERT INTO request_attempts (
    attempt_id, request_id, route_kind, server_id, socket_path, server_pid, server_start_time,
    pane_id, pane_pid, host, wait_active, status, inject_preamble, cadence_reserved,
    prepared_at_ms, expires_at_ms, retention_expires_at_ms
) VALUES ('a-pane', 'r-pane', 'pane', 's1', '/tmp/s', 11, 'start', '%1', 12, 'tmux', 0, 'sent',
    1, 0, 1000, 3601000, 604801000);
INSERT INTO request_attempts (
    attempt_id, request_id, route_kind, recipient_identity_id, wait_active, status,
    inject_preamble, cadence_reserved, prepared_at_ms, expires_at_ms, retention_expires_at_ms
) VALUES ('a-inbox', 'r-inbox', 'inbox', 'i1', 0, 'queued', 0, 0, 2000, 3602000, 604802000);
INSERT INTO request_responses (
    request_id, attempt_id, route_kind, server_id, socket_path, server_pid, server_start_time,
    pane_id, pane_pid, body, body_bytes, submitted_at_ms, response_expires_at_ms, host
) VALUES ('r-pane', 'a-pane', 'pane', 's1', '/tmp/s', 11, 'start', '%1', 12, 'pane reply', 10,
    1500, 604801500, 'tmux');
INSERT INTO request_responses (
    request_id, attempt_id, route_kind, route_recipient_identity_id, body, body_bytes,
    submitted_at_ms, response_expires_at_ms
) VALUES ('r-inbox', 'a-inbox', 'inbox', 'i1', 'the reply', 9, 2500, 604802500);
";

/// Name to SQL of the indexes and triggers on `request_responses`.
fn dependents(db: &Connection) -> BTreeMap<String, String> {
    rows(
        db,
        "SELECT name, sql FROM sqlite_schema WHERE tbl_name = 'request_responses'
         AND type IN ('index', 'trigger') AND sql IS NOT NULL",
    )
    .into_iter()
    .map(|row| match (&row[0], &row[1]) {
        (Value::Text(name), Value::Text(sql)) => (name.clone(), sql.clone()),
        other => panic!("{other:?}"),
    })
    .collect()
}

fn seeded(name: &str) -> (TestDirectory, std::path::PathBuf) {
    let directory = TestDirectory::new();
    let path = directory.path.join(name);
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 65).unwrap();
    old.execute_batch(SEED).unwrap();
    old.close().unwrap();
    (directory, path)
}

fn cursor(db: &Connection) -> i64 {
    db.query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
        row.get(0)
    })
    .unwrap()
}

/// Column name, type, nullability, default and key position, in table order.
fn columns(db: &Connection) -> Vec<Vec<Value>> {
    rows(
        db,
        "SELECT cid, name, type, \"notnull\", dflt_value, pk FROM pragma_table_info('request_responses')",
    )
}

#[test]
fn the_rebuild_keeps_every_column_final_index_and_trigger() {
    let (_directory, path) = seeded("schema65.db");
    let old = Connection::open(&path).unwrap();
    let before = rows(&old, "SELECT * FROM request_responses ORDER BY request_id");
    let before_columns = columns(&old);
    let before_dependents = dependents(&old);
    assert_eq!(before.len(), 2);
    assert_eq!(before_columns.len(), 15);
    assert_eq!(before_dependents.len(), 6, "3 indexes and 3 triggers");
    drop(old);

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 67);
    storage.close().unwrap();

    let db = Connection::open(&path).unwrap();
    assert_eq!(
        rows(&db, "SELECT * FROM request_responses ORDER BY request_id"),
        before,
        "rows are byte-identical"
    );
    assert_eq!(columns(&db), before_columns, "columns are 1:1");
    assert_eq!(
        dependents(&db),
        before_dependents,
        "indexes and triggers are equal"
    );
    assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());
    assert_eq!(
        rows(
            &db,
            "SELECT count(*) FROM sqlite_schema WHERE name = 'request_responses_v66'"
        ),
        vec![vec![Value::Integer(0)]]
    );
}

#[test]
fn a_remote_final_holds_no_pane_endpoint_recipient_or_host() {
    let (_directory, path) = seeded("columns.db");
    let mut storage = Storage::open(&path).unwrap();
    storage.close().unwrap();
    let db = Connection::open(&path).unwrap();
    let insert = |tag: &str, route_kind: &str, columns: &str, values: &str| {
        db.execute(
            &format!(
                "INSERT INTO request_responses (
                   request_id, attempt_id, route_kind, body, body_bytes, submitted_at_ms,
                   response_expires_at_ms{columns}
                 ) VALUES ('{tag}', 'a-{tag}', '{route_kind}', 'b', 1, 1, 2{values})"
            ),
            [],
        )
    };
    insert("ok", "remote", "", "").unwrap();

    for (columns, values) in [
        (", route_recipient_identity_id", ", 'i1'"),
        (", host", ", 'tmux'"),
        (", server_id", ", 's'"),
        (", socket_path", ", '/s'"),
        (", server_pid", ", 1"),
        (", server_start_time", ", 't'"),
        (", pane_id", ", '%1'"),
        (", pane_pid", ", 2"),
    ] {
        assert!(
            insert("bad", "remote", columns, values).is_err(),
            "{columns}"
        );
    }
    // The other two routes keep their own shape, and nothing else is a route.
    assert!(insert("pane-bare", "pane", "", "").is_err());
    assert!(insert("inbox-bare", "inbox", "", "").is_err());
    assert!(insert("other", "carrier-pigeon", "", "").is_err());
    assert_eq!(
        rows(
            &db,
            "SELECT route_kind FROM request_responses WHERE request_id = 'ok'"
        ),
        vec![vec![Value::Text("remote".into())]]
    );
}

#[test]
fn the_rebuilt_table_still_advances_the_change_cursor() {
    let (_directory, path) = seeded("cursor.db");
    let mut storage = Storage::open(&path).unwrap();
    storage.close().unwrap();
    let db = Connection::open(&path).unwrap();
    let start = cursor(&db);
    db.execute_batch(
        "INSERT INTO request_responses (
           request_id, attempt_id, route_kind, body, body_bytes, submitted_at_ms,
           response_expires_at_ms
         ) VALUES ('r-remote', 'a-remote', 'remote', 'b', 1, 1, 2);",
    )
    .unwrap();
    assert_eq!(cursor(&db), start + 1, "insert");
    db.execute(
        "UPDATE request_responses SET body = 'c' WHERE request_id = 'r-remote'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(&db), start + 2, "a change");
    db.execute(
        "UPDATE request_responses SET body = body WHERE request_id = 'r-remote'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(&db), start + 2, "a no-op update");
    db.execute(
        "DELETE FROM request_responses WHERE request_id = 'r-remote'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(&db), start + 3, "delete");
}

#[test]
fn a_failed_rebuild_changes_nothing() {
    let (_directory, path) = seeded("failing65.db");
    let old = Connection::open(&path).unwrap();
    let before = rows(&old, "SELECT * FROM request_responses ORDER BY request_id");
    let before_dependents = dependents(&old);
    let schema = rows(
        &old,
        "SELECT type, name, sql FROM sqlite_schema ORDER BY type, name",
    );
    // The migration's last step records it; refusing that record fails the whole rebuild.
    old.execute_batch("CREATE TRIGGER reject_66 BEFORE INSERT ON _migrations WHEN NEW.version=66 BEGIN SELECT RAISE(ABORT,'injected remote response migration failure'); END;").unwrap();
    drop(old);
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(66)
    );

    let db = Connection::open(&path).unwrap();
    db.execute_batch("DROP TRIGGER reject_66").unwrap();
    assert_eq!(
        rows(&db, "SELECT * FROM request_responses ORDER BY request_id"),
        before
    );
    assert_eq!(dependents(&db), before_dependents);
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
        vec![vec![Value::Integer(65)]]
    );

    // The same database then migrates, and nothing about the failed attempt lingers.
    drop(db);
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 67);
    storage.close().unwrap();
    assert_eq!(
        rows(
            &Connection::open(&path).unwrap(),
            "SELECT * FROM request_responses ORDER BY request_id"
        ),
        before
    );
}
