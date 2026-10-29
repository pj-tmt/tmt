//! Provider admission must follow the shipped runtime registry without losing captured history.
use super::remote_route_tests::rows;
use super::*;
use crate::drivers::Registry;
use rusqlite::types::Value;

const TABLES: &[&str] = &[
    "recent_responses",
    "request_passthrough",
    "recent_response_floor",
    "capture_batches",
    "capture_entries",
    "capture_acknowledgements",
    "capture_floor",
    "sqlite_sequence",
    "change_cursor",
];

fn seed(db: &mut Connection) {
    db.pragma_update(None, "foreign_keys", true).unwrap();
    apply_through(db, 78).unwrap();
    db.execute_batch("INSERT INTO identities(id,name,canonical_name,created_at,updated_at) VALUES ('i','Agent','agent','now','now');
        INSERT INTO recent_responses(sequence,identity_id,text,ended_ms,provider,truncated,session_id,expires_ms,original_bytes)
        VALUES (41,'i','first',100,'claude',0,'session',900,5),(72,'i','cut',200,'codex',1,NULL,1200,19000);
        INSERT INTO recent_responses(sequence,identity_id,text,ended_ms,provider,truncated,expires_ms,original_bytes)
        VALUES(900,'i','pruned',300,'claude',0,1300,6);
        DELETE FROM recent_responses WHERE sequence=900;
        INSERT INTO recent_response_floor VALUES('i',300);
        INSERT INTO capture_batches VALUES('batch','i','binding','claude','session',1,'start',NULL,80,300);
        INSERT INTO capture_entries(sequence,identity_id,batch_id,kind,response_sequence,created_ms) VALUES(11,'i','batch','final',41,100),(12,'i','batch','final',72,200);
        INSERT INTO capture_acknowledgements VALUES(11,'answered','i',2);
        INSERT INTO capture_floor VALUES('i',10);").unwrap();
    for state in ["pending", "bound", "answered", "no_answer"] {
        db.execute("INSERT INTO request_attempts(attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms) VALUES(?1,?1,'inbox','i',0,'queued',0,0,1,2000,3000)", [state]).unwrap();
    }
    db.execute_batch("INSERT INTO request_passthrough(request_id,delivered_at_ms,state) VALUES('pending',50,'pending');
        INSERT INTO request_passthrough(request_id,delivered_at_ms,state,bound_session,bound_turn,bound_at_ms,folded) VALUES('bound',51,'bound','session','turn',60,1);
        INSERT INTO request_passthrough VALUES('answered',52,'answered','session','turn',60,'first',0,'claude',100,100,1,41);
        INSERT INTO request_passthrough(request_id,delivered_at_ms,state,settled_at_ms) VALUES('no_answer',53,'no_answer',100);").unwrap();
}

fn snapshot(db: &Connection) -> Vec<Vec<Vec<Value>>> {
    TABLES
        .iter()
        .map(|table| rows(db, &format!("SELECT * FROM {table} ORDER BY 1")))
        .collect()
}

fn schema(db: &Connection, table: &str) -> Vec<Vec<Vec<Value>>> {
    [
        format!("SELECT cid,name,type,\"notnull\",dflt_value,pk FROM pragma_table_info('{table}')"),
        format!("SELECT * FROM pragma_foreign_key_list('{table}')"),
        format!("SELECT name,sql FROM sqlite_schema WHERE tbl_name='{table}' AND type IN ('index','trigger') ORDER BY name"),
    ].iter().map(|sql| rows(db, sql)).collect()
}

fn response(db: &Connection, provider: &str) -> rusqlite::Result<usize> {
    db.execute("INSERT INTO recent_responses(identity_id,text,ended_ms,provider,truncated,expires_ms,original_bytes) VALUES('i','agy final',400,?1,0,1400,9)", [provider])
}

#[test]
fn the_provider_rebuild_preserves_rows_ids_high_water_indexes_foreign_keys_and_triggers() {
    let mut db = Connection::open_in_memory().unwrap();
    seed(&mut db);
    let before = snapshot(&db);
    let schemas: Vec<_> = TABLES[..2].iter().map(|table| schema(&db, table)).collect();
    assert!(response(&db, "agy").is_err(), "the predecessor rejects agy");
    assert!(
        db.execute(
            "UPDATE request_passthrough SET answer_provider='agy' WHERE request_id='answered'",
            []
        )
        .is_err()
    );
    apply_through(&mut db, 79).unwrap();
    assert_eq!(
        snapshot(&db),
        before,
        "no row, retention, reference, sequence or cursor changed"
    );
    for (table, expected) in TABLES[..2].iter().zip(schemas) {
        assert_eq!(schema(&db, table), expected, "{table}");
    }
    assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());
    let before_cursor: i64 = db
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    response(&db, "agy").unwrap();
    assert_eq!(
        db.last_insert_rowid(),
        901,
        "pruned response IDs are never reused"
    );
    db.execute(
        "UPDATE request_passthrough SET answer_provider='agy' WHERE request_id='answered'",
        [],
    )
    .unwrap();
    assert_eq!(
        rows(&db, "SELECT value FROM change_cursor"),
        vec![vec![Value::Integer(before_cursor + 2)]]
    );
    // The parent FK still cascades; the final's identity FK still refuses an orphan.
    db.execute(
        "DELETE FROM request_attempts WHERE request_id='answered'",
        [],
    )
    .unwrap();
    assert!(
        rows(
            &db,
            "SELECT * FROM request_passthrough WHERE request_id='answered'"
        )
        .is_empty()
    );
    assert!(
        db.execute("UPDATE recent_responses SET identity_id='missing'", [])
            .is_err()
    );
}

#[test]
fn an_empty_pruned_table_keeps_its_high_water_and_a_fresh_table_starts_at_one() {
    for pruned in [false, true] {
        let mut db = Connection::open_in_memory().unwrap();
        seed(&mut db);
        db.execute("DELETE FROM recent_responses", []).unwrap();
        if !pruned {
            db.execute(
                "DELETE FROM sqlite_sequence WHERE name='recent_responses'",
                [],
            )
            .unwrap();
        }
        apply_through(&mut db, 79).unwrap();
        response(&db, "agy").unwrap();
        assert_eq!(db.last_insert_rowid(), if pruned { 901 } else { 1 });
        assert_eq!(
            rows(
                &db,
                "SELECT count(*) FROM sqlite_sequence WHERE name='recent_responses'"
            ),
            vec![vec![Value::Integer(1)]]
        );
    }
}

#[test]
fn a_refused_migration_record_rolls_back_both_rebuilds_and_can_be_retried() {
    let mut db = Connection::open_in_memory().unwrap();
    seed(&mut db);
    let before = snapshot(&db);
    let old_schema = rows(
        &db,
        "SELECT type,name,sql FROM sqlite_schema ORDER BY type,name",
    );
    db.execute_batch("CREATE TRIGGER refuse_capture_migration BEFORE INSERT ON _migrations WHEN NEW.version=79 BEGIN SELECT RAISE(ABORT,'forced capture migration rollback'); END;").unwrap();
    assert_eq!(
        apply_through(&mut db, 79).unwrap_err().migration_version,
        Some(79)
    );
    assert_eq!(snapshot(&db), before);
    db.execute_batch("DROP TRIGGER refuse_capture_migration")
        .unwrap();
    assert_eq!(
        rows(
            &db,
            "SELECT type,name,sql FROM sqlite_schema ORDER BY type,name"
        ),
        old_schema
    );
    assert!(rows(&db, "SELECT name FROM sqlite_temp_schema").is_empty());
    assert_eq!(
        rows(&db, "SELECT max(version) FROM _migrations"),
        vec![vec![Value::Integer(78)]]
    );
    assert!(response(&db, "agy").is_err());
    assert_eq!(
        rows(&db, "PRAGMA foreign_keys"),
        vec![vec![Value::Integer(1)]]
    );
    apply_through(&mut db, 79).unwrap();
    assert_eq!(snapshot(&db), before);
    response(&db, "agy").unwrap();
}

#[test]
fn both_capture_provider_checks_equal_the_shipped_full_runtime_registry() {
    let mut db = Connection::open_in_memory().unwrap();
    seed(&mut db);
    apply(&mut db).unwrap();
    let registry = Registry::builtin();
    let mut expected: Vec<_> = registry
        .iter()
        .filter(|driver| driver.runtime.is_some() && driver.descriptor.hooks.is_some())
        .map(|driver| driver.name())
        .collect();
    expected.sort_unstable();
    for table in TABLES[..2].iter() {
        let sql: String = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        let check = sql
            .split("provider IN (")
            .nth(1)
            .expect("a bounded provider CHECK")
            .split(')')
            .next()
            .unwrap();
        let mut admitted: Vec<_> = check
            .split(',')
            .map(|token| token.trim().trim_matches('\''))
            .collect();
        admitted.sort_unstable();
        assert_eq!(
            admitted, expected,
            "{table}: SQL admission must equal the shipped full runtimes"
        );
    }
    for driver in registry.iter() {
        let capture_capable = driver.runtime.is_some() && driver.descriptor.hooks.is_some();
        assert_eq!(
            response(&db, driver.name()).is_ok(),
            capture_capable,
            "recent_responses: {}",
            driver.name()
        );
        assert_eq!(
            db.execute(
                "UPDATE request_passthrough SET answer_provider=?1 WHERE request_id='answered'",
                [driver.name()]
            )
            .is_ok(),
            capture_capable,
            "request_passthrough: {}",
            driver.name()
        );
    }
    for unsupported in ["", "unknown", "AGY", "claude-next"] {
        assert!(response(&db, unsupported).is_err(), "{unsupported:?}");
        assert!(
            db.execute(
                "UPDATE request_passthrough SET answer_provider=?1 WHERE request_id='answered'",
                [unsupported]
            )
            .is_err(),
            "{unsupported:?}"
        );
    }
}
