//! Strict, versioned world-map wire admission over the shared pure topology owner.

mod modular;

use crate::json_integer::whole;
use crate::office_map::{Area, AreaKind, Axis, Edge, FloorSpan, MapDraft, MapError, OfficeMap};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Covers the bounded row runs, area labels and doors; unrelated protocols retain their limits.
pub const MAP_DOCUMENT_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapCodecError {
    TooLarge,
    InvalidJson,
    UnsupportedVersion,
    Geometry(MapError),
}

impl std::fmt::Display for MapCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("Office map document exceeds its byte budget."),
            Self::InvalidJson => f.write_str("Invalid Office map JSON shape."),
            Self::UnsupportedVersion => f.write_str("Unsupported Office map version."),
            Self::Geometry(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for MapCodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Geometry(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum MapDocument {
    Legacy(LegacyMapDocument),
    Modular(modular::ModuleDocument),
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LegacyMapDocument {
    #[serde(deserialize_with = "whole")]
    version: u32,
    primary_lobby_id: String,
    areas: Vec<AreaDocument>,
    floor: Vec<SpanDocument>,
    doors: Vec<DoorDocument>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AreaDocument {
    id: String,
    name: String,
    binding: AreaBinding,
}

impl AreaDocument {
    fn into_area(self) -> Area {
        Area {
            id: self.id,
            name: self.name,
            kind: match self.binding {
                AreaBinding::Lobby {} => AreaKind::Lobby,
                AreaBinding::Personal { identity_id } => AreaKind::Personal { identity_id },
                AreaBinding::Meeting { room_id } => AreaKind::Meeting { room_id },
            },
        }
    }

    fn from_area(area: &Area) -> Self {
        Self {
            id: area.id.clone(),
            name: area.name.clone(),
            binding: match &area.kind {
                AreaKind::Lobby => AreaBinding::Lobby {},
                AreaKind::Personal { identity_id } => AreaBinding::Personal {
                    identity_id: identity_id.clone(),
                },
                AreaKind::Meeting { room_id } => AreaBinding::Meeting {
                    room_id: room_id.clone(),
                },
            },
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum AreaBinding {
    Lobby {},
    Personal {
        #[serde(rename = "identityId", deserialize_with = "required_nullable_id")]
        identity_id: Option<String>,
    },
    Meeting {
        #[serde(rename = "roomId")]
        room_id: String,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpanDocument {
    #[serde(deserialize_with = "whole")]
    y: i32,
    #[serde(deserialize_with = "whole")]
    start: i32,
    #[serde(deserialize_with = "whole")]
    end: i32,
    #[serde(deserialize_with = "required_nullable_id")]
    area_id: Option<String>,
}

// Explicit null is meaningful; omission is a malformed document, not common floor.
fn required_nullable_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DoorDocument {
    #[serde(deserialize_with = "whole")]
    x: i32,
    #[serde(deserialize_with = "whole")]
    y: i32,
    axis: DoorAxis,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum DoorAxis {
    Horizontal,
    Vertical,
}

pub fn decode_map(bytes: &[u8]) -> Result<OfficeMap, MapCodecError> {
    if bytes.len() > MAP_DOCUMENT_LIMIT {
        return Err(MapCodecError::TooLarge);
    }
    let value: MapDocument =
        serde_json::from_slice(bytes).map_err(|_| MapCodecError::InvalidJson)?;
    admit_document(value)
}

pub(crate) fn admit_document(value: MapDocument) -> Result<OfficeMap, MapCodecError> {
    match value {
        MapDocument::Legacy(value) => admit_legacy(value),
        MapDocument::Modular(value) => modular::admit(value),
    }
}

fn admit_legacy(value: LegacyMapDocument) -> Result<OfficeMap, MapCodecError> {
    if value.version != 1 {
        return Err(MapCodecError::UnsupportedVersion);
    }
    OfficeMap::new(MapDraft {
        primary_lobby_id: value.primary_lobby_id,
        areas: value
            .areas
            .into_iter()
            .map(AreaDocument::into_area)
            .collect(),
        floor: value
            .floor
            .into_iter()
            .map(|span| FloorSpan {
                y: span.y,
                start: span.start,
                end: span.end,
                area_id: span.area_id,
            })
            .collect(),
        doors: value
            .doors
            .into_iter()
            .map(|edge| Edge {
                x: edge.x,
                y: edge.y,
                axis: match edge.axis {
                    DoorAxis::Horizontal => Axis::Horizontal,
                    DoorAxis::Vertical => Axis::Vertical,
                },
            })
            .collect(),
    })
    .map_err(MapCodecError::Geometry)
}

pub fn map_value(map: &OfficeMap) -> Value {
    if let Some(modules) = map.modules() {
        return modular::value(modules);
    }
    let draft = map.draft();
    let value = LegacyMapDocument {
        version: 1,
        primary_lobby_id: draft.primary_lobby_id.clone(),
        areas: draft.areas.iter().map(AreaDocument::from_area).collect(),
        floor: draft
            .floor
            .iter()
            .map(|span| SpanDocument {
                y: span.y,
                start: span.start,
                end: span.end,
                area_id: span.area_id.clone(),
            })
            .collect(),
        doors: draft
            .doors
            .iter()
            .map(|edge| DoorDocument {
                x: edge.x,
                y: edge.y,
                axis: match edge.axis {
                    Axis::Horizontal => DoorAxis::Horizontal,
                    Axis::Vertical => DoorAxis::Vertical,
                },
            })
            .collect(),
    };
    serde_json::to_value(value)
        .expect("admitted Office map contains only finite integer/string fields")
}

#[cfg(test)]
mod tests;
