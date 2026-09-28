use super::*;
use crate::office_map::{OfficeMap, Tile};

fn id(value: usize) -> String {
    format!("10000000-0000-4000-8000-{value:012}")
}

fn office(value: usize, column: i32, row: i32) -> Module {
    Module {
        area: Area {
            id: id(value),
            name: format!("Office {value}"),
            kind: AreaKind::Personal { identity_id: None },
        },
        slot: Slot::Office { column, row },
        material: Material::Workshop,
    }
}

fn meeting(value: usize, index: u32) -> Module {
    Module {
        area: Area {
            id: id(value),
            name: format!("Meeting {value}"),
            kind: AreaKind::Meeting {
                room_id: format!("20000000-0000-4000-8000-{value:012}"),
            },
        },
        slot: Slot::Meeting { index },
        material: Material::Workshop,
    }
}

fn starter() -> ModuleDraft {
    ModuleDraft {
        layout: ModuleLayout::ShortLinks,
        primary_lobby_id: id(1),
        modules: vec![
            Module {
                area: Area {
                    id: id(1),
                    name: "Lobby".into(),
                    kind: AreaKind::Lobby,
                },
                slot: Slot::Lobby,
                material: Material::Workshop,
            },
            office(2, 0, -1),
            office(3, 1, -1),
            office(4, 0, 1),
            office(5, 1, 1),
        ],
    }
}

fn central_starter() -> ModuleDraft {
    let mut source = starter();
    source.layout = ModuleLayout::CentralGrid;
    source.modules[3].slot = Slot::Office { column: 0, row: 2 };
    source.modules[4].slot = Slot::Office { column: 1, row: 2 };
    source
}

#[test]
fn unified_area_uses_do_not_change_bounds_or_cardinal_connections() {
    let mut plan = central_starter();
    plan.layout = ModuleLayout::UnifiedAreas;
    plan.modules.push(office(6, 2, 0));
    plan.modules.push(office(7, 2, 3));
    let before = OfficeMap::from_modules(plan.clone()).unwrap();
    plan.modules[1].area.kind = AreaKind::Meeting { room_id: id(101) };
    plan.modules[5].area.kind = AreaKind::Meeting { room_id: id(102) };
    let after = OfficeMap::from_modules(plan.clone()).unwrap();
    assert_eq!(before.draft().floor, after.draft().floor);
    assert_eq!(before.draft().doors, after.draft().doors);
    assert_eq!(
        before
            .modules()
            .unwrap()
            .modules
            .iter()
            .map(|module| module.bounds(plan.layout).unwrap())
            .collect::<Vec<_>>(),
        after
            .modules()
            .unwrap()
            .modules
            .iter()
            .map(|module| module.bounds(plan.layout).unwrap())
            .collect::<Vec<_>>()
    );
    // Every platform may stand alone; raw freeform maps retain their stricter contract.
    assert_eq!(
        OfficeMap::new(after.draft().clone()).unwrap_err(),
        MapError::DisconnectedFloor
    );
    plan.modules.push(meeting(8, 4));
    assert_eq!(
        OfficeMap::from_modules(plan).unwrap_err(),
        MapError::InvalidModule
    );
}

#[test]
fn unified_slots_still_reject_collision_and_invalid_primary_lobby() {
    let mut plan = central_starter();
    plan.layout = ModuleLayout::UnifiedAreas;
    plan.modules.push(office(6, 0, -1));
    assert_eq!(
        OfficeMap::from_modules(plan.clone()).unwrap_err(),
        MapError::ModuleCollision
    );
    plan.modules.pop();
    plan.modules[0].area.kind = AreaKind::Personal { identity_id: None };
    assert_eq!(
        OfficeMap::from_modules(plan).unwrap_err(),
        MapError::InvalidModule
    );
}

#[test]
fn skybridges_connect_neighbors_without_empty_slot_branches() {
    let mut source = central_starter();
    source.layout = ModuleLayout::Skybridges;
    let map = OfficeMap::from_modules(source).unwrap();
    let at = |x, y| {
        map.draft()
            .floor
            .iter()
            .find(|span| span.y == y && span.start <= x && x < span.end)
    };
    for (x, y) in skybridge_samples("starterPublic") {
        assert!(at(x, y).is_some_and(|span| span.area_id.is_none()));
    }
    for (x, y) in skybridge_samples("starterEmpty") {
        assert!(at(x, y).is_none());
    }
}

