use super::*;
use crate::office_block::{BUILTIN_PROP_PACK_DIGEST, LocalBlockLayout, PropCustomization};
use crate::office_map::{Area, AreaKind, Edge, FloorSpan, MapDraft, Tile};

const LOBBY: &str = "10000000-0000-4000-8000-000000000001";
const OFFICE: &str = "10000000-0000-4000-8000-000000000002";

fn map() -> OfficeMap {
    OfficeMap::new(MapDraft {
        primary_lobby_id: LOBBY.into(),
        areas: vec![Area {
            id: LOBBY.into(),
            name: "Lobby".into(),
            kind: AreaKind::Lobby,
        }],
        floor: (0..6)
            .map(|y| FloorSpan {
                y,
                start: 0,
                end: 6,
                area_id: Some(LOBBY.into()),
            })
            .collect(),
        doors: vec![],
    })
    .unwrap()
}

fn with_office() -> OfficeMap {
    let mut draft = map().draft().clone();
    draft.areas.push(Area {
        id: OFFICE.into(),
        name: "Studio".into(),
        kind: AreaKind::Personal { identity_id: None },
    });
    draft.floor.extend((0..6).map(|y| FloorSpan {
        y,
        start: 6,
        end: 10,
        area_id: Some(OFFICE.into()),
    }));
    draft.doors.push(Edge {
        x: 6,
        y: 3,
        axis: Axis::Vertical,
    });
    OfficeMap::new(draft).unwrap()
}

fn object(index: u32) -> WorldObject {
    WorldObject {
        id: format!("30000000-0000-4000-8000-{index:012}"),
        placement: PropPlacement {
            prop: format!("{BUILTIN_PROP_PACK_DIGEST}/desk"),
            footprint_width: 2,
            footprint_height: 1,
            x: 1,
            y: 1,
            rotation: 0,
            customization: Some(PropCustomization {
                tint: Some("#bb7755".into()),
                text: Some("Studio".into()),
            }),
        },
        surface: Surface::Floor { base: None },
        kind: ObjectKind::Decoration,
        extension: None,
    }
}

fn wall(index: u32) -> WorldObject {
    let mut object = object(index);
    object.placement.y = 0;
    object.placement.footprint_height = 3;
    object.surface = Surface::Wall {
        axis: Axis::Horizontal,
        face: WallFace::Positive,
        elevation: 2,
    };
    object
}

fn issues(map: OfficeMap, objects: Vec<WorldObject>) -> Vec<PlacementIssue> {
    match WorldLayout::new(map, objects).unwrap_err() {
        WorldError::Placements(issues) => issues,
        other => panic!("expected affected objects, got {other:?}"),
    }
}

#[test]
fn shallow_floor_base_allows_art_overhang_but_rejects_unsupported_or_invalid_bases() {
    let mut item = object(1);
    item.placement.footprint_width = 8;
    item.placement.footprint_height = 8;
    item.placement.x = -2;
    item.placement.y = -2;
    let base = FloorBase {
        x: 2,
        y: 2,
        width: 4,
        height: 2,
    };
    assert_eq!(
        issues(map(), vec![item.clone()])[0].reason,
        PlacementError::OutsideFloor
    );
    item.surface = Surface::Floor { base: Some(base) };
    let expected = [(2, 2, 4, 2), (4, 2, 2, 4), (2, 4, 4, 2), (2, 2, 2, 4)];
    for (rotation, (x, y, width, height)) in expected.into_iter().enumerate() {
        item.placement.rotation = rotation as u8;
        assert_eq!(
            base.rotated(&item.placement),
            FloorBase {
                x,
                y,
                width,
                height
            }
        );
        assert_eq!(
            WorldLayout::new(map(), vec![item.clone()])
                .unwrap()
                .objects(),
            &[item.clone()]
        );
    }
    item.placement.x = -3;
    assert_eq!(
        issues(map(), vec![item.clone()])[0].reason,
        PlacementError::OutsideFloor
    );
    item.surface = Surface::Floor {
        base: Some(FloorBase { width: 9, ..base }),
    };
    assert_eq!(
        issues(map(), vec![item.clone()])[0].reason,
        PlacementError::InvalidSurface
    );
    item.surface = Surface::Floor {
        base: Some(FloorBase { height: 0, ..base }),
    };
    assert_eq!(
        issues(map(), vec![item])[0].reason,
        PlacementError::InvalidSurface
    );
}

#[test]
fn shared_art_rules_preserve_customization_overlap_and_paint_order() {
    let objects = vec![object(2), object(1)];
    let world = WorldLayout::new(map(), objects.clone()).unwrap();
    assert_eq!(world.objects(), objects);
    assert_eq!(world.map().geometry().tile_count(), 36);
    let mut invalid = object(3);
    invalid.placement.customization.as_mut().unwrap().tint = Some("red".into());
    assert!(LocalBlockLayout::new(vec![invalid.placement.clone()]).is_err());
    assert_eq!(
        issues(map(), vec![invalid])[0].reason,
        PlacementError::InvalidAppearance
    );
}

