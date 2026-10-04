use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
use tmt_core::room::RoomRepository;

#[test]
fn room_retirement_migration_preserves_roster_and_rolls_back_on_failure() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema30.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..30].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    let room = "11111111-1111-4111-8111-111111111111";
    old.execute(
        "INSERT INTO office_meeting_rooms(room_id,name,revision) VALUES(?,'Retained',7)",
        [room],
    )
    .unwrap();
    old.execute(
        "INSERT INTO office_meeting_members VALUES(?,'old-id')",
        [room],
    )
    .unwrap();
    old.execute_batch("CREATE TRIGGER reject_retirement_upgrade BEFORE INSERT ON _migrations WHEN NEW.version=31 BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(31)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        30
    );
    assert_eq!(oracle.query_row("SELECT count(*) FROM pragma_table_info('office_meeting_rooms') WHERE name='retired'", [], |row| row.get::<_, u32>(0)).unwrap(), 0);
    assert_eq!(
        oracle
            .query_row("SELECT count(*) FROM office_meeting_members", [], |row| row
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
    oracle
        .execute_batch("DROP TRIGGER reject_retirement_upgrade;")
        .unwrap();
    oracle.close().unwrap();
    let mut upgraded = Storage::open(&path).unwrap();
    let retained = upgraded.find_meeting_room(room).unwrap().unwrap();
    assert!(!retained.retired);
    assert_eq!(retained.revision, 7);
    assert_eq!(retained.member_ids, vec!["old-id"]);
}

#[test]
fn meeting_migration_rolls_back_both_roster_tables_and_history() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema23.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..23].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    old.execute_batch("CREATE TRIGGER reject_rooms BEFORE INSERT ON _migrations WHEN NEW.version=24 BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(24)
    );
    let oracle = Connection::open(&path).unwrap();
    assert_eq!(
        oracle
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        23
    );
    assert_eq!(oracle.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('office_meeting_rooms','office_meeting_members')", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
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
    oracle.execute_batch("DROP TRIGGER reject_rooms;").unwrap();
    oracle.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 47);
    assert!(storage.list_meeting_rooms().unwrap().is_empty());
    storage.close().unwrap();
}
