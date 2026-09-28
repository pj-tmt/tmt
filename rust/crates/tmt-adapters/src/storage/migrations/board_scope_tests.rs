use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use rusqlite::types::Value;

fn contents(connection: &Connection, table: &str) -> Vec<Vec<Value>> {
    let mut statement = connection
        .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2"))
        .unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| (0..columns).map(|index| row.get(index)).collect())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn board_scope_upgrade_preserves_entries_receipts_and_atomic_rollback() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema29.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..29].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch("INSERT INTO office_board_entries VALUES
      ('11111111-1111-4111-8111-111111111111','11111111-1111-4111-8111-111111111111',1,'general',NULL,'owner','world',NULL,2,0,1,3,10,30,'General','Updated body'),
      ('22222222-2222-4222-8222-222222222222','22222222-2222-4222-8222-222222222222',1,'repository','example.com/team/project','identity','old-id','Alice',1,0,2,2,20,20,'Repo','Repository body'),
      ('33333333-3333-4333-8333-333333333333','11111111-1111-4111-8111-111111111111',0,'general',NULL,'identity','old-id','Alice',2,1,3,3,30,40,NULL,NULL);
      UPDATE office_board_state SET revision=4,next_sequence=4;
      INSERT INTO office_board_operations VALUES ('owner:world','44444444-4444-4444-8444-444444444444',printf('%064d',0),'create','11111111-1111-4111-8111-111111111111','11111111-1111-4111-8111-111111111111',1,1,NULL,NULL,NULL);").unwrap();
    let tables = [
        "office_board_entries",
        "office_board_operations",
        "office_board_state",
    ];
    let expected = tables.map(|table| contents(&old, table));
    old.execute_batch("CREATE TRIGGER reject_board_scope_upgrade BEFORE INSERT ON _migrations WHEN NEW.version=30 BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(30)
    );
    let observer = Connection::open(&path).unwrap();
    assert_eq!(tables.map(|table| contents(&observer, table)), expected);
    assert_eq!(
        observer
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        29
    );
    assert_eq!(observer.query_row("SELECT count(*) FROM pragma_table_info('office_board_entries') WHERE name='repository_id'", [], |row| row.get::<_,i64>(0)).unwrap(), 1);
    observer
        .execute_batch("DROP TRIGGER reject_board_scope_upgrade")
        .unwrap();
    observer.close().unwrap();
    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 36);
    let connection = storage.connection().unwrap();
    assert_eq!(tables.map(|table| contents(connection, table)), expected);
    assert_eq!(connection.query_row("SELECT count(*) FROM pragma_table_info('office_board_entries') WHERE name='category_id'", [], |row| row.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}
