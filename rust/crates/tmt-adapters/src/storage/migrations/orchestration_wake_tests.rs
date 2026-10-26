//! Pending wake state is small, restart-safe, and independent of historical checklist bodies.
use super::remote_route_tests::rows;
use super::*;
use rusqlite::types::Value;

const OLD_COLUMNS: &str = "id,identity_id,attempt_token,through_sequence,state,created_at_ms,opportunity,sealed_count,settled_at_ms,settlement_sequence,confirmed_at_ms,orchestration_context";
fn predecessor() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    apply_through(&mut db, 80).unwrap();
    db.execute_batch("INSERT INTO identities(id,name,canonical_name,created_at,updated_at) VALUES('i','First','first','now','now'),('j','Next','next','now','now');
        INSERT INTO focus_checklists(id,identity_id,attempt_token,through_sequence,state,created_at_ms,opportunity,sealed_count,settled_at_ms,settlement_sequence,confirmed_at_ms)
        VALUES('legacy','i','token',9,'delivered',100,'idle',3,110,7,120);").unwrap();
    db
}
fn cursor(db: &Connection) -> i64 {
    db.query_row("SELECT value FROM change_cursor", [], |row| row.get(0))
        .unwrap()
}
fn originals(db: &Connection) -> Vec<Vec<Value>> {
    rows(
        db,
        &format!("SELECT {OLD_COLUMNS} FROM focus_checklists ORDER BY id"),
    )
}

#[test]
fn pending_wake_migration_preserves_full_checklists_and_creates_no_synthetic_wake_or_clock() {
    let mut db = predecessor();
    let before = originals(&db);
    let before_cursor = cursor(&db);
    apply(&mut db).unwrap();
    assert_eq!(originals(&db), before);
    assert_eq!(
        cursor(&db),
        before_cursor,
        "schema admission is not wake admission"
    );
    assert_eq!(
        rows(
            &db,
            "SELECT projection,summary_urgent_count FROM focus_checklists"
        ),
        vec![vec![Value::Text("full".into()), Value::Null]]
    );
    for table in ["orchestration_pending_wakes", "orchestration_wake_clock"] {
        assert!(
            rows(&db, &format!("SELECT * FROM {table}")).is_empty(),
            "{table}"
        );
    }
    assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());
    // The journal remains on demand; scheduler state must not add its cursor triggers back.
    assert!(rows(&db, "SELECT name FROM sqlite_schema WHERE type='trigger' AND tbl_name='orchestration_decisions'").is_empty());
}

#[test]
fn marker_clock_and_summary_constraints_preserve_canonical_links_and_advance_only_on_change() {
    let mut db = predecessor();
    apply(&mut db).unwrap();
    let before = cursor(&db);
    db.execute_batch(
        "INSERT INTO orchestration_pending_wakes VALUES('i',1,100,NULL);
        INSERT INTO orchestration_wake_clock VALUES(1,1000,'i');",
    )
    .unwrap();
    assert_eq!(cursor(&db), before + 2);
    db.execute_batch(
        "UPDATE orchestration_pending_wakes SET released_classes=1 WHERE identity_id='i';
        UPDATE orchestration_wake_clock SET next_admission_ms=1000 WHERE id=1;",
    )
    .unwrap();
    assert_eq!(
        cursor(&db),
        before + 2,
        "identical state does not wake readers"
    );
    db.execute_batch("UPDATE orchestration_pending_wakes SET released_classes=3,released_at_ms=101,checklist_id='legacy' WHERE identity_id='i';
        UPDATE orchestration_wake_clock SET next_admission_ms=2000,scan_after_identity_id='j' WHERE id=1;").unwrap();
    assert_eq!(cursor(&db), before + 4);
    for invalid in [
        "UPDATE orchestration_pending_wakes SET released_classes=0",
        "UPDATE orchestration_pending_wakes SET released_classes=4",
        "UPDATE orchestration_pending_wakes SET released_classes=1.5",
        "UPDATE orchestration_pending_wakes SET released_at_ms=0",
        "UPDATE orchestration_pending_wakes SET checklist_id='missing'",
        "UPDATE orchestration_wake_clock SET next_admission_ms=-1",
        "UPDATE orchestration_wake_clock SET next_admission_ms=9007199254740992",
        "UPDATE orchestration_wake_clock SET scan_after_identity_id='missing'",
        "INSERT INTO orchestration_wake_clock VALUES(2,0,NULL)",
        "UPDATE focus_checklists SET projection='incoming_summary'",
        "UPDATE focus_checklists SET summary_urgent_count=0",
        "UPDATE focus_checklists SET projection='incoming_summary',summary_urgent_count=4",
        "UPDATE focus_checklists SET projection='incoming_summary',summary_urgent_count=-1",
    ] {
        assert!(db.execute(invalid, []).is_err(), "{invalid}");
    }
    assert_eq!(cursor(&db), before + 4, "refused writes retain the cursor");
    db.execute("UPDATE focus_checklists SET projection='incoming_summary',summary_urgent_count=2 WHERE id='legacy'", []).unwrap();
    assert_eq!(cursor(&db), before + 5);
    assert_eq!(
        rows(
            &db,
            "SELECT sealed_count,summary_urgent_count FROM focus_checklists"
        ),
        vec![vec![Value::Integer(3), Value::Integer(2)]]
    );
    db.execute("DELETE FROM identities WHERE id='j'", [])
        .unwrap();
    assert_eq!(
        rows(
            &db,
            "SELECT scan_after_identity_id FROM orchestration_wake_clock"
        ),
        vec![vec![Value::Null]]
    );
    db.execute("DELETE FROM identities WHERE id='i'", [])
        .unwrap();
    assert!(rows(&db, "SELECT * FROM orchestration_pending_wakes").is_empty());
    assert!(rows(&db, "SELECT * FROM focus_checklists").is_empty());
    assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());
}

#[test]
fn failed_wake_migration_rolls_back_tables_projection_and_cursor_then_retries() {
    let mut db = predecessor();
    let before = originals(&db);
    let before_cursor = cursor(&db);
    let schema = rows(
        &db,
        "SELECT type,name,sql FROM sqlite_schema ORDER BY type,name",
    );
    db.execute_batch("CREATE TRIGGER refuse_wake_migration BEFORE INSERT ON _migrations WHEN NEW.version=81 BEGIN SELECT RAISE(ABORT,'forced wake migration rollback'); END;").unwrap();
    assert_eq!(apply(&mut db).unwrap_err().migration_version, Some(81));
    db.execute_batch("DROP TRIGGER refuse_wake_migration")
        .unwrap();
    assert_eq!(originals(&db), before);
    assert_eq!(cursor(&db), before_cursor);
    assert_eq!(
        rows(
            &db,
            "SELECT type,name,sql FROM sqlite_schema ORDER BY type,name"
        ),
        schema
    );
    assert_eq!(
        rows(&db, "SELECT max(version) FROM _migrations"),
        vec![vec![Value::Integer(80)]]
    );
    apply(&mut db).unwrap();
    assert_eq!(originals(&db), before);
    assert_eq!(
        rows(&db, "PRAGMA foreign_keys"),
        vec![vec![Value::Integer(1)]]
    );
}
