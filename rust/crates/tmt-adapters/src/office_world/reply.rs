//! Shared public diagnostics and strict companion response admission.

use serde::{Deserialize, Serialize};
use tmt_core::dispatch::canonical_id;
use tmt_core::limits::MAX_JS_SAFE_INTEGER;
use tmt_office_model::codec::office_world::{
    WORLD_REPLY_LIMIT, WorldCodecError, WorldDocument, admit_world,
};
use tmt_office_model::json_integer::whole;
use tmt_office_model::office_world::LocalWorldSnapshot;
use tmt_office_model::office_world::PlacementError;
use tmt_office_model::office_world::WorldError;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorldFailureCode {
    WorldInvalid,
    WorldStoredInvalid,
    WorldMigrationInvalid,
    WorldRevisionConflict,
    WorldRevisionExhausted,
    WorldIdentityIneligible,
    WorldRoomMissing,
    WorldPropUnavailable,
    StorageUnavailable,
}
impl WorldFailureCode {
    pub fn code(self) -> &'static str {
        match self {
            Self::WorldInvalid => "WORLD_INVALID",
            Self::WorldStoredInvalid => "WORLD_STORED_INVALID",
            Self::WorldMigrationInvalid => "WORLD_MIGRATION_INVALID",
            Self::WorldRevisionConflict => "WORLD_REVISION_CONFLICT",
            Self::WorldRevisionExhausted => "WORLD_REVISION_EXHAUSTED",
            Self::WorldIdentityIneligible => "WORLD_IDENTITY_INELIGIBLE",
            Self::WorldRoomMissing => "WORLD_ROOM_MISSING",
            Self::WorldPropUnavailable => "WORLD_PROP_UNAVAILABLE",
            Self::StorageUnavailable => "STORAGE_UNAVAILABLE",
        }
    }
    pub fn status(self) -> u16 {
        match self {
            Self::WorldInvalid => 400,
            Self::WorldStoredInvalid | Self::StorageUnavailable => 500,
            _ => 409,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlacementDiagnostic {
    #[serde(deserialize_with = "Option::deserialize")]
    object_id: Option<String>,
    reason: PlacementReason,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum PlacementReason {
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
impl From<PlacementError> for PlacementReason {
    fn from(value: PlacementError) -> Self {
        match value {
            PlacementError::InvalidId => Self::InvalidId,
            PlacementError::DuplicateId => Self::DuplicateId,
            PlacementError::InvalidAppearance => Self::InvalidAppearance,
            PlacementError::InvalidExtension => Self::InvalidExtension,
            PlacementError::InvalidSurface => Self::InvalidSurface,
            PlacementError::OutsideFloor => Self::OutsideFloor,
            PlacementError::CrossesPartition => Self::CrossesPartition,
            PlacementError::BlocksDoor => Self::BlocksDoor,
            PlacementError::MissingWall => Self::MissingWall,
            PlacementError::WindowRequiresExterior => Self::WindowRequiresExterior,
            PlacementError::OverlapsWindow => Self::OverlapsWindow,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldFailure {
    #[serde(rename = "error")]
    pub code: WorldFailureCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issues: Option<Vec<PlacementDiagnostic>>,
}
impl WorldFailure {
    pub fn unavailable() -> Self {
        Self {
            code: WorldFailureCode::StorageUnavailable,
            message: None,
            issues: None,
        }
    }
    pub fn value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("finite world diagnostics")
    }
}
impl From<WorldCodecError> for WorldFailure {
    fn from(error: WorldCodecError) -> Self {
        let issues = match &error {
            WorldCodecError::Placement(WorldError::Placements(issues)) => issues
                .iter()
                .map(|issue| PlacementDiagnostic {
                    object_id: canonical_id(&issue.object_id).then(|| issue.object_id.clone()),
                    reason: issue.reason.into(),
                })
                .collect(),
            _ => Vec::new(),
        };
        Self {
            code: WorldFailureCode::WorldInvalid,
            message: Some(error.to_string()),
            issues: Some(issues),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    #[serde(deserialize_with = "Option::deserialize")]
    world_id: Option<String>,
    #[serde(deserialize_with = "whole")]
    revision: u64,
    #[serde(deserialize_with = "Option::deserialize")]
    legacy_basis: Option<String>,
    layout: WorldDocument,
    #[serde(deserialize_with = "whole")]
    updated_at_ms: u64,
    changed: bool,
}
pub fn decode_reply(
    bytes: &[u8],
) -> Result<Result<LocalWorldSnapshot, WorldFailure>, WorldCodecError> {
    if bytes.len() > WORLD_REPLY_LIMIT {
        return Err(WorldCodecError::TooLarge);
    }
    if let Ok(error) = serde_json::from_slice::<WorldFailure>(bytes) {
        if error.message.as_ref().is_some_and(|text| text.len() > 1024)
            || error.issues.as_ref().is_some_and(|issues| {
                issues.len() > tmt_office_model::office_world::MAX_OBJECTS
                    || issues
                        .iter()
                        .any(|issue| issue.object_id.as_ref().is_some_and(|id| !canonical_id(id)))
            })
        {
            return Err(WorldCodecError::InvalidJson);
        }
        return Ok(Err(error));
    }
    let snapshot: Snapshot =
        serde_json::from_slice(bytes).map_err(|_| WorldCodecError::InvalidJson)?;
    if snapshot
        .world_id
        .as_ref()
        .is_some_and(|id| !canonical_id(id))
        || (snapshot.revision > 0 && (snapshot.world_id.is_none() || snapshot.updated_at_ms == 0))
        || (snapshot.revision == 0 && (snapshot.updated_at_ms != 0 || snapshot.changed))
        || snapshot.revision > MAX_JS_SAFE_INTEGER
        || snapshot.updated_at_ms > MAX_JS_SAFE_INTEGER
        || (snapshot.revision == 0) != snapshot.legacy_basis.is_some()
        || snapshot
            .legacy_basis
            .as_ref()
            .is_some_and(|basis| !tmt_core::content_digest::is_sha256(basis))
    {
        return Err(WorldCodecError::InvalidJson);
    }
    Ok(Ok(LocalWorldSnapshot {
        world_id: snapshot.world_id,
        revision: snapshot.revision,
        legacy_basis: snapshot.legacy_basis,
        layout: admit_world(snapshot.layout)?,
        updated_at_ms: snapshot.updated_at_ms,
        changed: snapshot.changed,
    }))
}
