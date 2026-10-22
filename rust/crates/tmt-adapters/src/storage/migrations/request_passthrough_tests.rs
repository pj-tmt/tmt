use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

const REQUEST: &str = "INSERT INTO request_attempts (
    attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
    inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
    message_text,message_bytes,message_expires_at_ms
) VALUES ('a1','r1','inbox','agent',0,'queued',0,0,1000,3601000,604801000,'typed text',10,604801000)";

#[test]
fn passthrough_capture_rows_are_consistent_wake_readers_and_leave_with_their_request() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema59.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 59).unwrap();
    old.execute_batch(REQUEST).unwrap();
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 60);
    storage.close().unwrap();

    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    let cursor = || -> i64 {
        db.query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    // A capture starts pending, with nothing bound or answered.
    let before = cursor();
    db.execute(
        "INSERT INTO request_passthrough(request_id,delivered_at_ms,state) VALUES('r1',2000,'pending')",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 1);
    // Every state carries exactly its own facts.
    let invalid = [
        "UPDATE request_passthrough SET state='bound' WHERE request_id='r1'",
        "UPDATE request_passthrough SET state='no_answer' WHERE request_id='r1'",
        "UPDATE request_passthrough SET state='answered',bound_session='s',bound_at_ms=2100,settled_at_ms=2200 WHERE request_id='r1'",
        "UPDATE request_passthrough SET answer_text='orphan answer' WHERE request_id='r1'",
        "UPDATE request_passthrough SET state='robot' WHERE request_id='r1'",
    ];
    for statement in invalid {
        assert!(db.execute(statement, []).is_err(), "{statement}");
    }
    db.execute(
        "UPDATE request_passthrough SET state='bound',bound_session='s',bound_turn='t',bound_at_ms=2100 WHERE request_id='r1'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 2);
    db.execute(
        "UPDATE request_passthrough SET state='answered',answer_text='Done.',answer_truncated=0,answer_provider='codex',answer_ended_ms=2200,settled_at_ms=2200 WHERE request_id='r1'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 3);
    // The answer is plain bounded text from a known provider.
    assert!(
        db.execute(
            "UPDATE request_passthrough SET answer_provider='gemini' WHERE request_id='r1'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            &format!(
                "UPDATE request_passthrough SET answer_text='{}' WHERE request_id='r1'",
                "x".repeat(8193)
            ),
            []
        )
        .is_err()
    );
    db.execute(
        "UPDATE request_passthrough SET state='answered' WHERE request_id='r1'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 3, "an unchanged row does not advance it");
    // The capture is part of the request: it goes with it.
    db.execute("DELETE FROM request_attempts WHERE request_id='r1'", [])
        .unwrap();
    let left: i64 = db
        .query_row("SELECT count(*) FROM request_passthrough", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(left, 0);
}
