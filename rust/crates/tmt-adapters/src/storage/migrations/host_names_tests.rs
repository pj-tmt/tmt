use super::*;
use crate::test_support::TestDirectory;
use rusqlite::types::Value;

const TABLES: [&str; 4] = [
    "bindings",
    "request_attempts",
    "request_responses",
    "host_servers",
];
const SERVER: &str = "00000000-0000-4000-8000-000000000001";

/// A schema-40 database with a tmux binding, a pane request with its response
/// and notification, an inbox request and a Herdr server record.
pub(super) fn schema_40(path: &std::path::Path) -> Connection {
    let mut db = Connection::open(path).unwrap();
    test_support::seed_history(&mut db);
    apply_through(&mut db, 40).unwrap();
    db.execute_batch(&format!(
        "INSERT INTO bindings (id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at,runtime_state,last_transition,runtime_pid,runtime_start_identity,observed_provider_session_id,launch_owner_pid,launch_owner_start_identity)
         VALUES ('binding','old-id','tmux','%3','server','/tmp/fixture',41,'start',51,'bound','verified','running','started',61,'child-start','session',71,'owner-start');
         INSERT INTO request_attempts (attempt_id,request_id,route_kind,server_id,socket_path,server_pid,server_start_time,pane_id,pane_pid,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,host)
         VALUES ('pane-attempt','pane-request','pane','server','/tmp/fixture',41,'start','%3',51,0,'sent',0,0,1000,3601000,604801000,'herdr');
         INSERT INTO request_attempts (attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms)
         VALUES ('inbox-attempt','inbox-request','inbox','old-id',0,'queued',0,0,1000,3601000,604801000);
         INSERT INTO request_responses (request_id,attempt_id,server_id,socket_path,server_pid,server_start_time,pane_id,pane_pid,body,body_bytes,submitted_at_ms,host)
         VALUES ('pane-request','pane-attempt','server','/tmp/fixture',41,'start','%3',51,'done',4,2000,'herdr');
         INSERT INTO request_notifications (request_id,deadline_ms,timeout_ms) VALUES ('pane-request',5000,1000);
         INSERT INTO host_servers VALUES ('{SERVER}','herdr','/tmp/herdr.sock',81,'t0',900);"
    ))
    .unwrap();
    db
}

pub(super) fn version(db: &Connection) -> i64 {
    db.query_row("SELECT max(version) FROM _migrations", [], |row| row.get(0))
        .unwrap()
}

pub(super) fn foreign_keys(db: &Connection) -> bool {
    db.query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap()
}

pub(super) fn table_sql(db: &Connection, table: &str) -> String {
    db.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )
    .unwrap()
}

