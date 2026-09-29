//! Exact snapshot replies over the existing verified companion process owner.

use serde::Deserialize;
use std::{io, path::Path, time::Instant};
use tmt_office_model::codec::office_whiteboard::image::SNAPSHOT_PNG_LIMIT;
use tmt_office_model::codec::office_whiteboard::image::validate_image_response;
use tmt_office_model::codec::office_whiteboard::snapshot::SNAPSHOT_OUTPUT_LIMIT;
use tmt_office_model::codec::office_whiteboard::snapshot::decode_snapshot;
use tmt_office_model::office_protocol::OfficeError;
use tmt_office_model::office_protocol::OfficeInvocation;
use tmt_office_model::office_whiteboard::snapshot::WhiteboardSnapshot;
use tmt_office_model::office_whiteboard::snapshot::resolve_snapshot_reference;

pub enum SnapshotResource {
    Scene(WhiteboardSnapshot),
    Image(Vec<u8>),
}

pub fn read_office_snapshot(
    executable: &Path,
    reference: &str,
    image: bool,
    deadline: Instant,
) -> io::Result<Result<SnapshotResource, OfficeError>> {
    let Some(id) = resolve_snapshot_reference(reference) else {
        return Ok(Err(OfficeError::WhiteboardInvalid));
    };
    let operation = if image {
        OfficeInvocation::WhiteboardSnapshotImage
    } else {
        OfficeInvocation::WhiteboardSnapshotShow
    };
    let input = serde_json::to_vec(&serde_json::json!({"snapshotId":id}))?;
    let bytes = super::invoke_bytes_bounded(
        executable,
        operation,
        &input,
        deadline,
        if image {
            SNAPSHOT_PNG_LIMIT
        } else {
            SNAPSHOT_OUTPUT_LIMIT
        },
    )?;
    decode_reply(bytes, id, image)
}

fn decode_reply(
    bytes: Vec<u8>,
    id: &str,
    image: bool,
) -> io::Result<Result<SnapshotResource, OfficeError>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ErrorReply {
        error: String,
    }
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Office snapshot response.",
        )
    };
    if bytes.len() <= 256
        && let Ok(reply) = serde_json::from_slice::<ErrorReply>(&bytes)
    {
        return match OfficeError::parse(&reply.error) {
            Some(
                error @ (OfficeError::WhiteboardInvalid
                | OfficeError::WhiteboardNotFound
                | OfficeError::StorageUnavailable),
            ) => Ok(Err(error)),
            _ => Err(invalid()),
        };
    }
    if image {
        validate_image_response(&bytes).map_err(|_| invalid())?;
        Ok(Ok(SnapshotResource::Image(bytes)))
    } else {
        let snapshot = decode_snapshot(&bytes).map_err(|_| invalid())?;
        if snapshot.id != id {
            return Err(invalid());
        }
        Ok(Ok(SnapshotResource::Scene(snapshot)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office_whiteboard::image_tests::test_support;
    use tmt_office_model::codec::office_whiteboard::image::decode_snapshot_image;
    use tmt_office_model::codec::office_whiteboard::snapshot::encode_snapshot;
    use tmt_office_model::office_whiteboard::{
        document::empty_document, snapshot::WhiteboardSnapshot,
    };

    #[test]
    fn reply_rejects_wrong_resources_unknown_errors_and_invalid_images() {
        let id = "11111111-1111-4111-8111-111111111111";
        let scene = WhiteboardSnapshot {
            id: id.into(),
            document_id: "lobby".into(),
            document_revision: 1,
            scene: empty_document("lobby").scene,
            selected_element_ids: vec![],
            annotation: "Context".into(),
            created_at_ms: 100,
        };
        let bytes = encode_snapshot(&scene).unwrap();
        assert!(
            matches!(decode_reply(bytes.clone(), id, false).unwrap().unwrap(), SnapshotResource::Scene(value) if value == scene)
        );
        assert!(decode_reply(bytes, "22222222-2222-4222-8222-222222222222", false).is_err());
        assert_eq!(
            decode_reply(br#"{"error":"WHITEBOARD_NOT_FOUND"}"#.to_vec(), id, true)
                .unwrap()
                .err(),
            Some(OfficeError::WhiteboardNotFound)
        );
        assert!(decode_reply(br#"{"error":"OFFICE_REMOTE_DENIED"}"#.to_vec(), id, true).is_err());
        assert!(
            decode_reply(
                br#"{"error":"WHITEBOARD_NOT_FOUND","path":"private"}"#.to_vec(),
                id,
                true
            )
            .is_err()
        );
        assert!(decode_reply(b"PNG".to_vec(), id, true).is_err());
        let png = test_support::png(
            1600,
            1000,
            png::ColorType::Rgb,
            &[10, 20, 30].repeat(1600 * 1000),
            false,
        );
        let normalized = decode_snapshot_image(&png).unwrap().into_bytes();
        assert!(
            matches!(decode_reply(normalized.clone(), id, true).unwrap().unwrap(), SnapshotResource::Image(value) if value == normalized)
        );
    }
}
