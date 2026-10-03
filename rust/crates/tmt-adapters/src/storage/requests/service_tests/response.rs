use super::support::{
    Fixture, NOW_MS, count_rows, endpoint, preamble_count, prepare_input, service,
};
use tmt_core::limits::MAX_JS_SAFE_INTEGER;
use tmt_core::request::{
    AttemptStatus, Originator, PrepareRequest, RequestError, ResponseProof, ResponseRejection,
    Settlement, SubmitResponse,
};

fn prepare_sending(
    fixture: &mut Fixture,
    request_id: &str,
) -> (String, tmt_core::request::RequestEndpoint) {
    let target = endpoint("%10", 110);
    let input = prepare_input(
        fixture,
        request_id,
        target.clone(),
        true,
        NOW_MS + 3_600_001,
        Originator::Unknown,
        false,
    );
    let prepared = {
        let mut requests = service(fixture);
        requests
            .prepare(input, format!("attempt-{request_id}"), 7)
            .expect("prepare response request")
    };
    {
        let mut requests = service(fixture);
        requests
            .begin_send(&prepared.attempt_id)
            .expect("begin response request");
    }
    (prepared.attempt_id, target)
}

fn submit(
    fixture: &mut Fixture,
    request_id: &str,
    attempt_id: &str,
    target: tmt_core::request::RequestEndpoint,
    body: &str,
) -> Result<tmt_core::request::FinalResponse, RequestError<crate::storage::StorageError>> {
    let mut requests = service(fixture);
    requests.submit_response(SubmitResponse {
        request_id: request_id.into(),
        proof: ResponseProof::Recorded {
            attempt_id: attempt_id.into(),
            endpoint: target,
        },
        body: body.into(),
    })
}

#[test]
fn final_body_is_exact_and_identical_retry_is_immutable() {
    let mut fixture = Fixture::new();
    let (attempt_id, target) = prepare_sending(&mut fixture, "request-exact-final");
    let body = "\u{feff}  first\r\n\u{0000} 日本語 🙂  ";
    let first = submit(
        &mut fixture,
        "request-exact-final",
        &attempt_id,
        target.clone(),
        body,
    )
    .expect("submit exact final");
    assert_eq!(first.body, body);
    assert_eq!(first.body_bytes, body.len() as u64);
    assert_eq!(count_rows(&fixture.database, "request_responses"), 1);

    // A retry through a fresh service at a later clock value must not renew
    // the immutable final's submission time or expiry.
    fixture.set_now(NOW_MS + 1);
    let retry = submit(
        &mut fixture,
        "request-exact-final",
        &attempt_id,
        target.clone(),
        body,
    )
    .expect("retry identical final");
    assert_eq!(retry, first);
    assert!(matches!(
        submit(
            &mut fixture,
            "request-exact-final",
            &attempt_id,
            target,
            "different final"
        ),
        Err(RequestError::Response(ResponseRejection::Conflict))
    ));
    assert_eq!(count_rows(&fixture.database, "request_responses"), 1);
}

#[test]
fn every_endpoint_fence_mismatch_leaves_final_and_attempt_unchanged() {
    let mut fixture = Fixture::new();
    let (attempt_id, target) = prepare_sending(&mut fixture, "request-endpoint-fence");
    let before = {
        let mut requests = service(&mut fixture);
        requests
            .get_attempt(&attempt_id)
            .expect("read attempt")
            .expect("attempt exists")
    };
    let mismatches = [
        tmt_core::request::RequestEndpoint {
            server: tmt_core::endpoint::ServerEvidence {
                server_id: "other-server".into(),
                ..target.server.clone()
            },
            ..target.clone()
        },
        tmt_core::request::RequestEndpoint {
            server: tmt_core::endpoint::ServerEvidence {
                socket_path: "/tmp/other.sock".into(),
                ..target.server.clone()
            },
            ..target.clone()
        },
        tmt_core::request::RequestEndpoint {
            server: tmt_core::endpoint::ServerEvidence {
                server_pid: target.server.server_pid + 1,
                ..target.server.clone()
            },
            ..target.clone()
        },
        tmt_core::request::RequestEndpoint {
            server: tmt_core::endpoint::ServerEvidence {
                server_start_time: "other-start".into(),
                ..target.server.clone()
            },
            ..target.clone()
        },
        tmt_core::request::RequestEndpoint {
            pane_id: "%11".into(),
            ..target.clone()
        },
        tmt_core::request::RequestEndpoint {
            pane_pid: target.pane_pid + 1,
            ..target.clone()
        },
    ];
    assert!(matches!(
        submit(
            &mut fixture,
            "other-request",
            &attempt_id,
            target.clone(),
            "body"
        ),
        Err(RequestError::Response(ResponseRejection::RequestNotFound))
    ));
    assert!(matches!(
        submit(
            &mut fixture,
            "request-endpoint-fence",
            "other-attempt",
            target.clone(),
            "body"
        ),
        Err(RequestError::Response(ResponseRejection::AttemptMismatch))
    ));
    assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
    for mismatched in mismatches {
        assert!(matches!(
            submit(
                &mut fixture,
                "request-endpoint-fence",
                &attempt_id,
                mismatched,
                "body"
            ),
            Err(RequestError::Response(ResponseRejection::RecipientMismatch))
        ));
        assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
    }
    let after = {
        let mut requests = service(&mut fixture);
        requests
            .get_attempt(&attempt_id)
            .expect("read unchanged attempt")
            .expect("attempt exists")
    };
    assert_eq!(after, before);
}

