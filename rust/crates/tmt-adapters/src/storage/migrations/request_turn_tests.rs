use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

const REQUEST: &str = "INSERT INTO request_attempts (
    attempt_id,request_id,route_kind,recipient_identity_id,wait_active,status,
    inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,
    message_text,message_bytes,message_expires_at_ms
) VALUES ('a1','r1','inbox','agent',0,'queued',0,0,1000,3601000,604801000,'please review',13,604801000)";

#[test]
fn a_request_turn_is_one_bounded_row_that_moves_the_cursor_and_leaves_with_its_request() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema69.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 69).unwrap();
    old.execute_batch(REQUEST).unwrap();
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 87);
    storage.close().unwrap();

    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    let cursor = || -> i64 {
        db.query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let before = cursor();
    db.execute(
        "INSERT INTO request_turns(request_id,started_ms) VALUES('r1',2000)",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 1);
    // One turn per request, and it cannot end before it began or belong to nothing.
    for statement in [
        "INSERT INTO request_turns(request_id,started_ms) VALUES('r1',3000)",
        "INSERT INTO request_turns(request_id,started_ms) VALUES('nobody',3000)",
        "UPDATE request_turns SET ended_ms=1999 WHERE request_id='r1'",
        "UPDATE request_turns SET started_ms=0 WHERE request_id='r1'",
    ] {
        assert!(db.execute(statement, []).is_err(), "{statement}");
    }
    db.execute(
        "UPDATE request_turns SET ended_ms=2500 WHERE request_id='r1'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 2);
    db.execute(
        "UPDATE request_turns SET ended_ms=2500 WHERE request_id='r1'",
        [],
    )
    .unwrap();
    assert_eq!(cursor(), before + 2, "an unchanged row does not advance it");
    db.execute("DELETE FROM request_attempts WHERE request_id='r1'", [])
        .unwrap();
    let left: i64 = db
        .query_row("SELECT count(*) FROM request_turns", [], |row| row.get(0))
        .unwrap();
    assert_eq!(left, 0);
}
