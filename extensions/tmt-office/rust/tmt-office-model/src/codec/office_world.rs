//! One strict envelope for topology and ordered world placements, not resource contents.

mod preset;
pub mod reply;
mod starter;
pub use preset::{lobby_objects, placement_id};
pub use starter::new_world;

use crate::codec::office_block::{PropPlacementInput, placement_value};
use crate::codec::office_extension::{
    ExtensionError, ExtensionResourceBinding, validate_attachment,
};
use crate::codec::office_map::{MapCodecError, MapDocument, admit_document, map_value};
use crate::json_integer::whole;
use crate::office_extension::ExtensionAttachment;
use crate::office_map::Axis;
use crate::office_world::{
    FloorBase, ObjectKind, Surface, WallFace, WorldError, WorldLayout, WorldObject,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub const WORLD_DOCUMENT_LIMIT: usize = 4 * 1024 * 1024;
pub const WORLD_ENVELOPE_LIMIT: usize = WORLD_DOCUMENT_LIMIT + 512;
pub const WORLD_REPLY_LIMIT: usize = WORLD_DOCUMENT_LIMIT + 1024;

pub struct SaveWorld {
    pub expected_revision: u64,
    pub legacy_basis: Option<String>,
    pub layout: WorldLayout,
}

pub fn save_value(input: &SaveWorld) -> Value {
    json!({ "expectedRevision": input.expected_revision, "legacyBasis": input.legacy_basis,
        "layout": world_value(&input.layout) })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveInput {
    #[serde(deserialize_with = "whole")]
    expected_revision: u64,
    #[serde(deserialize_with = "Option::deserialize")]
    legacy_basis: Option<String>,
    layout: WorldDocument,
}

pub fn decode_save(bytes: &[u8]) -> Result<SaveWorld, WorldCodecError> {
    if bytes.len() > WORLD_ENVELOPE_LIMIT {
        return Err(WorldCodecError::TooLarge);
    }
    let input: SaveInput =
        serde_json::from_slice(bytes).map_err(|_| WorldCodecError::InvalidJson)?;
    if input.expected_revision > tmt_core::limits::MAX_JS_SAFE_INTEGER
        || (input.expected_revision == 0) != input.legacy_basis.is_some()
        || input
            .legacy_basis
            .as_ref()
            .is_some_and(|value| !tmt_core::content_digest::is_sha256(value))
    {
        return Err(WorldCodecError::InvalidJson);
    }
    let layout = admit_world(input.layout)?;
    if serde_json::to_vec(&world_value(&layout))
        .expect("finite world")
        .len()
        > WORLD_DOCUMENT_LIMIT
    {
        return Err(WorldCodecError::TooLarge);
    }
    Ok(SaveWorld {
        expected_revision: input.expected_revision,
        legacy_basis: input.legacy_basis,
        layout,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldCodecError {
    TooLarge,
    InvalidJson,
    UnsupportedVersion,
    Map(MapCodecError),
    Placement(WorldError),
    Extension(ExtensionError),
}

impl std::fmt::Display for WorldCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("Office world document exceeds its byte budget."),
            Self::InvalidJson => f.write_str("Invalid Office world document fields."),
            Self::UnsupportedVersion => f.write_str("Unsupported Office world document version."),
            Self::Map(error) => error.fmt(f),
            Self::Placement(error) => error.fmt(f),
            Self::Extension(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for WorldCodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Map(error) => Some(error),
            Self::Placement(error) => Some(error),
            Self::Extension(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldDocument {
    #[serde(deserialize_with = "whole")]
    version: u32,
    map: MapDocument,
    objects: Vec<ObjectDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectDocument {
    id: String,
    kind: Kind,
    placement: PropPlacementInput,
    surface: SurfaceDocument,
    #[serde(deserialize_with = "Option::deserialize")]
    extension: Option<ExtensionDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionDocument {
    definition: String,
    binding: ExtensionResourceBinding,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Kind {
    Decoration,
    Window,
    WallLight,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum SurfaceDocument {
    Floor {
        #[serde(default, deserialize_with = "present_base")]
        base: Option<FloorBaseDocument>,
    },
    Wall {
        axis: WallAxis,
        face: Face,
        #[serde(deserialize_with = "whole")]
        elevation: u8,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FloorBaseDocument {
    #[serde(deserialize_with = "whole")]
    x: u8,
    #[serde(deserialize_with = "whole")]
    y: u8,
    #[serde(deserialize_with = "whole")]
    width: u8,
    #[serde(deserialize_with = "whole")]
    height: u8,
}

fn present_base<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<FloorBaseDocument>, D::Error> {
    FloorBaseDocument::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum WallAxis {
    Horizontal,
    Vertical,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Face {
    Positive,
    Negative,
}

pub fn decode_world(bytes: &[u8]) -> Result<WorldLayout, WorldCodecError> {
    if bytes.len() > WORLD_DOCUMENT_LIMIT {
        return Err(WorldCodecError::TooLarge);
    }
    let document: WorldDocument =
        serde_json::from_slice(bytes).map_err(|_| WorldCodecError::InvalidJson)?;
    admit_world(document)
}

pub fn admit_world(document: WorldDocument) -> Result<WorldLayout, WorldCodecError> {
    if document.version != 1 {
        return Err(WorldCodecError::UnsupportedVersion);
    }
    if document.objects.len() > crate::office_world::MAX_OBJECTS {
        return Err(WorldCodecError::Placement(WorldError::TooManyObjects));
    }
    let map = admit_document(document.map).map_err(WorldCodecError::Map)?;
    let objects = document
        .objects
        .into_iter()
        .map(|object| {
            let extension = object.extension.map(|value| ExtensionAttachment {
                definition: value.definition,
                binding: value.binding.into(),
            });
            if let Some(attachment) = &extension {
                validate_attachment(attachment).map_err(WorldCodecError::Extension)?;
            }
            Ok(WorldObject {
                id: object.id,
                extension,
                kind: match object.kind {
                    Kind::Decoration => ObjectKind::Decoration,
                    Kind::Window => ObjectKind::Window,
                    Kind::WallLight => ObjectKind::WallLight,
                },
                placement: object.placement.into_placement(),
                surface: match object.surface {
                    SurfaceDocument::Floor { base } => Surface::Floor {
                        base: base.map(|base| FloorBase {
                            x: base.x,
                            y: base.y,
                            width: base.width,
                            height: base.height,
                        }),
                    },
                    SurfaceDocument::Wall {
                        axis,
                        face,
                        elevation,
                    } => Surface::Wall {
                        axis: match axis {
                            WallAxis::Horizontal => Axis::Horizontal,
                            WallAxis::Vertical => Axis::Vertical,
                        },
                        face: match face {
                            Face::Positive => WallFace::Positive,
                            Face::Negative => WallFace::Negative,
                        },
                        elevation,
                    },
                },
            })
        })
        .collect::<Result<Vec<_>, WorldCodecError>>()?;
    WorldLayout::new(map, objects).map_err(WorldCodecError::Placement)
}

pub fn world_value(world: &WorldLayout) -> Value {
    json!({
        "version": 1,
        "map": map_value(world.map()),
        "objects": world.objects().iter().map(|object| json!({
            "id": object.id,
            "extension": object.extension.as_ref().map(|attachment| json!({
                "definition": attachment.definition,
                "binding": ExtensionResourceBinding::from(&attachment.binding),
            })),
            "kind": match object.kind { ObjectKind::Decoration => "decoration", ObjectKind::Window => "window", ObjectKind::WallLight => "wallLight" },
            "placement": placement_value(&object.placement),
            "surface": match object.surface {
                Surface::Floor { base } => match base {
                    None => json!({"type": "floor"}),
                    Some(base) => json!({"type": "floor", "base": {
                        "x": base.x, "y": base.y, "width": base.width, "height": base.height,
                    }}),
                },
                Surface::Wall { axis, face, elevation } => json!({
                    "type": "wall",
                    "axis": match axis { Axis::Horizontal => "horizontal", Axis::Vertical => "vertical" },
                    "face": match face { WallFace::Positive => "positive", WallFace::Negative => "negative" },
                    "elevation": elevation,
                }),
            },
        })).collect::<Vec<_>>(),
    })
}

pub fn snapshot_value(snapshot: &crate::office_world::LocalWorldSnapshot) -> Value {
    json!({
        "worldId": snapshot.world_id, "revision": snapshot.revision,
        "legacyBasis": snapshot.legacy_basis, "layout": world_value(&snapshot.layout),
        "updatedAtMs": snapshot.updated_at_ms, "changed": snapshot.changed,
    })
}

#[cfg(test)]
mod tests;
