use super::*;
use crate::OfficeStore;
use crate::test_support::TestDirectory;
use rusqlite::params;
use tmt_office_model::codec::office_block::local_layout_value;
use tmt_office_model::codec::office_world::world_value;
use tmt_office_model::office_block::BlockLayout;
use tmt_office_model::office_block::Furniture;
use tmt_office_model::office_block::FurnitureAsset;
use tmt_office_model::office_block::LocalBlockLayout;
use tmt_office_model::office_block::LocalBlockTarget;
use tmt_office_model::office_map::AreaKind;
use tmt_office_model::office_map::OfficeMap;
use tmt_office_model::office_world::Surface;
use tmt_office_model::office_world::WorldLayout;

const ALICE: &str = "10000000-0000-4000-8000-000000000001";
const TEMP: &str = "10000000-0000-4000-8000-000000000002";
const RETIRED: &str = "10000000-0000-4000-8000-000000000003";
const UNUSED: &str = "10000000-0000-4000-8000-000000000004";
const ROOM: &str = "20000000-0000-4000-8000-000000000001";

fn identity(storage: &OfficeStore, id: &str, lifetime: &str) {
    storage.connection().unwrap().execute(
        "INSERT INTO identities(id,name,canonical_name,lifetime,created_at,updated_at) VALUES(?,?,?,?, 'now','now')",
        params![id,id,id,lifetime],
    ).unwrap();
}
fn layout() -> LocalBlockLayout {
    LocalBlockLayout::from_legacy(
        &BlockLayout::new(vec![
            Furniture {
                asset: FurnitureAsset::Desk,
                x: 0,
                y: 0,
                rotation: 0,
            },
            Furniture {
                asset: FurnitureAsset::Plant,
                x: 28,
                y: 28,
                rotation: 0,
            },
        ])
        .unwrap(),
    )
}
fn seed(storage: &mut OfficeStore, id: &str, lifetime: &str) {
    identity(storage, id, lifetime);
    storage
        .apply_local_block(&LocalBlockTarget::Identity(id.into()), 0, &layout())
        .unwrap();
}
fn count(storage: &OfficeStore, table: &str) -> i64 {
    storage
        .connection()
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
fn save(
    storage: &mut OfficeStore,
    before: &LocalWorldSnapshot,
    layout: &WorldLayout,
    time: u64,
) -> Result<LocalWorldSnapshot, WorldStoreError> {
    storage.apply_local_world(
        before.revision,
        before.legacy_basis.as_deref(),
        layout,
        time,
    )
}
fn moved(world: &WorldLayout) -> WorldLayout {
    let mut objects = world.objects().to_vec();
    objects[0].placement.x += 1;
    WorldLayout::new(world.map().clone(), objects).unwrap()
}

#[test]
fn fresh_world_has_furnished_central_lobby_and_four_unassigned_offices_without_writes() {
    let directory = TestDirectory::new();
    let path = directory.path.join("starter.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    identity(&storage, ALICE, "saved");
    identity(&storage, TEMP, "temporary");
    let initial = storage.show_local_world().unwrap();
    let value = world_value(&initial.layout);
    assert_eq!(value["map"]["version"], 8);
    assert!(initial.layout.objects().iter().all(|object| {
        object.surface == tmt_office_model::office_world::Surface::Floor { base: None }
            && object.kind == tmt_office_model::office_world::ObjectKind::Decoration
    }));
    let modules = value["map"]["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 5);
    assert_eq!(
        modules
            .iter()
            .find(|module| module["slot"]["type"] == "lobby")
            .unwrap()["area"]["name"],
        "Lobby"
    );
    for (column, row) in [(0, -1), (1, -1), (0, 2), (1, 2)] {
        let module = modules
            .iter()
            .find(|module| {
                module["slot"] == serde_json::json!({"type":"office", "column":column, "row":row})
            })
            .unwrap();
        assert_eq!(
            module["area"]["binding"],
            serde_json::json!({"type":"personal", "identityId":null})
        );
        let workstation: Vec<_> =
            initial
                .layout
                .objects()
                .iter()
                .filter(|object| {
                    object.placement.prop.starts_with(
                        tmt_office_model::codec::office_prop::MODULAR_WORKSTATION_DIGEST,
                    ) && object.placement.x >= column * 56
                        && object.placement.x < column * 56 + 48
                        && object.placement.y >= row * 48
                        && object.placement.y < row * 48 + 40
                })
                .collect();
        assert_eq!(workstation.len(), 4);
        let desk = workstation
            .iter()
            .find(|object| object.placement.prop.ends_with("/workstation-desk"))
            .unwrap();
        let terminal = workstation
            .iter()
            .find(|object| object.placement.prop.ends_with("/workstation-terminal"))
            .unwrap();
        assert_eq!(terminal.placement.x, desk.placement.x + 4);
        assert_eq!(terminal.placement.y, desk.placement.y + 2);
        assert!(initial.layout.objects().iter().any(|object| {
            object.placement.prop.ends_with("/woven-rug")
                && object.placement.x == column * 56 + 12
                && object.placement.y == row * 48 + 18
        }));
        for object in workstation {
            let floor_bottom =
                object.placement.y - row * 48 + i32::from(object.placement.footprint_height);
            assert!(floor_bottom <= 40, "workstation stays inside its platform");
        }
    }
    assert!(initial.layout.objects().len() > 40);
    for (definition, key, width) in [
        ("tmt-discussion-board", "lobby-discussion-board", 12),
        ("tmt-whiteboard", "lobby-whiteboard", 16),
        ("tmt-broadcaster", "lobby-radio", 8),
    ] {
        let object = initial
            .layout
            .objects()
            .iter()
            .find(|object| {
                object
                    .extension
                    .as_ref()
                    .is_some_and(|extension| extension.definition == definition)
            })
            .unwrap();
        assert_eq!(
            object.placement.prop,
            format!(
                "{}/{key}",
                tmt_office_model::codec::office_prop::MODULAR_FACILITIES_DIGEST
            )
        );
        assert_eq!(object.placement.footprint_width, width);
        assert_eq!(object.placement.footprint_height, width);
    }
    let reception = initial
        .layout
        .objects()
        .iter()
        .filter(|object| {
            object
                .placement
                .prop
                .starts_with(tmt_office_model::codec::office_prop::MODULAR_RECEPTION_DIGEST)
        })
        .map(|object| {
            (
                object.placement.prop.rsplit('/').next().unwrap(),
                object.placement.footprint_width,
                object.placement.footprint_height,
                object.placement.x,
                object.placement.y,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reception,
        [
            ("reception-table", 16, 16, 16, 24),
            ("reception-armchair", 12, 12, 4, 26),
            ("reception-armchair", 12, 12, 34, 26),
            ("reception-armchair", 12, 12, 28, 50),
            ("reception-table", 16, 16, 72, 54),
            ("reception-armchair", 12, 12, 58, 58),
        ]
    );
    for (key, x, y) in [("oak-bookcase", 4, 50), ("reading-lamp", 18, 52)] {
        assert!(initial.layout.objects().iter().any(|object| {
            object.placement.prop
                == format!(
                    "{}/{key}",
                    tmt_office_model::codec::office_prop::STUDY_DIGEST
                )
                && object.placement.x == x
                && object.placement.y == y
        }));
    }
    let radio_index = initial
        .layout
        .objects()
        .iter()
        .position(|object| {
            object
                .extension
                .as_ref()
                .is_some_and(|extension| extension.definition == "tmt-broadcaster")
        })
        .unwrap();
    let table_index = initial
        .layout
        .objects()
        .iter()
        .position(|object| {
            object.placement.prop.ends_with("/reception-table") && object.placement.x == 72
        })
        .unwrap();
    assert!(
        radio_index > table_index,
        "radio must paint above its supporting table"
    );
    // The default stays useful as a public hub: independently movable furniture
    // flanks both centerline passages instead of occupying circulation.
    for object in initial.layout.objects() {
        let placement = &object.placement;
        if !matches!(object.surface, Surface::Floor { .. }) || placement.y < 0 || placement.y >= 88
        {
            continue;
        }
        let right = placement.x + i32::from(placement.footprint_width);
        let bottom = placement.y + i32::from(placement.footprint_height);
        assert!(
            right <= 48 || placement.x >= 56,
            "{} blocks the north/south passage",
            placement.prop
        );
        assert!(
            bottom <= 40 || placement.y >= 48,
            "{} blocks the east/west passage",
            placement.prop
        );
    }
    assert_eq!(
        initial
            .layout
            .objects()
            .iter()
            .filter(|object| object.extension.is_some())
            .count(),
        3
    );
    for table in [
        "office_local_worlds",
        "office_local_blocks",
        "office_whiteboards",
    ] {
        assert_eq!(
            count(&storage, table),
            0,
            "{table} was materialized by a read"
        );
    }
    assert_eq!(
        world_value(&storage.show_local_world().unwrap().layout),
        value
    );
    assert_eq!(
        storage.show_local_world().unwrap().legacy_basis,
        initial.legacy_basis
    );
    drop(storage);
    let mut storage = OfficeStore::open(&path).unwrap();
    assert_eq!(
        world_value(&storage.show_local_world().unwrap().layout),
        value
    );
    let saved = save(&mut storage, &initial, &initial.layout, 10).unwrap();
    assert_eq!(world_value(&saved.layout), value);
    // Explicit removal stays removed; reading never refurnishes a saved world.
    let empty = WorldLayout::new(saved.layout.map().clone(), vec![]).unwrap();
    let cleared = save(&mut storage, &saved, &empty, 20).unwrap();
    drop(storage);
    let mut storage = OfficeStore::open(&path).unwrap();
    assert_eq!(
        world_value(&storage.show_local_world().unwrap().layout),
        world_value(&cleared.layout)
    );
    assert!(
        storage
            .show_local_world()
            .unwrap()
            .layout
            .objects()
            .is_empty()
    );
}

#[test]
fn compact_default_does_not_replace_a_saved_central_grid_world() {
    let directory = TestDirectory::new();
    let path = directory.path.join("retained-central-grid.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    let initial = storage.show_local_world().unwrap();
    let mut retained = world_value(&initial.layout);
    retained["map"]["version"] = serde_json::json!(4);
    let layout = tmt_office_model::codec::office_world::decode_world(
        &serde_json::to_vec(&retained).unwrap(),
    )
    .unwrap();
    save(&mut storage, &initial, &layout, 10).unwrap();
    drop(storage);

    let mut storage = OfficeStore::open(&path).unwrap();
    let reopened = storage.show_local_world().unwrap();
    assert_eq!(world_value(&reopened.layout), retained);
    assert_eq!(reopened.revision, 1);
}

#[test]
fn module_source_survives_storage_reopen_and_material_edits_preserve_objects() {
    use tmt_office_model::office_map::modules::{Material, Module, ModuleDraft, Slot};
    let directory = TestDirectory::new();
    let path = directory.path.join("modules.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    // This scenario covers the retained short-link source, not the new v4 preset.
    storage
        .apply_local_block(
            &LocalBlockTarget::Lobby,
            0,
            &tmt_office_model::codec::office_block::default_local_layout(&LocalBlockTarget::Lobby),
        )
        .unwrap();
    let initial = storage.show_local_world().unwrap();
    let lobby = initial.layout.map().draft().areas[0].clone();
    let modules = ModuleDraft {
        layout: tmt_office_model::office_map::modules::ModuleLayout::ShortLinks,
        primary_lobby_id: lobby.id.clone(),
        modules: vec![Module {
            area: lobby,
            slot: Slot::Lobby,
            material: Material::Workshop,
        }],
    };
    let layout = WorldLayout::new(
        OfficeMap::from_modules(modules.clone()).unwrap(),
        initial.layout.objects().to_vec(),
    )
    .unwrap();
    let saved = save(&mut storage, &initial, &layout, 100).unwrap();
    assert_eq!(saved.layout.objects(), initial.layout.objects());
    assert_eq!(world_value(&saved.layout)["map"]["version"], 2);
    assert!(world_value(&saved.layout)["map"].get("floor").is_none());
    let encoded: String = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT layout_json FROM office_local_worlds WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let persisted: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(persisted["map"]["version"], 2);
    assert_eq!(persisted["map"]["modules"][0]["slot"]["type"], "lobby");
    assert!(persisted["map"].get("floor").is_none());
    assert!(persisted["map"].get("doors").is_none());
    drop(storage);
    let mut storage = OfficeStore::open(path).unwrap();
    let reopened = storage.show_local_world().unwrap();
    assert_eq!(reopened.layout.map().modules(), Some(&modules));
    assert_eq!(world_value(&reopened.layout), world_value(&saved.layout));
    let mut themed = modules.clone();
    themed.modules[0].material = Material::Copper;
    let next = WorldLayout::new(
        OfficeMap::from_modules(themed.clone()).unwrap(),
        reopened.layout.objects().to_vec(),
    )
    .unwrap();
    let changed = save(&mut storage, &reopened, &next, 200).unwrap();
    assert_eq!(changed.revision, reopened.revision + 1);
    assert_eq!(changed.layout.map().draft(), reopened.layout.map().draft());
    assert_eq!(changed.layout.objects(), reopened.layout.objects());
    assert_eq!(changed.layout.map().modules(), Some(&themed));
    assert!(matches!(
        save(&mut storage, &reopened, &layout, 300),
        Err(WorldStoreError::RevisionConflict)
    ));
    assert_eq!(
        world_value(&storage.show_local_world().unwrap().layout),
        world_value(&changed.layout)
    );
}

#[test]
fn observation_is_stable_read_only_and_new_identities_do_not_build_rooms() {
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    let first = storage.show_local_world().unwrap();
    identity(&storage, UNUSED, "saved");
    let second = storage.show_local_world().unwrap();
    assert_eq!(world_value(&first.layout), world_value(&second.layout));
    assert_eq!(first.legacy_basis, second.legacy_basis);
    assert_eq!(first.revision, 0);
    assert_eq!(first.layout.map().draft().areas.len(), 5);
    assert_eq!(first.layout.objects().len(), 46); // Furnished platforms without mounted props.
    assert_eq!(count(&storage, "office_local_worlds"), 0);
    assert_eq!(count(&storage, "office_local_blocks"), 0);
    let saved = save(&mut storage, &first, &first.layout, 100).unwrap();
    assert_eq!(saved.revision, 1);
    assert!(saved.changed);
    assert!(saved.legacy_basis.is_none());
    drop(storage);
    let mut storage = OfficeStore::open(path).unwrap();
    let restored = storage.show_local_world().unwrap();
    assert_eq!(restored.world_id, saved.world_id);
    assert_eq!(world_value(&restored.layout), world_value(&first.layout));
    assert_eq!(restored.updated_at_ms, 100);
    let same = save(&mut storage, &restored, &restored.layout, 200).unwrap();
    assert!(!same.changed);
    assert_eq!((same.revision, same.updated_at_ms), (1, 100));
    assert!(matches!(
        save(&mut storage, &first, &first.layout, 300),
        Err(WorldStoreError::RevisionConflict)
    ));
}

#[test]
fn migration_preserves_all_saved_temporary_retired_layouts_and_empty_lobby_override() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    seed(&mut storage, ALICE, "saved");
    seed(&mut storage, TEMP, "temporary");
    seed(&mut storage, RETIRED, "saved");
    identity(&storage, UNUSED, "saved");
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms=10 WHERE id=?",
            [RETIRED],
        )
        .unwrap();
    storage
        .apply_local_block(
            &LocalBlockTarget::Lobby,
            0,
            &LocalBlockLayout::new(vec![]).unwrap(),
        )
        .unwrap();
    let before = storage.show_local_world().unwrap();
    let decorations: Vec<_> = before
        .layout
        .objects()
        .iter()
        .filter(|object| object.extension.is_none())
        .collect();
    assert_eq!(decorations.len(), 6); // No default furniture resurrected.
    assert_eq!(before.layout.objects().len(), 9); // Existing bundled functional entries remain.
    let assignments: Vec<_> = before
        .layout
        .map()
        .draft()
        .areas
        .iter()
        .filter_map(|area| match &area.kind {
            AreaKind::Personal {
                identity_id: Some(id),
            } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(assignments, vec![ALICE]);
    assert_eq!(before.layout.map().draft().areas.len(), 4);
    for objects in decorations.chunks_exact(2) {
        assert_eq!(objects[1].placement.x - objects[0].placement.x, 28);
        assert_eq!(objects[1].placement.y - objects[0].placement.y, 28);
        assert_eq!(objects[0].placement.prop, layout().objects()[0].prop);
    }
    let saved = save(&mut storage, &before, &before.layout, 100).unwrap();
    assert_eq!(count(&storage, "office_local_blocks"), 0);
    assert_eq!(count(&storage, "identities"), 4);
    assert_eq!(world_value(&saved.layout), world_value(&before.layout));
    assert!(
        storage
            .apply_local_block(&LocalBlockTarget::Identity(ALICE.into()), 0, &layout())
            .is_err()
    );
    assert_eq!(count(&storage, "office_local_blocks"), 0); // Old binaries cannot resurrect a second owner.
}

#[test]
fn concurrent_legacy_edits_and_world_edits_are_fenced_without_rebasing() {
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    let mut a = OfficeStore::open(&path).unwrap();
    let mut b = OfficeStore::open(&path).unwrap();
    let draft = a.show_local_world().unwrap();
    b.apply_local_block(
        &LocalBlockTarget::Lobby,
        0,
        &LocalBlockLayout::new(vec![]).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        save(&mut a, &draft, &draft.layout, 100),
        Err(WorldStoreError::RevisionConflict)
    ));
    let current = a.show_local_world().unwrap();
    assert_eq!(current.layout.objects().len(), 3);
    assert!(
        current
            .layout
            .objects()
            .iter()
            .all(|object| object.extension.is_some())
    );
    let saved = save(&mut a, &current, &current.layout, 101).unwrap();
    let other = b.show_local_world().unwrap();
    let mut map = saved.layout.map().draft().clone();
    map.areas[0].name = "Reception".into();
    let changed = WorldLayout::new(OfficeMap::new(map).unwrap(), vec![]).unwrap();
    save(&mut a, &saved, &changed, 102).unwrap();
    assert!(matches!(
        save(&mut b, &other, &other.layout, 103),
        Err(WorldStoreError::RevisionConflict)
    ));
    assert_eq!(
        b.show_local_world().unwrap().layout.map().draft().areas[0].name,
        "Reception"
    );
}

#[test]
fn failure_after_world_update_rolls_back_layout_revision_and_retired_rows_together() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    seed(&mut storage, ALICE, "saved");
    let before = storage.show_local_world().unwrap();
    storage.connection().unwrap().execute_batch("CREATE TRIGGER fail_cutover BEFORE DELETE ON office_local_blocks BEGIN SELECT RAISE(ABORT,'test cutover failure'); END;").unwrap();
    assert!(matches!(
        save(&mut storage, &before, &before.layout, 100),
        Err(WorldStoreError::Storage(_))
    ));
    assert_eq!(count(&storage, "office_local_blocks"), 1);
    let after = storage.show_local_world().unwrap();
    assert_eq!(after.revision, 0);
    assert_eq!(after.legacy_basis, before.legacy_basis);
    assert_eq!(world_value(&after.layout), world_value(&before.layout));
    storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_cutover")
        .unwrap();
    save(&mut storage, &after, &after.layout, 101).unwrap();
}

#[test]
fn eligibility_is_checked_inside_save_and_retirement_never_grants_a_new_assignment() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    seed(&mut storage, TEMP, "temporary");
    identity(&storage, ALICE, "saved");
    let before = storage.show_local_world().unwrap();
    let assign = |id: &str| {
        let mut map = before.layout.map().draft().clone();
        map.areas
            .iter_mut()
            .find(|area| matches!(area.kind, AreaKind::Personal { .. }))
            .unwrap()
            .kind = AreaKind::Personal {
            identity_id: Some(id.into()),
        };
        WorldLayout::new(
            OfficeMap::new(map).unwrap(),
            before.layout.objects().to_vec(),
        )
        .unwrap()
    };
    assert!(matches!(
        save(&mut storage, &before, &assign(TEMP), 100),
        Err(WorldStoreError::IdentityIneligible)
    ));
    assert_eq!(count(&storage, "office_local_blocks"), 1);
    let assigned = save(&mut storage, &before, &assign(ALICE), 101).unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms=102 WHERE id=?",
            [ALICE],
        )
        .unwrap();
    let retained = save(&mut storage, &assigned, &moved(&assigned.layout), 103).unwrap();
    let mut map = retained.layout.map().draft().clone();
    let area = map
        .areas
        .iter_mut()
        .find(|area| matches!(area.kind, AreaKind::Personal { .. }))
        .unwrap();
    area.id = "40000000-0000-4000-8000-000000000001".into();
    let new_id = area.id.clone();
    for span in &mut map.floor {
        if span
            .area_id
            .as_ref()
            .is_some_and(|id| id != &map.primary_lobby_id)
        {
            span.area_id = Some(new_id.clone());
        }
    }
    let relocated = WorldLayout::new(
        OfficeMap::new(map).unwrap(),
        retained.layout.objects().to_vec(),
    )
    .unwrap();
    assert!(matches!(
        save(&mut storage, &retained, &relocated, 104),
        Err(WorldStoreError::IdentityIneligible)
    ));
}

#[test]
fn lost_artwork_is_retained_and_moveable_but_cannot_be_duplicated_or_forged() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    seed(&mut storage, ALICE, "saved");
    let mut value = local_layout_value(&layout());
    value["objects"][0]["prop"] = serde_json::json!(format!("sha256:{}/missing", "a".repeat(64)));
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE office_local_blocks SET layout=?",
            [value.to_string()],
        )
        .unwrap();
    let before = storage.show_local_world().unwrap();
    let saved = save(&mut storage, &before, &before.layout, 100).unwrap();
    let missing_index = saved
        .layout
        .objects()
        .iter()
        .position(|object| object.placement.prop.contains("/missing"))
        .unwrap();
    let mut objects = saved.layout.objects().to_vec();
    objects[missing_index].placement.x += 1;
    let moved = WorldLayout::new(saved.layout.map().clone(), objects).unwrap();
    let moved = save(&mut storage, &saved, &moved, 101).unwrap();
    let mut objects = moved.layout.objects().to_vec();
    let mut copy = objects[missing_index].clone();
    copy.id = "50000000-0000-4000-8000-000000000001".into();
    objects.push(copy);
    let duplicate = WorldLayout::new(moved.layout.map().clone(), objects).unwrap();
    assert!(matches!(
        save(&mut storage, &moved, &duplicate, 102),
        Err(WorldStoreError::PropUnavailable)
    ));
}

#[test]
fn spatial_area_removal_keeps_core_meeting_roster_and_floor_furniture() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state.db");
    let mut storage = OfficeStore::open(&database).unwrap();
    seed(&mut storage, ALICE, "saved");
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO office_meeting_rooms(room_id,name,revision) VALUES(?,'Review',1)",
            [ROOM],
        )
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO office_meeting_members VALUES(?,?)",
            params![ROOM, ALICE],
        )
        .unwrap();
    let before = storage.show_local_world().unwrap();
    let mut map = before.layout.map().draft().clone();
    let area = map
        .areas
        .iter_mut()
        .find(|area| matches!(area.kind, AreaKind::Personal { .. }))
        .unwrap();
    let id = area.id.clone();
    area.kind = AreaKind::Meeting {
        room_id: ROOM.into(),
    };
    let meeting = WorldLayout::new(
        OfficeMap::new(map).unwrap(),
        before.layout.objects().to_vec(),
    )
    .unwrap();
    let saved = save(&mut storage, &before, &meeting, 100).unwrap();
    // Retiring communication does not rewrite a map or prevent editing retained
    // geometry. A different area cannot acquire the retired UUID as a new binding.
    use tmt_core::room::RoomRepository;
    // Rooms are core-owned; retire through the core store on the same file.
    let mut core = tmt_adapters::storage::Storage::open(&database).unwrap();
    core.retire_meeting_room(ROOM, 1).unwrap();
    core.close().unwrap();
    assert_eq!(
        world_value(&storage.show_local_world().unwrap().layout),
        world_value(&saved.layout)
    );
    let mut changed = saved.layout.map().draft().clone();
    changed
        .areas
        .iter_mut()
        .find(|area| area.id == id)
        .unwrap()
        .name = "Retained meeting".into();
    let changed = WorldLayout::new(
        OfficeMap::new(changed).unwrap(),
        saved.layout.objects().to_vec(),
    )
    .unwrap();
    let saved = save(&mut storage, &saved, &changed, 101).unwrap();
    let mut reassigned = saved.layout.map().draft().clone();
    let replacement_id = "50000000-0000-4000-8000-000000000009".to_string();
    reassigned
        .areas
        .iter_mut()
        .find(|area| area.id == id)
        .unwrap()
        .id = replacement_id.clone();
    for span in &mut reassigned.floor {
        if span.area_id.as_deref() == Some(&id) {
            span.area_id = Some(replacement_id.clone());
        }
    }
    let reassigned = WorldLayout::new(
        OfficeMap::new(reassigned).unwrap(),
        saved.layout.objects().to_vec(),
    )
    .unwrap();
    assert!(matches!(
        save(&mut storage, &saved, &reassigned, 102),
        Err(WorldStoreError::RoomMissing)
    ));
    assert_eq!(
        world_value(&storage.show_local_world().unwrap().layout),
        world_value(&saved.layout)
    );
    let mut map = saved.layout.map().draft().clone();
    map.areas.retain(|area| area.id != id);
    for span in &mut map.floor {
        if span.area_id.as_deref() == Some(&id) {
            span.area_id = None;
        }
    }
    map.doors.retain(|door| door.x < 72);
    let detached = WorldLayout::new(
        OfficeMap::new(map).unwrap(),
        saved.layout.objects().to_vec(),
    )
    .unwrap();
    let after = save(&mut storage, &saved, &detached, 101).unwrap();
    assert_eq!(after.layout.objects(), saved.layout.objects());
    assert_eq!(
        after.layout.map().geometry().tile_count(),
        saved.layout.map().geometry().tile_count()
    );
    assert_eq!(count(&storage, "office_meeting_rooms"), 1);
    assert_eq!(count(&storage, "office_meeting_members"), 1);
}

