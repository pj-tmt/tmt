//! Office world filesystem, storage response and runtime access adapters.
pub mod access;
mod reply;
pub use reply::{WorldFailure, WorldFailureCode, decode_reply};
use serde_json::{Value, json};
use tmt_office_model::codec::office_world::*;
use tmt_office_model::office_world::WorldLayout;

pub fn read_world_file(path: &std::path::Path) -> Result<WorldLayout, WorldCodecError> {
    let bytes = crate::bounded_file::read(path, WORLD_DOCUMENT_LIMIT)
        .map_err(|_| WorldCodecError::InvalidJson)?;
    decode_world(&bytes)
}

pub fn snapshot_value(snapshot: &crate::storage::LocalWorldSnapshot) -> Value {
    json!({
        "worldId": snapshot.world_id, "revision": snapshot.revision,
        "legacyBasis": snapshot.legacy_basis, "layout": world_value(&snapshot.layout),
        "updatedAtMs": snapshot.updated_at_ms, "changed": snapshot.changed,
    })
}