/// Words of a definition, ignoring quoting and layout.
fn words(sql: &str) -> Vec<String> {
    sql.replace('"', "")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// Every index and trigger on the four tables, by name and SQL.
fn dependents(db: &Connection) -> Vec<(String, String)> {
    let mut statement = db
        .prepare(
            "SELECT name, sql FROM sqlite_schema
             WHERE type IN ('index', 'trigger') AND sql IS NOT NULL
               AND tbl_name IN ('bindings', 'request_attempts', 'request_responses', 'host_servers')
             ORDER BY name",
        )
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

pub(super) fn rows(db: &Connection, table: &str) -> Vec<Vec<Value>> {
    let mut statement = db
        .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
        .unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| (0..width).map(|index| row.get(index)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Everything schema 41 must leave alone when it refuses or fails.
fn state(db: &Connection) -> Vec<String> {
    let mut state = vec![version(db).to_string()];
    for table in TABLES {
        state.push(table_sql(db, table));
        state.push(format!("{:?}", rows(db, table)));
    }
    state.push(format!("{:?}", dependents(db)));
    state.push(format!("{:?}", rows(db, "request_notifications")));
    state
}

pub(super) fn cursor(db: &Connection) -> i64 {
    db.query_row("SELECT value FROM change_cursor", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn the_host_upgrade_replaces_one_rule_per_table_and_keeps_rows_indexes_and_triggers() {
    let directory = TestDirectory::new();
    let mut db = schema_40(&directory.path.join("schema40.db"));
    let before_sql: Vec<String> = TABLES.iter().map(|table| table_sql(&db, table)).collect();
    let before_rows: Vec<_> = TABLES.iter().map(|table| rows(&db, table)).collect();
    let before_dependents = dependents(&db);

    apply_through(&mut db, 41).unwrap();

    assert_eq!(version(&db), 41);
    assert!(foreign_keys(&db));
    for (index, (table, old, new)) in host_names::TABLES.iter().enumerate() {
        assert_eq!(before_sql[index].matches(old).count(), 1, "{table}");
        assert_eq!(
            words(&table_sql(&db, table)),
            words(&before_sql[index].replacen(old, new, 1)),
            "{table}"
        );
        assert_eq!(rows(&db, table), before_rows[index], "{table}");
    }
    assert_eq!(dependents(&db), before_dependents);
    // The rebuilt database is exactly what a fresh one migrates to.
    host_names::validate_source(&db, &fresh_through(41).unwrap())
        .expect("same objects as a fresh schema 41");
}

#[test]
fn upgraded_tables_admit_any_host_name_and_keep_every_other_rule() {
    let directory = TestDirectory::new();
    let mut db = schema_40(&directory.path.join("schema40.db"));
    apply_through(&mut db, 41).unwrap();

    let before = cursor(&db);
    db.execute_batch(&format!(
        "UPDATE bindings SET transport = 'fake-host' WHERE id = 'binding';
         UPDATE request_attempts SET host = 'fake-host' WHERE attempt_id = 'pane-attempt';
         UPDATE request_responses SET host = 'fake-host' WHERE request_id = 'pane-request';
         INSERT INTO host_servers VALUES ('{}','fake-host','/tmp/fake.sock',82,'t1',901);",
        SERVER.replace("0001", "0002")
    ))
    .unwrap();
    assert_eq!(cursor(&db), before + 4, "the replayed triggers still count");

    for (statement, why) in [
        (
            "UPDATE bindings SET transport = 'Not a host'",
            "a binding host outside the grammar",
        ),
        (
            "UPDATE bindings SET transport = '1host'",
            "a host starting with a digit",
        ),
        (
            "UPDATE bindings SET transport = 'abcdefghijklmnopqrstuvwxyz0123456'",
            "a host over 32 characters",
        ),
        (
            "UPDATE request_attempts SET host = 'fake-host' WHERE attempt_id = 'inbox-attempt'",
            "a host on an inbox route",
        ),
        (
            "UPDATE request_responses SET host = 'fake_host'",
            "a response host outside the grammar",
        ),
        (
            "UPDATE host_servers SET host = 'tmux'",
            "tmux in host_servers",
        ),
    ] {
        assert!(db.execute_batch(statement).is_err(), "{why} was accepted");
    }

    // The notification still cascades from its rebuilt parent.
    assert_eq!(rows(&db, "request_notifications").len(), 1);
    db.execute_batch("DELETE FROM request_attempts WHERE attempt_id = 'pane-attempt'")
        .unwrap();
    assert_eq!(rows(&db, "request_notifications"), Vec::<Vec<Value>>::new());
}

#[test]
fn a_customized_host_table_is_refused_without_change() {
    for customization in [
        "ALTER TABLE bindings ADD COLUMN note TEXT",
        "CREATE INDEX host_servers_by_socket ON host_servers(socket_path)",
        "CREATE TRIGGER custom AFTER INSERT ON request_responses BEGIN SELECT 1; END",
        "CREATE VIEW recent AS SELECT * FROM request_attempts",
    ] {
        let directory = TestDirectory::new();
        let mut db = schema_40(&directory.path.join("schema40.db"));
        db.execute_batch(customization).unwrap();
        let before = state(&db);
        let error = apply_through(&mut db, 41).unwrap_err();
        assert_eq!(error.migration_version, Some(41), "{customization}");
        assert_eq!(state(&db), before, "{customization}");
        assert!(foreign_keys(&db), "{customization}");
    }
}

#[test]
fn a_broken_foreign_key_stops_the_upgrade_without_change() {
    let directory = TestDirectory::new();
    let mut db = schema_40(&directory.path.join("schema40.db"));
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute_batch(
        "INSERT INTO request_notifications (request_id,deadline_ms,timeout_ms) VALUES ('orphan',5000,1000)",
    )
    .unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let before = state(&db);

    let error = apply_through(&mut db, 41).unwrap_err();
    assert_eq!(error.migration_version, Some(41));
    assert_eq!(state(&db), before);
    assert!(foreign_keys(&db));
}

#[test]
fn a_failure_after_the_rebuild_rolls_back_to_schema_40() {
    let directory = TestDirectory::new();
    let mut db = schema_40(&directory.path.join("schema40.db"));
    db.execute_batch(
        "CREATE TRIGGER fail_41 BEFORE INSERT ON _migrations WHEN NEW.version = 41
         BEGIN SELECT RAISE(ABORT, 'injected'); END",
    )
    .unwrap();
    let before = state(&db);

    let error = apply_through(&mut db, 41).unwrap_err();
    assert_eq!(error.migration_version, Some(41));
    assert_eq!(state(&db), before);
    assert!(foreign_keys(&db));

    db.execute_batch("DROP TRIGGER fail_41").unwrap();
    apply_through(&mut db, 41).unwrap();
    assert_eq!(version(&db), 41);
    assert!(foreign_keys(&db));
}
