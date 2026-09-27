use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

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
    assert_eq!(storage.health().unwrap().schema_version, 33);
    assert_eq!(indexes(), 3);
    assert_eq!(oracle.query_row("SELECT message_text,room_id FROM request_attempts WHERE request_id='history-request'", [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))).unwrap(), ("old prompt".into(), None));
    storage.close().unwrap();
}
