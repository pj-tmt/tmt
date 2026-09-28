use super::*;

const LOBBY: &str = "10000000-0000-4000-8000-000000000001";
const OFFICE: &str = "10000000-0000-4000-8000-000000000002";
const OTHER: &str = "10000000-0000-4000-8000-000000000003";
const ALICE: &str = "20000000-0000-4000-8000-000000000001";

fn span(y: i32, start: i32, end: i32, area: Option<&str>) -> FloorSpan {
    FloorSpan {
        y,
        start,
        end,
        area_id: area.map(str::to_owned),
    }
}

fn area(id: &str, kind: AreaKind) -> Area {
    Area {
        id: id.into(),
        name: "Area".into(),
        kind,
    }
}

fn lobby() -> MapDraft {
    MapDraft {
        primary_lobby_id: LOBBY.into(),
        areas: vec![area(LOBBY, AreaKind::Lobby)],
        floor: (0..3).map(|y| span(y, 0, 3, Some(LOBBY))).collect(),
        doors: vec![],
    }
}

fn office() -> MapDraft {
    let mut draft = lobby();
    draft.areas.push(area(
        OFFICE,
        AreaKind::Personal {
            identity_id: Some(ALICE.into()),
        },
    ));
    draft
        .floor
        .extend((0..3).map(|y| span(y, 3, 5, Some(OFFICE))));
    draft.doors.push(Edge {
        x: 3,
        y: 1,
        axis: Axis::Vertical,
    });
    draft
}

fn rejected(draft: MapDraft, error: MapError) {
    assert_eq!(OfficeMap::new(draft).unwrap_err(), error);
}

#[test]
fn walls_are_derived_once_and_door_is_a_partition_opening() {
    let map = OfficeMap::new(office()).unwrap();
    let geometry = map.geometry();
    assert_eq!(geometry.tile_count(), 15);
    assert_eq!(geometry.area_at(Tile { x: 1, y: 1 }), Some(Some(LOBBY)));
    assert_eq!(geometry.area_at(Tile { x: 3, y: 1 }), Some(Some(OFFICE)));
    assert_eq!(geometry.area_at(Tile { x: 99, y: 99 }), None);
    assert_eq!(geometry.boundaries().count(), 19); // 16 exterior + 3 partition edges.
    assert_eq!(geometry.boundaries().filter(|b| b.open).count(), 1);
    assert_eq!(
        geometry
            .boundaries()
            .filter(|b| b.kind == BoundaryKind::Exterior)
            .count(),
        16
    );
    let edge = Edge {
        x: 3,
        y: 1,
        axis: Axis::Vertical,
    };
    assert_eq!(
        geometry.boundary(edge),
        Some(&Boundary {
            edge,
            kind: BoundaryKind::Partition,
            open: true
        })
    );
    assert!(
        geometry
            .boundary(Edge {
                x: 1,
                y: 1,
                axis: Axis::Vertical
            })
            .is_none()
    );
}

#[test]
fn occupied_tile_budget_admits_full_capacity_and_rejects_one_more() {
    let mut draft = lobby();
    draft.floor = (0..512).map(|y| span(y, 0, 512, Some(LOBBY))).collect();
    let map = OfficeMap::new(draft.clone()).unwrap();
    assert_eq!(map.geometry().tile_count(), MAX_TILES);
    assert_eq!(map.geometry().boundaries().count(), 2048);
    draft.floor.push(span(512, 0, 1, Some(LOBBY)));
    rejected(draft, MapError::LimitExceeded);
}

#[test]
fn removing_an_area_keeps_floor_but_not_its_partition_or_occupancy() {
    let mut draft = office();
    let original = OfficeMap::new(draft.clone()).unwrap();
    draft.areas.retain(|a| a.id != OFFICE);
    for run in &mut draft.floor {
        if run.area_id.as_deref() == Some(OFFICE) {
            // The region becomes the existing Lobby in this particular edit.
            run.area_id = Some(LOBBY.into());
        }
    }
    draft.doors.clear(); // The old partition no longer exists.
    let edited = OfficeMap::new(draft).unwrap();
    assert_eq!(
        edited.geometry().tile_count(),
        original.geometry().tile_count()
    );
    assert_eq!(edited.geometry().boundaries().count(), 16);
    assert_eq!(edited.draft().areas.len(), 1);
    assert_eq!(original.draft().areas.len(), 2); // Validation cannot mutate the original.
}

#[test]
fn common_floor_is_distinct_from_outside_and_needs_an_opening() {
    let mut draft = office();
    draft.areas.retain(|a| a.id != OFFICE);
    for run in &mut draft.floor {
        if run.area_id.as_deref() == Some(OFFICE) {
            run.area_id = None;
        }
    }
    let map = OfficeMap::new(draft.clone()).unwrap();
    assert_eq!(map.geometry().area_at(Tile { x: 4, y: 1 }), Some(None));
    draft.doors.clear();
    rejected(draft, MapError::InaccessibleFloor);
}

#[test]
fn canonicalization_preserves_cells_but_merges_brush_subdivision() {
    let canonical = OfficeMap::new(lobby()).unwrap();
    let mut draft = lobby();
    draft.floor = (0..3)
        .rev()
        .flat_map(|y| [span(y, 1, 3, Some(LOBBY)), span(y, 0, 1, Some(LOBBY))])
        .collect();
    assert_eq!(OfficeMap::new(draft).unwrap().draft(), canonical.draft());
}

