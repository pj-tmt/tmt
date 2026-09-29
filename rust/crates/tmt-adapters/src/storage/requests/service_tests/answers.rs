use super::support::{DAY_MS, Fixture, NOW_MS, endpoint, prepare_input, service};
use rusqlite::Connection;
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        AttemptStatus, Originator, RequestError, RequestKind, RequestRoute, ResponseRejection,
        SubmitResponse, WakeState,
        attention::AttentionRejection,
        inbox::{AnswerRejection, OpenPage},
    },
    retention::RESPONSE_ACCEPTANCE_WINDOW_MS,
};

fn asker(fixture: &mut Fixture, name: &str) -> String {
    create_or_resolve(&mut fixture.storage, name, Lifetime::Saved)
        .unwrap()
        .identity
        .id
}

/// Queues a request from `from` to the fixture identity, one millisecond
/// after the previous one so the order is deterministic.
fn ask(fixture: &mut Fixture, id: &str, from: &str, kind: RequestKind) {
    ask_as(fixture, id, Originator::Explicit(from.into()), kind);
}

fn ask_as(fixture: &mut Fixture, id: &str, originator: Originator, kind: RequestKind) {
    let mut input = prepare_input(
        fixture,
        id,
        endpoint("%1", 42),
        false,
        NOW_MS + 3_600_000,
        originator,
        false,
    );
    input.route = RequestRoute::Inbox {
        recipient_identity_id: fixture.identity_id.clone(),
    };
    input.kind = kind;
    service(fixture)
        .enqueue(input, format!("attempt-{id}"), 30)
        .unwrap();
    fixture.set_now(fixture.clock.load(std::sync::atomic::Ordering::SeqCst) + 1);
}

fn ids(page: &OpenPage) -> Vec<&str> {
    page.items
        .iter()
        .map(|item| item.request_id.as_str())
        .collect()
}

fn open(fixture: &mut Fixture, from: Option<&str>) -> Vec<String> {
    let me = fixture.identity_id.clone();
    let page = service(fixture).open_requests(&me, from, None).unwrap();
    ids(&page).into_iter().map(str::to_owned).collect()
}

/// Selects and submits as `tmt answer` does.
fn answer(
    fixture: &mut Fixture,
    from: &str,
    request: Option<&str>,
    body: &str,
) -> Result<(String, u64), RequestError<crate::storage::StorageError>> {
    answer_as(fixture, Some(from), request, body)
}

fn answer_as(
    fixture: &mut Fixture,
    from: Option<&str>,
    request: Option<&str>,
    body: &str,
) -> Result<(String, u64), RequestError<crate::storage::StorageError>> {
    let me = fixture.identity_id.clone();
    let (request_id, proof, _) = service(fixture).answer_target(&me, from, request)?;
    let (response, _) = service(fixture).submit_response_with_hint(
        SubmitResponse {
            request_id: request_id.clone(),
            proof,
            body: body.into(),
        },
        None,
    )?;
    Ok((request_id, response.submitted_at_ms))
}

