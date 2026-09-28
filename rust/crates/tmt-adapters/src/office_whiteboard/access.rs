//! Read-only companion access to the same retained resources served by HTTP.

use crate::{
    config::ConfigPaths,
    storage::{Storage, WhiteboardStoreError},
};
use serde::Deserialize;
use std::path::Path;
use tmt_office_model::codec::office_whiteboard::snapshot::encode_snapshot;
use tmt_office_model::office_protocol::OfficeError;
use tmt_office_model::office_protocol::OfficeInvocation;
use tmt_office_model::office_whiteboard::snapshot::valid_snapshot_id;

pub const READ_INPUT_LIMIT: usize = 4096;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadInput {
    snapshot_id: String,
}

fn admit(operation: OfficeInvocation, bytes: &[u8]) -> Result<String, OfficeError> {
    if !matches!(
        operation,
        OfficeInvocation::WhiteboardSnapshotShow | OfficeInvocation::WhiteboardSnapshotImage
    ) || bytes.len() > READ_INPUT_LIMIT
    {
        return Err(OfficeError::WhiteboardInvalid);
    }
    let input: ReadInput =
        serde_json::from_slice(bytes).map_err(|_| OfficeError::WhiteboardInvalid)?;
    if !valid_snapshot_id(&input.snapshot_id) {
        return Err(OfficeError::WhiteboardInvalid);
    }
    Ok(input.snapshot_id)
}

pub fn execute(operation: OfficeInvocation, input: &[u8]) -> Vec<u8> {
    let result = admit(operation, input).and_then(|id| {
        let paths = ConfigPaths::discover().map_err(|_| OfficeError::StorageUnavailable)?;
        read(&paths.database, operation, &id)
    });
    result.unwrap_or_else(|error| {
        serde_json::to_vec(&serde_json::json!({"error":error.code()}))
            .expect("static error serializes")
    })
}

fn storage_error(error: WhiteboardStoreError) -> OfficeError {
    OfficeError::parse(error.code()).unwrap_or(OfficeError::StorageUnavailable)
}

fn read(database: &Path, operation: OfficeInvocation, id: &str) -> Result<Vec<u8>, OfficeError> {
    let mut storage = Storage::open(database).map_err(|_| OfficeError::StorageUnavailable)?;
    let result = if operation == OfficeInvocation::WhiteboardSnapshotImage {
        storage
            .show_whiteboard_snapshot_image(id)
            .map(|image| image.into_bytes())
            .map_err(storage_error)
    } else {
        storage
            .show_whiteboard_snapshot(id)
            .map_err(storage_error)
            .and_then(|snapshot| {
                encode_snapshot(&snapshot).map_err(|_| OfficeError::StorageUnavailable)
            })
    };
    let close = storage.close().map_err(|_| OfficeError::StorageUnavailable);
    close.and(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_input_has_no_path_actor_or_command_authority() {
        let operation = OfficeInvocation::WhiteboardSnapshotShow;
        let bytes = br#"{"snapshotId":"11111111-1111-4111-8111-111111111111"}"#;
        assert!(admit(operation, bytes).is_ok());
        assert!(admit(OfficeInvocation::Sync, bytes).is_err());
        for key in ["path", "output", "actor", "command", "token"] {
            let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            value[key] = serde_json::json!("not allowed");
            assert_eq!(
                admit(operation, &serde_json::to_vec(&value).unwrap()),
                Err(OfficeError::WhiteboardInvalid)
            );
        }
        let mut padded = bytes.to_vec();
        padded.resize(READ_INPUT_LIMIT, b' ');
        assert!(admit(operation, &padded).is_ok());
        padded.push(b' ');
        assert!(admit(operation, &padded).is_err());
    }

    #[test]
    fn native_invocations_round_trip_exactly() {
        for operation in [
            OfficeInvocation::WhiteboardSnapshotShow,
            OfficeInvocation::WhiteboardSnapshotImage,
        ] {
            assert_eq!(
                OfficeInvocation::parse(&operation.arguments()),
                Ok(operation)
            );
            let mut args = operation.arguments().to_vec();
            args.push("extra");
            assert!(OfficeInvocation::parse(&args).is_err());
        }
    }
}
