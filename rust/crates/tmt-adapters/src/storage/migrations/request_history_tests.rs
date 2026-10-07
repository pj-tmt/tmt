use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn withdrawal_migration_rolls_back_columns_and_cursor_trigger_together() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema47.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 47).unwrap();
    old.execute_batch(
        "CREATE TRIGGER reject_withdrawal BEFORE INSERT ON _migrations WHEN NEW.version=48
        BEGIN SELECT RAISE(ABORT, 'injected withdrawal migration failure'); END;",
    )
    .unwrap();
    let before: String = old.query_row("SELECT sql FROM sqlite_schema WHERE name='request_attempts_advances_change_cursor_on_update'", [], |row| row.get(0)).unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(48)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT MAX(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        47
    );
    assert_eq!(oracle.query_row("SELECT COUNT(*) FROM pragma_table_info('request_attempts') WHERE name IN ('withdrawn_at_ms','withdrawal_reason')", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(oracle.query_row("SELECT sql FROM sqlite_schema WHERE name='request_attempts_advances_change_cursor_on_update'", [], |row| row.get::<_, String>(0)).unwrap(), before);
    oracle
        .execute_batch("DROP TRIGGER reject_withdrawal")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 48);
    assert_eq!(oracle.query_row("SELECT COUNT(*) FROM pragma_table_info('request_attempts') WHERE name IN ('withdrawn_at_ms','withdrawal_reason')", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
    storage.close().unwrap();
}

#[test]
fn request_history_indexes_commit_together_without_rewriting_requests() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema26.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..26].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch("INSERT INTO request_attempts (
        attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
        inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
        message_text,message_bytes,message_expires_at_ms
    ) VALUES ('history-attempt','history-request','inbox','old-id',0,'queued',0,0,1000,3601000,604801000,'old prompt',10,604801000);
    CREATE TRIGGER reject_history_indexes BEFORE INSERT ON _migrations WHEN NEW.version=27
    BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(27)
    );
    let oracle = Connection::open(&path).unwrap();
    let indexes = || {
        oracle.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name IN ('request_history_recipient','request_history_recipient_room','request_history_room')",
        [], |row| row.get::<_, i64>(0)).unwrap()
    };
    assert_eq!(indexes(), 0);
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        26
    );
    oracle
        .execute_batch("DROP TRIGGER reject_history_indexes;")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 48);
    assert_eq!(indexes(), 3);
    assert_eq!(oracle.query_row("SELECT message_text,room_id FROM request_attempts WHERE request_id='history-request'", [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))).unwrap(), ("old prompt".into(), None));
    storage.close().unwrap();
}

#[test]
fn originator_results_index_migration_rolls_back_and_preserves_existing_requests() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema46.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 46).unwrap();
    old.execute_batch("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES ('old-id','Old','old','2023-11-14T22:13:20Z','2023-11-14T22:13:20Z','saved');
        INSERT INTO request_attempts (
            attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
            inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
            message_text,message_bytes,message_expires_at_ms,response_submitted_at_ms
        ) VALUES ('results-attempt','results-request','inbox','old-id',0,'queued',0,0,1000,3601000,604801000,'old prompt',10,604801000,2000);
        CREATE TRIGGER reject_results_index BEFORE INSERT ON _migrations WHEN NEW.version=47
        BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;").unwrap();
    let before: i64 = old
        .query_row("SELECT value FROM change_cursor", [], |row| row.get(0))
        .unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(47)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        46
    );
    assert_eq!(oracle.query_row("SELECT count(*) FROM sqlite_schema WHERE name='request_history_originator_results'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    oracle
        .execute_batch("DROP TRIGGER reject_results_index")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 48);
    assert_eq!(
        oracle
            .query_row("SELECT value FROM change_cursor", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(oracle.query_row("SELECT message_text,response_submitted_at_ms FROM request_attempts WHERE request_id='results-request'", [],
        |row| Ok((row.get::<_, String>(0)?,row.get::<_, i64>(1)?))).unwrap(), ("old prompt".into(),2000));
    let columns = oracle
        .prepare("PRAGMA index_xinfo(request_history_originator_results)")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, Option<String>>(2)?, row.get::<_, i64>(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        &columns[..3],
        &[
            (Some("originator_identity_id".into()), 0),
            (Some("response_submitted_at_ms".into()), 1),
            (Some("request_id".into()), 1)
        ]
    );
    storage.close().unwrap();
}
