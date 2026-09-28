//! Shared native document envelopes. Preserve raw scenes through outer decoding.

use super::{decode_scene, encode_scene, whole};
use crate::office_whiteboard::{
    InvalidScene,
    document::{SaveDocument, SaveReceipt, WhiteboardDocument, validate_intent},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, value::RawValue};

/// The scene retains its own 2 MiB limit; reserve separate space for envelope fields.
pub const SAVE_INPUT_LIMIT: usize = crate::office_whiteboard::DOCUMENT_BYTES + 16 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveInput<'a> {
    #[serde(deserialize_with = "whole")]
    expected_revision: u64,
    operation_id: String,
    #[serde(borrow)]
    scene: &'a RawValue,
}

pub fn decode_save(document_id: &str, bytes: &[u8]) -> Result<SaveDocument, InvalidScene> {
    if bytes.len() > SAVE_INPUT_LIMIT {
        return Err(InvalidScene);
    }
    let input: SaveInput<'_> = serde_json::from_slice(bytes).map_err(|_| InvalidScene)?;
    let request = SaveDocument {
        document_id: document_id.into(),
        expected_revision: input.expected_revision,
        operation_id: input.operation_id,
        scene: decode_scene(input.scene.get().as_bytes())?,
    };
    validate_intent(&request).map_err(|_| InvalidScene)?;
    Ok(request)
}

pub fn encode_document(document: &WhiteboardDocument) -> Result<Vec<u8>, InvalidScene> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Output<'a> {
        id: &'a str,
        revision: u64,
        scene: &'a RawValue,
        updated_at_ms: u64,
    }
    let bytes = encode_scene(&document.scene)?;
    let scene = serde_json::from_slice(&bytes).map_err(|_| InvalidScene)?;
    serde_json::to_vec(&Output {
        id: &document.id,
        revision: document.revision,
        scene,
        updated_at_ms: document.updated_at_ms,
    })
    .map_err(|_| InvalidScene)
}

pub fn encode_receipt(receipt: &SaveReceipt) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "documentId": receipt.document_id, "operationId": receipt.operation_id,
        "revision": receipt.revision, "changed": receipt.changed, "updatedAtMs": receipt.updated_at_ms
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn input() -> String {
        format!(
            r#"{{"expectedRevision":0,"operationId":"11111111-1111-4111-8111-111111111111","scene":{}}}"#,
            std::str::from_utf8(include_bytes!(
                "../../../../../../../contracts/office/whiteboard-scene-v1.json"
            ))
            .unwrap()
        )
    }

    #[test]
    fn envelope_does_not_erase_duplicate_scene_fields_or_accept_author_spoofing() {
        let source = input();
        assert!(decode_save("lobby", source.as_bytes()).is_ok());
        for (before, after) in [
            (
                "\"expectedRevision\":0",
                "\"expectedRevision\":0,\"expectedRevision\":0",
            ),
            (
                "\"formatVersion\": 1",
                "\"formatVersion\": 1, \"formatVersion\": 1",
            ),
            (
                "\"expectedRevision\":0",
                "\"actor\":\"alice\",\"expectedRevision\":0",
            ),
        ] {
            assert!(source.contains(before));
            assert!(decode_save("lobby", source.replacen(before, after, 1).as_bytes()).is_err());
        }
        assert!(decode_save("../private", source.as_bytes()).is_err());
        assert!(
            decode_save(
                "lobby",
                source
                    .replacen(
                        "\"expectedRevision\":0",
                        "\"expectedRevision\":9007199254740992",
                        1
                    )
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            decode_save(
                "lobby",
                source
                    .replacen("\"expectedRevision\":0", "\"expectedRevision\":1e0", 1)
                    .as_bytes()
            )
            .is_ok()
        );
    }

    #[test]
    fn maximum_scene_fits_its_envelope_and_output_retains_structured_values() {
        let mut scene = serde_json::to_string(
            &serde_json::from_slice::<Value>(include_bytes!(
                "../../../../../../../contracts/office/whiteboard-scene-v1.json"
            ))
            .unwrap(),
        )
        .unwrap();
        scene.pop();
        scene.extend(std::iter::repeat_n(
            ' ',
            crate::office_whiteboard::DOCUMENT_BYTES - scene.len() - 1,
        ));
        scene.push('}');
        let source = format!(
            r#"{{"expectedRevision":0,"operationId":"11111111-1111-4111-8111-111111111111","scene":{scene}}}"#
        );
        assert!(source.len() > crate::office_whiteboard::DOCUMENT_BYTES);
        assert!(decode_save("lobby", source.as_bytes()).is_ok());
        scene.insert(scene.len() - 1, ' ');
        let oversized = format!(
            r#"{{"expectedRevision":0,"operationId":"11111111-1111-4111-8111-111111111111","scene":{scene}}}"#
        );
        assert!(decode_save("lobby", oversized.as_bytes()).is_err());
        let document = crate::office_whiteboard::document::empty_document("lobby");
        let output: Value = serde_json::from_slice(&encode_document(&document).unwrap()).unwrap();
        assert_eq!(
            output,
            json!({"id":"lobby","revision":0,"updatedAtMs":0,"scene":{"formatVersion":1,"width":1600,"height":1000,"background":"#fff7e7","elements":[]}})
        );
    }
}