#[test]
fn compact_legacy_tokens_are_read_without_reseeding_or_erasing_their_objects() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    seed(&mut storage, ALICE, "saved");
    storage
        .connection()
        .unwrap()
        .execute("UPDATE office_local_blocks SET layout='[\"d000\"]'", [])
        .unwrap();
    let before = storage.show_local_world().unwrap();
    assert_eq!(before.layout.objects().len(), 14);
    save(&mut storage, &before, &before.layout, 100).unwrap();
    assert_eq!(
        storage.show_local_world().unwrap().layout.objects().len(),
        14
    );
}

#[test]
fn invalid_legacy_payload_is_not_replaced_by_a_default_world() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    seed(&mut storage, ALICE, "saved");
    let valid = storage.show_local_world().unwrap();
    storage
        .connection()
        .unwrap()
        .execute("UPDATE office_local_blocks SET layout='not-json'", [])
        .unwrap();
    assert!(matches!(
        storage.show_local_world(),
        Err(WorldStoreError::StoredInvalid)
    ));
    assert!(matches!(
        save(&mut storage, &valid, &valid.layout, 200),
        Err(WorldStoreError::StoredInvalid)
    ));
    let retained: String = storage
        .connection()
        .unwrap()
        .query_row("SELECT layout FROM office_local_blocks", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(retained, "not-json");
    let revision: i64 = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT layout_revision FROM office_local_worlds",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, 0);
}

