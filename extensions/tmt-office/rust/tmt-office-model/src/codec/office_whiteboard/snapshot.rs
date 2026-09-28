//! Snapshot metadata cannot submit replacement scene content or select a sender.

use crate::office_whiteboard::{
    InvalidScene,
    snapshot::{CaptureWhiteboard, WhiteboardSnapshot, validate_capture, validate_snapshot},
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::{decode_scene, encode_scene, whole};

/// 2,048 UUID selections plus a 16 KiB annotation and their JSON envelope.
pub const CAPTURE_INPUT_LIMIT: usize = 128 * 1024;
pub const SNAPSHOT_OUTPUT_LIMIT: usize =
    crate::office_whiteboard::DOCUMENT_BYTES + CAPTURE_INPUT_LIMIT + 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotInput<'a> {
    id: String,
    document_id: String,
    #[serde(deserialize_with = "whole")]
    document_revision: u64,
    #[serde(borrow)]
    scene: &'a RawValue,
    selected_element_ids: Vec<String>,
    annotation: String,
    #[serde(deserialize_with = "whole")]
    created_at_ms: u64,
}

pub fn decode_snapshot(bytes: &[u8]) -> Result<WhiteboardSnapshot, InvalidScene> {
    if bytes.len() > SNAPSHOT_OUTPUT_LIMIT {
        return Err(InvalidScene);
    }
    let input: SnapshotInput<'_> = serde_json::from_slice(bytes).map_err(|_| InvalidScene)?;
    let snapshot = WhiteboardSnapshot {
        id: input.id,
        document_id: input.document_id,
        document_revision: input.document_revision,
        scene: decode_scene(input.scene.get().as_bytes())?,
        selected_element_ids: input.selected_element_ids,
        annotation: input.annotation,
        created_at_ms: input.created_at_ms,
    };
    validate_snapshot(&snapshot).map_err(|_| InvalidScene)?;
    Ok(snapshot)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaptureInput {
    #[serde(deserialize_with = "whole")]
    expected_revision: u64,
    operation_id: String,
    selected_element_ids: Vec<String>,
    annotation: String,
}

pub fn decode_capture(document_id: &str, bytes: &[u8]) -> Result<CaptureWhiteboard, InvalidScene> {
    if bytes.len() > CAPTURE_INPUT_LIMIT {
        return Err(InvalidScene);
    }
    let input: CaptureInput = serde_json::from_slice(bytes).map_err(|_| InvalidScene)?;
    let capture = CaptureWhiteboard {
        document_id: document_id.into(),
        expected_revision: input.expected_revision,
        operation_id: input.operation_id,
        selected_element_ids: input.selected_element_ids,
        annotation: input.annotation,
    };
    validate_capture(&capture).map_err(|_| InvalidScene)?;
    Ok(capture)
}

pub fn encode_snapshot(snapshot: &WhiteboardSnapshot) -> Result<Vec<u8>, InvalidScene> {
    validate_snapshot(snapshot).map_err(|_| InvalidScene)?;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Output<'a> {
        id: &'a str,
        document_id: &'a str,
        document_revision: u64,
        scene: &'a RawValue,
        selected_element_ids: &'a [String],
        annotation: &'a str,
        created_at_ms: u64,
    }
    let scene_bytes = encode_scene(&snapshot.scene)?;
    let scene = serde_json::from_slice(&scene_bytes).map_err(|_| InvalidScene)?;
    let bytes = serde_json::to_vec(&Output {
        id: &snapshot.id,
        document_id: &snapshot.document_id,
        document_revision: snapshot.document_revision,
        scene,
        selected_element_ids: &snapshot.selected_element_ids,
        annotation: &snapshot.annotation,
        created_at_ms: snapshot.created_at_ms,
    })
    .map_err(|_| InvalidScene)?;
    if bytes.len() > SNAPSHOT_OUTPUT_LIMIT {
        return Err(InvalidScene);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_reference_resolution_matches_shared_vectors() {
        use crate::office_whiteboard::snapshot::resolve_snapshot_reference;
        let vectors: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/snapshot-reference-vectors.json"
        ))
        .unwrap();
        for item in vectors["valid"].as_array().unwrap() {
            assert_eq!(
                resolve_snapshot_reference(item["input"].as_str().unwrap()),
                item["id"].as_str()
            );
        }
        for item in vectors["invalid"].as_array().unwrap() {
            assert_eq!(resolve_snapshot_reference(item.as_str().unwrap()), None);
        }
    }

    #[test]
    fn snapshot_decoder_preserves_raw_scene_validation_and_bounds() {
        let scene: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/whiteboard-scene-v1.json"
        ))
        .unwrap();
        let value = json!({"id":"11111111-1111-4111-8111-111111111111","documentId":"lobby","documentRevision":1,"scene":scene,"selectedElementIds":[],"annotation":"Review this.","createdAtMs":100});
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            decode_snapshot(&encode_snapshot(&decode_snapshot(&bytes).unwrap()).unwrap()).unwrap(),
            decode_snapshot(&bytes).unwrap()
        );
        for (key, value) in [
            ("documentRevision", json!(0)),
            ("scene", json!({})),
            (
                "selectedElementIds",
                json!(["77777777-7777-4777-8777-777777777777"]),
            ),
            ("token", json!("secret")),
        ] {
            let mut invalid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            invalid[key] = value;
            assert!(
                decode_snapshot(&serde_json::to_vec(&invalid).unwrap()).is_err(),
                "{key}"
            );
        }
        let duplicated = String::from_utf8(bytes.clone()).unwrap().replace(
            "\"documentRevision\":1",
            "\"documentRevision\":1,\"documentRevision\":1",
        );
        assert!(decode_snapshot(duplicated.as_bytes()).is_err());
        let mut padded = bytes;
        padded.resize(SNAPSHOT_OUTPUT_LIMIT, b' ');
        assert!(decode_snapshot(&padded).is_ok());
        padded.push(b' ');
        assert!(decode_snapshot(&padded).is_err());
    }

    #[test]
    fn capture_admission_is_exact_and_cannot_replace_content_or_spoof_an_actor() {
        let input = json!({
            "expectedRevision":1,"operationId":"11111111-1111-4111-8111-111111111111",
            "selectedElementIds":[],"annotation":"Review this revision."
        });
        assert!(decode_capture("lobby", &serde_json::to_vec(&input).unwrap()).is_ok());
        for key in ["scene", "actor", "image", "documentId", "command"] {
            let mut invalid = input.clone();
            invalid[key] = json!("unsupported");
            assert!(decode_capture("lobby", &serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let encoded = input.to_string();
        let duplicate = encoded.replacen(
            "\"expectedRevision\":1",
            "\"expectedRevision\":1,\"expectedRevision\":1",
            1,
        );
        assert_ne!(duplicate, encoded);
        assert!(decode_capture("lobby", duplicate.as_bytes()).is_err());
        assert!(decode_capture("lobby", &vec![b' '; CAPTURE_INPUT_LIMIT + 1]).is_err());
        assert!(
            decode_capture(
                "lobby",
                encoded
                    .replace("\"expectedRevision\":1", "\"expectedRevision\":1e0")
                    .as_bytes()
            )
            .is_ok()
        );
        assert!(
            decode_capture(
                "lobby",
                encoded
                    .replace("\"expectedRevision\":1", "\"expectedRevision\":1.5")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(decode_capture("../private", encoded.as_bytes()).is_err());
    }

    #[test]
    fn full_selection_and_annotation_fit_the_metadata_envelope() {
        let ids: Vec<_> = (0..crate::office_whiteboard::ELEMENT_LIMIT)
            .map(|index| format!("11111111-1111-4111-8111-{index:012x}"))
            .collect();
        let input = json!({
            "expectedRevision":1,"operationId":"11111111-1111-4111-8111-111111111111",
            "selectedElementIds":ids,"annotation":"x".repeat(crate::office_whiteboard::TEXT_BYTES)
        });
        let bytes = serde_json::to_vec(&input).unwrap();
        assert!(bytes.len() > 64 * 1024);
        assert!(bytes.len() < CAPTURE_INPUT_LIMIT);
        assert_eq!(
            decode_capture("lobby", &bytes)
                .unwrap()
                .selected_element_ids
                .len(),
            ids.len()
        );
    }
}
