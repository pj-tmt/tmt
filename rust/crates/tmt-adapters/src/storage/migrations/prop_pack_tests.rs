//! Schema-17 upgrade preserves exact catalog content and rolls back atomically.
use super::*;
use crate::{storage::Storage, test_support::TestDirectory};
// Frozen schema-17 input from builtin-props-v1.tmtprop.json, Git blob d03b9a9b406059ff675ac30f77e8d8f4a8d1c708.
// Kept as migration evidence after its product/model retirement; no decoder generates expectations.
const PACK: &[u8] = br###"{
  "formatVersion": 1,
  "label": "Office built-ins",
  "credit": "tmux-team",
  "license": "MIT",
  "palette": [
    "#00000000",
    "#24313aff",
    "#dca77dff",
    "#f1c999ff",
    "#4f8f5bff",
    "#83c56fff",
    "#b65d66ff",
    "#d98b8fff"
  ],
  "props": [
    {
      "key": "desk",
      "label": "Desk",
      "footprint": { "width": 4, "height": 2 },
      "pixels": [
        "0000000000000000",
        "0011111111111100",
        "0122222222222210",
        "0133333333333310",
        "0011111111111100",
        "0011000000001100",
        "0011000000001100",
        "0000000000000000"
      ]
    },
    {
      "key": "chair",
      "label": "Chair",
      "footprint": { "width": 2, "height": 2 },
      "pixels": [
        "00111100",
        "01222210",
        "01233210",
        "01222210",
        "00111100",
        "00011000",
        "00100100",
        "00000000"
      ]
    },
    {
      "key": "plant",
      "label": "Plant",
      "footprint": { "width": 2, "height": 2 },
      "pixels": [
        "00011000",
        "00515500",
        "05555550",
        "00545500",
        "00044000",
        "00133100",
        "00122100",
        "00011000"
      ]
    },
    {
      "key": "rug",
      "label": "Rug",
      "footprint": { "width": 6, "height": 4 },
      "pixels": [
        "0011111111111100",
        "0166666666666610",
        "0167777777777610",
        "0167666666677610",
        "0167666666677610",
        "0167777777777610",
        "0166666666666610",
        "0011111111111100"
      ]
    }
  ]
}
"###;
const DIGEST: &str = "sha256:5aa6a2d239d7111586abc06be799b2a1ec2ca46619752a90ae08a13e414afb6a";
const PROP_COUNT: i64 = 4;

#[test]
fn prop_bound_upgrade_preserves_content_and_rolls_back_on_history_failure() {
    assert_eq!(
        tmt_core::content_digest::sha256(PACK),
        "a63b9c1b1c375c125b3a8508c75441634cea489e4d17cc01dd77c6403a6a3314"
    );
    let directory = TestDirectory::new();
    let path = directory.path.join("schema17.db");
    let mut old = Connection::open(&path).unwrap();
    test_support::seed_history(&mut old);
    apply_identity_lifetime(&mut old, &MIGRATIONS[8]).unwrap();
    for (index, migration) in MIGRATIONS[9..17].iter().enumerate() {
        apply_version(&mut old, index as u32 + 10, migration).unwrap();
    }
    let layout = serde_json::json!({
        "version": 2,
        "objects": [{
            "prop": "sha256:5aa6a2d239d7111586abc06be799b2a1ec2ca46619752a90ae08a13e414afb6a/desk",
            "footprint": {"width": 4, "height": 2},
            "x": 4, "y": 5, "rotation": 0
        }]
    })
    .to_string();
    old.execute(
        "INSERT INTO office_local_blocks VALUES ('00000000-0000-4000-8000-000000000001', 'old-id', 3, ?, 123)",
        [&layout],
    ).unwrap();
    old.execute(
        "INSERT INTO office_prop_packs VALUES (?, ?, ?, 7, 123)",
        params![DIGEST, PACK, PROP_COUNT],
    )
    .unwrap();
    old.execute_batch(
        "UPDATE office_prop_catalog SET revision = 7;
        CREATE TRIGGER reject_eighteenth BEFORE INSERT ON _migrations
        WHEN NEW.version = 18 BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
    )
    .unwrap();
    old.close().unwrap();

    let error = Storage::open(&path)
        .err()
        .expect("history failure must roll back schema 18");
    assert_eq!(error.migration_version, Some(18));
    let observer = Connection::open(&path).unwrap();
    let original_schema: String = observer
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'office_prop_packs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(original_schema.contains("131072"));
    let staging_tables: i64 = observer
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'office_prop_packs_previous'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(staging_tables, 0);
    let history_count: i64 = observer
        .query_row(
            "SELECT count(*) FROM _migrations WHERE version = 18",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history_count, 0);
    let preserved: (Vec<u8>, String) = observer
        .query_row(
            "SELECT bytes, layout FROM office_prop_packs CROSS JOIN office_local_blocks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(preserved, (PACK.to_vec(), layout.clone()));
    observer
        .execute_batch("DROP TRIGGER reject_eighteenth")
        .unwrap();
    observer.close().unwrap();

    let mut upgraded = Storage::open(&path).unwrap();
    assert_eq!(upgraded.health().unwrap().schema_version, 62);
    let connection = upgraded.connection().unwrap();
    let row: (String, Vec<u8>, i64, i64, i64) = connection.query_row(
        "SELECT digest, bytes, prop_count, installed_revision, installed_at_ms FROM office_prop_packs",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).unwrap();
    assert_eq!(row, (DIGEST.into(), PACK.to_vec(), PROP_COUNT, 7, 123));
    let revision: i64 = connection
        .query_row("SELECT revision FROM office_prop_catalog", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(revision, 7);
    let block: (String, i64, String, i64) = connection
        .query_row(
            "SELECT identity_id, revision, layout, updated_at_ms FROM office_local_blocks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(block, ("old-id".into(), 3, layout, 123));
    let history_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM _migrations WHERE version = 18",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history_count, 1);
    let role: String = connection
        .query_row(
            "SELECT content FROM role_profiles WHERE identity_id = 'old-id'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(role, "original role");
    upgraded.close().unwrap();
}
