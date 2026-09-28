//! CLI companion and HTTP share one close-before-publication storage operation.

use super::{SaveWorld, WorldCodecError, WorldFailure, decode_save, snapshot_value};
use crate::{
    config::ConfigPaths,
    storage::{LocalWorldSnapshot, Storage},
};
use std::path::Path;
use tmt_office_model::office_protocol::OfficeInvocation;

pub fn run(
    database: &Path,
    edit: Option<SaveWorld>,
    now_ms: u64,
) -> Result<LocalWorldSnapshot, WorldFailure> {
    let mut storage = Storage::open(database).map_err(|_| WorldFailure::unavailable())?;
    let result = match edit {
        Some(edit) => storage.apply_local_world(
            edit.expected_revision,
            edit.legacy_basis.as_deref(),
            &edit.layout,
            now_ms,
        ),
        None => storage.show_local_world(),
    }
    .map_err(WorldFailure::from);
    let closed = storage.close();
    match result {
        Err(error) => Err(error),
        Ok(snapshot) if closed.is_ok() => Ok(snapshot),
        Ok(_) => Err(WorldFailure::unavailable()),
    }
}

pub fn execute(operation: OfficeInvocation, bytes: &[u8]) -> Vec<u8> {
    let result = match operation {
        OfficeInvocation::LocalWorldShow if bytes == b"{}" => Ok(None),
        OfficeInvocation::LocalWorldApply => decode_save(bytes).map(Some),
        _ => Err(WorldCodecError::InvalidJson),
    }
    .map_err(WorldFailure::from)
    .and_then(|edit| {
        let paths = ConfigPaths::discover().map_err(|_| WorldFailure::unavailable())?;
        run(
            &paths.database,
            edit,
            crate::request_runtime::wall_time_ms(),
        )
    });
    let value = match result {
        Ok(snapshot) => snapshot_value(&snapshot),
        Err(error) => error.value(),
    };
    serde_json::to_vec(&value).expect("finite world reply")
}
