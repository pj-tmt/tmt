use crate::codec::office_map::{MapCodecError, decode_map, map_value};
use crate::codec::office_world::{decode_world, world_value};
use serde_json::{Value, json};

fn fixture() -> Value {
    json!({"version": 2, "primaryLobbyId": "10000000-0000-4000-8000-000000000001",
    "modules": [
        {"area": {"id": "10000000-0000-4000-8000-000000000001", "name": "Lobby", "binding": {"type": "lobby"}},
            "slot": {"type": "lobby"}, "material": "workshop"},
        {"area": {"id": "10000000-0000-4000-8000-000000000002", "name": "Studio", "binding": {"type": "personal", "identityId": null}},
            "slot": {"type": "office", "column": 0, "row": -1}, "material": "moonlight"}
    ]})
}

#[test]
fn skybridge_source_round_trips_without_reinterpreting_old_versions() {
    let mut source = fixture();
    for version in [4, 5, 6, 7] {
        source["version"] = json!(version);
        let admitted = decode_map(&serde_json::to_vec(&source).unwrap()).unwrap();
        assert_eq!(map_value(&admitted), source);
        assert_eq!(
            decode_map(&serde_json::to_vec(&map_value(&admitted)).unwrap())
                .unwrap()
                .draft(),
            admitted.draft()
        );
    }
}

#[test]
fn independent_meeting_projection_matches_shared_literal_vectors() {
    use crate::office_map::Tile;
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/modules-island-vectors.json"
    ))
    .unwrap();
    let source = &corpus["map"];
    let map = decode_map(&serde_json::to_vec(source).unwrap()).unwrap();
    assert_eq!(map_value(&map), *source);
    assert_eq!(
        map.geometry().tile_count(),
        corpus["floorTiles"].as_u64().unwrap() as usize
    );
    assert_eq!(
        map.draft().doors.len(),
        corpus["doorEdges"].as_u64().unwrap() as usize
    );
    for (key, expected) in [("publicSamples", Some(None)), ("emptySamples", None)] {
        for sample in corpus[key].as_array().unwrap() {
            let tile = Tile {
                x: sample[0].as_i64().unwrap() as i32,
                y: sample[1].as_i64().unwrap() as i32,
            };
            assert_eq!(map.geometry().area_at(tile), expected, "{key} at {tile:?}");
        }
    }
    let mut legacy = source.clone();
    legacy["version"] = json!(6);
    let old = decode_map(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(old.geometry().tile_count() > map.geometry().tile_count());
    assert_eq!(map_value(&old), legacy);
}

#[test]
fn compact_revision_round_trips_without_reinterpreting_central_grid() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/modules-central-grid-vectors.json"
    ))
    .unwrap();
    let old = corpus["map"].clone();
    let mut compact = old.clone();
    compact["version"] = json!(5);
    let decode = |value: &Value| decode_map(&serde_json::to_vec(value).unwrap()).unwrap();
    let old_map = decode(&old);
    let next = decode(&compact);
    assert_eq!(map_value(&next), compact);
    assert_eq!(decode(&map_value(&next)).draft(), next.draft());
    assert_eq!(map_value(&old_map), old);
    assert_eq!(decode(&old).draft(), old_map.draft());
    assert_ne!(next.draft(), old_map.draft());
}

