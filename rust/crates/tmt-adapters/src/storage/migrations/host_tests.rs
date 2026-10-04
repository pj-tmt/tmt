use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

/// A schema-38 database with one live tmux binding, one pane request with its
/// response and one inbox request.
fn schema_38(path: &std::path::Path) -> Connection {
    let mut old = Connection::open(path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..38].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch(
        "INSERT INTO bindings (id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at,runtime_state,last_transition,runtime_pid,runtime_start_identity,observed_provider_session_id,launch_owner_pid,launch_owner_start_identity)
         VALUES ('binding','old-id','tmux','%3','server','/tmp/fixture',41,'start',51,'bound','verified','running','started',61,'child-start','session',71,'owner-start');
         INSERT INTO request_attempts (attempt_id,request_id,route_kind,server_id,socket_path,server_pid,server_start_time,pane_id,pane_pid,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms)
         VALUES ('pane-attempt','pane-request','pane','server','/tmp/fixture',41,'start','%3',51,0,'sent',0,0,1000,3601000,604801000);
         INSERT INTO request_attempts (attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms)
         VALUES ('inbox-attempt','inbox-request','inbox','old-id',0,'queued',0,0,1000,3601000,604801000);
         INSERT INTO request_responses (request_id,attempt_id,server_id,socket_path,server_pid,server_start_time,pane_id,pane_pid,body,body_bytes,submitted_at_ms)
         VALUES ('pane-request','pane-attempt','server','/tmp/fixture',41,'start','%3',51,'done',4,2000);",
    )
    .unwrap();
    old
}

/// Schema 39's own rules, before schema 41 widens the host columns.
fn through_40(path: &std::path::Path) -> Connection {
    let mut db = Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    apply_through(&mut db, 40).unwrap();
    db
}

