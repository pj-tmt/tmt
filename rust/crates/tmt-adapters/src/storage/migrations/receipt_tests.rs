//! Frozen schema-8 fixtures plus an independently TS-generated v1 envelope.
//! These prove native service handoff, not live mixed-writer/public-CLI support.
use super::test_support::seed_history;
use crate::{reply_receipt::decode_reply_receipt, storage::Storage, test_support::TestDirectory};
use rusqlite::{Connection, params};
use tmt_core::request::{RequestError, RequestService, ResponseRejection, SubmitResponse};

const NOW: i64 = 1_700_000_000_000;
const DAY: i64 = 86_400_000;
// Produced by src/reply-receipt.ts encodeReplyReceipt at main 4478e36.
const V1: &str = "eyJ2ZXJzaW9uIjoxLCJyZXF1ZXN0SWQiOiJsZWdhY3kiLCJhdHRlbXB0SWQiOiJsZWdhY3ktYXR0ZW1wdCIsImVuZHBvaW50Ijp7InNlcnZlcklkIjoic2VydmVyLWZvci1yZXF1ZXN0LXRlc3RzIiwic29ja2V0UGF0aCI6Ii90bXAvdG10LXJlcXVlc3QtdGVzdHMuc29jayIsInNlcnZlclBpZCI6NDEsInNlcnZlclN0YXJ0VGltZSI6InJlcXVlc3QtdGVzdC1zZXJ2ZXItc3RhcnQiLCJwYW5lSWQiOiIlOTAiLCJwYW5lUGlkIjoxOTB9fQ";

fn submission(body: &str) -> SubmitResponse {
    SubmitResponse {
        request_id: "legacy".into(),
        proof: decode_reply_receipt(V1, "legacy").unwrap(),
        body: body.into(),
    }
}

#[test]
fn v1_executes_after_migration_for_every_eligible_inflight_state() {
    for status in ["sending", "sent", "uncertain"] {
        let directory = TestDirectory::new();
        let path = directory.path.join("legacy.db");
        let mut old = Connection::open(&path).unwrap();
        seed_history(&mut old);
        old.execute(
            "INSERT INTO request_attempts (attempt_id, request_id, nonce, server_id,
              socket_path, server_pid, server_start_time, pane_id, pane_pid,
              wait_active, status, inject_preamble, cadence_reserved, prepared_at_ms,
              expires_at_ms, retention_expires_at_ms, originator_kind, originator_identity_id,
              recipient_identity_id, attention_revision)
             VALUES ('legacy-attempt', 'legacy', 'historical-marker', 'server-for-request-tests',
              '/tmp/tmt-request-tests.sock', 41, 'request-test-server-start', '%90', 190,
              1, ?, 0, 0, ?, ?, ?, 'explicit', 'old-id', 'old-id', 9)",
            params![status, NOW, NOW + 3_600_000, NOW + 7 * DAY],
        )
        .unwrap();
        old.close().unwrap();
        let mut native = Storage::open(&path).unwrap();
        assert_eq!(native.health().unwrap().schema_version, 33);
        let final_response = RequestService::new(&mut native, || (NOW + 2 * DAY) as u64)
            .submit_response(submission("\u{feff}late\0\r\n"))
            .unwrap();
        assert_eq!(final_response.body, "\u{feff}late\0\r\n");
        assert_eq!(
            final_response.response_expires_at_ms,
            (NOW + 9 * DAY) as u64
        );
        native.close().unwrap();
        let oracle =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let row: (String, String, i64, i64, i64, i64) = oracle
            .query_row(
                "SELECT a.status, a.nonce, a.response_submitted_at_ms, a.retention_expires_at_ms,
              s.latest_revision, s.acknowledged_through FROM request_attempts a
              JOIN request_attention_identities s ON s.identity_id = a.originator_identity_id",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (
                status.into(),
                "historical-marker".into(),
                NOW + 2 * DAY,
                NOW + 9 * DAY,
                10,
                4
            )
        );
    }
}

#[test]
fn v1_retained_orphan_final_survives_upgrade_restart_and_stops_at_expiry() {
    let directory = TestDirectory::new();
    let path = directory.path.join("legacy-final.db");
    let mut old = Connection::open(&path).unwrap();
    seed_history(&mut old);
    old.execute(
        "INSERT INTO request_responses VALUES ('legacy', 'legacy-attempt',
          'server-for-request-tests', '/tmp/tmt-request-tests.sock', 41,
          'request-test-server-start', '%90', 190, 'before upgrade', 14, ?, ?)",
        params![NOW + 200, NOW + 7 * DAY + 200],
    )
    .unwrap();
    old.close().unwrap();
    let mut native = Storage::open(&path).unwrap();
    let first = RequestService::new(&mut native, || (NOW + DAY) as u64)
        .submit_response(submission("before upgrade"))
        .unwrap();
    assert_eq!(first.submitted_at_ms, (NOW + 200) as u64);
    native.close().unwrap();
    let mut native = Storage::open(&path).unwrap();
    assert_eq!(
        RequestService::new(&mut native, || (NOW + 2 * DAY) as u64)
            .submit_response(submission("before upgrade"))
            .unwrap(),
        first
    );
    assert!(matches!(
        RequestService::new(&mut native, || (NOW + 2 * DAY) as u64)
            .submit_response(submission("conflict")),
        Err(RequestError::Response(ResponseRejection::Conflict))
    ));
    assert!(matches!(
        RequestService::new(&mut native, || first.response_expires_at_ms)
            .submit_response(submission("before upgrade")),
        Err(RequestError::Response(ResponseRejection::Expired))
    ));
    native.close().unwrap();
    let oracle =
        Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let row: (String, i64, i64) = oracle
        .query_row(
            "SELECT body, submitted_at_ms, response_expires_at_ms FROM request_responses",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        ("before upgrade".into(), NOW + 200, NOW + 7 * DAY + 200)
    );
    let attempts: i64 = oracle
        .query_row("SELECT COUNT(*) FROM request_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(attempts, 0);
}