#[test]
fn skybridge_meeting_spine_retains_access_across_an_empty_slot() {
    let mut source = central_starter();
    source.layout = ModuleLayout::Skybridges;
    source.modules.extend([meeting(6, 0), meeting(7, 2)]);
    let map = OfficeMap::from_modules(source).unwrap();
    let at = |x, y| {
        map.draft()
            .floor
            .iter()
            .find(|span| span.y == y && span.start <= x && x < span.end)
    };
    for (x, y) in skybridge_samples("meetingPublic") {
        assert!(at(x, y).is_some_and(|span| span.area_id.is_none()));
    }
    for (x, y) in skybridge_samples("meetingEmpty") {
        assert!(at(x, y).is_none());
    }
}

#[test]
fn independent_meetings_keep_slots_without_spine_or_portals() {
    let mut source = central_starter();
    source.layout = ModuleLayout::Skybridges;
    source.modules.extend([meeting(6, 0), meeting(7, 2)]);
    let old = OfficeMap::from_modules(source.clone()).unwrap();
    source.layout = ModuleLayout::IndependentMeetings;
    let islands = OfficeMap::from_modules(source.clone()).unwrap();
    let main = OfficeMap::from_modules(ModuleDraft {
        modules: source.modules[..5].to_vec(),
        ..source.clone()
    })
    .unwrap();
    assert_eq!(islands.draft().doors, main.draft().doors);
    assert_eq!(
        islands
            .draft()
            .floor
            .iter()
            .filter(|row| row.area_id.is_none())
            .collect::<Vec<_>>(),
        main.draft()
            .floor
            .iter()
            .filter(|row| row.area_id.is_none())
            .collect::<Vec<_>>()
    );
    for area in [id(6), id(7)] {
        assert_eq!(
            islands
                .draft()
                .floor
                .iter()
                .filter(|row| row.area_id.as_ref() == Some(&area))
                .collect::<Vec<_>>(),
            old.draft()
                .floor
                .iter()
                .filter(|row| row.area_id.as_ref() == Some(&area))
                .collect::<Vec<_>>()
        );
    }
    // Freeform input cannot use the modular exception to admit disconnected floor.
    assert_eq!(
        OfficeMap::new(islands.draft().clone()).unwrap_err(),
        MapError::DisconnectedFloor
    );
    source.modules.retain(|module| module.area.id != id(6));
    let removed = OfficeMap::from_modules(source).unwrap();
    assert_eq!(
        removed
            .draft()
            .floor
            .iter()
            .filter(|row| row.area_id.as_ref() == Some(&id(7)))
            .collect::<Vec<_>>(),
        islands
            .draft()
            .floor
            .iter()
            .filter(|row| row.area_id.as_ref() == Some(&id(7)))
            .collect::<Vec<_>>()
    );
}

#[test]
fn independent_meetings_do_not_allow_disconnected_personal_offices() {
    let mut source = central_starter();
    source.layout = ModuleLayout::IndependentMeetings;
    source.modules.truncate(1);
    source.modules.extend([meeting(6, 0), office(7, -2, 0)]);
    assert!(OfficeMap::from_modules(source).is_err());
}

fn skybridge_samples(key: &str) -> Vec<(i32, i32)> {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../contracts/modules-skybridge-vectors.json"
    ))
    .unwrap();
    serde_json::from_value(vectors[key].clone()).unwrap()
}

#[test]
fn skybridge_extended_row_has_only_cardinal_center_links() {
    let mut source = central_starter();
    source.layout = ModuleLayout::Skybridges;
    source.modules.extend([office(8, 2, -1), office(9, 3, -1)]);
    let map = OfficeMap::from_modules(source.clone()).unwrap();
    for (x, y) in skybridge_samples("extendedRowPublic") {
        assert!(
            map.draft()
                .floor
                .iter()
                .any(|s| s.y == y && s.start <= x && x < s.end && s.area_id.is_none())
        );
    }
    for (x, y) in skybridge_samples("extendedRowEmpty") {
        assert!(
            !map.draft()
                .floor
                .iter()
                .any(|s| s.y == y && s.start <= x && x < s.end)
        );
    }
    source
        .modules
        .retain(|m| m.slot != (Slot::Office { column: 2, row: -1 }));
    assert!(
        OfficeMap::from_modules(source).is_err(),
        "removing the middle platform must not invent a bypass"
    );
}

