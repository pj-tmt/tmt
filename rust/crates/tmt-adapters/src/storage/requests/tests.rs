use crate::test_support::TestDirectory;
use tmt_core::{
    endpoint::ServerEvidence,
    limits::MAX_JS_SAFE_INTEGER,
    request::{
        Originator, PrepareRequest, RequestEndpoint, RequestRepository, RequestRoute,
        RequestService,
    },
};

use super::super::*;

fn endpoint() -> RequestEndpoint {
    RequestEndpoint {
        server: ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "server-id".into(),
            socket_path: "/tmp/tmt-request-test.sock".into(),
            server_pid: 41,
            server_start_time: "start".into(),
        },
        pane_id: "%1".into(),
        pane_pid: 42,
    }
}

fn seed_attempt(storage: &mut Storage, request_id: &str, attempt_id: &str) {
    let mut service = RequestService::new(storage, || 1_000);
    service
        .prepare(
            PrepareRequest {
                room_id: None,
                kind: tmt_core::request::RequestKind::Request,
                request_id: request_id.into(),
                message: "original prompt".into(),
                route: RequestRoute::Pane(endpoint()),
                wait: false,
                expires_at_ms: 2_000,
                originator: Originator::Unknown,
                recipient_identity_id: None,
                preamble: None,
            },
            attempt_id.into(),
            7,
        )
        .unwrap();
}

#[test]
fn context_keeps_scrubbed_prompt_expiry_marker() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state").join("requests.db");
    let mut storage = Storage::open(database).unwrap();
    seed_attempt(&mut storage, "request-1", "attempt-1");

    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE request_attempts SET message_text = NULL, message_bytes = NULL WHERE request_id = ?",
            ["request-1"],
        )
        .unwrap();
    let context = storage
        .with_request_transaction(|records| records.find_context("request-1"))
        .unwrap()
        .unwrap();
    assert_eq!(context.message, None);
    assert_eq!(context.message_bytes, None);
    assert!(context.expires_at_ms.is_some());
    storage.close().unwrap();
}

#[test]
fn row_decoder_accepts_safe_maximum_and_rejects_out_of_range_values() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state").join("requests.db");
    let mut storage = Storage::open(database).unwrap();
    seed_attempt(&mut storage, "request-1", "attempt-1");

    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE request_attempts SET prepared_at_ms = ? WHERE request_id = ?",
            rusqlite::params![MAX_JS_SAFE_INTEGER as i64, "request-1"],
        )
        .unwrap();
    assert_eq!(
        storage
            .with_request_transaction(|records| records.find_request("request-1"))
            .unwrap()
            .unwrap()
            .prepared_at_ms,
        MAX_JS_SAFE_INTEGER
    );

    for value in [MAX_JS_SAFE_INTEGER as i64 + 1, -1] {
        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE request_attempts SET prepared_at_ms = ? WHERE request_id = ?",
                rusqlite::params![value, "request-1"],
            )
            .unwrap();
        assert!(
            storage
                .with_request_transaction(|records| records.find_request("request-1"))
                .is_err()
        );
    }
    storage.close().unwrap();
}

#[test]
fn response_marker_mismatch_rolls_back_orphan_final_insert() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state").join("requests.db");
    let mut storage = Storage::open(database).unwrap();
    seed_attempt(&mut storage, "request-1", "attempt-1");

    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE request_attempts SET response_submitted_at_ms = ? WHERE request_id = ?",
            rusqlite::params![1_i64, "request-1"],
        )
        .unwrap();
    let response = tmt_core::request::FinalResponse {
        request_id: "request-1".into(),
        attempt_id: "attempt-1".into(),
        route: RequestRoute::Pane(endpoint()),
        body: "final".into(),
        body_bytes: 5,
        submitted_at_ms: 1_500,
        response_expires_at_ms: 10_000,
    };
    let result: Result<(), StorageError> =
        storage.with_request_transaction(|records| records.create_response(&response));
    assert!(result.is_err());
    let count: i64 = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM request_responses WHERE request_id = ?",
            ["request-1"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let marker: i64 = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT response_submitted_at_ms FROM request_attempts WHERE request_id = ?",
            ["request-1"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(marker, 1);
    storage.close().unwrap();
}

#[test]
fn retained_cleanup_plan_uses_horizon_index_without_sort() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state").join("requests.db");
    let mut storage = Storage::open(database).unwrap();
    let plan_sql = format!("EXPLAIN QUERY PLAN {}", super::DELETE_RETAINED_SQL);
    let details = {
        let mut statement = storage.connection().unwrap().prepare(&plan_sql).unwrap();
        statement
            .query_map(rusqlite::params![1_i64, 2_i64, 2_i64, 100_i64], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("request_attempts_retention_horizon"))
    );
    assert!(
        !details
            .iter()
            .any(|detail| detail.contains("USE TEMP B-TREE FOR ORDER BY"))
    );
    storage.close().unwrap();
}

