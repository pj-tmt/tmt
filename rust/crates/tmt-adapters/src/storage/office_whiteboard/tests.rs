use super::*;
use crate::test_support::TestDirectory;
use tmt_office_model::office_whiteboard::document::LOBBY_DOCUMENT;

const OP: &str = "11111111-1111-4111-8111-111111111111";
const OP2: &str = "22222222-2222-4222-8222-222222222222";
const OP3: &str = "33333333-3333-4333-8333-333333333333";

fn request() -> SaveDocument {
    SaveDocument {
        document_id: LOBBY_DOCUMENT.into(),
        expected_revision: 0,
        operation_id: OP.into(),
        scene: tmt_office_model::codec::office_whiteboard::decode_scene(include_bytes!(
            "../../../../../../extensions/tmt-office/contracts/whiteboard-scene-v1.json"
        ))
        .unwrap(),
    }
}

fn counts(storage: &Storage) -> (i64, i64, i64, i64) {
    storage.connection().unwrap().query_row(
        "SELECT (SELECT count(*) FROM office_local_worlds), (SELECT count(*) FROM office_whiteboards), (SELECT count(*) FROM office_whiteboard_operations), (SELECT count(*) FROM request_attempts)", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap()
}

#[test]
fn observation_is_virtual_and_invalid_writes_do_not_create_resources() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("state.db")).unwrap();
    assert_eq!(
        storage.show_whiteboard(LOBBY_DOCUMENT).unwrap(),
        empty_document(LOBBY_DOCUMENT)
    );
    assert_eq!(storage.show_whiteboard(OP).unwrap(), empty_document(OP));
    let mut invalid = request();
    invalid.document_id = "../private".into();
    assert_eq!(
        storage.save_whiteboard(&invalid, 100).unwrap_err().code(),
        "WHITEBOARD_INVALID"
    );
    invalid = request();
    invalid.expected_revision = 1;
    assert_eq!(
        storage.save_whiteboard(&invalid, 100).unwrap_err().code(),
        "WHITEBOARD_REVISION_CONFLICT"
    );
    assert_eq!(counts(&storage), (0, 0, 0, 0));
}

#[test]
fn receipts_replay_original_outcomes_after_newer_edits_and_restart() {
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    let mut storage = Storage::open(&path).unwrap();
    let original = request();
    let first = storage.save_whiteboard(&original, 100).unwrap();
    assert_eq!(
        (first.revision, first.changed, first.updated_at_ms),
        (1, true, 100)
    );
    let mut noop = original.clone();
    noop.expected_revision = 1;
    noop.operation_id = OP2.into();
    let unchanged = storage.save_whiteboard(&noop, 200).unwrap();
    assert_eq!(
        (
            unchanged.revision,
            unchanged.changed,
            unchanged.updated_at_ms
        ),
        (1, false, 100)
    );
    let mut newer = noop.clone();
    newer.operation_id = OP3.into();
    newer.scene.background = "#ffffff".into();
    let second = storage.save_whiteboard(&newer, 300).unwrap();
    assert_eq!(second.revision, 2);
    storage.close().unwrap();

    let mut storage = Storage::open(&path).unwrap();
    assert_eq!(storage.save_whiteboard(&original, 400).unwrap(), first);
    assert_eq!(storage.save_whiteboard(&noop, 400).unwrap(), unchanged);
    assert_eq!(
        storage.show_whiteboard(LOBBY_DOCUMENT).unwrap().scene,
        newer.scene
    );
    let mut reused = original.clone();
    reused.scene.background = "#000000".into();
    assert_eq!(
        storage.save_whiteboard(&reused, 400).unwrap_err().code(),
        "WHITEBOARD_IDEMPOTENCY_CONFLICT"
    );
    reused = original.clone();
    reused.document_id = OP2.into();
    assert_eq!(
        storage.save_whiteboard(&reused, 400).unwrap_err().code(),
        "WHITEBOARD_IDEMPOTENCY_CONFLICT"
    );
    let mut stale = newer.clone();
    stale.operation_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        storage.save_whiteboard(&stale, 400).unwrap_err().code(),
        "WHITEBOARD_REVISION_CONFLICT"
    );
    assert_eq!(counts(&storage), (1, 1, 3, 0));
}

#[test]
fn failed_receipt_insert_rolls_back_document_and_world_then_same_intent_can_retry() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("state.db")).unwrap();
    storage.connection().unwrap().execute_batch(
        "CREATE TRIGGER reject_whiteboard_receipt BEFORE INSERT ON office_whiteboard_operations BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;"
    ).unwrap();
    assert_eq!(
        storage.save_whiteboard(&request(), 100).unwrap_err().code(),
        "STORAGE_UNAVAILABLE"
    );
    assert_eq!(counts(&storage), (0, 0, 0, 0));
    storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_whiteboard_receipt")
        .unwrap();
    assert_eq!(
        storage.save_whiteboard(&request(), 200).unwrap().revision,
        1
    );
    assert_eq!(counts(&storage), (1, 1, 1, 0));
    assert_eq!(
        storage.show_whiteboard(LOBBY_DOCUMENT).unwrap().scene,
        request().scene
    );
}

#[test]
fn concurrent_editors_cannot_overwrite_the_same_revision() {
    let directory = TestDirectory::new();
    let path = directory.path.join("state.db");
    let mut storage = Storage::open(&path).unwrap();
    storage.save_whiteboard(&request(), 100).unwrap();
    storage.close().unwrap();
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let tasks = [(OP2, "#ffffff"), (OP3, "#000000")]
            .into_iter()
            .map(|(operation_id, color)| {
                let path = &path;
                let barrier = &barrier;
                scope.spawn(move || {
                    let mut storage = Storage::open(path).unwrap();
                    let mut request = request();
                    request.expected_revision = 1;
                    request.operation_id = operation_id.into();
                    request.scene.background = color.into();
                    barrier.wait();
                    storage.save_whiteboard(&request, 200)
                })
            })
            .collect::<Vec<_>>();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .next()
            .unwrap()
            .code(),
        "WHITEBOARD_REVISION_CONFLICT"
    );
    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.show_whiteboard(LOBBY_DOCUMENT).unwrap().revision, 2);
    assert_eq!(counts(&storage), (1, 1, 2, 0));
}

#[test]
fn empty_saved_documents_survive_and_exhausted_revisions_do_not_wrap() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("state.db")).unwrap();
    let mut empty = request();
    empty.scene.elements.clear();
    assert!(storage.save_whiteboard(&empty, 100).unwrap().changed);
    assert!(
        storage
            .show_whiteboard(LOBBY_DOCUMENT)
            .unwrap()
            .scene
            .elements
            .is_empty()
    );
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE office_whiteboards SET revision=?",
            [tmt_core::limits::MAX_JS_SAFE_INTEGER as i64],
        )
        .unwrap();
    empty.expected_revision = tmt_core::limits::MAX_JS_SAFE_INTEGER;
    empty.operation_id = OP2.into();
    assert!(!storage.save_whiteboard(&empty, 200).unwrap().changed);
    empty.scene.background = "#000000".into();
    empty.operation_id = OP3.into();
    assert_eq!(
        storage.save_whiteboard(&empty, 300).unwrap_err().code(),
        "WHITEBOARD_REVISION_EXHAUSTED"
    );
    assert_eq!(counts(&storage), (1, 1, 2, 0));
}
