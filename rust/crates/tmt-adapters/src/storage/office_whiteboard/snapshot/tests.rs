use super::*;
use crate::test_support::TestDirectory;
use tmt_office_model::office_whiteboard::document::SaveDocument;

const SAVE: &str = "11111111-1111-4111-8111-111111111111";
pub(super) const CAPTURE: &str = "22222222-2222-4222-8222-222222222222";
const NEXT_SAVE: &str = "33333333-3333-4333-8333-333333333333";
pub(super) const MISSING: &str = "77777777-7777-4777-8777-777777777777";

pub(super) fn document() -> SaveDocument {
    SaveDocument {
        document_id: "lobby".into(),
        expected_revision: 0,
        operation_id: SAVE.into(),
        scene: decode_scene(include_bytes!(
            "../../../../../../../contracts/office/whiteboard-scene-v1.json"
        ))
        .unwrap(),
    }
}

pub(super) fn input() -> CaptureWhiteboard {
    CaptureWhiteboard {
        document_id: "lobby".into(),
        expected_revision: 1,
        operation_id: CAPTURE.into(),
        selected_element_ids: document()
            .scene
            .elements
            .iter()
            .take(2)
            .map(|item| item.id.clone())
            .collect(),
        annotation: "Please review these elements.\n<script>Inert text.</script>".into(),
    }
}

pub(super) fn counts(storage: &Storage) -> (i64, i64, i64, i64) {
    storage.connection().unwrap().query_row(
        "SELECT (SELECT count(*) FROM office_local_worlds), (SELECT count(*) FROM office_whiteboard_snapshots), (SELECT count(*) FROM office_whiteboard_operations), (SELECT count(*) FROM request_attempts)", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap()
}

#[test]
fn snapshot_retains_original_content_and_replays_after_live_edits_and_reopen() {
    let directory = TestDirectory::new();
    let path = directory.path.join("snapshot.db");
    let mut storage = Storage::open(&path).unwrap();
    let mut save = document();
    storage.save_whiteboard(&save, 100).unwrap();
    let request = input();
    assert_eq!(request.selected_element_ids.len(), 2);
    let snapshot = storage.capture_whiteboard(&request, 200).unwrap();
    assert_eq!(snapshot.scene, save.scene);
    assert_eq!(snapshot.annotation, request.annotation);
    assert_eq!(snapshot.document_revision, 1);
    assert_eq!(snapshot.created_at_ms, 200);
    assert_eq!(counts(&storage), (1, 1, 1, 0));
    let mut equivalent = request.clone();
    equivalent.selected_element_ids.reverse();
    assert_eq!(
        storage.capture_whiteboard(&equivalent, 201).unwrap(),
        snapshot
    );

    save.operation_id = NEXT_SAVE.into();
    save.expected_revision = 1;
    save.scene.elements.clear();
    storage.save_whiteboard(&save, 300).unwrap();
    assert!(
        storage
            .show_whiteboard("lobby")
            .unwrap()
            .scene
            .elements
            .is_empty()
    );
    storage.close().unwrap();
    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.show_whiteboard_snapshot(CAPTURE).unwrap(), snapshot);
    assert_eq!(storage.capture_whiteboard(&request, 400).unwrap(), snapshot);
    assert_eq!(counts(&storage), (1, 1, 2, 0));
    let output: serde_json::Value = serde_json::from_slice(
        &tmt_office_model::codec::office_whiteboard::snapshot::encode_snapshot(&snapshot).unwrap(),
    )
    .unwrap();
    assert_eq!(output["documentRevision"], 1);
    assert_eq!(output["annotation"], request.annotation);
    assert_eq!(
        output["scene"]["elements"].as_array().unwrap().len(),
        document().scene.elements.len()
    );

    for conflicting in [
        CaptureWhiteboard {
            annotation: "Changed request".into(),
            ..request.clone()
        },
        CaptureWhiteboard {
            expected_revision: 2,
            ..request.clone()
        },
        CaptureWhiteboard {
            selected_element_ids: vec![],
            ..request.clone()
        },
        CaptureWhiteboard {
            document_id: MISSING.into(),
            ..request.clone()
        },
    ] {
        assert_eq!(
            storage
                .capture_whiteboard(&conflicting, 500)
                .unwrap_err()
                .code(),
            "WHITEBOARD_IDEMPOTENCY_CONFLICT"
        );
    }
    assert_eq!(storage.show_whiteboard_snapshot(CAPTURE).unwrap(), snapshot);
}