#[test]
fn cardinal_shapes_accept_a_bend_but_not_diagonal_contact() {
    let mut draft = lobby();
    draft.floor = vec![span(-2, -2, 0, Some(LOBBY)), span(-1, -1, 0, Some(LOBBY))];
    assert_eq!(
        OfficeMap::new(draft.clone())
            .unwrap()
            .geometry()
            .tile_count(),
        3
    );
    draft.floor[1] = span(-1, 0, 1, Some(LOBBY));
    rejected(draft, MapError::DisconnectedArea);
}

#[test]
fn floor_may_not_form_a_separate_island_even_if_each_area_is_connected() {
    let mut draft = lobby();
    draft.floor.push(span(3, 0, 1, None));
    draft.doors.push(Edge {
        x: 0,
        y: 3,
        axis: Axis::Horizontal,
    });
    OfficeMap::new(draft.clone()).unwrap();
    draft.floor.last_mut().unwrap().y = 4;
    draft.doors.clear();
    rejected(draft, MapError::DisconnectedFloor);
}

#[test]
fn area_overlap_unknown_references_and_empty_areas_fail_explicitly() {
    let base = office();
    OfficeMap::new(base.clone()).unwrap();
    let mut draft = base.clone();
    draft.floor.push(span(0, 0, 1, Some(OFFICE)));
    rejected(draft, MapError::OverlappingFloor);
    let mut draft = base.clone();
    draft.floor[0].area_id = Some(OTHER.into());
    rejected(draft, MapError::UnknownArea);
    let mut draft = base;
    draft.areas.push(area(OTHER, AreaKind::Lobby));
    rejected(draft, MapError::EmptyArea);
}

#[test]
fn removing_last_lobby_fails_but_explicit_replacement_is_valid() {
    let mut draft = office();
    draft.areas[0].kind = AreaKind::Personal { identity_id: None };
    rejected(draft.clone(), MapError::PrimaryLobbyRequired);
    draft.areas[1].kind = AreaKind::Lobby;
    draft.primary_lobby_id = OFFICE.into();
    OfficeMap::new(draft).unwrap();
}

#[test]
fn missing_duplicate_exterior_and_nonboundary_doors_are_rejected() {
    OfficeMap::new(office()).unwrap();
    let mut draft = office();
    draft.doors.clear();
    rejected(draft, MapError::InaccessibleFloor);
    let mut draft = office();
    draft.doors.push(draft.doors[0]);
    rejected(draft, MapError::DuplicateDoor);
    for edge in [
        Edge {
            x: 0,
            y: 0,
            axis: Axis::Horizontal,
        },
        Edge {
            x: 1,
            y: 1,
            axis: Axis::Vertical,
        },
    ] {
        let mut draft = office();
        draft.doors.push(edge);
        rejected(draft, MapError::InvalidDoor);
    }
}

#[test]
fn duplicate_occupancy_and_repeated_meeting_projection_are_not_membership() {
    let mut draft = office();
    draft.areas.push(area(
        OTHER,
        AreaKind::Personal {
            identity_id: Some(ALICE.into()),
        },
    ));
    rejected(draft, MapError::DuplicateOccupant);
    let mut draft = office();
    draft.areas[1].kind = AreaKind::Meeting {
        room_id: ALICE.into(),
    };
    OfficeMap::new(draft.clone()).unwrap();
    draft.areas.push(area(
        OTHER,
        AreaKind::Meeting {
            room_id: ALICE.into(),
        },
    ));
    rejected(draft, MapError::DuplicateMeetingArea);
}

#[test]
fn malformed_extreme_coordinates_and_labels_fail_without_overflow() {
    for (start, end, y) in [
        (i32::MIN, i32::MAX, 0),
        (1, 1, 0),
        (2, 1, 0),
        (0, 1, i32::MAX),
    ] {
        let mut draft = lobby();
        draft.floor = vec![span(y, start, end, Some(LOBBY))];
        rejected(draft, MapError::InvalidSpan);
    }
    let mut draft = lobby();
    draft.floor = vec![span(
        -COORDINATE_LIMIT,
        -COORDINATE_LIMIT,
        -COORDINATE_LIMIT + 1,
        Some(LOBBY),
    )];
    OfficeMap::new(draft).unwrap();
    for name in [
        " ".into(),
        "x".repeat(81),
        "unsafe\nlabel".into(),
        "fake\u{202e}label".into(),
    ] {
        let mut draft = lobby();
        draft.areas[0].name = name;
        rejected(draft, MapError::InvalidArea);
    }
}

#[test]
fn budgets_reject_oversized_input_not_a_large_empty_bounding_rectangle() {
    let mut draft = lobby();
    draft.floor = vec![span(0, 0, 1, Some(LOBBY)); MAX_SPANS + 1];
    rejected(draft, MapError::LimitExceeded);
    let mut draft = lobby();
    draft.floor = vec![span(0, -COORDINATE_LIMIT, COORDINATE_LIMIT, Some(LOBBY))];
    assert_eq!(OfficeMap::new(draft).unwrap().geometry().tile_count(), 8192);
}
