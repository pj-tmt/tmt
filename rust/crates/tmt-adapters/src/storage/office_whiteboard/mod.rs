//! Revision-checked documents and replay receipts share one SQLite transaction.

mod snapshot;

use rusqlite::{Connection, OptionalExtension, params};
use tmt_office_model::office_whiteboard::document::{
    DocumentError, SaveDocument, SaveReceipt, WhiteboardDocument, empty_document, plan_save,
    valid_document_id, validate_save,
};

use super::{
    Storage, StorageError, StorageErrorCode, errors::classify,
    identities::with_immediate_transaction,
};
use tmt_core::content_digest::framed_sha256;
use tmt_office_model::codec::office_whiteboard::decode_scene;
use tmt_office_model::codec::office_whiteboard::encode_scene;

#[derive(Debug)]
pub enum WhiteboardStoreError {
    Policy(DocumentError),
    Storage(StorageError),
}

impl WhiteboardStoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Policy(DocumentError::Invalid) => "WHITEBOARD_INVALID",
            Self::Policy(DocumentError::NotFound) => "WHITEBOARD_NOT_FOUND",
            Self::Policy(DocumentError::RevisionConflict) => "WHITEBOARD_REVISION_CONFLICT",
            Self::Policy(DocumentError::RevisionExhausted) => "WHITEBOARD_REVISION_EXHAUSTED",
            Self::Policy(DocumentError::IdempotencyConflict) => "WHITEBOARD_IDEMPOTENCY_CONFLICT",
            Self::Storage(_) => "STORAGE_UNAVAILABLE",
        }
    }
}
impl std::fmt::Display for WhiteboardStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}
impl std::error::Error for WhiteboardStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Policy(_) => None,
        }
    }
}
impl From<StorageError> for WhiteboardStoreError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}
impl From<DocumentError> for WhiteboardStoreError {
    fn from(error: DocumentError) -> Self {
        Self::Policy(error)
    }
}

impl Storage {
    pub fn show_whiteboard(&self, id: &str) -> Result<WhiteboardDocument, WhiteboardStoreError> {
        if !valid_document_id(id) {
            return Err(DocumentError::Invalid.into());
        }
        match read_document(self.connection()?, id)? {
            Some(document) => Ok(document),
            None => Ok(empty_document(id)),
        }
    }

    pub fn save_whiteboard(
        &mut self,
        request: &SaveDocument,
        now_ms: u64,
    ) -> Result<SaveReceipt, WhiteboardStoreError> {
        validate_save(request, now_ms)?;
        let encoded =
            String::from_utf8(encode_scene(&request.scene).map_err(|_| DocumentError::Invalid)?)
                .expect("scene encoder emits UTF-8 JSON");
        // IDs exclude NUL and the revision is decimal: the framed intent is unambiguous.
        let intent = framed_sha256(
            b"tmt:whiteboard:save:v1\0",
            format!(
                "{}\0{}\0{encoded}",
                request.document_id, request.expected_revision
            )
            .as_bytes(),
        );
        with_immediate_transaction(self, "whiteboard save", |transaction| {
            let world_id = super::office_world::ensure_world(transaction, || {
                Ok(i64::try_from(now_ms).expect("validated timestamp"))
            })?;
            let replay = transaction.query_row(
                "SELECT intent_digest, document_id, revision, changed, updated_at_ms FROM office_whiteboard_operations WHERE world_id=? AND operation_id=?",
                params![world_id, request.operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, bool>(3)?, row.get::<_, i64>(4)?)),
            ).optional().map_err(|error| classify(error, "Read whiteboard operation"))?;
            if let Some((digest, document_id, revision, changed, updated_at_ms)) = replay {
                if digest != intent {
                    return Err(DocumentError::IdempotencyConflict.into());
                }
                return Ok(SaveReceipt {
                    document_id,
                    operation_id: request.operation_id.clone(),
                    revision: stored_positive(revision)?,
                    changed,
                    updated_at_ms: stored_positive(updated_at_ms)?,
                });
            }
            let current = read_document(transaction, &request.document_id)?;
            let receipt = plan_save(current.as_ref(), request, now_ms)?;
            if receipt.changed {
                let count = if current.is_some() {
                    transaction.execute(
                        "UPDATE office_whiteboards SET revision=?, scene=?, updated_at_ms=? WHERE document_id=? AND revision=?",
                        params![receipt.revision as i64, encoded, receipt.updated_at_ms as i64, request.document_id, request.expected_revision as i64],
                    )
                } else {
                    transaction.execute(
                        "INSERT INTO office_whiteboards (document_id, world_id, revision, scene, updated_at_ms) VALUES (?, ?, ?, ?, ?)",
                        params![request.document_id, world_id, receipt.revision as i64, encoded, receipt.updated_at_ms as i64],
                    )
                }.map_err(|error| classify(error, "Save whiteboard document"))?;
                if count != 1 {
                    return Err(DocumentError::RevisionConflict.into());
                }
            }
            transaction.execute(
                "INSERT INTO office_whiteboard_operations (world_id, operation_id, intent_digest, document_id, revision, changed, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![world_id, receipt.operation_id, intent, receipt.document_id, receipt.revision as i64, receipt.changed, receipt.updated_at_ms as i64],
            ).map_err(|error| classify(error, "Record whiteboard operation"))?;
            Ok(receipt)
        })
    }
}

fn stored_positive(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0 && *value <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
        .ok_or_else(|| {
            StorageError::new(
                StorageErrorCode::Corrupt,
                "Invalid stored whiteboard revision or timestamp",
            )
        })
}

fn read_document(
    connection: &Connection,
    id: &str,
) -> Result<Option<WhiteboardDocument>, WhiteboardStoreError> {
    let row = connection
        .query_row(
            "SELECT revision, scene, updated_at_ms FROM office_whiteboards WHERE document_id=?",
            [id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| classify(error, "Read whiteboard document"))?;
    row.map(|(revision, scene, updated_at_ms)| {
        Ok(WhiteboardDocument {
            id: id.into(),
            revision: stored_positive(revision)?,
            scene: decode_scene(scene.as_bytes()).map_err(|error| {
                StorageError::new(StorageErrorCode::Corrupt, "Invalid stored whiteboard scene")
                    .caused_by(error)
            })?,
            updated_at_ms: stored_positive(updated_at_ms)?,
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests;
