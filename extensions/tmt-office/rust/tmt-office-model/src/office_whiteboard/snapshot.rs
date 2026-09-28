//! Immutable captures of saved documents. Capturing is not request dispatch.

use std::collections::HashSet;

use super::{
    ELEMENT_LIMIT, Scene,
    document::{DocumentError, WhiteboardDocument, valid_document_id},
    valid_id, valid_text,
};
use tmt_core::limits::MAX_JS_SAFE_INTEGER;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureWhiteboard {
    pub document_id: String,
    pub expected_revision: u64,
    /// The operation ID is also the immutable snapshot ID; retries create no second resource.
    pub operation_id: String,
    /// An empty selection means the whole scene; otherwise IDs form an unordered set.
    pub selected_element_ids: Vec<String>,
    pub annotation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhiteboardSnapshot {
    pub id: String,
    pub document_id: String,
    pub document_revision: u64,
    pub scene: Scene,
    pub selected_element_ids: Vec<String>,
    pub annotation: String,
    pub created_at_ms: u64,
}

pub fn validate_capture(input: &CaptureWhiteboard) -> Result<(), DocumentError> {
    if !valid_document_id(&input.document_id)
        || !valid_id(&input.operation_id)
        || !(1..=MAX_JS_SAFE_INTEGER).contains(&input.expected_revision)
        || input.selected_element_ids.len() > ELEMENT_LIMIT
        || !valid_text(&input.annotation)
    {
        return Err(DocumentError::Invalid);
    }
    let mut seen = HashSet::new();
    for id in &input.selected_element_ids {
        if !valid_id(id) || !seen.insert(id) {
            return Err(DocumentError::Invalid);
        }
    }
    Ok(())
}

/// Only called after resolving a stored replay. Never replace a requested old revision
/// with the latest document, and never implicitly save an unsaved draft.
pub fn capture_document(
    document: Option<&WhiteboardDocument>,
    input: &CaptureWhiteboard,
    now_ms: u64,
) -> Result<WhiteboardSnapshot, DocumentError> {
    validate_capture(input)?;
    let document = document.ok_or(DocumentError::NotFound)?;
    if document.id != input.document_id || document.revision != input.expected_revision {
        return Err(DocumentError::RevisionConflict);
    }
    let mut selected_element_ids = input.selected_element_ids.clone();
    selected_element_ids.sort_unstable();
    let snapshot = WhiteboardSnapshot {
        id: input.operation_id.clone(),
        document_id: input.document_id.clone(),
        document_revision: document.revision,
        scene: document.scene.clone(),
        selected_element_ids,
        annotation: input.annotation.clone(),
        created_at_ms: now_ms,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn validate_snapshot(snapshot: &WhiteboardSnapshot) -> Result<(), DocumentError> {
    validate_capture(&CaptureWhiteboard {
        document_id: snapshot.document_id.clone(),
        expected_revision: snapshot.document_revision,
        operation_id: snapshot.id.clone(),
        selected_element_ids: snapshot.selected_element_ids.clone(),
        annotation: snapshot.annotation.clone(),
    })?;
    if !(1..=MAX_JS_SAFE_INTEGER).contains(&snapshot.created_at_ms)
        || snapshot.scene.validate().is_err()
    {
        return Err(DocumentError::Invalid);
    }
    let elements: HashSet<_> = snapshot
        .scene
        .elements
        .iter()
        .map(|element| &element.id)
        .collect();
    if snapshot
        .selected_element_ids
        .iter()
        .any(|id| !elements.contains(id))
    {
        return Err(DocumentError::Invalid);
    }
    Ok(())
}

pub fn valid_snapshot_id(id: &str) -> bool {
    valid_id(id)
}

pub const SNAPSHOT_REFERENCE_PREFIX: &str = "tmt:whiteboard:snapshot:";

/// A local resource locator, never a URL, credential or latest-revision alias.
pub fn resolve_snapshot_reference(value: &str) -> Option<&str> {
    let id = value
        .strip_prefix(SNAPSHOT_REFERENCE_PREFIX)
        .unwrap_or(value);
    valid_snapshot_id(id).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office_whiteboard::{Bounds, Content, Element, document::empty_document};

    const OP: &str = "11111111-1111-4111-8111-111111111111";
    const ELEMENT: &str = "22222222-2222-4222-8222-222222222222";

    fn fixture() -> (WhiteboardDocument, CaptureWhiteboard) {
        let mut document = empty_document("lobby");
        document.revision = 1;
        document.updated_at_ms = 100;
        document.scene.elements.push(Element {
            id: ELEMENT.into(),
            content: Content::Text {
                note: true,
                bounds: Bounds {
                    x: 20,
                    y: 30,
                    width: 200,
                    height: 100,
                },
                text: "Read this design".into(),
                color: "#123456".into(),
                fill: "#fff7e7".into(),
                font_size: 24,
            },
        });
        let capture = CaptureWhiteboard {
            document_id: "lobby".into(),
            expected_revision: 1,
            operation_id: OP.into(),
            selected_element_ids: vec![ELEMENT.into()],
            annotation: "Review the selected note.\nKeep its original context.".into(),
        };
        (document, capture)
    }

    #[test]
    fn capture_owns_original_content_selection_and_annotation() {
        let (mut document, input) = fixture();
        let snapshot = capture_document(Some(&document), &input, 200).unwrap();
        document.revision = 2;
        document.scene.elements.clear();
        assert_eq!(snapshot.document_revision, 1);
        assert_eq!(snapshot.scene.elements.len(), 1);
        assert_eq!(snapshot.selected_element_ids, input.selected_element_ids);
        assert_eq!(snapshot.annotation, input.annotation);
        assert_eq!(snapshot.id, OP);
        assert_eq!(validate_snapshot(&snapshot), Ok(()));
        let all = CaptureWhiteboard {
            selected_element_ids: vec![],
            expected_revision: 2,
            ..input
        };
        assert!(
            capture_document(Some(&document), &all, 201)
                .unwrap()
                .selected_element_ids
                .is_empty()
        );
    }

    #[test]
    fn missing_stale_unsaved_and_unknown_selections_fail_without_fallback() {
        let (document, mut input) = fixture();
        assert_eq!(
            capture_document(None, &input, 200),
            Err(DocumentError::NotFound)
        );
        input.expected_revision = 2;
        assert_eq!(
            capture_document(Some(&document), &input, 200),
            Err(DocumentError::RevisionConflict)
        );
        input.expected_revision = 0;
        assert_eq!(validate_capture(&input), Err(DocumentError::Invalid));
        input.expected_revision = 1;
        input.selected_element_ids = vec![OP.into()];
        assert_eq!(
            capture_document(Some(&document), &input, 200),
            Err(DocumentError::Invalid)
        );
        input.selected_element_ids = vec![ELEMENT.into(), ELEMENT.into()];
        assert_eq!(validate_capture(&input), Err(DocumentError::Invalid));
    }

    #[test]
    fn annotations_and_ids_use_the_existing_inert_text_and_uuid_boundaries() {
        let (document, mut input) = fixture();
        input.annotation = "a".repeat(super::super::TEXT_BYTES);
        assert!(capture_document(Some(&document), &input, 200).is_ok());
        input.annotation.push('a');
        assert_eq!(validate_capture(&input), Err(DocumentError::Invalid));
        input.annotation = "\0".into();
        assert_eq!(validate_capture(&input), Err(DocumentError::Invalid));
        input.annotation = "<script>not executable</script>\n\t".into();
        assert!(capture_document(Some(&document), &input, 200).is_ok());
        assert_eq!(
            capture_document(Some(&document), &input, 0),
            Err(DocumentError::Invalid)
        );
        assert_eq!(
            capture_document(Some(&document), &input, MAX_JS_SAFE_INTEGER + 1),
            Err(DocumentError::Invalid)
        );
        assert!(!valid_snapshot_id("lobby"));
        assert!(!valid_snapshot_id("../private"));
    }
}
