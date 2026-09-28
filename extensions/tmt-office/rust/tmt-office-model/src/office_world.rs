//! Whole-world placement admission over the map's single derived surface index.

mod placement;
#[cfg(test)]
mod tests;

use crate::office_block::PropPlacement;
use crate::office_extension::ExtensionAttachment;
use crate::office_map::{Axis, OfficeMap};
use std::collections::HashSet;
use tmt_core::dispatch::canonical_id;

pub const MAX_OBJECTS: usize = 4096;
/// Wall-local vertical units, independent of the camera's pixel scale.
pub const WALL_HEIGHT: u8 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WallFace {
    /// South of a horizontal edge, or east of a vertical edge.
    Positive,
    /// North of a horizontal edge, or west of a vertical edge.
    Negative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Floor {
        base: Option<FloorBase>,
    },
    Wall {
        axis: Axis,
        face: WallFace,
        elevation: u8,
    },
}

/// Physical support inside the unrotated artwork envelope. Artwork and picking
/// retain the complete envelope; only floor support uses this smaller rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloorBase {
    pub x: u8,
    pub y: u8,
    pub width: u8,
    pub height: u8,
}

impl FloorBase {
    pub fn fits(self, placement: &PropPlacement) -> bool {
        self.width > 0
            && self.height > 0
            && u16::from(self.x) + u16::from(self.width) <= u16::from(placement.footprint_width)
            && u16::from(self.y) + u16::from(self.height) <= u16::from(placement.footprint_height)
    }

    /// Rotate support in the same clockwise coordinate system as placement.
    /// Call only after `fits`; no raster or catalog is needed after art removal.
    pub fn rotated(self, placement: &PropPlacement) -> Self {
        let Self {
            x,
            y,
            width,
            height,
        } = self;
        let w = placement.footprint_width;
        let h = placement.footprint_height;
        match placement.rotation {
            1 => Self {
                x: h - y - height,
                y: x,
                width: height,
                height: width,
            },
            2 => Self {
                x: w - x - width,
                y: h - y - height,
                width,
                height,
            },
            3 => Self {
                x: y,
                y: w - x - width,
                width: height,
                height: width,
            },
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Decoration,
    Window,
    WallLight,
}

/// IDs identify placements, never the contents of linked resources or artwork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldObject {
    pub id: String,
    pub placement: PropPlacement,
    pub surface: Surface,
    pub kind: ObjectKind,
    pub extension: Option<ExtensionAttachment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementError {
    InvalidId,
    DuplicateId,
    InvalidAppearance,
    InvalidExtension,
    InvalidSurface,
    OutsideFloor,
    CrossesPartition,
    BlocksDoor,
    MissingWall,
    WindowRequiresExterior,
    OverlapsWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementIssue {
    pub object_id: String,
    pub reason: PlacementError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldError {
    TooManyObjects,
    Placements(Vec<PlacementIssue>),
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooManyObjects => "Office world exceeds its object budget.",
            Self::Placements(_) => "Resolve the affected Office objects before saving the world.",
        })
    }
}
impl std::error::Error for WorldError {}

#[derive(Debug, Clone)]
pub struct WorldLayout {
    map: OfficeMap,
    objects: Vec<WorldObject>,
}

impl WorldLayout {
    pub fn new(map: OfficeMap, objects: Vec<WorldObject>) -> Result<Self, WorldError> {
        if objects.len() > MAX_OBJECTS {
            return Err(WorldError::TooManyObjects);
        }
        let mut issues = Vec::new();
        let mut ids = HashSet::new();
        let mut mounts = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            let error = if !canonical_id(&object.id) {
                Some(PlacementError::InvalidId)
            } else if !ids.insert(&object.id) {
                Some(PlacementError::DuplicateId)
            } else if object
                .extension
                .as_ref()
                .is_some_and(|extension| !extension.is_valid())
            {
                Some(PlacementError::InvalidExtension)
            } else {
                match placement::validate(&map, object) {
                    Ok(Some(mount)) => {
                        mounts.push((index, mount));
                        None
                    }
                    Ok(None) => None,
                    Err(error) => Some(error),
                }
            };
            if let Some(reason) = error {
                issues.push(PlacementIssue {
                    object_id: object.id.clone(),
                    reason,
                });
            }
        }
        for index in placement::window_conflicts(&mounts) {
            issues.push(PlacementIssue {
                object_id: objects[index].id.clone(),
                reason: PlacementError::OverlapsWindow,
            });
        }
        if !issues.is_empty() {
            return Err(WorldError::Placements(issues));
        }
        Ok(Self { map, objects })
    }

    pub fn map(&self) -> &OfficeMap {
        &self.map
    }
    /// Furniture overlap and the user's paint order are intentionally retained.
    pub fn objects(&self) -> &[WorldObject] {
        &self.objects
    }
}

/// One observed local world: the reply shared by the Office store and the
/// CLI-side decoder.
#[derive(Debug, Clone)]
pub struct LocalWorldSnapshot {
    pub world_id: Option<String>,
    pub revision: u64,
    /// Required only for the unsaved projection; fences concurrent legacy edits.
    pub legacy_basis: Option<String>,
    pub layout: WorldLayout,
    pub updated_at_ms: u64,
    pub changed: bool,
}