fn bindings_sql(connection: &Connection) -> String {
    connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'bindings'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn binding_row(connection: &Connection) -> Vec<rusqlite::types::Value> {
    connection
        .query_row("SELECT * FROM bindings WHERE id = 'binding'", [], |row| {
            (0..18).map(|index| row.get(index)).collect()
        })
        .unwrap()
}

#[test]
fn host_upgrade_widens_only_the_transport_check_and_keeps_every_row() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema38.db");
    let old = schema_38(&path);
    let before_sql = bindings_sql(&old);
    let before_row = binding_row(&old);
    old.close().unwrap();

    let db = through_40(&path);

    // Only the table name's quoting (from the rename) and the transport
    // CHECK differ from the schema-38 definition.
    let expected = before_sql.replace(
        "CHECK (transport = 'tmux')",
        "CHECK (transport IN ('tmux', 'herdr'))",
    );
    let after_sql = bindings_sql(&db).replace("CREATE TABLE \"bindings\"", "CREATE TABLE bindings");
    assert!(
        after_sql.split_whitespace().eq(expected.split_whitespace()),
        "{after_sql}"
    );
    assert_eq!(binding_row(&db), before_row);
    let index: String = db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'bindings_endpoint'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        index,
        "CREATE INDEX bindings_endpoint ON bindings(transport, server_id, pane_id)"
    );

    // Old request fences are tmux (NULL); history is not rewritten.
    let hosts: Vec<Option<String>> = db
        .prepare("SELECT host FROM request_attempts ORDER BY attempt_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(hosts, [None, None]);
    let response_host: Option<String> = db
        .query_row("SELECT host FROM request_responses", [], |row| row.get(0))
        .unwrap();
    assert_eq!(response_host, None);
    assert_eq!(
        db.query_row("SELECT count(*) FROM host_servers", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn upgraded_bindings_and_fences_keep_every_rule() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema38.db");
    schema_38(&path).close().unwrap();
    let db = through_40(&path);
    db.execute_batch(
        "INSERT INTO identities (id,name,canonical_name,created_at,updated_at) VALUES ('second','Bob','bob','created','updated');",
    )
    .unwrap();
    let columns = "id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at";
    let insert = |values: &str| {
        db.execute(
            &format!("INSERT INTO bindings ({columns}) VALUES ({values})"),
            [],
        )
    };
    // The same server and pane IDs on another host are another endpoint.
    insert("'herdr','second','herdr','%3','server','/tmp/h',1,'s',2,'b','v'").unwrap();
    db.execute("DELETE FROM bindings WHERE id = 'herdr'", [])
        .unwrap();
    for (values, rule) in [
        (
            "'x','second','screen','%9','server','/tmp/h',1,'s',2,'b','v'",
            "an unknown host",
        ),
        (
            "'x','second','tmux','%3','server','/tmp/h',1,'s',2,'b','v'",
            "an occupied endpoint",
        ),
        (
            "'x','old-id','herdr','%9','server','/tmp/h',1,'s',2,'b','v'",
            "a second binding",
        ),
        (
            "'x','missing','herdr','%9','server','/tmp/h',1,'s',2,'b','v'",
            "a missing identity",
        ),
    ] {
        assert!(insert(values).is_err(), "{rule} was accepted");
    }
    for statement in [
        "UPDATE bindings SET runtime_state = 'lost'",
        "UPDATE bindings SET runtime_pid = NULL, runtime_start_identity = NULL",
        "UPDATE bindings SET launch_owner_start_identity = NULL",
    ] {
        assert!(db.execute(statement, []).is_err(), "{statement}");
    }
    db.execute(
        "UPDATE request_attempts SET host = 'herdr' WHERE route_kind = 'pane'",
        [],
    )
    .unwrap();
    db.execute("UPDATE request_responses SET host = 'herdr'", [])
        .unwrap();
    for statement in [
        "UPDATE request_attempts SET host = 'tmux' WHERE route_kind = 'inbox'",
        "UPDATE request_attempts SET host = 'screen' WHERE route_kind = 'pane'",
        "UPDATE request_responses SET host = 'screen'",
    ] {
        assert!(db.execute(statement, []).is_err(), "{statement}");
    }
    // Deleting the identity still cascades to its binding.
    db.execute("DELETE FROM identities WHERE id = 'old-id'", [])
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM bindings", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn a_failed_host_upgrade_leaves_schema_38_intact() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema38.db");
    let old = schema_38(&path);
    let before_sql = bindings_sql(&old);
    let before_row = binding_row(&old);
    old.execute_batch(
        "CREATE TRIGGER reject_host_migration BEFORE INSERT ON _migrations WHEN NEW.version=39
         BEGIN SELECT RAISE(ABORT, 'injected host migration failure'); END;",
    )
    .unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(39)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(bindings_sql(&oracle), before_sql);
    assert_eq!(binding_row(&oracle), before_row);
    for (table, column) in [("request_attempts", "host"), ("request_responses", "host")] {
        let present: bool = oracle
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name = '{column}')"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!present, "{table}.{column}");
    }
    let tables: i64 = oracle
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name IN ('host_servers', 'bindings_039')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    oracle
        .execute_batch("DROP TRIGGER reject_host_migration;")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 47);
    storage.close().unwrap();
}

#[test]
fn the_rebuild_refuses_a_customized_bindings_table_or_its_dependents() {
    for customization in [
        "ALTER TABLE bindings ADD COLUMN note TEXT;",
        "CREATE TRIGGER audit_bindings AFTER INSERT ON bindings BEGIN SELECT 1; END;",
        "CREATE VIEW bound_panes AS SELECT pane_id FROM bindings;",
    ] {
        let directory = TestDirectory::new();
        let path = directory.path.join("schema38.db");
        let old = schema_38(&path);
        old.execute_batch(customization).unwrap();
        let before_sql = bindings_sql(&old);
        old.close().unwrap();
        let error = Storage::open(&path).err().unwrap();
        assert_eq!(error.migration_version, Some(39), "{customization}");
        let oracle = Connection::open(&path).unwrap();
        assert_eq!(bindings_sql(&oracle), before_sql, "{customization}");
        assert_eq!(binding_row(&oracle).len(), 18);
        assert_eq!(
            oracle
                .query_row("SELECT max(version) FROM _migrations", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            38
        );
    }
}
