use super::super::tests::{CAPTURE, MISSING, counts, document, input};
use super::*;
use crate::office_whiteboard::image_tests::test_support;
use crate::test_support::TestDirectory;
use tmt_office_model::codec::office_whiteboard::image::decode_snapshot_image;

fn image(pixel: [u8; 4]) -> ValidatedSnapshotImage {
    decode_snapshot_image(&test_support::solid(pixel)).unwrap()
}

fn image_count(storage: &Storage) -> i64 {
    storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM office_whiteboard_snapshot_images",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn attach_is_immutable_and_replay_survives_live_content_removal_and_reopen() {
    let directory = TestDirectory::new();
    let path = directory.path.join("image.db");
    let mut storage = Storage::open(&path).unwrap();
    storage.save_whiteboard(&document(), 100).unwrap();
    let snapshot = storage.capture_whiteboard(&input(), 200).unwrap();
    let original = image([24, 98, 81, 255]);
    let attached = storage
        .attach_whiteboard_snapshot_image(CAPTURE, &original)
        .unwrap();
    assert_eq!(attached, original);
    assert_eq!(counts(&storage), (1, 1, 1, 0));
    assert_eq!(image_count(&storage), 1);
    let stored: Vec<u8> = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT png FROM office_whiteboard_snapshot_images WHERE snapshot_id=?",
            [CAPTURE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, original.bytes());
    let mut cleared = document();
    cleared.operation_id = MISSING.into();
    cleared.expected_revision = 1;
    cleared.scene.elements.clear();
    storage.save_whiteboard(&cleared, 300).unwrap();
    assert!(
        storage
            .show_whiteboard("lobby")
            .unwrap()
            .scene
            .elements
            .is_empty()
    );
    storage.close().unwrap();
    let mut storage = Storage::open(path).unwrap();
    assert_eq!(storage.show_whiteboard_snapshot(CAPTURE).unwrap(), snapshot);
    assert_eq!(
        storage.show_whiteboard_snapshot_image(CAPTURE).unwrap(),
        attached
    );
    let rgb = [24, 98, 81].repeat(1600 * 1000);
    let equivalent = decode_snapshot_image(&test_support::png(
        1600,
        1000,
        png::ColorType::Rgb,
        &rgb,
        true,
    ))
    .unwrap();
    assert_eq!(
        storage
            .attach_whiteboard_snapshot_image(CAPTURE, &equivalent)
            .unwrap(),
        attached
    );
    assert_eq!(
        storage
            .attach_whiteboard_snapshot_image(CAPTURE, &image([25, 98, 81, 255]))
            .unwrap_err()
            .code(),
        "WHITEBOARD_IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(
        storage.show_whiteboard_snapshot_image(CAPTURE).unwrap(),
        attached
    );
    assert_eq!(image_count(&storage), 1);
    assert_eq!(counts(&storage), (1, 1, 2, 0));
}

#[test]
fn absent_capture_and_failed_insert_leave_no_image_and_retry_uses_same_capture() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("image.db")).unwrap();
    let original = image([24, 98, 81, 255]);
    assert_eq!(
        storage
            .attach_whiteboard_snapshot_image(MISSING, &original)
            .unwrap_err()
            .code(),
        "WHITEBOARD_NOT_FOUND"
    );
    assert_eq!(
        storage
            .show_whiteboard_snapshot_image(MISSING)
            .unwrap_err()
            .code(),
        "WHITEBOARD_NOT_FOUND"
    );
    assert_eq!(
        storage
            .attach_whiteboard_snapshot_image("lobby", &original)
            .unwrap_err()
            .code(),
        "WHITEBOARD_INVALID"
    );
    assert_eq!(
        storage
            .show_whiteboard_snapshot_image("lobby")
            .unwrap_err()
            .code(),
        "WHITEBOARD_INVALID"
    );
    assert_eq!(counts(&storage), (0, 0, 0, 0));
    assert_eq!(image_count(&storage), 0);
    storage.save_whiteboard(&document(), 100).unwrap();
    let snapshot = storage.capture_whiteboard(&input(), 200).unwrap();
    assert_eq!(
        storage
            .show_whiteboard_snapshot_image(CAPTURE)
            .unwrap_err()
            .code(),
        "WHITEBOARD_NOT_FOUND"
    );
    storage.connection().unwrap().execute_batch("CREATE TRIGGER reject_image BEFORE INSERT ON office_whiteboard_snapshot_images BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
    assert!(matches!(
        storage.attach_whiteboard_snapshot_image(CAPTURE, &original),
        Err(WhiteboardStoreError::Storage(_))
    ));
    assert_eq!(image_count(&storage), 0);
    assert_eq!(storage.show_whiteboard_snapshot(CAPTURE).unwrap(), snapshot);
    storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_image")
        .unwrap();
    assert_eq!(
        storage
            .attach_whiteboard_snapshot_image(CAPTURE, &original)
            .unwrap(),
        original
    );
    assert_eq!(image_count(&storage), 1);
}

#[test]
fn corrupt_image_is_not_returned_or_silently_replaced_by_retry() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("image.db")).unwrap();
    storage.save_whiteboard(&document(), 100).unwrap();
    storage.capture_whiteboard(&input(), 200).unwrap();
    let original = image([24, 98, 81, 255]);
    storage
        .attach_whiteboard_snapshot_image(CAPTURE, &original)
        .unwrap();
    for corrupt in [
        b"invalid png".to_vec(),
        image([25, 98, 81, 255]).bytes().to_vec(),
    ] {
        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_whiteboard_snapshot_images SET png=? WHERE snapshot_id=?",
                params![corrupt, CAPTURE],
            )
            .unwrap();
        for result in [
            storage.show_whiteboard_snapshot_image(CAPTURE),
            storage.attach_whiteboard_snapshot_image(CAPTURE, &original),
        ] {
            assert!(
                matches!(result, Err(WhiteboardStoreError::Storage(error)) if error.code == StorageErrorCode::Corrupt)
            );
        }
        let retained: Vec<u8> = storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT png FROM office_whiteboard_snapshot_images WHERE snapshot_id=?",
                [CAPTURE],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, corrupt);
    }
}

#[test]
fn concurrent_different_images_have_one_winner_without_overwriting_it() {
    let directory = TestDirectory::new();
    let path = directory.path.join("image.db");
    let mut storage = Storage::open(&path).unwrap();
    storage.save_whiteboard(&document(), 100).unwrap();
    storage.capture_whiteboard(&input(), 200).unwrap();
    storage.close().unwrap();
    let candidates = [image([24, 98, 81, 255]), image([25, 98, 81, 255])];
    let stores: Vec<_> = (0..2).map(|_| Storage::open(&path).unwrap()).collect();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = stores
        .into_iter()
        .zip(candidates)
        .map(|(mut storage, candidate)| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                storage.attach_whiteboard_snapshot_image(CAPTURE, &candidate)
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap()
            .code(),
        "WHITEBOARD_IDEMPOTENCY_CONFLICT"
    );
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    let storage = Storage::open(path).unwrap();
    assert_eq!(
        storage.show_whiteboard_snapshot_image(CAPTURE).unwrap(),
        winner
    );
    assert_eq!(image_count(&storage), 1);
}