fn column(fixture: &Fixture, request: &str, name: &str) -> i64 {
    Connection::open(&fixture.database)
        .unwrap()
        .query_row(
            &format!("SELECT {name} FROM request_attempts WHERE request_id=?"),
            [request],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn one_open_request_is_answered_without_a_receipt_and_leaves_the_inbox() {
    let mut fixture = Fixture::new();
    let alice = asker(&mut fixture, "alice");
    ask(&mut fixture, "q1", &alice, RequestKind::Request);
    assert_eq!(open(&mut fixture, None), ["q1"]);
    let (id, _) = answer(&mut fixture, &alice, None, "done").unwrap();
    assert_eq!(id, "q1");
    assert!(open(&mut fixture, None).is_empty());
    assert!(matches!(
        answer(&mut fixture, &alice, None, "again"),
        Err(RequestError::Answer(AnswerRejection::NotWaiting))
    ));
    assert_eq!(
        column(&fixture, "q1", "recipient_attention_acknowledged_revision"),
        0,
        "answering never acknowledges incoming attention"
    );
    assert!(
        column(&fixture, "q1", "attention_revision") > 1,
        "the final creates a newer originator revision"
    );
}

#[test]
fn several_open_requests_are_refused_oldest_first_until_one_is_chosen() {
    let mut fixture = Fixture::new();
    let alice = asker(&mut fixture, "alice");
    let bob = asker(&mut fixture, "bob");
    ask(&mut fixture, "a1", &alice, RequestKind::Request);
    ask(&mut fixture, "b1", &bob, RequestKind::Request);
    ask(&mut fixture, "a2", &alice, RequestKind::Request);
    assert_eq!(open(&mut fixture, None), ["a1", "b1", "a2"]);
    assert_eq!(open(&mut fixture, Some(&alice)), ["a1", "a2"]);
    match answer(&mut fixture, &alice, None, "which?") {
        Err(RequestError::Answer(AnswerRejection::Ambiguous(items))) => assert_eq!(
            items
                .iter()
                .map(|item| item.request_id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "a2"]
        ),
        other => panic!("expected ambiguity, got {other:?}"),
    }
    assert_eq!(
        open(&mut fixture, None).len(),
        3,
        "a refusal changes nothing"
    );
    assert_eq!(
        answer(&mut fixture, &alice, Some("a2"), "second")
            .unwrap()
            .0,
        "a2"
    );
    assert_eq!(answer(&mut fixture, &alice, None, "first").unwrap().0, "a1");
    assert_eq!(answer(&mut fixture, &bob, None, "bob's").unwrap().0, "b1");
}

#[test]
fn an_explicit_request_must_be_from_that_originator_to_you() {
    let mut fixture = Fixture::new();
    let alice = asker(&mut fixture, "alice");
    let bob = asker(&mut fixture, "bob");
    ask(&mut fixture, "a1", &alice, RequestKind::Request);
    for (from, request) in [(&bob, "a1"), (&alice, "missing")] {
        assert!(matches!(
            answer(&mut fixture, from, Some(request), "x"),
            Err(RequestError::Attention(AttentionRejection::NotFound))
        ));
    }
    let me = fixture.identity_id.clone();
    assert!(matches!(
        service(&mut fixture).answer_target(&alice, Some(&me), Some("a1")),
        Err(RequestError::Attention(AttentionRejection::NotFound))
    ));
    assert_eq!(open(&mut fixture, None), ["a1"]);
}

#[test]
fn an_explicit_retry_is_idempotent_and_a_different_body_conflicts() {
    let mut fixture = Fixture::new();
    let alice = asker(&mut fixture, "alice");
    ask(&mut fixture, "q1", &alice, RequestKind::Request);
    let (_, first) = answer(&mut fixture, &alice, None, "done").unwrap();
    fixture.set_now(NOW_MS + 60_000);
    assert_eq!(
        answer(&mut fixture, &alice, Some("q1"), "done").unwrap().1,
        first,
        "an identical retry keeps the original submission"
    );
    assert!(matches!(
        answer(&mut fixture, &alice, Some("q1"), "changed"),
        Err(RequestError::Response(ResponseRejection::Conflict))
    ));
}

#[test]
fn acknowledgment_and_live_delivery_do_not_remove_an_open_request() {
    let mut fixture = Fixture::new();
    let me = fixture.identity_id.clone();
    let alice = asker(&mut fixture, "alice");
    ask(&mut fixture, "acked", &alice, RequestKind::Request);
    ask(&mut fixture, "delivered", &alice, RequestKind::Request);
    let incoming = service(&mut fixture)
        .list_incoming(&me, None, None, None)
        .unwrap();
    let acked = &incoming.items[0].exchange;
    service(&mut fixture)
        .acknowledge_incoming_request(&me, &acked.request_id, acked.revision)
        .unwrap();
    assert!(
        service(&mut fixture)
            .claim_wake("delivered")
            .unwrap()
            .claimed
    );
    service(&mut fixture)
        .settle_request_delivery("delivered", WakeState::Sent)
        .unwrap();
    // Both left the attention view (the old Squad path found neither),
    // yet both still wait for a final.
    assert!(
        service(&mut fixture)
            .list_incoming(&me, None, None, None)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(open(&mut fixture, None), ["acked", "delivered"]);
    assert_eq!(
        answer(&mut fixture, &alice, Some("delivered"), "ok")
            .unwrap()
            .0,
        "delivered"
    );
    assert_eq!(answer(&mut fixture, &alice, None, "ok").unwrap().0, "acked");
}

#[test]
fn the_inbox_offers_exactly_what_an_answer_accepts_for_every_delivery_state() {
    for status in [
        AttemptStatus::Prepared,
        AttemptStatus::Sending,
        AttemptStatus::Sent,
        AttemptStatus::Queued,
        AttemptStatus::Uncertain,
        AttemptStatus::DefinitelyFailed,
    ] {
        let mut fixture = Fixture::new();
        let alice = asker(&mut fixture, "alice");
        ask(&mut fixture, "q1", &alice, RequestKind::Request);
        Connection::open(&fixture.database)
            .unwrap()
            .execute(
                "UPDATE request_attempts SET status=? WHERE request_id='q1'",
                [status.as_str()],
            )
            .unwrap();
        let listed = !open(&mut fixture, None).is_empty();
        let accepted = answer(&mut fixture, &alice, Some("q1"), "done").is_ok();
        assert_eq!(listed, accepted, "{status:?}");
        assert_eq!(
            listed,
            tmt_core::request::inbox::ANSWERABLE.contains(&status),
            "{status:?}"
        );
    }
}

#[test]
fn the_acceptance_deadline_ends_waiting_and_announcements_never_wait() {
    let mut fixture = Fixture::new();
    let alice = asker(&mut fixture, "alice");
    ask(&mut fixture, "q1", &alice, RequestKind::Request);
    ask(&mut fixture, "note", &alice, RequestKind::Announcement);
    assert_eq!(open(&mut fixture, None), ["q1"]);
    // Past its expiry but inside the acceptance window: still waiting.
    fixture.set_now(NOW_MS + DAY_MS);
    assert_eq!(open(&mut fixture, None), ["q1"]);
    fixture.set_now(NOW_MS + RESPONSE_ACCEPTANCE_WINDOW_MS);
    assert!(open(&mut fixture, None).is_empty());
    assert!(matches!(
        answer(&mut fixture, &alice, None, "late"),
        Err(RequestError::Answer(AnswerRejection::NotWaiting))
    ));
    assert!(matches!(
        answer(&mut fixture, &alice, Some("q1"), "late"),
        Err(RequestError::Response(ResponseRejection::Expired))
    ));
}

#[test]
fn the_inbox_limit_reports_more_and_rejects_invalid_input() {
    let mut fixture = Fixture::new();
    let me = fixture.identity_id.clone();
    let alice = asker(&mut fixture, "alice");
    for id in ["q1", "q2", "q3"] {
        ask(&mut fixture, id, &alice, RequestKind::Request);
    }
    let page = service(&mut fixture)
        .open_requests(&me, None, Some(2))
        .unwrap();
    assert_eq!((ids(&page), page.more), (vec!["q1", "q2"], true));
    let page = service(&mut fixture)
        .open_requests(&me, None, Some(3))
        .unwrap();
    assert!(!page.more);
    for limit in [0, 201] {
        assert!(matches!(
            service(&mut fixture).open_requests(&me, None, Some(limit)),
            Err(RequestError::Invalid(_))
        ));
    }
    assert!(matches!(
        service(&mut fixture).open_requests(&me, Some(""), None),
        Err(RequestError::Invalid(_))
    ));
}

#[test]
fn an_anonymous_request_is_answered_only_by_its_request_id() {
    let mut fixture = Fixture::new();
    let alice = asker(&mut fixture, "alice");
    ask_as(
        &mut fixture,
        "anon",
        Originator::Unknown,
        RequestKind::Request,
    );
    let me = fixture.identity_id.clone();
    let page = service(&mut fixture)
        .open_requests(&me, None, None)
        .unwrap();
    assert_eq!(page.items[0].originator, Originator::Unknown);
    assert!(matches!(
        answer(&mut fixture, &alice, None, "by name"),
        Err(RequestError::Answer(AnswerRejection::NotWaiting))
    ));
    assert!(matches!(
        answer(&mut fixture, &alice, Some("anon"), "wrong sender"),
        Err(RequestError::Attention(AttentionRejection::NotFound))
    ));
    assert!(matches!(
        answer_as(&mut fixture, None, None, "neither"),
        Err(RequestError::Invalid(_))
    ));
    let (id, _) = answer_as(&mut fixture, None, Some("anon"), "done").unwrap();
    assert_eq!(id, "anon");
    assert!(open(&mut fixture, None).is_empty());
}

#[test]
fn a_request_id_alone_must_still_be_addressed_to_you() {
    let mut fixture = Fixture::new();
    let alice = asker(&mut fixture, "alice");
    ask(&mut fixture, "a1", &alice, RequestKind::Request);
    assert!(matches!(
        service(&mut fixture).answer_target(&alice, None, Some("a1")),
        Err(RequestError::Attention(AttentionRejection::NotFound))
    ));
    let (_, _, originator) = {
        let me = fixture.identity_id.clone();
        service(&mut fixture)
            .answer_target(&me, None, Some("a1"))
            .unwrap()
    };
    assert_eq!(originator, Originator::Explicit(alice));
}