#[test]
fn skybridges_reject_isolated_diagonal_and_skipped_slots() {
    for (column, row) in [(-1, -1), (-2, 0)] {
        let mut source = central_starter();
        source.layout = ModuleLayout::Skybridges;
        source.modules.truncate(1);
        source.modules.push(office(9, column, row));
        assert!(OfficeMap::from_modules(source).is_err());
    }
}

#[test]
fn compact_routes_skip_unused_branches_and_meeting_slots_touch() {
    let mut source = central_starter();
    source.layout = ModuleLayout::CompactGrid;
    source.modules.truncate(1);
    source
        .modules
        .extend([office(2, -2, -3), meeting(6, 0), meeting(7, 1)]);
    let map = OfficeMap::from_modules(source.clone()).unwrap();
    let contains = |x, y| {
        map.draft()
            .floor
            .iter()
            .any(|span| span.y == y && span.start <= x && x < span.end)
    };
    assert!(contains(-88, -104));
    assert!(contains(50, -50));
    assert!(!contains(-88, -56));
    assert!(!contains(-110, -104));
    assert_eq!(
        source.modules[2].bounds(source.layout).unwrap().bottom(),
        source.modules[3].bounds(source.layout).unwrap().y
    );
    assert_eq!(
        source.modules[3]
            .bounds(ModuleLayout::CentralGrid)
            .unwrap()
            .y,
        48
    );
}