#[test]
fn central_lobby_lattice_and_sparse_meetings_match_literal_vectors() {
    use crate::office_map::{Axis, Edge, Tile};
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/modules-central-grid-vectors.json"
    ))
    .unwrap();
    let source = &corpus["map"];
    let map = decode_map(&serde_json::to_vec(source).unwrap()).unwrap();
    assert_eq!(map_value(&map), *source);
    for sample in corpus["publicSamples"].as_array().unwrap() {
        let tile = Tile {
            x: sample[0].as_i64().unwrap() as i32,
            y: sample[1].as_i64().unwrap() as i32,
        };
        assert_eq!(
            map.geometry().area_at(tile),
            Some(None),
            "missing public floor at {tile:?}"
        );
    }
    for sample in corpus["emptySamples"].as_array().unwrap() {
        let tile = Tile {
            x: sample[0].as_i64().unwrap() as i32,
            y: sample[1].as_i64().unwrap() as i32,
        };
        assert_eq!(
            map.geometry().area_at(tile),
            None,
            "invented floor at {tile:?}"
        );
    }
    for sample in corpus["lobbyDoorSamples"].as_array().unwrap() {
        assert!(map.draft().doors.contains(&Edge {
            x: sample["x"].as_i64().unwrap() as i32,
            y: sample["y"].as_i64().unwrap() as i32,
            axis: if sample["axis"] == "horizontal" {
                Axis::Horizontal
            } else {
                Axis::Vertical
            },
        }));
    }
    let bounds = map.modules().unwrap().modules[0]
        .bounds(crate::office_map::modules::ModuleLayout::CentralGrid)
        .unwrap();
    assert_eq!(
        json!({"x":bounds.x,"y":bounds.y,"width":bounds.width,"height":bounds.height}),
        corpus["lobbyBounds"]
    );
    let mut reversed = source.clone();
    reversed["modules"].as_array_mut().unwrap().reverse();
    assert_eq!(
        decode_map(&serde_json::to_vec(&reversed).unwrap())
            .unwrap()
            .draft(),
        map.draft()
    );
}

#[test]
fn grid_version_does_not_reinterpret_existing_short_link_sources() {
    let old = fixture();
    let mut next = old.clone();
    next["version"] = json!(3);
    let decode = |value: &Value| decode_map(&serde_json::to_vec(value).unwrap()).unwrap();
    let old_map = decode(&old);
    let grid = decode(&next);
    assert_eq!(map_value(&old_map), old);
    assert_eq!(map_value(&grid), next);
    assert_eq!(old_map.draft().areas, grid.draft().areas);
    assert_eq!(old_map.draft().doors, grid.draft().doors);
    let at = |map: &crate::office_map::OfficeMap, x, y| {
        map.draft()
            .floor
            .iter()
            .any(|span| span.y == y && span.start <= x && x < span.end && span.area_id.is_none())
    };
    assert!(!at(&old_map, 1, -4));
    assert!(at(&grid, 1, -4));
    assert_eq!(decode(&old).draft(), old_map.draft());
}

#[test]
fn literal_starter_vectors_lock_metrics_bounds_and_real_openings() {
    use crate::office_map::{Axis, Edge, modules::*};
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/modules-v2-vectors.json"
    ))
    .unwrap();
    assert_eq!(
        corpus["metrics"],
        json!({"roomWidth": ROOM_WIDTH, "roomHeight": ROOM_HEIGHT,
        "passageWidth": PASSAGE_WIDTH, "columnStep": COLUMN_STEP, "rowStep": ROW_STEP,
        "lobbyWidth": LOBBY_WIDTH, "meetingX": MEETING_X})
    );
    let map = decode_map(&serde_json::to_vec(&corpus["starter"]).unwrap()).unwrap();
    assert_eq!(
        map.geometry().tile_count() as u64,
        corpus["expected"]["tiles"].as_u64().unwrap()
    );
    assert_eq!(
        map.draft().doors.len() as u64,
        corpus["expected"]["openings"].as_u64().unwrap()
    );
    let bounds: Vec<Value> = map
        .modules()
        .unwrap()
        .modules
        .iter()
        .map(|module| {
            let bounds = module.bounds(ModuleLayout::ShortLinks).unwrap();
            json!({"x": bounds.x, "y": bounds.y, "width": bounds.width, "height": bounds.height})
        })
        .collect();
    assert_eq!(json!(bounds), corpus["expected"]["bounds"]);
    for door in corpus["expected"]["doorSamples"].as_array().unwrap() {
        assert!(map.draft().doors.contains(&Edge {
            x: door["x"].as_i64().unwrap() as i32,
            y: door["y"].as_i64().unwrap() as i32,
            axis: if door["axis"] == "horizontal" {
                Axis::Horizontal
            } else {
                Axis::Vertical
            }
        }));
    }
    assert_eq!(map_value(&map), corpus["starter"]);
}

