use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn the_digest_hold_migration_keeps_older_policies_unknown_and_guards_the_column() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema60.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 60).unwrap();
    old.execute_batch(
        "INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime)
            VALUES ('member','Member','member','t','t','saved');
        INSERT INTO focus_policies(identity_id,revision,until_ms,owner_identity_id,setter_identity_id)
            VALUES ('member',1,5000,'member','member');",
    )
    .unwrap();
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 70);
    storage.close().unwrap();

    let oracle = Connection::open(&path).unwrap();
    let hold = || -> Option<i64> {
        oracle
            .query_row(
                "SELECT max_hold_ms FROM focus_policies WHERE identity_id = 'member'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(hold(), None, "an older policy states no deadline");

    // Null or a positive whole number of milliseconds only.
    let set = |value: &str| {
        oracle.execute(
            &format!(
                "UPDATE focus_policies SET max_hold_ms = {value} WHERE identity_id = 'member'"
            ),
            [],
        )
    };
    assert!(set("0").is_err());
    assert!(set("-5").is_err());
    assert!(set("9007199254740992").is_err());
    // A change is a change: readers waiting on the change cursor must wake.
    let cursor = || -> i64 {
        oracle
            .query_row("SELECT value FROM change_cursor WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    let before = cursor();
    set("300000").unwrap();
    assert_eq!(hold(), Some(300_000));
    assert_eq!(cursor(), before + 1);
    set("300000").unwrap();
    assert_eq!(cursor(), before + 1, "an unchanged row does not advance it");
    set("NULL").unwrap();
    assert_eq!(cursor(), before + 2);
}
