use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use tmt_core::request::RequestService;

#[test]
fn room_context_migration_rolls_back_and_does_not_invent_historical_scope() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema25.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..25].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch("INSERT INTO request_attempts (
        attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
        inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
        message_text,message_bytes,message_expires_at_ms
    ) VALUES ('old-attempt','old-request','inbox','old-id',0,'queued',0,0,1000,3601000,604801000,'old prompt',10,604801000);
    CREATE TRIGGER reject_room_context BEFORE INSERT ON _migrations WHEN NEW.version=26
    BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(26)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        25
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM pragma_table_info('request_attempts') WHERE name='room_id'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(oracle.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('request_attempts_room_recipient_attention','request_attempts_room_response_attention')", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    oracle
        .execute_batch("DROP TRIGGER reject_room_context;")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 34);
    let context = RequestService::new(&mut storage, || 2000)
        .get_context("old-request")
        .unwrap()
        .unwrap();
    assert_eq!(context.attempt.room_id, None);
    assert_eq!(
        oracle
            .query_row(
                "SELECT message_text FROM request_attempts WHERE request_id='old-request'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "old prompt"
    );
    storage.close().unwrap();
}
