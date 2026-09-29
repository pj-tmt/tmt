//! Schema upgrade never rewrites retained legacy layouts; saving a world is Office-owned.
use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

const WORLD: &str = "11111111-1111-4111-8111-111111111111";
const LOBBY: &str = "22222222-2222-4222-8222-222222222222";

fn legacy_row(connection: &Connection) -> (String, i64, Vec<u8>, i64) {
    connection
        .query_row(
            "SELECT block_id,revision,CAST(layout AS BLOB),updated_at_ms FROM office_local_blocks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
}

fn definitions(connection: &Connection) -> Vec<(String, Option<String>)> {
    connection
        .prepare(
            "SELECT name,sql FROM sqlite_schema WHERE tbl_name IN
             ('office_local_worlds','office_local_blocks') ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn upgrade_rolls_back_on_history_failure_and_never_materializes_a_layout_on_open() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema27.db");
    let mut predecessor = Connection::open(&path).unwrap();
    test_support::seed_history(&mut predecessor);
    apply_identity_lifetime(&mut predecessor, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..27].iter().enumerate() {
        apply_version(&mut predecessor, index as u32 + 10, migration).unwrap();
    }
    predecessor
        .execute(
            "INSERT INTO office_local_worlds(singleton,id,created_at_ms) VALUES(1,?,100)",
            [WORLD],
        )
        .unwrap();
    // Whitespace is intentional: opening the database must not re-encode old layouts.
    let layout = "  {\"version\":2,\"objects\":[]}  ";
    predecessor
        .execute(
            "INSERT INTO office_local_blocks VALUES(?,'lobby',NULL,7,?,123)",
            params![LOBBY, layout],
        )
        .unwrap();
    let before_row = legacy_row(&predecessor);
    let before_schema = definitions(&predecessor);
    predecessor
        .execute_batch(
            "CREATE TRIGGER reject_world_schema BEFORE INSERT ON _migrations
             WHEN NEW.version=28 BEGIN SELECT RAISE(ABORT,'fixture failure'); END;",
        )
        .unwrap();
    predecessor.close().unwrap();

    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(28)
    );
    let observer = Connection::open(&path).unwrap();
    assert_eq!(definitions(&observer), before_schema);
    assert_eq!(legacy_row(&observer), before_row);
    assert_eq!(
        observer
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        27
    );
    observer
        .execute_batch("DROP TRIGGER reject_world_schema")
        .unwrap();
    observer.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 38);
    assert_eq!(legacy_row(storage.connection().unwrap()), before_row);
    // Reading and saving the retained layout is the Office store's contract;
    // see tmt-office-storage's legacy world tests.
    storage.close().unwrap();
}