#[test]
fn central_axes_survive_empty_cells_and_only_use_public_floor() {
    let mut source = central_starter();
    source.modules.push(office(6, -2, 0));
    let map = OfficeMap::from_modules(source.clone()).unwrap();
    let walkable = map
        .draft()
        .floor
        .iter()
        .filter(|span| {
            span.area_id.is_none()
                || span.area_id.as_deref() == Some(source.primary_lobby_id.as_str())
        })
        .flat_map(|span| (span.start..span.end).map(move |x| (x, span.y)))
        .collect::<BTreeSet<_>>();
    for tile in [(-100, 44), (-40, 44), (-4, 44), (52, -40), (52, 130)] {
        assert!(
            walkable.contains(&tile),
            "missing independent corridor at {tile:?}"
        );
    }
    assert!(!walkable.contains(&(-40, 20))); // Empty cell remains space.
    assert!(!walkable.contains(&(20, -20))); // Private room is not a corridor.
    let mut pending = vec![(52, 44)];
    let mut reached = BTreeSet::new();
    while let Some((x, y)) = pending.pop() {
        if !walkable.contains(&(x, y)) || !reached.insert((x, y)) {
            continue;
        }
        pending.extend([(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]);
    }
    assert_eq!(reached, walkable);
    source.modules.push(office(7, -1, 0));
    let occupied = OfficeMap::from_modules(source.clone()).unwrap();
    source.modules.retain(|module| module.area.id != id(7));
    assert_eq!(
        OfficeMap::from_modules(source).unwrap().draft(),
        map.draft()
    );
    assert_eq!(
        occupied.geometry().area_at(Tile { x: -40, y: 20 }),
        Some(Some(id(7).as_str()))
    );
}

#[test]
fn central_lobby_cannot_overlap_old_south_slots_and_lattice_expansion_is_bounded() {
    let mut overlapping = starter();
    overlapping.layout = ModuleLayout::CentralGrid;
    assert_eq!(
        OfficeMap::from_modules(overlapping).unwrap_err(),
        MapError::ModuleCollision
    );
    let mut huge = central_starter();
    huge.modules.push(office(6, -70, -70));
    assert_eq!(
        OfficeMap::from_modules(huge).unwrap_err(),
        MapError::LimitExceeded
    );
}

#[test]
fn near_capacity_central_grid_rejects_expansion_but_accepts_an_empty_cell() {
    let mut source = central_starter();
    source.modules.push(office(6, -14, -18));
    assert_eq!(
        OfficeMap::from_modules(source.clone())
            .unwrap()
            .geometry()
            .tile_count(),
        260_160
    );
    let mut outside = source.clone();
    outside.modules.push(office(7, -15, -18));
    assert_eq!(
        OfficeMap::from_modules(outside).unwrap_err(),
        MapError::LimitExceeded
    );
    source.modules.push(office(7, -13, -18));
    assert_eq!(
        OfficeMap::from_modules(source)
            .unwrap()
            .geometry()
            .tile_count(),
        262_080
    );
}

#[test]
fn grid_corridors_join_without_walking_through_private_offices() {
    let mut source = starter();
    source.layout = ModuleLayout::Grid;
    let map = OfficeMap::from_modules(source.clone()).unwrap();
    assert_eq!(
        map.draft()
            .floor
            .iter()
            .map(|span| span.end - span.start)
            .sum::<i32>(),
        14_144
    );
    assert_eq!(map.draft().doors.len(), 96);
    let walkable = map
        .draft()
        .floor
        .iter()
        .filter(|span| {
            span.area_id.is_none()
                || span.area_id.as_deref() == Some(source.primary_lobby_id.as_str())
        })
        .flat_map(|span| (span.start..span.end).map(move |x| (x, span.y)))
        .collect::<BTreeSet<_>>();
    for tile in [(50, -40), (50, -4), (1, -4), (100, -4), (50, 44), (50, 80)] {
        assert!(
            walkable.contains(&tile),
            "missing lane/junction at {tile:?}"
        );
    }
    assert!(!walkable.contains(&(20, -20))); // Private office, not a shortcut.
    let mut pending = vec![(0, 0)];
    let mut reached = BTreeSet::new();
    while let Some((x, y)) = pending.pop() {
        if !walkable.contains(&(x, y)) || !reached.insert((x, y)) {
            continue;
        }
        pending.extend([(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]);
    }
    assert_eq!(reached, walkable);
    source.modules.push(office(6, 0, -2));
    source.modules.push(office(7, 1, -2));
    let expanded = OfficeMap::from_modules(source).unwrap();
    assert!(
        expanded.draft().floor.iter().any(|span| span.y == -52
            && span.start <= 50
            && 50 < span.end
            && span.area_id.is_none())
    );
}

#[test]
fn four_unassigned_equal_offices_and_one_lobby_have_derived_access() {
    let map = OfficeMap::from_modules(starter()).unwrap();
    let draft = map.draft();
    assert_eq!(draft.areas.len(), 5);
    assert_eq!(
        draft
            .floor
            .iter()
            .map(|span| span.end - span.start)
            .sum::<i32>(),
        12_224
    );
    assert_eq!(draft.doors.len(), 96);
    assert!(draft.doors.contains(&Edge {
        x: 20,
        y: -8,
        axis: Axis::Horizontal
    }));
    assert!(draft.doors.contains(&Edge {
        x: 20,
        y: 0,
        axis: Axis::Horizontal
    }));
    assert!(
        draft.floor.iter().any(|span| span.y == -4
            && span.start == 20
            && span.end == 28
            && span.area_id.is_none())
    );
    for module in &map.modules().unwrap().modules[1..] {
        assert_eq!(module.area.kind, AreaKind::Personal { identity_id: None });
        assert_eq!(
            (
                module.bounds(ModuleLayout::ShortLinks).unwrap().width,
                module.bounds(ModuleLayout::ShortLinks).unwrap().height
            ),
            (48, 40)
        );
    }
}

#[test]
fn cardinal_additions_use_existing_connectivity_and_do_not_move_other_modules() {
    for (column, row) in [(-1, 0), (0, -2), (0, 2), (2, -1)] {
        let original = starter();
        let mut next = original.clone();
        next.modules.push(office(6, column, row));
        let map = OfficeMap::from_modules(next).unwrap();
        for module in original.modules {
            let retained = map
                .modules()
                .unwrap()
                .modules
                .iter()
                .find(|entry| entry.area.id == module.area.id)
                .unwrap();
            assert_eq!(retained, &module);
        }
    }
    let mut detached = starter();
    detached.modules.push(office(6, 8, -8));
    assert_eq!(
        OfficeMap::from_modules(detached).unwrap_err(),
        MapError::DisconnectedFloor
    );
}

#[test]
fn material_switches_and_input_order_do_not_change_floor_doors_or_bindings() {
    let mut plan = starter();
    plan.modules[1].area.kind = AreaKind::Personal {
        identity_id: Some(id(90)),
    };
    let original = OfficeMap::from_modules(plan.clone()).unwrap();
    for material in [Material::Moonlight, Material::Copper] {
        plan.modules.reverse();
        for module in &mut plan.modules {
            module.material = material;
        }
        let map = OfficeMap::from_modules(plan.clone()).unwrap();
        assert_eq!(map.draft(), original.draft());
        assert!(
            map.modules()
                .unwrap()
                .modules
                .iter()
                .all(|module| module.material == material)
        );
    }
}

#[test]
fn removing_a_bridge_office_cannot_disconnect_a_retained_module() {
    let mut plan = starter();
    plan.modules.extend([office(6, -1, 0), office(7, -2, 0)]);
    let original = OfficeMap::from_modules(plan.clone()).unwrap();
    plan.modules.retain(|module| module.area.id != id(6));
    assert_eq!(
        OfficeMap::from_modules(plan).unwrap_err(),
        MapError::DisconnectedFloor
    );
    assert_eq!(original.modules().unwrap().modules.len(), 7);
    assert!(original.draft().areas.iter().any(|area| area.id == id(6)));
}

#[test]
fn meeting_spine_preserves_sparse_slots_when_an_earlier_room_is_removed() {
    let mut plan = starter();
    plan.modules.extend([meeting(6, 0), meeting(7, 2)]);
    let before = OfficeMap::from_modules(plan.clone()).unwrap();
    let retained = before
        .modules()
        .unwrap()
        .modules
        .iter()
        .find(|module| module.area.id == id(7))
        .unwrap()
        .clone();
    assert_eq!(
        retained.bounds(ModuleLayout::ShortLinks).unwrap(),
        ModuleRect {
            x: 120,
            y: 96,
            width: 48,
            height: 40
        }
    );
    plan.modules.retain(|module| module.area.id != id(6));
    let after = OfficeMap::from_modules(plan).unwrap();
    assert_eq!(after.modules().unwrap().modules.last().unwrap(), &retained);
    assert_eq!(
        after.geometry().area_at(Tile { x: 130, y: 100 }),
        Some(Some(id(7).as_str()))
    );
    assert!(after.draft().floor.iter().any(|span| span.y == 60
        && span.start == 112
        && span.end == 120
        && span.area_id.is_none()));
    assert!(after.draft().doors.contains(&Edge {
        x: 120,
        y: 112,
        axis: Axis::Vertical
    }));
    assert!(!after.draft().areas.iter().any(|area| area.id == id(6)));
}

#[test]
fn rejects_collisions_purpose_mismatches_and_missing_primary_lobby() {
    let mut overlap = starter();
    overlap.modules.push(office(6, 0, -1));
    assert_eq!(
        OfficeMap::from_modules(overlap).unwrap_err(),
        MapError::ModuleCollision
    );
    let mut reserved = starter();
    reserved.modules.push(office(6, 2, 1));
    assert_eq!(
        OfficeMap::from_modules(reserved).unwrap_err(),
        MapError::ModuleCollision
    );
    let mut wrong_kind = starter();
    wrong_kind.modules[1].slot = Slot::Meeting { index: 0 };
    assert_eq!(
        OfficeMap::from_modules(wrong_kind).unwrap_err(),
        MapError::InvalidModule
    );
    let mut no_lobby = starter();
    no_lobby.modules.remove(0);
    assert_eq!(
        OfficeMap::from_modules(no_lobby).unwrap_err(),
        MapError::PrimaryLobbyRequired
    );
}

#[test]
fn rejects_extreme_indices_without_overflow_or_unbounded_projection() {
    for column in [i32::MIN, i32::MAX, 73, -74] {
        let mut plan = starter();
        plan.modules.push(office(6, column, -1));
        assert_eq!(
            OfficeMap::from_modules(plan).unwrap_err(),
            MapError::InvalidModule
        );
    }
    let mut plan = starter();
    plan.modules.push(meeting(6, u32::MAX));
    assert_eq!(
        OfficeMap::from_modules(plan).unwrap_err(),
        MapError::InvalidModule
    );
    let mut large = starter();
    large.modules.resize(MAX_AREAS + 1, office(6, -1, 0));
    assert_eq!(
        OfficeMap::from_modules(large).unwrap_err(),
        MapError::LimitExceeded
    );
}

#[test]
fn legacy_topology_remains_readable_without_fabricating_a_module_plan() {
    let projected = OfficeMap::from_modules(starter()).unwrap();
    let legacy = OfficeMap::new(projected.draft().clone()).unwrap();
    assert!(legacy.modules().is_none());
    assert_eq!(legacy.draft(), projected.draft());
}
