use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

/// Every checklist handed over before migration 68 (delivered or uncertain) is confirmed at its
/// settlement: only a checklist handed over afterwards waits on later evidence (#4858).
#[test]
fn the_typed_checklist_migration_confirms_every_earlier_delivery() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema67.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 67).unwrap();
    old.execute_batch(
        "INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime)
            VALUES ('member','Member','member','t','t','saved');
        INSERT INTO focus_checklists(id,identity_id,attempt_token,through_sequence,state,created_at_ms,opportunity,sealed_count,settled_at_ms)
            VALUES ('delivered','member','t1',1,'delivered',100,'idle',1,150),
                   ('uncertain','member','t2',2,'uncertain',200,'idle',1,250),
                   ('claimed','member','t3',3,'claimed',300,'turn_boundary',1,NULL);",
    )
    .unwrap();
    old.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 68);
    storage.close().unwrap();

    let oracle = Connection::open(&path).unwrap();
    let confirmed = |id: &str| -> Option<i64> {
        oracle
            .query_row(
                "SELECT confirmed_at_ms FROM focus_checklists WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        confirmed("delivered"),
        Some(150),
        "a settled delivery is confirmed"
    );
    assert_eq!(
        confirmed("uncertain"),
        Some(250),
        "so is a settled uncertain one"
    );
    assert_eq!(confirmed("claimed"), None, "a claim in flight is not");
    // A positive whole number of milliseconds or null.
    for value in ["0", "-1", "9007199254740992"] {
        assert!(
            oracle
                .execute(
                    &format!(
                        "UPDATE focus_checklists SET confirmed_at_ms = {value} WHERE id = 'claimed'"
                    ),
                    [],
                )
                .is_err(),
            "{value}"
        );
    }
}
