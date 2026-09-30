use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use tmt_core::request::{RequestKind, RequestService};

#[test]
fn announcement_migration_rolls_back_and_preserves_historical_request_semantics() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema24.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..24].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch("INSERT INTO request_attempts (
        attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
        inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
        message_text,message_bytes,message_expires_at_ms
    ) VALUES ('old-attempt','old-request','inbox','old-id',0,'queued',0,0,1000,3601000,604801000,'old prompt',10,604801000);
    CREATE TRIGGER reject_kind BEFORE INSERT ON _migrations WHEN NEW.version=25
    BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(25)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        24
    );
    assert_eq!(oracle.query_row("SELECT count(*) FROM pragma_table_info('request_attempts') WHERE name='request_kind'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(
        oracle
            .query_row("SELECT message_text FROM request_attempts", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "old prompt"
    );
    oracle.execute_batch("DROP TRIGGER reject_kind;").unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 41);
    let attempt = RequestService::new(&mut storage, || 2000)
        .get_attempt("old-attempt")
        .unwrap()
        .unwrap();
    assert_eq!(attempt.kind, RequestKind::Request);
    assert!(
        oracle
            .execute("UPDATE request_attempts SET request_kind='unknown'", [])
            .is_err()
    );
    assert!(
        oracle
            .execute(
                "UPDATE request_attempts SET request_kind='announcement',wait_active=1",
                []
            )
            .is_err()
    );
    storage.close().unwrap();
}
