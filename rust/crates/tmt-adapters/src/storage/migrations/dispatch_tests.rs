use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn dispatch_migration_rolls_back_receipt_table_and_history_together() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema22.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..22].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch("CREATE TRIGGER reject_dispatch_migration BEFORE INSERT ON _migrations WHEN NEW.version=23 BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(23)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        22
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='office_dispatch_operations'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        oracle
            .query_row(
                "SELECT content FROM role_profiles WHERE identity_id='old-id'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "original role"
    );
    oracle
        .execute_batch("DROP TRIGGER reject_dispatch_migration;")
        .unwrap();
    oracle.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 38);
    assert_eq!(
        storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM office_dispatch_operations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    storage.close().unwrap();
}