#[test]
fn capture_never_creates_a_world_saves_a_draft_or_falls_back_to_latest() {
    assert!(
        document()
            .scene
            .elements
            .iter()
            .all(|element| element.id != MISSING)
    );
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("snapshot.db")).unwrap();
    assert_eq!(
        storage
            .capture_whiteboard(&input(), 100)
            .unwrap_err()
            .code(),
        "WHITEBOARD_NOT_FOUND"
    );
    assert_eq!(
        storage
            .show_whiteboard_snapshot(CAPTURE)
            .unwrap_err()
            .code(),
        "WHITEBOARD_NOT_FOUND"
    );
    assert_eq!(counts(&storage), (0, 0, 0, 0));
    storage.save_whiteboard(&document(), 100).unwrap();
    for (invalid, code) in [
        (
            CaptureWhiteboard {
                expected_revision: 0,
                ..input()
            },
            "WHITEBOARD_INVALID",
        ),
        (
            CaptureWhiteboard {
                expected_revision: 2,
                ..input()
            },
            "WHITEBOARD_REVISION_CONFLICT",
        ),
        (
            CaptureWhiteboard {
                selected_element_ids: vec![MISSING.into()],
                ..input()
            },
            "WHITEBOARD_INVALID",
        ),
    ] {
        assert_eq!(
            storage
                .capture_whiteboard(&invalid, 200)
                .unwrap_err()
                .code(),
            code
        );
    }
    assert_eq!(counts(&storage), (1, 0, 1, 0));
    assert_eq!(
        storage.show_whiteboard("lobby").unwrap().scene,
        document().scene
    );
    assert_eq!(
        storage
            .show_whiteboard_snapshot(MISSING)
            .unwrap_err()
            .code(),
        "WHITEBOARD_NOT_FOUND"
    );
}

#[test]
fn failed_insert_is_retryable_and_malformed_retained_content_is_not_returned() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("snapshot.db")).unwrap();
    storage.save_whiteboard(&document(), 100).unwrap();
    storage.connection().unwrap().execute_batch("CREATE TRIGGER reject_capture BEFORE INSERT ON office_whiteboard_snapshots BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    assert!(matches!(
        storage.capture_whiteboard(&input(), 200),
        Err(WhiteboardStoreError::Storage(_))
    ));
    assert_eq!(counts(&storage), (1, 0, 1, 0));
    assert_eq!(storage.show_whiteboard("lobby").unwrap().revision, 1);
    storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_capture")
        .unwrap();
    storage.capture_whiteboard(&input(), 201).unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE office_whiteboard_snapshots SET selected_element_ids=? WHERE snapshot_id=?",
            params![format!("[\"{MISSING}\"]"), CAPTURE],
        )
        .unwrap();
    assert!(
        matches!(storage.show_whiteboard_snapshot(CAPTURE), Err(WhiteboardStoreError::Storage(error)) if error.code == StorageErrorCode::Corrupt)
    );
    assert!(matches!(
        storage.capture_whiteboard(&input(), 202),
        Err(WhiteboardStoreError::Storage(_))
    ));
}

#[test]
fn concurrent_retries_share_one_immutable_resource() {
    let directory = TestDirectory::new();
    let path = directory.path.join("snapshot.db");
    let mut storage = Storage::open(&path).unwrap();
    storage.save_whiteboard(&document(), 100).unwrap();
    storage.close().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    // Open before either worker enters the barrier; an open failure must not strand its peer.
    let stores: Vec<_> = (0..2).map(|_| Storage::open(&path).unwrap()).collect();
    let handles: Vec<_> = stores
        .into_iter()
        .enumerate()
        .map(|(index, mut storage)| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                storage
                    .capture_whiteboard(&input(), 200 + index as u64)
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results[0], results[1]);
    let storage = Storage::open(path).unwrap();
    assert_eq!(counts(&storage), (1, 1, 1, 0));
}