#[test]
fn extension_references_are_data_and_invalid_resource_ids_report_the_placement() {
    use crate::office_extension::{ExtensionAttachment, ResourceBinding};
    let mut attached = object(1);
    attached.extension = Some(ExtensionAttachment {
        definition: "tmt-whiteboard".into(),
        binding: ResourceBinding::Whiteboard {
            document_id: "lobby".into(),
        },
    });
    WorldLayout::new(map(), vec![attached.clone()]).unwrap();
    attached.extension.as_mut().unwrap().binding = ResourceBinding::Whiteboard {
        document_id: "file:///private/data".into(),
    };
    assert_eq!(
        issues(map(), vec![attached])[0],
        PlacementIssue {
            object_id: object(1).id,
            reason: PlacementError::InvalidExtension,
        }
    );
}

#[test]
fn signed_world_positions_do_not_relax_legacy_block_bounds() {
    let mut draft = map().draft().clone();
    for span in &mut draft.floor {
        span.start -= 6;
        span.end -= 6;
        span.y -= 6;
    }
    let mut prop = object(1);
    prop.placement.x = -5;
    prop.placement.y = -5;
    assert!(LocalBlockLayout::new(vec![prop.placement.clone()]).is_err());
    WorldLayout::new(OfficeMap::new(draft).unwrap(), vec![prop]).unwrap();
}

#[test]
fn erase_reports_every_affected_object_without_rewriting_input() {
    let mut objects = vec![object(1), object(2)];
    objects[0].placement.x = 4;
    objects[1].placement.y = 5;
    let original = WorldLayout::new(map(), objects.clone()).unwrap();
    let mut draft = map().draft().clone();
    draft.floor.retain(|span| span.y < 5);
    for span in &mut draft.floor {
        span.end = 5;
    }
    let errors = issues(OfficeMap::new(draft).unwrap(), objects.clone());
    assert_eq!(
        errors
            .iter()
            .map(|issue| issue.object_id.clone())
            .collect::<Vec<_>>(),
        objects.iter().map(|o| o.id.clone()).collect::<Vec<_>>()
    );
    assert!(
        errors
            .iter()
            .all(|issue| issue.reason == PlacementError::OutsideFloor)
    );
    assert_eq!(original.objects(), objects);
    assert!(original.map().geometry().contains(Tile { x: 5, y: 5 }));
}

#[test]
fn furniture_cannot_straddle_partitions_or_cover_either_side_of_a_door() {
    let mut prop = object(1);
    prop.placement.x = 5;
    assert_eq!(
        issues(with_office(), vec![prop.clone()])[0].reason,
        PlacementError::CrossesPartition
    );
    prop.placement.y = 3;
    prop.placement.footprint_width = 1;
    assert_eq!(
        issues(with_office(), vec![prop.clone()])[0].reason,
        PlacementError::BlocksDoor
    );
    prop.placement.x = 6;
    assert_eq!(
        issues(with_office(), vec![prop])[0].reason,
        PlacementError::BlocksDoor
    );
}

#[test]
fn rotation_changes_the_occupied_footprint_and_checks_every_tile_not_just_bounds() {
    let mut prop = object(1);
    prop.placement.x = 5;
    assert_eq!(
        issues(map(), vec![prop.clone()])[0].reason,
        PlacementError::OutsideFloor
    );
    prop.placement.rotation = 1;
    WorldLayout::new(map(), vec![prop]).unwrap();
    let mut draft = map().draft().clone();
    draft.floor[1].end = 1;
    draft.floor.push(FloorSpan {
        y: 1,
        start: 2,
        end: 6,
        area_id: Some(LOBBY.into()),
    });
    let holed = OfficeMap::new(draft).unwrap();
    assert_eq!(
        issues(holed, vec![object(1)])[0].reason,
        PlacementError::OutsideFloor
    );
}

#[test]
fn wall_mounts_require_a_real_indoor_face_and_fit_the_full_elevation() {
    WorldLayout::new(map(), vec![wall(1)]).unwrap();
    let mut prop = wall(1);
    prop.surface = Surface::Wall {
        axis: Axis::Horizontal,
        face: WallFace::Negative,
        elevation: 2,
    };
    assert_eq!(
        issues(map(), vec![prop.clone()])[0].reason,
        PlacementError::MissingWall
    );
    prop.surface = Surface::Wall {
        axis: Axis::Horizontal,
        face: WallFace::Positive,
        elevation: 14,
    };
    assert_eq!(
        issues(map(), vec![prop.clone()])[0].reason,
        PlacementError::InvalidSurface
    );
    prop.placement.x = i32::MAX;
    assert_eq!(
        issues(map(), vec![prop])[0].reason,
        PlacementError::InvalidSurface
    );
}

