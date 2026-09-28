//! User-built floor topology. Resource contents, presence and room rosters stay separate.

mod geometry;
pub mod modules;
#[cfg(test)]
mod tests;

pub use geometry::{Boundary, BoundaryKind, Geometry};
use std::collections::BTreeSet;
use tmt_core::dispatch::canonical_id;

/// Admission budgets apply to occupied tiles, never to the bounding rectangle.
pub const MAX_TILES: usize = 262_144;
pub const MAX_SPANS: usize = 16_384;
pub const MAX_AREAS: usize = 256;
pub const MAX_DOORS: usize = 4096;
pub const COORDINATE_LIMIT: i32 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
}

/// Canonical lattice edges: horizontal extends east; vertical extends south.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Edge {
    pub x: i32,
    pub y: i32,
    pub axis: Axis,
}

/// One half-open horizontal run. None means common circulation floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorSpan {
    pub y: i32,
    pub start: i32,
    pub end: i32,
    pub area_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaKind {
    Lobby,
    Personal { identity_id: Option<String> },
    Meeting { room_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Area {
    pub id: String,
    pub name: String,
    pub kind: AreaKind,
}

/// Untrusted topology input. Membership, decoration and wall material are not duplicated here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapDraft {
    pub primary_lobby_id: String,
    pub areas: Vec<Area>,
    pub floor: Vec<FloorSpan>,
    pub doors: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    LimitExceeded,
    InvalidArea,
    DuplicateArea,
    PrimaryLobbyRequired,
    DuplicateOccupant,
    DuplicateMeetingArea,
    InvalidSpan,
    UnknownArea,
    OverlappingFloor,
    EmptyArea,
    DisconnectedArea,
    DisconnectedFloor,
    InvalidDoor,
    DuplicateDoor,
    InaccessibleFloor,
    InvalidModule,
    ModuleCollision,
}

impl std::fmt::Display for MapError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::LimitExceeded => "Office map exceeds its bounded geometry budget.",
            Self::InvalidArea => "Office area requires a canonical UUID, label and valid binding.",
            Self::DuplicateArea => "Office area UUIDs must be unique.",
            Self::PrimaryLobbyRequired => "Office requires a primary Lobby with floor.",
            Self::DuplicateOccupant => "An identity may occupy only one personal office.",
            Self::DuplicateMeetingArea => "A meeting room may bind only one spatial area.",
            Self::InvalidSpan => "Office floor runs must be nonempty and within coordinate bounds.",
            Self::UnknownArea => "Office floor references an unknown area.",
            Self::OverlappingFloor => "Office floor runs must not overlap.",
            Self::EmptyArea => "Office areas must contain floor tiles.",
            Self::DisconnectedArea => "Each Office area must be cardinally connected.",
            Self::DisconnectedFloor => "Office floor must be cardinally connected.",
            Self::InvalidDoor => "Doors must open a partition between adjacent indoor tiles.",
            Self::DuplicateDoor => "Office door edges must be unique.",
            Self::InaccessibleFloor => {
                "Every Office tile must be reachable from the primary Lobby through openings."
            }
            Self::InvalidModule => {
                "Office module slots must match their area purpose and stay within bounds."
            }
            Self::ModuleCollision => {
                "Office modules must not overlap another module or the reserved meeting wing."
            }
        })
    }
}

impl std::error::Error for MapError {}

/// Validated immutable topology and its one derived geometry index.
#[derive(Debug, Clone)]
pub struct OfficeMap {
    draft: MapDraft,
    geometry: Geometry,
    modules: Option<modules::ModuleDraft>,
}

impl OfficeMap {
    pub fn new(draft: MapDraft) -> Result<Self, MapError> {
        Self::admit(draft, geometry::Connectivity::Lobby)
    }

    fn admit(mut draft: MapDraft, connectivity: geometry::Connectivity) -> Result<Self, MapError> {
        validate_areas(&draft)?;
        let geometry = Geometry::build(&draft, connectivity)?;
        // Input order is not paint order. Canonical runs make equality independent
        // of brush gesture subdivision; object paint order belongs to placements.
        draft.areas.sort_by(|left, right| left.id.cmp(&right.id));
        draft.floor.sort_by_key(|span| (span.y, span.start));
        let mut floor: Vec<FloorSpan> = Vec::with_capacity(draft.floor.len());
        for span in draft.floor {
            if let Some(previous) = floor.last_mut()
                && previous.y == span.y
                && previous.end == span.start
                && previous.area_id == span.area_id
            {
                previous.end = span.end;
            } else {
                floor.push(span);
            }
        }
        draft.floor = floor;
        draft.doors.sort();
        Ok(Self {
            draft,
            geometry,
            modules: None,
        })
    }

    /// The module draft is authoritative; floor and openings are derived once.
    pub fn from_modules(mut modules: modules::ModuleDraft) -> Result<Self, MapError> {
        let mut map = Self::admit(
            modules.project()?,
            match modules.layout {
                modules::ModuleLayout::IndependentMeetings => geometry::Connectivity::Meetings,
                modules::ModuleLayout::UnifiedAreas => geometry::Connectivity::Areas,
                _ => geometry::Connectivity::Lobby,
            },
        )?;
        modules
            .modules
            .sort_by(|left, right| left.area.id.cmp(&right.area.id));
        map.modules = Some(modules);
        Ok(map)
    }

    pub fn modules(&self) -> Option<&modules::ModuleDraft> {
        self.modules.as_ref()
    }

    pub fn draft(&self) -> &MapDraft {
        &self.draft
    }

    pub fn geometry(&self) -> &Geometry {
        &self.geometry
    }
}

fn validate_areas(draft: &MapDraft) -> Result<(), MapError> {
    if draft.areas.len() > MAX_AREAS
        || draft.floor.len() > MAX_SPANS
        || draft.doors.len() > MAX_DOORS
    {
        return Err(MapError::LimitExceeded);
    }
    let mut ids = BTreeSet::new();
    let mut occupants = BTreeSet::new();
    let mut rooms = BTreeSet::new();
    for area in &draft.areas {
        if !canonical_id(&area.id)
            || area.name.trim().is_empty()
            || area.name.len() > 80
            || area.name.chars().any(|c| {
                c.is_control() || matches!(c, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
        {
            return Err(MapError::InvalidArea);
        }
        if !ids.insert(&area.id) {
            return Err(MapError::DuplicateArea);
        }
        match &area.kind {
            AreaKind::Personal {
                identity_id: Some(id),
            } => {
                if !canonical_id(id) {
                    return Err(MapError::InvalidArea);
                }
                if !occupants.insert(id) {
                    return Err(MapError::DuplicateOccupant);
                }
            }
            AreaKind::Meeting { room_id } => {
                if !canonical_id(room_id) {
                    return Err(MapError::InvalidArea);
                }
                if !rooms.insert(room_id) {
                    return Err(MapError::DuplicateMeetingArea);
                }
            }
            _ => {}
        }
    }
    if !draft
        .areas
        .iter()
        .any(|area| area.id == draft.primary_lobby_id && matches!(area.kind, AreaKind::Lobby))
    {
        return Err(MapError::PrimaryLobbyRequired);
    }
    Ok(())
}
