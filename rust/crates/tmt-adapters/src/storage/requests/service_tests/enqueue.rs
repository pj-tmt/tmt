use super::support::{
    Fixture, NOW_MS, count_rows, endpoint, preamble_count, prepare_input, service,
};
use rusqlite::{Connection, OpenFlags};
use tmt_core::request::{AttemptStatus, Originator, PrepareRequest, RequestError, RequestRoute};

fn inbox_input(fixture: &Fixture) -> PrepareRequest {
    let mut input = prepare_input(
        fixture,
        "enqueue-request",
        endpoint("%1", 101),
        true,
        NOW_MS + 3_600_000,
        Originator::Explicit(fixture.identity_id.clone()),
        true,
    );
    input.route = RequestRoute::Inbox {
        recipient_identity_id: fixture.identity_id.clone(),
    };
    input.message = "Review this immutable snapshot.\nKeep exact message bytes!".into();
    input
}

#[test]
fn final_before_live_claim_refunds_unused_preamble_without_rewaking() {
    let mut fixture = Fixture::new();
    let input = inbox_input(&fixture);
    let route = input.route.clone();
    service(&mut fixture)
        .enqueue(input, "enqueue-attempt".into(), 7)
        .unwrap();
    service(&mut fixture)
        .submit_response(tmt_core::request::SubmitResponse {
            request_id: "enqueue-request".into(),
            proof: tmt_core::request::ResponseProof::Compact(
                tmt_core::request::correlation::response_token(
                    "enqueue-request",
                    "enqueue-attempt",
                    &route,
                ),
            ),
            body: "Read directly from Inbox".into(),
        })
        .unwrap();
    let claim = service(&mut fixture).claim_wake("enqueue-request").unwrap();
    assert!(!claim.claimed);
    assert_eq!(claim.state, tmt_core::request::WakeState::Unavailable);
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 0);
    assert!(
        !service(&mut fixture)
            .claim_wake("enqueue-request")
            .unwrap()
            .claimed
    );
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 0);
}

#[test]
fn full_delivery_settles_only_incoming_attention_and_never_rewakes() {
    use tmt_core::request::WakeState;
    for (outcome, incoming, cadence) in [
        (WakeState::Sent, 0, 1),
        (WakeState::Unavailable, 1, 0),
        (WakeState::Uncertain, 1, 1),
    ] {
        let mut fixture = Fixture::new();
        let identity = fixture.identity_id.clone();
        let input = inbox_input(&fixture);
        service(&mut fixture)
            .enqueue(input, "enqueue-attempt".into(), 7)
            .unwrap();
        assert!(
            service(&mut fixture)
                .claim_wake("enqueue-request")
                .unwrap()
                .claimed
        );
        service(&mut fixture)
            .settle_request_delivery("enqueue-request", outcome)
            .unwrap();
        assert_eq!(
            service(&mut fixture)
                .list_incoming(&identity, None, None, None)
                .unwrap()
                .items
                .len(),
            incoming
        );
        assert_eq!(preamble_count(&fixture.database, &identity), cadence);
        assert!(
            !service(&mut fixture)
                .claim_wake("enqueue-request")
                .unwrap()
                .claimed
        );
        let oracle = Connection::open(&fixture.database).unwrap();
        let state: (String, String, i64, i64) = oracle.query_row(
            "SELECT route_kind, status, attention_revision, attention_acknowledged_revision FROM request_attempts",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        ).unwrap();
        assert_eq!(state.0, "inbox");
        assert_eq!(state.1, "queued");
        assert!(state.2 > 0);
        assert_eq!(
            state.3, 0,
            "delivery never acknowledges originator attention"
        );
    }
}

