//! Installation-owned documents. Request dispatch and World placement are unrelated.

use super::{Scene, valid_id};
use tmt_core::limits::MAX_JS_SAFE_INTEGER;

pub const LOBBY_DOCUMENT: &str = "lobby";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhiteboardDocument {
    pub id: String,
    pub revision: u64,
    pub scene: Scene,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDocument {
    pub document_id: String,
    pub expected_revision: u64,
    pub operation_id: String,
    pub scene: Scene,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReceipt {
    pub document_id: String,
    pub operation_id: String,
    pub revision: u64,
    pub changed: bool,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentError {
    Invalid,
    NotFound,
    RevisionConflict,
    RevisionExhausted,
    IdempotencyConflict,
}

pub fn valid_document_id(id: &str) -> bool {
    id == LOBBY_DOCUMENT || valid_id(id)
}

/// A virtual blank value for an admitted document ID, not a persistence action.
pub fn empty_document(id: &str) -> WhiteboardDocument {
    WhiteboardDocument {
        id: id.into(),
        revision: 0,
        scene: Scene {
            background: "#fff7e7".into(),
            elements: Vec::new(),
        },
        updated_at_ms: 0,
    }
}

pub fn validate_save(request: &SaveDocument, now_ms: u64) -> Result<(), DocumentError> {
    validate_intent(request)?;
    if now_ms == 0 || now_ms > MAX_JS_SAFE_INTEGER {
        return Err(DocumentError::Invalid);
    }
    Ok(())
}

pub fn validate_intent(request: &SaveDocument) -> Result<(), DocumentError> {
    if !valid_document_id(&request.document_id)
        || !valid_id(&request.operation_id)
        || request.expected_revision > MAX_JS_SAFE_INTEGER
        || request.scene.validate().is_err()
    {
        return Err(DocumentError::Invalid);
    }
    Ok(())
}

/// A receipt replay must be resolved before this policy. Similar content alone
/// never makes a stale write safe; only the original operation can be replayed.
pub fn plan_save(
    current: Option<&WhiteboardDocument>,
    request: &SaveDocument,
    now_ms: u64,
) -> Result<SaveReceipt, DocumentError> {
    validate_save(request, now_ms)?;
    let revision = current.map_or(0, |document| document.revision);
    if current.is_some_and(|document| document.id != request.document_id)
        || revision != request.expected_revision
    {
        return Err(DocumentError::RevisionConflict);
    }
    let changed = current.is_none_or(|document| document.scene != request.scene);
    let next_revision = if changed {
        revision
            .checked_add(1)
            .filter(|value| *value <= MAX_JS_SAFE_INTEGER)
            .ok_or(DocumentError::RevisionExhausted)?
    } else {
        revision
    };
    Ok(SaveReceipt {
        document_id: request.document_id.clone(),
        operation_id: request.operation_id.clone(),
        revision: next_revision,
        changed,
        updated_at_ms: if changed {
            now_ms
        } else {
            current.expect("existing no-op document").updated_at_ms
        },
    })
}
