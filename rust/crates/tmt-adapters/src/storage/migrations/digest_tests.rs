use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn digest_stats_migration_is_atomic_and_seeds_only_retained_successes() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema50.db");
    let mut old = Connection::open(&path).unwrap();
    apply_through(&mut old, 50).unwrap();
    old.execute_batch("INSERT INTO identities(id,name,canonical_name,created_at,updated_at,lifetime) VALUES('receiver','Receiver','receiver','t','t','saved');
        INSERT INTO focus_checklists(id,identity_id,attempt_token,through_sequence,state,created_at_ms) VALUES
        ('delivered','receiver','token-delivered',1,'delivered',1000),
        ('unsent','receiver','token-unsent',2,'definitely_unsent',1001),
        ('uncertain','receiver','token-uncertain',3,'uncertain',1002),
        ('claimed','receiver','token-claimed',4,'claimed',1003);
        CREATE TRIGGER refuse_digest_stats BEFORE INSERT ON _migrations WHEN NEW.version=51
        BEGIN SELECT RAISE(ABORT,'injected digest migration failure'); END;").unwrap();
    let trigger: String = old.query_row("SELECT sql FROM sqlite_schema WHERE name='focus_items_advances_change_cursor_on_update'",[],|r|r.get(0)).unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(51)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT MAX(version) FROM _migrations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        50
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name='digest_counters'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('focus_items') WHERE name LIKE 'context_%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(oracle.query_row("SELECT sql FROM sqlite_schema WHERE name='focus_items_advances_change_cursor_on_update'",[],|r|r.get::<_,String>(0)).unwrap(),trigger);
    oracle
        .execute_batch("DROP TRIGGER refuse_digest_stats")
        .unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 51);
    assert_eq!(oracle.query_row("SELECT due_through_sequence,delivered_digests FROM digest_counters WHERE identity_id='receiver'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),(0,1));
    assert_eq!(
        oracle
            .query_row("SELECT COUNT(*) FROM focus_checklists", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        4
    );
    storage.close().unwrap();
}
