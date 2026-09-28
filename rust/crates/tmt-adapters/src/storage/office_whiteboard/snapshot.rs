//! Append-only captures, with the capture operation itself serving as replay receipt.

mod image;

use super::{
    DocumentError, Storage, StorageError, StorageErrorCode, WhiteboardStoreError, classify,
    decode_scene, encode_scene, framed_sha256, params, read_document, stored_positive,
    with_immediate_transaction,
};
use rusqlite::{Connection, OptionalExtension};
use tmt_core::limits::MAX_JS_SAFE_INTEGER;
use tmt_office_model::office_whiteboard::snapshot::CaptureWhiteboard;
use tmt_office_model::office_whiteboard::snapshot::WhiteboardSnapshot;
use tmt_office_model::office_whiteboard::snapshot::capture_document;
use tmt_office_model::office_whiteboard::snapshot::valid_snapshot_id;
use tmt_office_model::office_whiteboard::snapshot::validate_capture;
use tmt_office_model::office_whiteboard::snapshot::validate_snapshot;

impl Storage {
    pub fn capture_whiteboard(
        &mut self,
        input: &CaptureWhiteboard,
        now_ms: u64,
    ) -> Result<WhiteboardSnapshot, WhiteboardStoreError> {
        validate_capture(input)?;
        if !(1..=MAX_JS_SAFE_INTEGER).contains(&now_ms) {
            return Err(DocumentError::Invalid.into());
        }
        let mut selected = input.selected_element_ids.clone();
        selected.sort_unstable();
        let intent = framed_sha256(
            b"tmt:whiteboard:capture:v1\0",
            &serde_json::to_vec(&(
                &input.document_id,
                input.expected_revision,
                selected,
                &input.annotation,
            ))
            .expect("validated capture metadata serializes"),
        );
        with_immediate_transaction(self, "whiteboard capture", |transaction| {
            if let Some((digest, snapshot)) = read_snapshot(transaction, &input.operation_id)? {
                return if digest == intent {
                    Ok(snapshot)
                } else {
                    Err(DocumentError::IdempotencyConflict.into())
                };
            }
            let document = read_document(transaction, &input.document_id)?;
            let snapshot = capture_document(document.as_ref(), input, now_ms)?;
            let scene = String::from_utf8(
                encode_scene(&snapshot.scene).map_err(|_| DocumentError::Invalid)?,
            )
            .expect("scene encoder emits UTF-8 JSON");
            let selected = serde_json::to_string(&snapshot.selected_element_ids)
                .expect("validated selection serializes");
            // The saved document already owns a world. Capture never creates one or saves a draft.
            let count = transaction.execute(
                "INSERT INTO office_whiteboard_snapshots (snapshot_id, world_id, intent_digest, document_id, document_revision, scene, selected_element_ids, annotation, created_at_ms) SELECT ?, world_id, ?, document_id, ?, ?, ?, ?, ? FROM office_whiteboards WHERE document_id=? AND revision=?",
                params![snapshot.id, intent, snapshot.document_revision as i64, scene, selected, snapshot.annotation, now_ms as i64, input.document_id, input.expected_revision as i64],
            ).map_err(|error| classify(error, "Capture whiteboard snapshot"))?;
            if count != 1 {
                return Err(DocumentError::RevisionConflict.into());
            }
            Ok(snapshot)
        })
    }

    pub fn show_whiteboard_snapshot(
        &self,
        id: &str,
    ) -> Result<WhiteboardSnapshot, WhiteboardStoreError> {
        if !valid_snapshot_id(id) {
            return Err(DocumentError::Invalid.into());
        }
        read_snapshot(self.connection()?, id)?
            .map(|(_, snapshot)| snapshot)
            .ok_or_else(|| DocumentError::NotFound.into())
    }
}

fn read_snapshot(
    connection: &Connection,
    id: &str,
) -> Result<Option<(String, WhiteboardSnapshot)>, WhiteboardStoreError> {
    let row = connection.query_row(
        "SELECT intent_digest, document_id, document_revision, scene, selected_element_ids, annotation, created_at_ms FROM office_whiteboard_snapshots WHERE snapshot_id=?",
        [id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, i64>(6)?)),
    ).optional().map_err(|error| classify(error, "Read whiteboard snapshot"))?;
    row.map(
        |(digest, document_id, revision, scene, selected, annotation, created_at_ms)| {
            let corrupt = || {
                StorageError::new(
                    StorageErrorCode::Corrupt,
                    "Invalid stored whiteboard snapshot",
                )
            };
            let snapshot = WhiteboardSnapshot {
                id: id.into(),
                document_id,
                document_revision: stored_positive(revision)?,
                scene: decode_scene(scene.as_bytes()).map_err(|_| corrupt())?,
                selected_element_ids: serde_json::from_str(&selected).map_err(|_| corrupt())?,
                annotation,
                created_at_ms: stored_positive(created_at_ms)?,
            };
            validate_snapshot(&snapshot).map_err(|_| corrupt())?;
            Ok((digest, snapshot))
        },
    )
    .transpose()
}

#[cfg(test)]
mod tests;