#[test]
fn windows_require_exterior_walls_and_lamps_require_wall_mounts() {
    let mut window = wall(1);
    window.kind = ObjectKind::Window;
    WorldLayout::new(map(), vec![window.clone()]).unwrap();
    window.placement.x = 6;
    window.placement.y = 0;
    window.surface = Surface::Wall {
        axis: Axis::Vertical,
        face: WallFace::Negative,
        elevation: 2,
    };
    assert_eq!(
        issues(with_office(), vec![window])[0].reason,
        PlacementError::WindowRequiresExterior
    );
    let mut lamp = object(2);
    lamp.kind = ObjectKind::WallLight;
    assert_eq!(
        issues(map(), vec![lamp])[0].reason,
        PlacementError::InvalidSurface
    );
}

#[test]
fn wall_objects_cannot_cover_doors_or_survive_a_removed_partition_without_resolution() {
    let mut prop = wall(1);
    prop.placement.x = 6;
    prop.placement.y = 2;
    prop.surface = Surface::Wall {
        axis: Axis::Vertical,
        face: WallFace::Negative,
        elevation: 2,
    };
    assert_eq!(
        issues(with_office(), vec![prop.clone()])[0].reason,
        PlacementError::BlocksDoor
    );
    prop.placement.y = 0;
    WorldLayout::new(with_office(), vec![prop.clone()]).unwrap();
    let mut draft = with_office().draft().clone();
    draft.areas.retain(|area| area.id != OFFICE);
    for span in &mut draft.floor {
        span.area_id = Some(LOBBY.into());
    }
    draft.doors.clear();
    assert_eq!(
        issues(OfficeMap::new(draft).unwrap(), vec![prop])[0].reason,
        PlacementError::MissingWall
    );
}

#[test]
fn window_exclusions_report_both_objects_but_allow_separate_elevations_and_plain_art_layers() {
    let mut window = wall(1);
    window.kind = ObjectKind::Window;
    let art = wall(2);
    let errors = issues(map(), vec![window.clone(), art.clone()]);
    assert_eq!(errors.len(), 2);
    assert!(
        errors
            .iter()
            .all(|issue| issue.reason == PlacementError::OverlapsWindow)
    );
    let mut higher = art;
    higher.surface = Surface::Wall {
        axis: Axis::Horizontal,
        face: WallFace::Positive,
        elevation: 5,
    };
    WorldLayout::new(map(), vec![window, higher]).unwrap();
    WorldLayout::new(map(), vec![wall(1), wall(2)]).unwrap();
}

#[test]
fn budgets_and_placement_identity_fail_before_ambiguous_commits() {
    assert_eq!(
        WorldLayout::new(map(), vec![object(1); MAX_OBJECTS + 1]).unwrap_err(),
        WorldError::TooManyObjects
    );
    assert_eq!(
        issues(map(), vec![object(1), object(1)])[0].reason,
        PlacementError::DuplicateId
    );
    let mut prop = object(1);
    prop.id = "display-name-is-not-an-instance".into();
    assert_eq!(
        issues(map(), vec![prop])[0].reason,
        PlacementError::InvalidId
    );
}

#[test]
fn lattice_boundary_mounts_support_both_coordinate_extremes_without_overflow() {
    for coordinate in [-4096, 4095] {
        let mut draft = map().draft().clone();
        draft.floor = vec![FloorSpan {
            y: coordinate,
            start: coordinate,
            end: coordinate + 1,
            area_id: Some(LOBBY.into()),
        }];
        let map = OfficeMap::new(draft).unwrap();
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let mut prop = wall(1);
            prop.placement.footprint_width = 1;
            prop.placement.footprint_height = 1;
            prop.placement.x = coordinate + i32::from(coordinate > 0 && axis == Axis::Vertical);
            prop.placement.y = coordinate + i32::from(coordinate > 0 && axis == Axis::Horizontal);
            prop.surface = Surface::Wall {
                axis,
                face: if coordinate < 0 {
                    WallFace::Positive
                } else {
                    WallFace::Negative
                },
                elevation: 0,
            };
            WorldLayout::new(map.clone(), vec![prop.clone()]).unwrap();
            prop.placement.x = if coordinate < 0 { -4097 } else { 4097 };
            assert_eq!(
                issues(map.clone(), vec![prop])[0].reason,
                PlacementError::InvalidSurface
            );
        }
    }
}

#[test]
fn rotated_window_partial_overlap_reports_both_ids_and_touching_edges_are_valid() {
    let mut window = wall(1);
    window.kind = ObjectKind::Window;
    window.placement.rotation = 1; // Width 3, height 2 after rotation.
    let mut art = wall(2);
    art.placement.x = 3;
    art.surface = Surface::Wall {
        axis: Axis::Horizontal,
        face: WallFace::Positive,
        elevation: 3,
    };
    let errors = issues(map(), vec![window.clone(), art.clone()]);
    assert_eq!(
        errors,
        vec![
            PlacementIssue {
                object_id: window.id.clone(),
                reason: PlacementError::OverlapsWindow
            },
            PlacementIssue {
                object_id: art.id.clone(),
                reason: PlacementError::OverlapsWindow
            },
        ]
    );
    art.placement.x = 4;
    WorldLayout::new(map(), vec![window, art]).unwrap();
}