#[test]
fn incoming_watermark_plan_uses_participant_indexes() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state").join("requests.db");
    let mut storage = Storage::open(database).unwrap();
    for room_id in [None, Some("11111111-1111-4111-8111-111111111111")] {
        let prefix = if room_id.is_some() {
            "request_attempts_room"
        } else {
            "request_attempts"
        };
        let plan_sql = format!(
            "EXPLAIN QUERY PLAN {}",
            super::incoming_watermark_sql(room_id.is_some())
        );
        let details = {
            let mut statement = storage.connection().unwrap().prepare(&plan_sql).unwrap();
            statement
                .query_map(rusqlite::params!["identity-id", 1_i64, room_id], |row| {
                    row.get::<_, String>(3)
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(
            details
                .iter()
                .any(|detail| detail.contains(&format!("{prefix}_recipient_attention")))
        );
        assert!(
            details
                .iter()
                .any(|detail| detail.contains(&format!("{prefix}_response_attention")))
        );
        assert!(
            !details
                .iter()
                .any(|detail| detail == "SCAN request_attempts")
        );
        assert!(
            !details
                .iter()
                .any(|detail| detail.contains("USE TEMP B-TREE FOR ORDER BY"))
        );
    }
    storage.close().unwrap();
}

#[test]
fn request_history_plans_seek_the_scope_and_cursor_without_sorting() {
    use tmt_core::request::history::HistoryScope;
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("history.db")).unwrap();
    let recipient = "11111111-1111-4111-8111-111111111111";
    let room = "22222222-2222-4222-8222-222222222222";
    for (scope, index, identity, room_id) in [
        (
            HistoryScope::Recipient {
                identity_id: recipient.into(),
                room_id: None,
            },
            "request_history_recipient",
            Some(recipient),
            None,
        ),
        (
            HistoryScope::Recipient {
                identity_id: recipient.into(),
                room_id: Some(room.into()),
            },
            "request_history_recipient_room",
            Some(recipient),
            Some(room),
        ),
        (
            HistoryScope::OriginatorResults(recipient.into()),
            "request_history_originator_results",
            Some(recipient),
            None,
        ),
        (
            HistoryScope::Room(room.into()),
            "request_history_room",
            None,
            Some(room),
        ),
    ] {
        for cursor in [false, true] {
            let sql = format!(
                "EXPLAIN QUERY PLAN {}",
                super::history::history_query(&scope, cursor)
            );
            let mut statement = storage.connection().unwrap().prepare(&sql).unwrap();
            let plan = statement
                .query_map(
                    rusqlite::params![
                        identity,
                        room_id,
                        cursor.then_some(2000_i64),
                        cursor.then_some("request-z"),
                        21_i64,
                        1000_i64
                    ],
                    |row| row.get::<_, String>(3),
                )
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert!(
                plan.iter()
                    .any(|line| line.contains(&format!("SEARCH a USING INDEX {index} ("))),
                "{plan:?}"
            );
            assert!(
                !plan
                    .iter()
                    .any(|line| line.contains("TEMP B-TREE") || line.starts_with("SCAN a")),
                "{plan:?}"
            );
            if cursor {
                assert!(
                    plan.iter().any(|line| line.contains(
                        if matches!(scope, HistoryScope::OriginatorResults(_)) {
                            "response_submitted_at_ms"
                        } else {
                            "prepared_at_ms"
                        }
                    )),
                    "Cursor must seek the indexed range: {plan:?}"
                );
            }
        }
    }
    storage.close().unwrap();
}

#[test]
fn request_fences_record_their_host_and_read_legacy_rows_as_tmux() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state").join("requests.db");
    let mut storage = Storage::open(database).unwrap();
    seed_attempt(&mut storage, "request-1", "attempt-1");
    let connection = storage.connection().unwrap();
    let stored: Option<String> = connection
        .query_row(
            "SELECT host FROM request_attempts WHERE request_id = 'request-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored.as_deref(), Some("tmux"));
    // A fence written before schema 39 has no host: it is tmux.
    connection
        .execute("UPDATE request_attempts SET host = NULL", [])
        .unwrap();
    let attempt = storage
        .with_request_transaction(|records| records.find_request("request-1"))
        .unwrap()
        .unwrap();
    assert_eq!(attempt.route, RequestRoute::Pane(endpoint()));
    storage.close().unwrap();
}

#[test]
fn an_active_legacy_fence_still_matches_its_tmux_endpoint() {
    let directory = TestDirectory::new();
    let database = directory.path.join("state").join("requests.db");
    let mut storage = Storage::open(database).unwrap();
    seed_attempt(&mut storage, "request-1", "attempt-1");
    let connection = storage.connection().unwrap();
    connection
        .execute(
            "UPDATE request_attempts SET wait_active = 1, host = NULL",
            [],
        )
        .unwrap();
    let active = storage
        .with_request_transaction(|records| {
            records.find_active_request(&RequestRoute::Pane(endpoint()))
        })
        .unwrap();
    assert_eq!(active.as_deref(), Some("request-1"));
    storage.close().unwrap();
}

#[test]
fn retained_request_id_sample_and_overflow_count_use_indexed_ranges() {
    let directory = TestDirectory::new();
    let mut storage = Storage::open(directory.path.join("requests.db")).unwrap();
    for sql in [
        super::RETAINED_REQUEST_IDS_SQL,
        super::RETAINED_REQUEST_IDS_COUNT_SQL,
    ] {
        let plan = format!("EXPLAIN QUERY PLAN {sql}");
        let connection = storage.connection().unwrap();
        let mut statement = connection.prepare(&plan).unwrap();
        let mut bindings = vec![
            rusqlite::types::Value::from("req_12345678".to_owned()),
            rusqlite::types::Value::from("req_12345679".to_owned()),
            1000_i64.into(),
        ];
        if sql == super::RETAINED_REQUEST_IDS_SQL {
            bindings.push(6_i64.into());
        }
        let details = statement
            .query_map(rusqlite::params_from_iter(bindings), |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(
            details.iter().any(
                |detail| detail.contains("SEARCH request_attempts USING INDEX")
                    && detail.contains("request_id>? AND request_id<?")
            ),
            "{details:?}"
        );
        assert!(
            !details
                .iter()
                .any(|detail| detail.contains("SCAN request_attempts")
                    || detail.contains("TEMP B-TREE")),
            "{details:?}"
        );
    }
    storage.close().unwrap();
}
