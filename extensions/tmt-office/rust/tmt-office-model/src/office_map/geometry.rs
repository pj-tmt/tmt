//! Derived edges and structural reachability; never a second persisted wall model.

use super::{AreaKind, Axis, COORDINATE_LIMIT, Edge, MAX_TILES, MapDraft, MapError, Tile};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    Exterior,
    Partition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boundary {
    pub edge: Edge,
    pub kind: BoundaryKind,
    pub open: bool,
}

/// Owned by the admitted map; renderers and mount validation read the same index.
#[derive(Debug, Clone)]
pub struct Geometry {
    tiles: BTreeMap<Tile, Option<String>>,
    boundaries: BTreeMap<Edge, Boundary>,
}

pub(super) enum Connectivity {
    Lobby,
    Meetings,
    Areas,
}

impl Geometry {
    pub(super) fn build(draft: &MapDraft, connectivity: Connectivity) -> Result<Self, MapError> {
        let areas: BTreeSet<&str> = draft.areas.iter().map(|area| area.id.as_str()).collect();
        let mut tiles = BTreeMap::new();
        let mut area_tiles: BTreeMap<&str, Vec<Tile>> = BTreeMap::new();
        for span in &draft.floor {
            if span.start < -COORDINATE_LIMIT
                || span.end > COORDINATE_LIMIT
                || span.start >= span.end
                || !(-COORDINATE_LIMIT..COORDINATE_LIMIT).contains(&span.y)
            {
                return Err(MapError::InvalidSpan);
            }
            if span
                .area_id
                .as_deref()
                .is_some_and(|id| !areas.contains(id))
            {
                return Err(MapError::UnknownArea);
            }
            if tiles.len() + (span.end - span.start) as usize > MAX_TILES {
                return Err(MapError::LimitExceeded);
            }
            for x in span.start..span.end {
                let tile = Tile { x, y: span.y };
                if tiles.insert(tile, span.area_id.clone()).is_some() {
                    return Err(MapError::OverlappingFloor);
                }
                if let Some(id) = span.area_id.as_deref() {
                    area_tiles.entry(id).or_default().push(tile);
                }
            }
        }
        for area in &draft.areas {
            let points = area_tiles
                .get(area.id.as_str())
                .ok_or(MapError::EmptyArea)?;
            let reached = flood(points[0], |_, next, _| {
                tiles
                    .get(&next)
                    .is_some_and(|id| id.as_deref() == Some(&area.id))
            });
            if reached != points.len() {
                return Err(MapError::DisconnectedArea);
            }
        }
        let origin = area_tiles[draft.primary_lobby_id.as_str()][0];
        // Only versioned module projectors may supply separate platforms. Every
        // common tile must still be reachable from a real, connected area floor.
        let mut origins = vec![origin];
        if !matches!(connectivity, Connectivity::Lobby) {
            origins.extend(
                draft
                    .areas
                    .iter()
                    .filter(|area| {
                        matches!(connectivity, Connectivity::Areas)
                            || matches!(area.kind, AreaKind::Meeting { .. })
                    })
                    .map(|area| area_tiles[area.id.as_str()][0]),
            );
        }
        if flood_from(&origins, |_, next, _| tiles.contains_key(&next)) != tiles.len() {
            return Err(MapError::DisconnectedFloor);
        }
        let mut boundaries = BTreeMap::new();
        for (tile, area) in &tiles {
            for (neighbor, edge) in neighbors(*tile) {
                let kind = match tiles.get(&neighbor) {
                    None => BoundaryKind::Exterior,
                    Some(other) if other != area => BoundaryKind::Partition,
                    Some(_) => continue,
                };
                boundaries.insert(
                    edge,
                    Boundary {
                        edge,
                        kind,
                        open: false,
                    },
                );
            }
        }
        for edge in &draft.doors {
            let boundary = boundaries.get_mut(edge).ok_or(MapError::InvalidDoor)?;
            if boundary.kind != BoundaryKind::Partition {
                return Err(MapError::InvalidDoor);
            }
            if boundary.open {
                return Err(MapError::DuplicateDoor);
            }
            boundary.open = true;
        }
        if flood_from(&origins, |_, next, edge| {
            tiles.contains_key(&next) && boundaries.get(&edge).is_none_or(|b| b.open)
        }) != tiles.len()
        {
            return Err(MapError::InaccessibleFloor);
        }
        Ok(Self { tiles, boundaries })
    }

    pub fn contains(&self, tile: Tile) -> bool {
        self.tiles.contains_key(&tile)
    }

    /// None is outside the floor; Some(None) is common floor, not a missing area.
    pub fn area_at(&self, tile: Tile) -> Option<Option<&str>> {
        self.tiles.get(&tile).map(|area| area.as_deref())
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn boundary(&self, edge: Edge) -> Option<&Boundary> {
        self.boundaries.get(&edge)
    }

    pub fn adjacent_boundaries(&self, tile: Tile) -> impl Iterator<Item = &Boundary> {
        self.contains(tile)
            .then(|| neighbors(tile))
            .into_iter()
            .flatten()
            .filter_map(|(_, edge)| self.boundary(edge))
    }

    pub fn boundaries(&self) -> impl Iterator<Item = &Boundary> {
        self.boundaries.values()
    }
}

/// Edge spelling is independent of which adjacent tile asks for it.
fn neighbors(tile: Tile) -> [(Tile, Edge); 4] {
    let Tile { x, y } = tile;
    [
        (
            Tile { x, y: y - 1 },
            Edge {
                x,
                y,
                axis: Axis::Horizontal,
            },
        ),
        (
            Tile { x, y: y + 1 },
            Edge {
                x,
                y: y + 1,
                axis: Axis::Horizontal,
            },
        ),
        (
            Tile { x: x - 1, y },
            Edge {
                x,
                y,
                axis: Axis::Vertical,
            },
        ),
        (
            Tile { x: x + 1, y },
            Edge {
                x: x + 1,
                y,
                axis: Axis::Vertical,
            },
        ),
    ]
}

fn flood(origin: Tile, can_enter: impl FnMut(Tile, Tile, Edge) -> bool) -> usize {
    flood_from(&[origin], can_enter)
}

fn flood_from(origins: &[Tile], mut can_enter: impl FnMut(Tile, Tile, Edge) -> bool) -> usize {
    let mut seen: HashSet<_> = origins.iter().copied().collect();
    let mut pending: VecDeque<_> = seen.iter().copied().collect();
    while let Some(tile) = pending.pop_front() {
        for (next, edge) in neighbors(tile) {
            if !seen.contains(&next) && can_enter(tile, next, edge) {
                seen.insert(next);
                pending.push_back(next);
            }
        }
    }
    seen.len()
}
