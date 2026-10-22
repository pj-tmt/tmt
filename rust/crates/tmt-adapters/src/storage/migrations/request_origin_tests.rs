use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use tmt_core::request::{RequestOrigin, RequestService};

#[test]
fn origin_migration_adds_the_column_without_inventing_history_and_guards_it() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema56.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..56].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch(
        "INSERT INTO request_attempts (
            attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
            inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
            message_text,message_bytes,message_expires_at_ms
        ) VALUES ('old-attempt','old-request','inbox','old-id',0,'queued',0,0,1000,3601000,604801000,'old prompt',10,604801000);",
    )
    .unwrap();
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 59);
    let context = RequestService::new(&mut storage, || 2000)
        .get_context("old-request")
        .unwrap()
        .unwrap();
    assert_eq!(context.attempt.origin, RequestOrigin::Unknown);
    storage.close().unwrap();

    let oracle = Connection::open(&path).unwrap();
    let origin: Option<String> = oracle
        .query_row(
            "SELECT origin FROM request_attempts WHERE request_id = 'old-request'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(origin, None);
    let insert = |origin: &str| {
        oracle.execute(
            "INSERT INTO request_attempts (
                attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
                inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
                message_text,message_bytes,message_expires_at_ms,origin
            ) VALUES (?1,?1,'inbox','old-id',0,'queued',0,0,1000,3601000,604801000,'prompt',6,604801000,?2)",
            [format!("new-{origin}"), origin.to_owned()],
        )
    };
    insert("person").unwrap();
    insert("automation").unwrap();
    assert!(
        insert("robot").is_err(),
        "only a person or automation is stored"
    );
}