#[test]
fn oversized_legacy_inventory_fails_without_truncating_the_source() {
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let preview = storage.show_local_world().unwrap();
    let count = tmt_office_model::office_map::MAX_AREAS + 1;
    let transaction = storage.connection_mut().unwrap().transaction().unwrap();
    for index in 1..=count {
        let id = format!("10000000-0000-4000-8000-{index:012x}");
        transaction
            .execute(
                "INSERT INTO identities(id,name,canonical_name,lifetime,created_at,updated_at)
             VALUES(?,?,?,'saved','now','now')",
                params![id, id, id],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO office_local_blocks VALUES(?,'identity',?,1,'[]',123)",
                params![id, id],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    assert!(matches!(
        storage.show_local_world(),
        Err(WorldStoreError::MigrationInvalid)
    ));
    assert!(matches!(
        save(&mut storage, &preview, &preview.layout, 200),
        Err(WorldStoreError::MigrationInvalid)
    ));
    let (retained, intact): (i64, i64) = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT count(*),sum(layout='[]' AND revision=1 AND updated_at_ms=123)
         FROM office_local_blocks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((retained, intact), (count as i64, count as i64));
    assert_eq!(self::count(&storage, "office_local_worlds"), 0);
}

#[test]
fn functional_objects_share_existing_resources_and_removal_never_resets_the_preset() {
    use tmt_office_model::office_whiteboard::document::SaveDocument;
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    let preview = storage.show_local_world().unwrap();
    assert_eq!(count(&storage, "office_whiteboards"), 0);
    let initial = save(&mut storage, &preview, &preview.layout, 100).unwrap();
    assert_eq!(count(&storage, "office_whiteboards"), 0);
    assert_eq!(count(&storage, "office_board_entries"), 0);
    assert_eq!(count(&storage, "request_attempts"), 0);
    let scene = tmt_office_model::codec::office_whiteboard::decode_scene(include_bytes!(
        "../../../../../../extensions/tmt-office/contracts/whiteboard-scene-v1.json"
    ))
    .unwrap();
    storage
        .save_whiteboard(
            &SaveDocument {
                document_id: "lobby".into(),
                expected_revision: 0,
                operation_id: "70000000-0000-4000-8000-000000000001".into(),
                scene,
            },
            101,
        )
        .unwrap();
    let resource = storage.show_whiteboard("lobby").unwrap();
    let mut objects = initial.layout.objects().to_vec();
    let mut second = objects
        .iter()
        .find(|object| {
            object
                .extension
                .as_ref()
                .is_some_and(|attachment| attachment.definition == "tmt-whiteboard")
        })
        .unwrap()
        .clone();
    second.id = "70000000-0000-4000-8000-000000000002".into();
    second.placement.y += 1;
    objects.push(second);
    let multiple = WorldLayout::new(initial.layout.map().clone(), objects).unwrap();
    let moved = save(&mut storage, &initial, &multiple, 102).unwrap();
    assert_eq!(storage.show_whiteboard("lobby").unwrap(), resource);
    assert_eq!(count(&storage, "office_whiteboards"), 1);
    let removed = WorldLayout::new(
        moved.layout.map().clone(),
        moved
            .layout
            .objects()
            .iter()
            .filter(|object| object.extension.is_none())
            .cloned()
            .collect(),
    )
    .unwrap();
    let removed = save(&mut storage, &moved, &removed, 103).unwrap();
    assert_eq!(removed.layout.objects().len(), 43);
    assert_eq!(storage.show_whiteboard("lobby").unwrap(), resource);
    drop(storage);
    let mut reopened = OfficeStore::open(path).unwrap();
    assert_eq!(
        world_value(&reopened.show_local_world().unwrap().layout),
        world_value(&removed.layout)
    );
    assert_eq!(reopened.show_whiteboard("lobby").unwrap(), resource);
    assert_eq!(count(&reopened, "office_whiteboard_operations"), 1);
    assert_eq!(count(&reopened, "request_attempts"), 0);
}

#[test]
fn save_rejects_known_binding_mismatch_without_materializing_resources() {
    use tmt_office_model::office_extension::ResourceBinding;
    let directory = TestDirectory::new();
    let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
    let preview = storage.show_local_world().unwrap();
    let initial = save(&mut storage, &preview, &preview.layout, 100).unwrap();
    let mut objects = initial.layout.objects().to_vec();
    let board = objects
        .iter_mut()
        .find_map(|object| {
            object
                .extension
                .as_mut()
                .filter(|attachment| attachment.definition == "tmt-discussion-board")
        })
        .unwrap();
    board.binding = ResourceBinding::Whiteboard {
        document_id: "lobby".into(),
    };
    let invalid = WorldLayout::new(initial.layout.map().clone(), objects).unwrap();
    assert!(matches!(
        save(&mut storage, &initial, &invalid, 101),
        Err(WorldStoreError::Extension(
            tmt_office_model::codec::office_extension::ExtensionError::BindingMismatch
        ))
    ));
    let current = storage.show_local_world().unwrap();
    assert_eq!(current.revision, initial.revision);
    assert_eq!(world_value(&current.layout), world_value(&initial.layout));
    assert_eq!(count(&storage, "office_whiteboards"), 0);
    assert_eq!(count(&storage, "request_attempts"), 0);
}

#[test]
fn external_link_is_layout_data_and_survives_reopen_without_creating_resources() {
    use tmt_office_model::office_extension::{ExtensionAttachment, ResourceBinding};
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    let preview = storage.show_local_world().unwrap();
    let mut objects = preview.layout.objects().to_vec();
    objects[0].extension = Some(ExtensionAttachment {
        definition: "tmt-link".into(),
        binding: ResourceBinding::ExternalLink {
            url: "https://example.com/docs?q=office#wall".into(),
        },
    });
    let layout = WorldLayout::new(preview.layout.map().clone(), objects.clone()).unwrap();
    let saved = save(&mut storage, &preview, &layout, 100).unwrap();
    objects[0].extension.as_mut().unwrap().binding = ResourceBinding::ExternalLink {
        url: "javascript:alert(1)".into(),
    };
    assert!(WorldLayout::new(preview.layout.map().clone(), objects).is_err());
    drop(storage);
    let mut reopened = OfficeStore::open(path).unwrap();
    let current = reopened.show_local_world().unwrap();
    assert_eq!(current.revision, saved.revision);
    assert_eq!(world_value(&current.layout), world_value(&layout));
    assert_eq!(count(&reopened, "office_whiteboards"), 0);
    assert_eq!(count(&reopened, "office_board_entries"), 0);
    assert_eq!(count(&reopened, "request_attempts"), 0);
}

#[test]
fn wall_catalog_objects_persist_with_native_mount_rules_without_a_second_store() {
    use serde_json::json;
    use tmt_office_model::codec::office_prop::WALL_DIGEST;
    use tmt_office_model::codec::office_prop::builtin_by_digest;
    use tmt_office_model::codec::office_world::decode_world;
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    // Fixed legacy geometry isolates catalog/mount admission from starter design.
    storage
        .apply_local_block(
            &LocalBlockTarget::Lobby,
            0,
            &tmt_office_model::codec::office_block::default_local_layout(&LocalBlockTarget::Lobby),
        )
        .unwrap();
    let initial = storage.show_local_world().unwrap();
    let mut document = world_value(&initial.layout);
    let pack = builtin_by_digest(WALL_DIGEST).unwrap();
    for (index, (key, kind, x, elevation)) in [
        ("observatory-window", "window", 0, 3),
        ("brass-wall-lamp", "wallLight", 12, 3),
        ("orbit-poster", "decoration", 15, 3),
        ("crew-sign", "decoration", 21, 3),
        ("link-plaque", "decoration", 0, 12),
    ]
    .iter()
    .enumerate()
    {
        let prop = pack.prop(key).unwrap();
        let mut placement = json!({ "prop": format!("{WALL_DIGEST}/{key}"),
            "footprint": prop.footprint, "x": x, "y": 0, "rotation": 0 });
        if *key == "crew-sign" {
            placement["customization"] = json!({"text": "CREW"});
        }
        document["objects"].as_array_mut().unwrap().push(json!({
            "id": format!("50000000-0000-4000-8000-{:012}", index + 1),
            "kind": kind, "placement": placement,
            "surface": { "type": "wall", "axis": "horizontal", "face": "positive", "elevation": elevation },
            "extension": null,
        }));
    }
    let layout = decode_world(&serde_json::to_vec(&document).unwrap()).unwrap();
    let saved = save(&mut storage, &initial, &layout, 200).unwrap();
    let mut invalid = document.clone();
    let index = initial.layout.objects().len();
    invalid["objects"][index]["surface"] = json!({ "type": "floor" });
    assert!(decode_world(&serde_json::to_vec(&invalid).unwrap()).is_err());
    drop(storage);
    let mut reopened = OfficeStore::open(path).unwrap();
    let current = reopened.show_local_world().unwrap();
    assert_eq!(current.revision, saved.revision);
    assert_eq!(world_value(&current.layout), document);
    assert_eq!(count(&reopened, "office_whiteboards"), 0);
    assert_eq!(count(&reopened, "office_board_entries"), 0);
    assert_eq!(count(&reopened, "request_attempts"), 0);
}

/// Ported from the core schema-28 upgrade test: after the upgrade, reading a
/// retained lobby block projects an unsaved world without materializing it,
/// and the first explicit save retires the legacy row.
#[test]
fn upgraded_legacy_lobby_is_projected_unsaved_and_retired_by_the_first_save() {
    const WORLD: &str = "11111111-1111-4111-8111-111111111111";
    const LOBBY: &str = "22222222-2222-4222-8222-222222222222";
    let directory = TestDirectory::new();
    let path = directory.path.join("upgraded.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    // Whitespace is intentional: reading must not re-encode old layouts.
    let layout = "  {\"version\":2,\"objects\":[]}  ";
    storage
        .connection()
        .unwrap()
        .execute_batch(&format!(
            "INSERT INTO office_local_worlds(singleton,id,created_at_ms) VALUES(1,'{WORLD}',100);"
        ))
        .unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO office_local_blocks VALUES(?,'lobby',NULL,7,?,123)",
            params![LOBBY, layout],
        )
        .unwrap();
    let legacy = |storage: &OfficeStore| -> (String, i64, Vec<u8>, i64) {
        storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT block_id,revision,CAST(layout AS BLOB),updated_at_ms FROM office_local_blocks",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    };
    let before = legacy(&storage);
    let preview = storage.show_local_world().unwrap();
    assert_eq!(preview.world_id.as_deref(), Some(WORLD));
    assert_eq!(preview.revision, 0);
    assert_eq!(preview.layout.map().draft().primary_lobby_id, LOBBY);
    assert_eq!(preview.layout.objects().len(), 3);
    assert!(
        preview
            .layout
            .objects()
            .iter()
            .all(|object| object.extension.is_some())
    );
    assert_eq!(legacy(&storage), before);
    let unmaterialized: (i64, Option<String>, i64) = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT layout_revision,layout_json,layout_updated_at_ms FROM office_local_worlds",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(unmaterialized, (0, None, 0));
    let saved = storage
        .apply_local_world(0, preview.legacy_basis.as_deref(), &preview.layout, 200)
        .unwrap();
    assert_eq!(saved.world_id.as_deref(), Some(WORLD));
    assert_eq!(saved.revision, 1);
    storage.close().unwrap();

    let mut reopened = OfficeStore::open(&path).unwrap();
    let restored = reopened.show_local_world().unwrap();
    assert_eq!(restored.world_id, saved.world_id);
    assert_eq!(restored.revision, 1);
    assert_eq!(restored.updated_at_ms, 200);
    assert_eq!(restored.layout.objects(), saved.layout.objects());
    assert_eq!(
        reopened
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM office_local_blocks", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn references_retired_inside_the_preflight_window_commit_like_save_then_retire() {
    let directory = TestDirectory::new();
    let path = directory.path.join("race.db");
    let mut storage = OfficeStore::open(&path).unwrap();
    identity(&storage, ALICE, "saved");
    let before = storage.show_local_world().unwrap();
    let mut map = before.layout.map().draft().clone();
    map.areas
        .iter_mut()
        .find(|area| matches!(area.kind, AreaKind::Personal { .. }))
        .unwrap()
        .kind = AreaKind::Personal {
        identity_id: Some(ALICE.into()),
    };
    let assigned = WorldLayout::new(
        OfficeMap::new(map).unwrap(),
        before.layout.objects().to_vec(),
    )
    .unwrap();
    crate::test_support::at_next_preflight_execute(
        &path,
        "UPDATE identities SET retired_at_ms=5 WHERE id=?",
        ALICE,
    );
    let saved = save(&mut storage, &before, &assigned, 100).unwrap();
    // Serial equivalent: the assignment is retained and stays editable.
    let reread = storage.show_local_world().unwrap();
    assert_eq!(reread.layout.map().draft(), saved.layout.map().draft());
    let retained = save(&mut storage, &saved, &moved(&saved.layout), 101).unwrap();

    // A room retired inside the window: the new meeting area commits and is retained.
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO office_meeting_rooms(room_id,name,revision) VALUES(?,'Review',1)",
            [ROOM],
        )
        .unwrap();
    let mut map = retained.layout.map().draft().clone();
    let meeting = map
        .areas
        .iter_mut()
        .find(|area| matches!(area.kind, AreaKind::Personal { identity_id: None }))
        .unwrap();
    meeting.kind = AreaKind::Meeting {
        room_id: ROOM.into(),
    };
    let with_meeting = WorldLayout::new(
        OfficeMap::new(map).unwrap(),
        retained.layout.objects().to_vec(),
    )
    .unwrap();
    crate::test_support::at_next_preflight_execute(
        &path,
        "UPDATE office_meeting_rooms SET retired=1,revision=revision+1 WHERE room_id=?",
        ROOM,
    );
    let met = save(&mut storage, &retained, &with_meeting, 102).unwrap();
    save(&mut storage, &met, &moved(&met.layout), 103).unwrap();
}
