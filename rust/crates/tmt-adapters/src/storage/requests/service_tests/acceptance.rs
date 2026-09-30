//! Cross-operation retention and ownership assertions use independent SQL.

use super::support::{
    DAY_MS, Fixture, NOW_MS, count_rows, endpoint, preamble_count, prepare_input, request_snapshot,
    service,
};
use crate::storage::Storage;
use tmt_core::request::{
    Originator, RequestError, RequestPrompt, RequestService, ResponseLookup, ResponseProof,
    ResponseRejection, Settlement, SubmitResponse,
};

#[test]
fn retained_final_survives_restart_and_missing_attempt_without_renewing_retry_deadline() {
    let mut fixture = Fixture::new();
    let target = endpoint("%70", 170);
    let input = prepare_input(
        &fixture,
        "orphan",
        target.clone(),
        false,
        NOW_MS + 1,
        Originator::Unknown,
        false,
    );
    let prepared = service(&mut fixture)
        .prepare(input, "orphan-attempt".into(), 1)
        .unwrap();
    service(&mut fixture)
        .begin_send(&prepared.attempt_id)
        .unwrap();
    let submission = || SubmitResponse {
        request_id: prepared.request_id.clone(),
        proof: ResponseProof::Recorded {
            attempt_id: prepared.attempt_id.clone(),
            endpoint: target.clone(),
        },
        body: "\u{feff}\0 exact\r\n".into(),
    };
    let first = service(&mut fixture).submit_response(submission()).unwrap();
    fixture.storage.close().unwrap();
    let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
    assert_eq!(
        oracle
            .execute(
                "DELETE FROM request_attempts WHERE request_id = 'orphan'",
                []
            )
            .unwrap(),
        1
    );
    drop(oracle);
    fixture.storage = Storage::open(&fixture.database).unwrap();
    fixture.set_now(NOW_MS + 42_000);
    assert_eq!(
        service(&mut fixture).submit_response(submission()).unwrap(),
        first
    );
    assert_eq!(
        service(&mut fixture).get_response("orphan").unwrap(),
        ResponseLookup::Available(Box::new(first.clone()))
    );
    assert_eq!(count_rows(&fixture.database, "request_attempts"), 0);
    let oracle = rusqlite::Connection::open_with_flags(
        &fixture.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let stored: (String, i64, i64) = oracle.query_row(
        "SELECT body, submitted_at_ms, response_expires_at_ms FROM request_responses WHERE request_id = 'orphan'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(
        stored,
        (first.body, NOW_MS as i64, (NOW_MS + DAY_MS) as i64)
    );
    drop(oracle);
    fixture.set_now(NOW_MS + DAY_MS);
    assert!(matches!(
        service(&mut fixture).submit_response(submission()),
        Err(RequestError::Response(ResponseRejection::Expired))
    ));
    assert_eq!(
        count_rows(&fixture.database, "request_responses"),
        1,
        "rejection does not run housekeeping"
    );
    assert_eq!(
        service(&mut fixture).get_response("orphan").unwrap(),
        ResponseLookup::Unavailable
    );
    assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
    assert!(matches!(
        service(&mut fixture).submit_response(submission()),
        Err(RequestError::Response(ResponseRejection::RequestNotFound))
    ));
}

#[test]
fn pruned_body_completion_marker_prevents_recreation_and_false_refund() {
    let mut fixture = Fixture::new();
    let target = endpoint("%71", 171);
    let input = prepare_input(
        &fixture,
        "marker",
        target.clone(),
        false,
        NOW_MS + 14 * DAY_MS,
        Originator::Unknown,
        true,
    );
    let prepared = service(&mut fixture)
        .prepare(input, "marker-attempt".into(), 1)
        .unwrap();
    service(&mut fixture)
        .begin_send(&prepared.attempt_id)
        .unwrap();
    let submission = || SubmitResponse {
        request_id: prepared.request_id.clone(),
        proof: ResponseProof::Recorded {
            attempt_id: prepared.attempt_id.clone(),
            endpoint: target.clone(),
        },
        body: "final".into(),
    };
    service(&mut fixture).submit_response(submission()).unwrap();
    fixture.set_now(NOW_MS + DAY_MS);
    service(&mut fixture).cleanup().unwrap();
    assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
    assert!(matches!(
        service(&mut fixture).submit_response(submission()),
        Err(RequestError::Response(ResponseRejection::Expired))
    ));
    service(&mut fixture)
        .settle(&prepared.attempt_id, Settlement::DefinitelyFailed)
        .unwrap();
    let attempt = service(&mut fixture)
        .get_attempt(&prepared.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(attempt.status, tmt_core::request::AttemptStatus::Uncertain);
    assert_eq!(attempt.response_submitted_at_ms, Some(NOW_MS));
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 1);
    assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
}

#[test]
fn late_final_extends_metadata_not_original_prompt_or_attention_acknowledgment() {
    let mut fixture = Fixture::new();
    let target = endpoint("%72", 172);
    let originator = Originator::Verified(fixture.identity_id.clone());
    let mut input = prepare_input(
        &fixture,
        "horizons",
        target.clone(),
        false,
        NOW_MS + 1,
        originator,
        false,
    );
    input.message = "\0original !\r\n".into();
    let prepared = service(&mut fixture)
        .prepare(input, "horizons-attempt".into(), 90)
        .unwrap();
    service(&mut fixture)
        .begin_send(&prepared.attempt_id)
        .unwrap();
    service(&mut fixture)
        .settle(&prepared.attempt_id, Settlement::Sent)
        .unwrap();
    let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
    oracle
        .execute(
            "UPDATE request_attention_identities SET acknowledged_through = latest_revision",
            [],
        )
        .unwrap();
    drop(oracle);
    fixture.set_now(NOW_MS + 3 * DAY_MS);
    let submission = |body: &str| SubmitResponse {
        request_id: prepared.request_id.clone(),
        proof: ResponseProof::Recorded {
            attempt_id: prepared.attempt_id.clone(),
            endpoint: target.clone(),
        },
        body: body.into(),
    };
    let final_response = service(&mut fixture)
        .submit_response(submission("late final"))
        .unwrap();
    assert_eq!(final_response.submitted_at_ms, NOW_MS + 3 * DAY_MS);
    assert_eq!(final_response.response_expires_at_ms, NOW_MS + 93 * DAY_MS);
    let oracle = rusqlite::Connection::open_with_flags(
        &fixture.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let stored: (String, i64, i64, i64, i64) = oracle
        .query_row(
            "SELECT message_text, message_expires_at_ms, retention_expires_at_ms,
                attention_revision, attention_acknowledged_revision
         FROM request_attempts WHERE request_id = 'horizons'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        stored,
        (
            "\0original !\r\n".into(),
            (NOW_MS + 90 * DAY_MS) as i64,
            (NOW_MS + 93 * DAY_MS) as i64,
            2,
            0
        )
    );
    let attention: (i64, i64) = oracle.query_row(
        "SELECT latest_revision, acknowledged_through FROM request_attention_identities WHERE identity_id = ?",
        [&fixture.identity_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(attention, (2, 1));
    drop(oracle);

    // The first-submission window has closed, but this body's independent
    // retention is still live and the original attempt remains retained.
    let before_retry = request_snapshot(&fixture.database);
    let before_cursor = fixture.storage.change_cursor().unwrap();
    for now in [NOW_MS + 7 * DAY_MS, NOW_MS + 7 * DAY_MS + 1] {
        fixture.set_now(now);
        assert_eq!(
            service(&mut fixture)
                .submit_response(submission("late final"))
                .unwrap(),
            final_response
        );
        assert_eq!(request_snapshot(&fixture.database), before_retry);
        assert_eq!(fixture.storage.change_cursor().unwrap(), before_cursor);
        assert!(matches!(
            service(&mut fixture).submit_response(submission("conflicting final")),
            Err(RequestError::Response(ResponseRejection::Conflict))
        ));
        assert_eq!(request_snapshot(&fixture.database), before_retry);
        assert_eq!(fixture.storage.change_cursor().unwrap(), before_cursor);
    }

    fixture.set_now(NOW_MS + 90 * DAY_MS);
    assert!(matches!(
        service(&mut fixture)
            .get_context("horizons")
            .unwrap()
            .unwrap()
            .prompt,
        RequestPrompt::Expired { .. }
    ));
    assert_eq!(
        service(&mut fixture).get_response("horizons").unwrap(),
        ResponseLookup::Available(Box::new(final_response))
    );
    fixture.set_now(NOW_MS + 93 * DAY_MS);
    assert_eq!(
        service(&mut fixture).get_response("horizons").unwrap(),
        ResponseLookup::Unavailable
    );
    assert!(
        service(&mut fixture)
            .get_context("horizons")
            .unwrap()
            .is_none()
    );
}

#[test]
fn request_clock_is_sampled_only_while_the_real_writer_lock_is_held() {
    let mut fixture = Fixture::new();
    let input = prepare_input(
        &fixture,
        "clock",
        endpoint("%73", 173),
        false,
        NOW_MS + 1,
        Originator::Unknown,
        false,
    );
    let database = fixture.database.clone();
    let clock = || {
        let probe = rusqlite::Connection::open(&database).unwrap();
        probe.busy_timeout(std::time::Duration::ZERO).unwrap();
        let error = probe
            .execute_batch("BEGIN IMMEDIATE")
            .expect_err("service clock must run after its writer lock is held");
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
        NOW_MS
    };
    let mut requests = RequestService::new(&mut fixture.storage, clock);
    requests.prepare(input, "clock-attempt".into(), 1).unwrap();
    requests.begin_send("clock-attempt").unwrap();
    requests.settle("clock-attempt", Settlement::Sent).unwrap();
    requests.release_wait("clock-attempt").unwrap();
    requests
        .submit_response(SubmitResponse {
            request_id: "clock".into(),
            proof: ResponseProof::Recorded {
                attempt_id: "clock-attempt".into(),
                endpoint: endpoint("%73", 173),
            },
            body: "done".into(),
        })
        .unwrap();
    requests.get_response("clock").unwrap();
    requests.get_context("clock").unwrap();
    requests.cleanup().unwrap();
}
