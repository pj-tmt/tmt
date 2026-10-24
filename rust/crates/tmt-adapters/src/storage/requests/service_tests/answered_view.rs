//! `request_answers`: an originator's submitted finals and answered passthrough turns, one
//! list by arrival with one cursor (#4685).
use super::passthrough::{claimed_passthrough, identity, mark_user};
use super::support::{Fixture, NOW_MS, endpoint, prepare_input, request_snapshot, service};
use tmt_core::request::{
    Originator, RequestError, RequestRoute, ResponseProof, SubmitResponse, WakeState, correlation,
    history::{Answer, AnswerCursor, AnswerKind, AnswersQuery},
};

fn ask(f: &mut Fixture, id: &str, from: &str) {
    let recipient = f.identity_id.clone();
    let mut input = prepare_input(
        f,
        id,
        endpoint("%1", 42),
        false,
        NOW_MS + 3_600_000,
        Originator::Explicit(from.into()),
        false,
    );
    input.recipient_identity_id = Some(recipient.clone());
    input.route = RequestRoute::Inbox {
        recipient_identity_id: recipient,
    };
    service(f)
        .enqueue(input, format!("attempt-{id}"), 1)
        .unwrap();
}

/// The recipient's final for `id`, submitted at `at`.
fn final_at(f: &mut Fixture, id: &str, at: u64) {
    f.set_now(at);
    let route = RequestRoute::Inbox {
        recipient_identity_id: f.identity_id.clone(),
    };
    service(f)
        .submit_response(SubmitResponse {
            request_id: id.into(),
            proof: ResponseProof::Compact(correlation::response_token(
                id,
                &format!("attempt-{id}"),
                &route,
            )),
            body: format!("reply to {id}"),
        })
        .unwrap();
}

/// A passthrough from `person` whose text reached the pane, so it has a capture.
fn typed(f: &mut Fixture, person: &str, id: &str) {
    let target = f.identity_id.clone();
    claimed_passthrough(f, &target, person, id, "typed text");
    service(f)
        .settle_request_delivery(id, WakeState::Sent)
        .unwrap();
}

/// The turn `id` earned, settled at `at`.
fn turn_at(f: &Fixture, id: &str, at: u64) {
    let changed = rusqlite::Connection::open(&f.database)
        .unwrap()
        .execute(
            "UPDATE request_passthrough SET state='answered', bound_session='session',
             bound_turn='turn', bound_at_ms=?2, answer_text='an answer', answer_truncated=0,
             answer_provider='claude', answer_ended_ms=?2, settled_at_ms=?2 WHERE request_id=?1",
            rusqlite::params![id, at as i64],
        )
        .unwrap();
    assert_eq!(changed, 1, "{id} has a capture");
}

fn query(originator: &str, limit: u64, before: Option<AnswerCursor>) -> AnswersQuery {
    AnswersQuery {
        originator_id: originator.into(),
        before,
        limit,
    }
}

fn shown(answers: &[Answer]) -> Vec<(&str, &str, u64)> {
    answers
        .iter()
        .map(|a| (a.request_id.as_str(), a.answer.as_str(), a.answered_at_ms))
        .collect()
}

