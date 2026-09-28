use super::{ObjectKind, PlacementError, Surface, WALL_HEIGHT, WallFace, WorldObject};
use crate::office_map::{Axis, BoundaryKind, COORDINATE_LIMIT, Edge, OfficeMap, Tile};
use std::collections::{BTreeSet, HashMap};

pub(super) struct WallMount {
    edges: Vec<Edge>,
    face: WallFace,
    bottom: u8,
    top: u8,
    window: bool,
}

pub(super) fn validate(
    map: &OfficeMap,
    object: &WorldObject,
) -> Result<Option<WallMount>, PlacementError> {
    let prop = &object.placement;
    prop.validate_appearance()
        .map_err(|_| PlacementError::InvalidAppearance)?;
    if !(-COORDINATE_LIMIT..=COORDINATE_LIMIT).contains(&prop.x)
        || !(-COORDINATE_LIMIT..=COORDINATE_LIMIT).contains(&prop.y)
    {
        return Err(PlacementError::InvalidSurface);
    }
    let (width, height) = prop.dimensions();
    let geometry = map.geometry();
    match object.surface {
        Surface::Floor { base } => {
            if object.kind != ObjectKind::Decoration {
                return Err(PlacementError::InvalidSurface);
            }
            let (left, top, width, height) = if let Some(base) = base {
                if !base.fits(prop) {
                    return Err(PlacementError::InvalidSurface);
                }
                let base = base.rotated(prop);
                (
                    prop.x + i32::from(base.x),
                    prop.y + i32::from(base.y),
                    base.width,
                    base.height,
                )
            } else {
                (prop.x, prop.y, width, height)
            };
            for y in top..top + i32::from(height) {
                for x in left..left + i32::from(width) {
                    if !geometry.contains(Tile { x, y }) {
                        return Err(PlacementError::OutsideFloor);
                    }
                    if geometry
                        .adjacent_boundaries(Tile { x, y })
                        .any(|boundary| boundary.open)
                    {
                        return Err(PlacementError::BlocksDoor);
                    }
                    if (x > left
                        && geometry
                            .boundary(Edge {
                                x,
                                y,
                                axis: Axis::Vertical,
                            })
                            .is_some())
                        || (y > top
                            && geometry
                                .boundary(Edge {
                                    x,
                                    y,
                                    axis: Axis::Horizontal,
                                })
                                .is_some())
                    {
                        return Err(PlacementError::CrossesPartition);
                    }
                }
            }
            Ok(None)
        }
        Surface::Wall {
            axis,
            face,
            elevation,
        } => {
            let top = elevation
                .checked_add(height)
                .filter(|top| *top <= WALL_HEIGHT)
                .ok_or(PlacementError::InvalidSurface)?;
            let mut edges = Vec::with_capacity(usize::from(width));
            for offset in 0..i32::from(width) {
                let edge = Edge {
                    x: prop.x + if axis == Axis::Horizontal { offset } else { 0 },
                    y: prop.y + if axis == Axis::Vertical { offset } else { 0 },
                    axis,
                };
                let boundary = geometry.boundary(edge).ok_or(PlacementError::MissingWall)?;
                if boundary.open {
                    return Err(PlacementError::BlocksDoor);
                }
                let interior = Tile {
                    x: edge.x - i32::from(axis == Axis::Vertical && face == WallFace::Negative),
                    y: edge.y - i32::from(axis == Axis::Horizontal && face == WallFace::Negative),
                };
                if !geometry.contains(interior) {
                    return Err(PlacementError::MissingWall);
                }
                if object.kind == ObjectKind::Window && boundary.kind != BoundaryKind::Exterior {
                    return Err(PlacementError::WindowRequiresExterior);
                }
                edges.push(edge);
            }
            Ok(Some(WallMount {
                edges,
                face,
                bottom: elevation,
                top,
                window: object.kind == ObjectKind::Window,
            }))
        }
    }
}

/// Bounded wall cells avoid pairwise object comparisons for crowded walls.
pub(super) fn window_conflicts(mounts: &[(usize, WallMount)]) -> BTreeSet<usize> {
    let mut cells_by_edge: HashMap<(Edge, WallFace), [(u16, u16); WALL_HEIGHT as usize]> =
        HashMap::new();
    for (_, mount) in mounts {
        for edge in &mount.edges {
            let cells = cells_by_edge.entry((*edge, mount.face)).or_default();
            for (total, windows) in &mut cells[usize::from(mount.bottom)..usize::from(mount.top)] {
                *total += 1;
                *windows += u16::from(mount.window);
            }
        }
    }
    mounts
        .iter()
        .filter_map(|(index, mount)| {
            mount
                .edges
                .iter()
                .any(|edge| {
                    cells_by_edge
                        .get(&(*edge, mount.face))
                        .is_some_and(|cells| {
                            cells[usize::from(mount.bottom)..usize::from(mount.top)]
                                .iter()
                                .any(|(total, windows)| {
                                    if mount.window {
                                        *total > 1
                                    } else {
                                        *windows > 0
                                    }
                                })
                        })
                })
                .then_some(*index)
        })
        .collect()
}