#[test]
fn first_final_attention_overflow_rolls_back_body_and_completion_marker() {
    let mut fixture = Fixture::new();
    let target = endpoint("%12", 112);
    let input: PrepareRequest = prepare_input(
        &fixture,
        "request-final-attention-overflow",
        target.clone(),
        false,
        NOW_MS + 3_600_001,
        Originator::Explicit(fixture.identity_id.clone()),
        false,
    );
    let prepared = {
        let mut requests = service(&mut fixture);
        requests
            .prepare(input, "attempt-final-attention-overflow".into(), 7)
            .expect("prepare attention request")
    };
    {
        let mut requests = service(&mut fixture);
        requests
            .begin_send(&prepared.attempt_id)
            .expect("begin attention request");
        requests
            .settle(&prepared.attempt_id, Settlement::Sent)
            .expect("settle attention request");
    }
    let connection = rusqlite::Connection::open(&fixture.database).expect("open SQL oracle");
    connection
        .execute(
            "UPDATE request_attention_identities SET latest_revision = ? WHERE identity_id = ?",
            rusqlite::params![MAX_JS_SAFE_INTEGER as i64, fixture.identity_id],
        )
        .expect("exhaust attention revision");
    drop(connection);

    let result = submit(
        &mut fixture,
        &prepared.request_id,
        &prepared.attempt_id,
        target,
        "final before overflow",
    );
    assert!(matches!(result, Err(RequestError::RevisionExhausted)));
    assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
    let connection = rusqlite::Connection::open(&fixture.database).expect("reopen SQL oracle");
    let marker: Option<i64> = connection
        .query_row(
            "SELECT response_submitted_at_ms FROM request_attempts WHERE attempt_id = ?",
            [&prepared.attempt_id],
            |row| row.get(0),
        )
        .expect("read completion marker");
    assert_eq!(marker, None);
    let status: String = connection
        .query_row(
            "SELECT status FROM request_attempts WHERE attempt_id = ?",
            [&prepared.attempt_id],
            |row| row.get(0),
        )
        .expect("read attempt status");
    assert_eq!(status, AttemptStatus::Sent.as_str());
}

#[test]
fn response_first_settlement_does_not_refund_reserved_cadence() {
    let mut fixture = Fixture::new();
    let target = endpoint("%13", 113);
    let input = prepare_input(
        &fixture,
        "request-response-first",
        target.clone(),
        false,
        NOW_MS + 3_600_001,
        Originator::Explicit(fixture.identity_id.clone()),
        true,
    );
    let prepared = {
        let mut requests = service(&mut fixture);
        requests
            .prepare(input, "attempt-response-first".into(), 7)
            .expect("prepare response-first request")
    };
    {
        let mut requests = service(&mut fixture);
        requests
            .begin_send(&prepared.attempt_id)
            .expect("begin response-first request");
        requests
            .submit_response(SubmitResponse {
                request_id: prepared.request_id.clone(),
                proof: ResponseProof::Recorded {
                    attempt_id: prepared.attempt_id.clone(),
                    endpoint: target,
                },
                body: "response arrived first".into(),
            })
            .expect("submit response before settlement");
        requests
            .settle(&prepared.attempt_id, Settlement::DefinitelyFailed)
            .expect("settle response-first attempt as uncertain");
        let attempt = requests
            .get_attempt(&prepared.attempt_id)
            .expect("read response-first attempt")
            .expect("response-first attempt remains retained");
        assert_eq!(attempt.status, AttemptStatus::Uncertain);
        assert!(attempt.cadence_reserved);
    }
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 1);
}

