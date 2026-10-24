use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn delivery_mode_migration_keeps_every_stored_request_a_request_and_guards_the_column() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema58.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 58).unwrap();
    old.execute_batch(
        "INSERT INTO request_attempts (
            attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
            inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
            message_text,message_bytes,message_expires_at_ms
        ) VALUES ('old-attempt','old-request','inbox','old-id',0,'queued',0,0,1000,3601000,604801000,'old prompt',10,604801000);
        INSERT INTO request_delivery_policies(request_id,urgent,kind,automatic)
            VALUES ('old-request',0,'fyi',1);",
    )
    .unwrap();
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 68);
    storage.close().unwrap();

    let oracle = Connection::open(&path).unwrap();
    let mode: String = oracle
        .query_row(
            "SELECT mode FROM request_delivery_policies WHERE request_id = 'old-request'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        mode, "request",
        "an older request is not retroactively typed text"
    );

    // Only the two modes exist.
    let update = |mode: &str| {
        oracle.execute(
            "UPDATE request_delivery_policies SET mode = ?1 WHERE request_id = 'old-request'",
            [mode],
        )
    };
    assert!(update("robot").is_err());
    // A mode change is a change: readers waiting on the change cursor must wake.
    let cursor = || -> i64 {
        oracle
            .query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    let before = cursor();
    update("passthrough").unwrap();
    assert_eq!(cursor(), before + 1);
    update("passthrough").unwrap();
    assert_eq!(cursor(), before + 1, "an unchanged row does not advance it");
}