#[test]
fn finals_and_answered_turns_share_one_order_and_one_cursor_and_nothing_else_counts() {
    let mut f = Fixture::new();
    let ben = identity(&mut f, "Ben");
    mark_user(&f, &ben);
    let other = identity(&mut f, "Other");
    mark_user(&f, &other);

    // Ben's finals and turns, interleaved by arrival; two arrive in the same millisecond.
    ask(&mut f, "f-old", &ben);
    ask(&mut f, "f-tie", &ben);
    ask(&mut f, "f-new", &ben);
    typed(&mut f, &ben, "t-mid");
    typed(&mut f, &ben, "t-tie");
    typed(&mut f, &ben, "t-new");
    final_at(&mut f, "f-old", NOW_MS + 10);
    turn_at(&f, "t-mid", NOW_MS + 20);
    final_at(&mut f, "f-tie", NOW_MS + 30);
    turn_at(&f, "t-tie", NOW_MS + 30);
    final_at(&mut f, "f-new", NOW_MS + 40);
    turn_at(&f, "t-new", NOW_MS + 50);

    // Not answers: an unanswered request, a turn nobody answered, one that was never typed,
    // and everything another identity sent.
    ask(&mut f, "unanswered", &ben);
    typed(&mut f, &ben, "t-pending");
    ask(&mut f, "foreign-final", &other);
    final_at(&mut f, "foreign-final", NOW_MS + 60);
    typed(&mut f, &other, "foreign-turn");
    turn_at(&f, "foreign-turn", NOW_MS + 70);
    let target = f.identity_id.clone();
    claimed_passthrough(&mut f, &target, &ben, "t-refused", "not typed");
    service(&mut f)
        .settle_request_delivery("t-refused", WakeState::Unavailable)
        .unwrap();
    f.set_now(NOW_MS + 100);
    let before = request_snapshot(&f.database);

    let expected = [
        ("t-new", "turn", NOW_MS + 50),
        ("f-new", "final", NOW_MS + 40),
        // The same millisecond: the larger request id first, as `results` orders.
        ("t-tie", "turn", NOW_MS + 30),
        ("f-tie", "final", NOW_MS + 30),
        ("t-mid", "turn", NOW_MS + 20),
        ("f-old", "final", NOW_MS + 10),
    ];
    let all = service(&mut f)
        .request_answers(query(&ben, 50, None))
        .unwrap();
    assert_eq!(shown(&all.items), expected);
    assert_eq!(all.next_before, None);
    let first = &all.items[0];
    assert_eq!(first.originator, Originator::Explicit(ben.clone()));
    assert_eq!(
        first.recipient_identity_id.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(first.kind.as_str(), "request");

    // Every page size walks the same list: no entry is repeated or skipped at a boundary,
    // including between the two that share a millisecond.
    for size in 1..=5 {
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = service(&mut f)
                .request_answers(query(&ben, size, cursor))
                .unwrap();
            assert!(page.items.len() as u64 <= size);
            seen.extend(
                page.items
                    .iter()
                    .map(|a| (a.request_id.clone(), a.answer.as_str(), a.answered_at_ms)),
            );
            cursor = page.next_before;
            if cursor.is_none() {
                break;
            }
        }
        let seen = seen
            .iter()
            .map(|(id, kind, at)| (id.as_str(), *kind, *at))
            .collect::<Vec<_>>();
        assert_eq!(seen, expected, "page size {size}");
    }

    // The other identity sees only its own, and an identity that sent nothing sees none.
    let theirs = service(&mut f)
        .request_answers(query(&other, 50, None))
        .unwrap();
    assert_eq!(
        shown(&theirs.items),
        [
            ("foreign-turn", "turn", NOW_MS + 70),
            ("foreign-final", "final", NOW_MS + 60)
        ]
    );
    // The reading is not an acknowledgement: nothing in the request tables moved.
    assert_eq!(request_snapshot(&f.database), before);
    let nobody = identity(&mut f, "Nobody");
    assert!(
        service(&mut f)
            .request_answers(query(&nobody, 50, None))
            .unwrap()
            .items
            .is_empty()
    );
    f.storage.close().unwrap();
}

#[test]
fn the_answers_page_is_bounded_and_refuses_what_it_cannot_answer() {
    let mut f = Fixture::new();
    let ben = identity(&mut f, "Ben");
    for (limit, cursor_at, cursor_id) in [
        (0, None, ""),
        (51, None, ""),
        (8, Some(0), "req_1"),
        (8, Some(1), ""),
        (8, Some(9_007_199_254_740_992), "req_1"),
    ] {
        let before = cursor_at.map(|answered_at_ms| AnswerCursor {
            answered_at_ms,
            request_id: cursor_id.into(),
        });
        assert!(
            matches!(
                service(&mut f).request_answers(query(&ben, limit, before)),
                Err(RequestError::Invalid(_))
            ),
            "{limit} {cursor_at:?}"
        );
    }
    assert!(matches!(
        service(&mut f).request_answers(query("not-an-id", 8, None)),
        Err(RequestError::Invalid(_))
    ));
    // A full page at the bound is accepted.
    assert!(
        service(&mut f)
            .request_answers(query(&ben, 50, None))
            .is_ok()
    );
    f.storage.close().unwrap();
}

#[test]
fn an_answered_request_past_its_retention_is_not_listed() {
    use super::support::DAY_MS;
    let mut f = Fixture::new();
    let ben = identity(&mut f, "Ben");
    mark_user(&f, &ben);
    ask(&mut f, "kept", &ben);
    final_at(&mut f, "kept", NOW_MS + 5);
    typed(&mut f, &ben, "turn");
    turn_at(&f, "turn", NOW_MS + 6);
    assert_eq!(
        service(&mut f)
            .request_answers(query(&ben, 50, None))
            .unwrap()
            .items
            .len(),
        2
    );
    f.set_now(NOW_MS + 400 * DAY_MS);
    assert!(
        service(&mut f)
            .request_answers(query(&ben, 50, None))
            .unwrap()
            .items
            .is_empty()
    );
    f.storage.close().unwrap();
}

#[test]
fn answer_kinds_name_what_a_reader_sees() {
    assert_eq!(AnswerKind::Final.as_str(), "final");
    assert_eq!(AnswerKind::Turn.as_str(), "turn");
}