#[test]
fn advisory_wake_does_not_consume_queued_request_attention() {
    let mut fixture = Fixture::new();
    let identity = fixture.identity_id.clone();
    let input = inbox_input(&fixture);
    service(&mut fixture)
        .enqueue(input, "enqueue-attempt".into(), 7)
        .unwrap();
    assert!(
        service(&mut fixture)
            .claim_wake("enqueue-request")
            .unwrap()
            .claimed
    );
    service(&mut fixture)
        .settle_wake("enqueue-request", tmt_core::request::WakeState::Sent)
        .unwrap();
    assert_eq!(
        service(&mut fixture)
            .list_incoming(&identity, None, None, None)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn enqueue_publishes_exact_prompt_and_both_attention_sides_before_returning() {
    let mut fixture = Fixture::new();
    let input = inbox_input(&fixture);
    let expected_message = input.message.clone();
    let prepared = service(&mut fixture)
        .enqueue(input, "enqueue-attempt".into(), 7)
        .unwrap();
    assert_eq!(prepared.request_id, "enqueue-request");
    assert!(prepared.inject_preamble);
    assert_eq!(prepared.previous_request_id, None);

    let oracle =
        Connection::open_with_flags(&fixture.database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (status, message, bytes, sender_revision, recipient_revision, ack, wait): (
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = oracle
        .query_row(
            "SELECT status, message_text, message_bytes, attention_revision,
                    recipient_attention_revision, recipient_attention_acknowledged_revision,
                    wait_active FROM request_attempts WHERE attempt_id = ?",
            [&prepared.attempt_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(status, "queued");
    assert_eq!(message, expected_message);
    assert_eq!(bytes, expected_message.len() as i64);
    assert!(sender_revision > 0);
    assert!(recipient_revision > sender_revision);
    assert_eq!(ack, 0);
    assert_eq!(wait, 1);
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 1);

    service(&mut fixture)
        .release_wait(&prepared.attempt_id)
        .unwrap();
    let (status, wait, revision): (String, i64, i64) = oracle
        .query_row(
            "SELECT status, wait_active, recipient_attention_revision
         FROM request_attempts WHERE attempt_id = ?",
            [&prepared.attempt_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (status.as_str(), wait, revision),
        ("queued", 0, recipient_revision)
    );
}

#[test]
fn enqueue_without_sender_identity_needs_no_pane_or_synthetic_originator() {
    let mut fixture = Fixture::new();
    let mut input = inbox_input(&fixture);
    input.originator = Originator::Unknown;
    input.preamble = None;
    input.wait = false;
    service(&mut fixture)
        .enqueue(input, "enqueue-attempt".into(), 7)
        .unwrap();
    let oracle =
        Connection::open_with_flags(&fixture.database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let state: (String, Option<String>, Option<String>, i64, i64) = oracle
        .query_row(
            "SELECT originator_kind, originator_identity_id, pane_id,
                    attention_revision, recipient_attention_revision FROM request_attempts",
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
    assert_eq!(state, ("unknown".into(), None, None, 0, 1));
    assert_eq!(count_rows(&fixture.database, "identities"), 1);
    let identity_id = fixture.identity_id.clone();
    let incoming = service(&mut fixture)
        .list_incoming(&identity_id, None, None, None)
        .unwrap();
    assert_eq!(incoming.items.len(), 1);
    assert_eq!(incoming.items[0].sender_identity_id, None);
}

#[test]
fn enqueue_storage_failure_rolls_back_prompt_cadence_and_attention_then_retries_once() {
    let mut fixture = Fixture::new();
    let input = inbox_input(&fixture);
    let oracle = Connection::open(&fixture.database).unwrap();
    // Fail after creation and attention allocation, at the publication transition.
    oracle
        .execute_batch(
            "CREATE TRIGGER reject_enqueue BEFORE UPDATE OF status ON request_attempts
         WHEN NEW.status = 'queued'
         BEGIN SELECT RAISE(ABORT, 'injected enqueue failure'); END;",
        )
        .unwrap();
    let error = service(&mut fixture)
        .enqueue(input, "enqueue-attempt".into(), 7)
        .unwrap_err();
    assert!(matches!(error, RequestError::Repository(_)), "{error:?}");
    assert_eq!(count_rows(&fixture.database, "request_attempts"), 0);
    assert_eq!(
        count_rows(&fixture.database, "request_attention_identities"),
        0
    );
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 0);

    oracle
        .execute_batch("DROP TRIGGER reject_enqueue;")
        .unwrap();
    let input = inbox_input(&fixture);
    service(&mut fixture)
        .enqueue(input, "enqueue-attempt".into(), 7)
        .unwrap();
    let before: (i64, i64) = oracle
        .query_row(
            "SELECT attention_revision, recipient_attention_revision FROM request_attempts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    // Exercise each unique key independently; neither may advance attention.
    for (request_id, attempt_id) in [
        ("enqueue-request", "new-attempt"),
        ("new-request", "enqueue-attempt"),
    ] {
        let mut duplicate = inbox_input(&fixture);
        duplicate.request_id = request_id.into();
        assert!(matches!(
            service(&mut fixture).enqueue(duplicate, attempt_id.into(), 7),
            Err(RequestError::Repository(_))
        ));
    }
    assert_eq!(count_rows(&fixture.database, "request_attempts"), 1);
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 1);
    let after: (i64, i64) = oracle
        .query_row(
            "SELECT attention_revision, recipient_attention_revision FROM request_attempts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, after);
    let watermark: i64 = oracle
        .query_row(
            "SELECT latest_revision FROM request_attention_identities WHERE identity_id = ?",
            [&fixture.identity_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(watermark, after.1);
}

#[test]
fn enqueue_retired_recipient_commits_failed_attempt_without_recipient_attention() {
    let mut fixture = Fixture::new();
    let input = inbox_input(&fixture);
    let oracle = Connection::open(&fixture.database).unwrap();
    oracle
        .execute(
            "UPDATE identities SET retired_at_ms = ? WHERE id = ?",
            rusqlite::params![NOW_MS as i64, fixture.identity_id],
        )
        .unwrap();
    assert!(matches!(
        service(&mut fixture).enqueue(input, "enqueue-attempt".into(), 7),
        Err(RequestError::NotFound)
    ));
    let attempt = service(&mut fixture)
        .get_attempt("enqueue-attempt")
        .unwrap()
        .unwrap();
    assert_eq!(attempt.status, AttemptStatus::DefinitelyFailed);
    assert!(!attempt.wait_active);
    assert!(!attempt.cadence_reserved);
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 0);
    let recipient_revision: i64 = oracle
        .query_row(
            "SELECT recipient_attention_revision FROM request_attempts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recipient_revision, 0);
}

#[test]
fn enqueue_rejects_pane_and_mismatched_inbox_routes_without_writing() {
    let mut fixture = Fixture::new();
    let mut input = inbox_input(&fixture);
    input.route = RequestRoute::Pane(endpoint("%1", 101));
    assert!(matches!(
        service(&mut fixture).enqueue(input, "pane-attempt".into(), 7),
        Err(RequestError::StateInvalid)
    ));
    let mut input = inbox_input(&fixture);
    input.route = RequestRoute::Inbox {
        recipient_identity_id: "different-identity".into(),
    };
    assert!(matches!(
        service(&mut fixture).enqueue(input, "mismatch-attempt".into(), 7),
        Err(RequestError::Invalid(_))
    ));
    assert_eq!(count_rows(&fixture.database, "request_attempts"), 0);
    assert_eq!(
        count_rows(&fixture.database, "request_attention_identities"),
        0
    );
    assert_eq!(preamble_count(&fixture.database, &fixture.identity_id), 0);
}
