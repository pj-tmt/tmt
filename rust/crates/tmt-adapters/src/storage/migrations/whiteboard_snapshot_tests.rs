use super::*;
use crate::{storage::Storage, test_support::TestDirectory};

#[test]
fn snapshot_upgrade_preserves_saved_scene_and_rolls_back_schema_with_history() {
    let directory = TestDirectory::new();
    let path = directory.path.join("schema21.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..21].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    let world_id = "11111111-1111-4111-8111-111111111111";
    old.execute(
        "INSERT INTO office_local_worlds (singleton,id,created_at_ms) VALUES (1,?,100)",
        [world_id],
    )
    .unwrap();
    // Frozen schema-21 scene from Git blob c1d238b477a58f6f36bde06bc9b193858a29f575; preserved migration input.
    let original = r###"{
  "formatVersion": 1,
  "width": 1600,
  "height": 1000,
  "background": "#fff7e7",
  "elements": [
    {
      "id": "11111111-1111-4111-8111-111111111111",
      "kind": "note",
      "x": 100,
      "y": 100,
      "width": 320,
      "height": 200,
      "text": "What should we build next?\nLeave a sketch or a note.",
      "color": "#263d33",
      "fill": "#ffeaa3",
      "fontSize": 24
    },
    {
      "id": "22222222-2222-4222-8222-222222222222",
      "kind": "rectangle",
      "x": 600,
      "y": 200,
      "width": 300,
      "height": 200,
      "color": "#237e70",
      "fill": "none",
      "strokeWidth": 4
    },
    {
      "id": "33333333-3333-4333-8333-333333333333",
      "kind": "arrow",
      "points": [
        [440, 200],
        [580, 280]
      ],
      "color": "#237e70",
      "strokeWidth": 4
    },
    {
      "id": "44444444-4444-4444-8444-444444444444",
      "kind": "stroke",
      "points": [
        [650, 450],
        [700, 420],
        [760, 450]
      ],
      "color": "#b45941",
      "strokeWidth": 3
    },
    {
      "id": "55555555-5555-4555-8555-555555555555",
      "kind": "ellipse",
      "x": 950,
      "y": 200,
      "width": 200,
      "height": 200,
      "color": "#263d33",
      "fill": "#d8eadb",
      "strokeWidth": 2
    },
    {
      "id": "66666666-6666-4666-8666-666666666666",
      "kind": "text",
      "x": 600,
      "y": 100,
      "width": 500,
      "height": 60,
      "text": "A shared idea, not an implicit request.",
      "color": "#263d33",
      "fill": "none",
      "fontSize": 24
    }
  ]
}
"###;
    assert_eq!(
        tmt_core::content_digest::sha256(original.as_bytes()),
        "6b7344cc4d57cac194ae3c7c4436b94318283e9758083fa27da4fc005e1b5458"
    );
    old.execute("INSERT INTO office_whiteboards (document_id,world_id,revision,scene,updated_at_ms) VALUES ('lobby',?,1,?,100)", params![world_id, original]).unwrap();
    old.execute_batch("CREATE TRIGGER reject_snapshot_upgrade BEFORE INSERT ON _migrations WHEN NEW.version=22 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    old.close().unwrap();
    assert_eq!(
        Storage::open(&path).err().unwrap().migration_version,
        Some(22)
    );
    let observer = Connection::open(&path).unwrap();
    assert_eq!(
        observer
            .query_row("SELECT max(version) FROM _migrations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        21
    );
    assert_eq!(
        observer
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name IN ('office_whiteboard_snapshots', 'office_whiteboard_snapshot_images')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        observer
            .query_row(
                "SELECT scene FROM office_whiteboards WHERE document_id='lobby'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        original
    );
    observer
        .execute_batch("DROP TRIGGER reject_snapshot_upgrade")
        .unwrap();
    observer.close().unwrap();
    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 87);
    let connection = storage.connection().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT scene FROM office_whiteboards WHERE document_id='lobby'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        original
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT (SELECT count(*) FROM office_whiteboard_snapshots) + (SELECT count(*) FROM office_whiteboard_snapshot_images)",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}