#[test]
fn module_round_trip_does_not_persist_derived_floor_or_door_arrays() {
    let source = fixture();
    let map = decode_map(&serde_json::to_vec(&source).unwrap()).unwrap();
    assert_eq!(map.draft().areas.len(), 2);
    assert!(!map.draft().floor.is_empty());
    assert!(!map.draft().doors.is_empty());
    let saved = map_value(&map);
    assert_eq!(saved, source);
    assert!(saved.get("floor").is_none());
    assert!(saved.get("doors").is_none());
    let again = decode_map(&serde_json::to_vec(&saved).unwrap()).unwrap();
    assert_eq!(again.draft(), map.draft());
    assert_eq!(again.modules(), map.modules());
}

#[test]
fn unified_area_vectors_match_browser_geometry_and_round_trip_without_derived_state() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../contracts/modules-unified-vectors.json"
    ))
    .unwrap();
    let map = decode_map(&serde_json::to_vec(&vectors["map"]).unwrap()).unwrap();
    assert_eq!(map_value(&map), vectors["map"]);
    assert_eq!(
        map.draft()
            .floor
            .iter()
            .map(|span| (span.end - span.start) as u64)
            .sum::<u64>(),
        vectors["floorTiles"].as_u64().unwrap()
    );
    assert_eq!(
        map.draft().doors.len() as u64,
        vectors["doorEdges"].as_u64().unwrap()
    );
    for pair in vectors["publicSamples"].as_array().unwrap() {
        assert_eq!(
            map.geometry().area_at(crate::office_map::Tile {
                x: pair[0].as_i64().unwrap() as i32,
                y: pair[1].as_i64().unwrap() as i32
            }),
            Some(None)
        );
    }
    for pair in vectors["emptySamples"].as_array().unwrap() {
        assert_eq!(
            map.geometry().area_at(crate::office_map::Tile {
                x: pair[0].as_i64().unwrap() as i32,
                y: pair[1].as_i64().unwrap() as i32
            }),
            None
        );
    }
}

#[test]
fn world_envelope_reuses_module_admission_and_keeps_the_source_after_round_trip() {
    let source = json!({"version": 1, "map": fixture(), "objects": []});
    let world = decode_world(&serde_json::to_vec(&source).unwrap()).unwrap();
    assert_eq!(world_value(&world), source);
}

#[test]
fn rejects_parallel_geometry_unknown_materials_and_incomplete_slots() {
    for (pointer, value) in [
        ("/floor", json!([])),
        ("/doors", json!([])),
        ("/modules/0/material", json!("execute-theme")),
        ("/modules/1/slot", json!({"type": "office", "column": 0})),
        ("/modules/1/slot/column", json!(1.5)),
        ("/modules/1/slot/row", json!(2147483648_i64)),
        ("/modules/1/area/binding", json!({"type": "personal"})),
    ] {
        let mut source = fixture();
        if pointer == "/floor" || pointer == "/doors" {
            source[pointer.trim_start_matches('/')] = value;
        } else {
            *source.pointer_mut(pointer).unwrap() = value;
        }
        assert!(
            decode_map(&serde_json::to_vec(&source).unwrap()).is_err(),
            "accepted {pointer}"
        );
    }
    let mut source = fixture();
    source["modules"][0]["slot"]["shell"] = json!("unexpected");
    assert_eq!(
        decode_map(&serde_json::to_vec(&source).unwrap()).unwrap_err(),
        MapCodecError::InvalidJson
    );
}

#[test]
fn no_version_guessing_and_no_implicit_conversion_of_legacy_maps() {
    let mut source = fixture();
    source["version"] = json!(1);
    assert_eq!(
        decode_map(&serde_json::to_vec(&source).unwrap()).unwrap_err(),
        MapCodecError::UnsupportedVersion
    );
    let module_map = decode_map(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
    let legacy = crate::office_map::OfficeMap::new(module_map.draft().clone()).unwrap();
    let saved = map_value(&legacy);
    assert_eq!(saved["version"], 1);
    assert!(saved.get("modules").is_none());
    assert!(
        decode_map(&serde_json::to_vec(&saved).unwrap())
            .unwrap()
            .modules()
            .is_none()
    );
}
