//! Independent predecessor rows and SQL bounds, without touching frozen archives.
use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn orchestration_receiving_migration_preserves_checklists_and_missing_input() {
    let d = TestDirectory::new();
    let path = d.path.join("predecessor.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 76).unwrap();
    old.execute_batch("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES('member','Member','member','t','t','saved');
        INSERT INTO focus_checklists(id,identity_id,attempt_token,through_sequence,state,created_at_ms)
        VALUES('legacy-claim','member','legacy-token',3,'delivered',1000);").unwrap();
    old.close().unwrap();
    Storage::open(&path).unwrap().close().unwrap();
    let db = Connection::open(&path).unwrap();
    let actual:(String,String,i64,Option<String>)=db.query_row(
        "SELECT attempt_token,state,through_sequence,orchestration_context FROM focus_checklists WHERE id='legacy-claim'",[],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(actual, ("legacy-token".into(), "delivered".into(), 3, None));
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM digest_receiving_settings", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    for revision in ["0", "1.5", "9007199254740992"] {
        assert!(
            db.execute(
                &format!("INSERT INTO digest_receiving_settings VALUES(1,{revision},'{{}}')"),
                []
            )
            .is_err()
        );
    }
    assert!(
        db.execute(
            "INSERT INTO digest_receiving_settings VALUES(1,1,'invalid JSON')",
            []
        )
        .is_err()
    );
    db.execute("INSERT INTO digest_receiving_settings VALUES(1,1,'{}')", [])
        .unwrap();
    let cursor = || {
        db.query_row("SELECT value FROM change_cursor", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    let before = cursor();
    assert!(
        db.execute(
            "UPDATE focus_checklists SET orchestration_context='bad JSON'",
            []
        )
        .is_err()
    );
    assert_eq!(cursor(), before);
    db.execute("UPDATE focus_checklists SET orchestration_context='{}'", [])
        .unwrap();
    assert_eq!(cursor(), before + 1);
    db.execute("UPDATE focus_checklists SET orchestration_context='{}'", [])
        .unwrap();
    assert_eq!(
        cursor(),
        before + 1,
        "unchanged provenance is not a board change"
    );
}