#[test]
fn result_prefix_preserves_exact_body_state_and_full_id_lookup() {
    use super::support::request_snapshot;
    use tmt_core::request::{ResponseLookup, ResultSelectionRejection};
    let mut fixture = Fixture::new();
    let id = "req_a1b2c3d4-1234-4234-8234-123456789abc";
    let target = endpoint("%10", 110);
    let input = prepare_input(
        &fixture,
        id,
        target.clone(),
        true,
        NOW_MS + 3_600_001,
        Originator::Explicit(fixture.identity_id.clone()),
        false,
    );
    let attempt = service(&mut fixture)
        .prepare(input, format!("attempt-{id}"), 7)
        .unwrap()
        .attempt_id;
    service(&mut fixture).begin_send(&attempt).unwrap();
    let stored = submit(&mut fixture, id, &attempt, target, "\u{feff}exact\r\n\0 🙂").unwrap();
    let before = request_snapshot(&fixture.database);
    for selection in [
        id,
        "a1b2c3d4",
        "req_a1b2c3d4",
        "A1B2C3D4",
        "a1b2c3d4-1",
        &id[4..],
    ] {
        let (resolved, lookup) = service(&mut fixture)
            .get_response_by_prefix(selection)
            .unwrap();
        assert_eq!(resolved, id);
        assert_eq!(lookup, ResponseLookup::Available(Box::new(stored.clone())));
    }
    assert_eq!(request_snapshot(&fixture.database), before);
    assert_eq!(
        service(&mut fixture).get_response(id).unwrap(),
        ResponseLookup::Available(Box::new(stored))
    );
    assert_eq!(
        service(&mut fixture)
            .get_response_by_prefix("aaaaaaaa")
            .unwrap(),
        ("aaaaaaaa".into(), ResponseLookup::Unavailable)
    );
    for short in ["a1b2c3d", "req_a1b2c3d"] {
        assert!(matches!(
            service(&mut fixture).get_response_by_prefix(short),
            Err(RequestError::ResultSelection(
                ResultSelectionRejection::PrefixTooShort
            ))
        ));
    }
    let legacy = "request-legacy";
    let (attempt, target) = prepare_sending(&mut fixture, legacy);
    let stored = submit(&mut fixture, legacy, &attempt, target, "legacy").unwrap();
    assert_eq!(
        service(&mut fixture)
            .get_response_by_prefix(legacy)
            .unwrap(),
        (legacy.into(), ResponseLookup::Available(Box::new(stored)))
    );
}

#[test]
fn result_prefix_ambiguity_is_sorted_capped_and_excludes_logically_expired_rows() {
    use tmt_core::request::{ResponseLookup, ResultSelectionRejection};
    let mut fixture = Fixture::new();
    let ids: Vec<_> = (0..8)
        .map(|n| format!("req_ffffffff-{n:04x}-4000-8000-000000000000"))
        .collect();
    for id in ids.iter().rev() {
        prepare_sending(&mut fixture, id);
    }
    let Err(RequestError::ResultSelection(ResultSelectionRejection::Ambiguous(matches))) =
        service(&mut fixture).get_response_by_prefix("ffffffff")
    else {
        panic!("must be ambiguous");
    };
    assert_eq!(matches.ids, ids[..5]);
    assert_eq!(matches.total, 8);
    let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
    // Waiters keep physical expired rows. Filtering after LIMIT would miss the
    // only live match; expiry must narrow inside the indexed range query.
    oracle
        .execute(
            "UPDATE request_attempts SET retention_expires_at_ms=? WHERE request_id < ?",
            rusqlite::params![NOW_MS as i64, ids[7]],
        )
        .unwrap();
    let (resolved, lookup) = service(&mut fixture)
        .get_response_by_prefix("req_ffffffff")
        .unwrap();
    assert_eq!(resolved, ids[7]);
    assert_eq!(lookup, ResponseLookup::Unavailable);
    assert_eq!(count_rows(&fixture.database, "request_attempts"), 8);
    prepare_sending(&mut fixture, "req_fffffff0-0000-4000-8000-000000000000");
    assert_eq!(
        service(&mut fixture)
            .get_response_by_prefix("ffffffff-")
            .unwrap()
            .0,
        ids[7]
    );
}

#[test]
fn result_prefix_keeps_late_final_and_announcement_retention_semantics() {
    use super::support::DAY_MS;
    use tmt_core::request::{RequestKind, RequestRoute, ResponseLookup};
    let mut fixture = Fixture::new();
    let id = "req_12345678-0000-4000-8000-000000000000";
    let (attempt, target) = prepare_sending(&mut fixture, id);
    fixture.set_now(NOW_MS + DAY_MS);
    let stored = submit(&mut fixture, id, &attempt, target, "late final").unwrap();
    fixture.set_now(NOW_MS + 7 * DAY_MS);
    assert_eq!(
        service(&mut fixture)
            .get_response_by_prefix("12345678")
            .unwrap()
            .1,
        ResponseLookup::Available(Box::new(stored.clone()))
    );
    fixture.set_now(stored.response_expires_at_ms);
    assert_eq!(
        service(&mut fixture)
            .get_response_by_prefix("12345678")
            .unwrap()
            .1,
        ResponseLookup::Unavailable
    );
    let id = "req_abcdef12-0000-4000-8000-000000000000";
    let input = PrepareRequest {
        kind: RequestKind::Announcement,
        room_id: None,
        request_id: id.into(),
        message: "announcement".into(),
        route: RequestRoute::Inbox {
            recipient_identity_id: fixture.identity_id.clone(),
        },
        wait: false,
        expires_at_ms: stored.response_expires_at_ms + 10_000,
        originator: Originator::Unknown,
        recipient_identity_id: Some(fixture.identity_id.clone()),
        preamble: None,
    };
    service(&mut fixture)
        .enqueue(input, "announcement-prefix".into(), 7)
        .unwrap();
    assert_eq!(
        service(&mut fixture)
            .get_response_by_prefix("abcdef12")
            .unwrap(),
        (id.into(), ResponseLookup::NotRequired)
    );
}
