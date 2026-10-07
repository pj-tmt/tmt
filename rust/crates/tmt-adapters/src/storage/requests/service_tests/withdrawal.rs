use super::support::{
    Fixture, NOW_MS, count_rows, endpoint, prepare_input, request_snapshot, service,
};
use crate::storage::{
    Storage, StorageError,
    test_support::{Operation, concurrent_pair},
};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        Originator, RequestError, RequestKind, RequestRoute, RequestService, ResponseLookup,
        ResponseProof, ResponseRejection, SubmitResponse, WithdrawalRejection,
        attention::FinalState,
        correlation::response_token,
        history::{HistoryQuery, HistoryScope},
    },
};

fn enqueue(fixture: &mut Fixture, id: &str, originator: Originator, kind: RequestKind) {
    let mut input = prepare_input(
        fixture,
        id,
        endpoint("%1", 42),
        true,
        NOW_MS + 3_600_000,
        originator,
        false,
    );
    input.route = RequestRoute::Inbox {
        recipient_identity_id: fixture.identity_id.clone(),
    };
    input.kind = kind;
    if kind == RequestKind::Announcement {
        input.wait = false;
    }
    service(fixture)
        .enqueue(input, format!("attempt-{id}"), 7)
        .unwrap();
}

fn response(fixture: &Fixture, id: &str) -> SubmitResponse {
    SubmitResponse {
        request_id: id.into(),
        proof: ResponseProof::Compact(response_token(
            id,
            &format!("attempt-{id}"),
            &RequestRoute::Inbox {
                recipient_identity_id: fixture.identity_id.clone(),
            },
        )),
        body: "exact final\r\n".into(),
    }
}

#[test]
fn withdrawal_is_durable_idempotent_preserves_prompt_and_excludes_open_pages() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    for id in ["old", "new", "next"] {
        enqueue(
            &mut fixture,
            id,
            Originator::Explicit(owner.clone()),
            RequestKind::Request,
        );
        fixture.set_now(NOW_MS + if id == "old" { 1 } else { 2 });
    }
    let first = service(&mut fixture)
        .withdraw_request(&owner, "old", "already merged")
        .unwrap();
    assert!(first.changed);
    assert_eq!(first.withdrawal.withdrawn_at_ms, NOW_MS + 2);
    let snapshot = request_snapshot(&fixture.database);
    fixture.set_now(NOW_MS + 10);
    let repeated = service(&mut fixture)
        .withdraw_request(&owner, "old", "already merged")
        .unwrap();
    assert!(!repeated.changed);
    assert_eq!(first.withdrawal, repeated.withdrawal);
    assert_eq!(request_snapshot(&fixture.database), snapshot);
    assert!(matches!(
        service(&mut fixture).withdraw_request(&owner, "old", "other reason"),
        Err(RequestError::Withdrawal(WithdrawalRejection::Conflict))
    ));
    assert_eq!(request_snapshot(&fixture.database), snapshot);
    let page = service(&mut fixture)
        .open_requests(&owner, None, Some(1))
        .unwrap();
    assert_eq!(page.items[0].request_id, "new");
    assert!(
        page.more,
        "withdrawn rows must be excluded before the query limit"
    );
    let shown = service(&mut fixture).show_exchange(&owner, "old").unwrap();
    assert!(shown.exchange.settled);
    assert!(
        matches!(shown.exchange.final_state, FinalState::Withdrawn(ref value) if value == &first.withdrawal)
    );
    assert!(
        matches!(shown.prompt, tmt_core::request::RequestPrompt::Retained(ref prompt) if prompt.message == "prompt for old")
    );
    let history = service(&mut fixture)
        .request_history(HistoryQuery {
            scope: HistoryScope::Recipient {
                identity_id: owner.clone(),
                room_id: None,
            },
            before: None,
            limit: 20,
        })
        .unwrap();
    assert!(history.items.iter().any(|row| row.item.request_id == "old"
        && matches!(row.item.final_state, FinalState::Withdrawn(_))));
    assert!(
        matches!(service(&mut fixture).get_response("old").unwrap(), ResponseLookup::Withdrawn(ref value) if value == &first.withdrawal)
    );
    let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
    let stored: (Option<i64>, String, i64, i64, Option<i64>) = oracle.query_row(
        "SELECT response_submitted_at_ms, withdrawal_reason, withdrawn_at_ms, wait_active, wait_released_at_ms FROM request_attempts WHERE request_id='old'", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).unwrap();
    assert_eq!(
        stored,
        (
            None,
            "already merged".into(),
            (NOW_MS + 2) as i64,
            0,
            Some((NOW_MS + 2) as i64)
        )
    );
    assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
    assert_eq!(count_rows(&fixture.database, "request_notifications"), 0);
}

#[test]
fn withdrawal_authority_and_reason_byte_bounds_refuse_without_mutation() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    let stranger = create_or_resolve(&mut fixture.storage, "Stranger", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    enqueue(
        &mut fixture,
        "owned",
        Originator::Verified(owner.clone()),
        RequestKind::Request,
    );
    enqueue(
        &mut fixture,
        "anonymous",
        Originator::Unknown,
        RequestKind::Request,
    );
    enqueue(
        &mut fixture,
        "one-way",
        Originator::Explicit(owner.clone()),
        RequestKind::Announcement,
    );
    let before = request_snapshot(&fixture.database);
    for (caller, id, reason, expected) in [
        (
            stranger.as_str(),
            "owned",
            "obsolete",
            WithdrawalRejection::NotOriginator,
        ),
        (
            owner.as_str(),
            "anonymous",
            "obsolete",
            WithdrawalRejection::NotOriginator,
        ),
        (
            owner.as_str(),
            "one-way",
            "obsolete",
            WithdrawalRejection::NotRequired,
        ),
        (
            owner.as_str(),
            "missing",
            "obsolete",
            WithdrawalRejection::NotFound,
        ),
        ("", "owned", "obsolete", WithdrawalRejection::InputInvalid),
        (
            owner.as_str(),
            "owned",
            "",
            WithdrawalRejection::InputInvalid,
        ),
    ] {
        assert!(
            matches!(service(&mut fixture).withdraw_request(caller, id, reason), Err(RequestError::Withdrawal(actual)) if actual == expected)
        );
        assert_eq!(request_snapshot(&fixture.database), before);
    }
    let oversized = "🙂".repeat(257);
    assert!(matches!(
        service(&mut fixture).withdraw_request(&owner, "owned", &oversized),
        Err(RequestError::Withdrawal(WithdrawalRejection::InputInvalid))
    ));
    assert_eq!(request_snapshot(&fixture.database), before);
    service(&mut fixture)
        .withdraw_request(&owner, "owned", &"🙂".repeat(256))
        .unwrap();
}

#[test]
fn reply_and_withdrawal_each_win_when_their_transaction_commits_first() {
    for reply_first in [true, false] {
        let mut fixture = Fixture::new();
        let owner = fixture.identity_id.clone();
        enqueue(
            &mut fixture,
            "race",
            Originator::Explicit(owner.clone()),
            RequestKind::Request,
        );
        let mut other = Storage::open(&fixture.database).unwrap();
        let reply = response(&fixture, "race");
        if reply_first {
            RequestService::new(&mut other, || NOW_MS + 1)
                .submit_response(reply)
                .unwrap();
            let before = request_snapshot(&fixture.database);
            assert!(matches!(
                service(&mut fixture).withdraw_request(&owner, "race", "obsolete"),
                Err(RequestError::Withdrawal(WithdrawalRejection::AlreadyFinal))
            ));
            assert_eq!(request_snapshot(&fixture.database), before);
            assert_eq!(count_rows(&fixture.database, "request_responses"), 1);
            // Expired body removal cannot erase the permanent-within-retention
            // completion marker and turn a final into a withdrawable request.
            rusqlite::Connection::open(&fixture.database)
                .unwrap()
                .execute("DELETE FROM request_responses", [])
                .unwrap();
            assert!(matches!(
                service(&mut fixture).withdraw_request(&owner, "race", "obsolete"),
                Err(RequestError::Withdrawal(WithdrawalRejection::AlreadyFinal))
            ));
        } else {
            service(&mut fixture)
                .withdraw_request(&owner, "race", "obsolete")
                .unwrap();
            let before = request_snapshot(&fixture.database);
            assert!(matches!(
                RequestService::new(&mut other, || NOW_MS + 1).submit_response(reply),
                Err(RequestError::Response(ResponseRejection::Withdrawn))
            ));
            assert_eq!(request_snapshot(&fixture.database), before);
            assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
        }
        other.close().unwrap();
    }
}

#[test]
fn simultaneous_reply_and_withdrawal_have_exactly_one_winner() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    enqueue(
        &mut fixture,
        "race",
        Originator::Explicit(owner.clone()),
        RequestKind::Request,
    );
    let reply = response(&fixture, "race");
    let withdraw: Operation<Result<bool, RequestError<StorageError>>> = Box::new(move |storage| {
        RequestService::new(storage, || NOW_MS)
            .withdraw_request(&owner, "race", "obsolete")
            .map(|result| result.changed)
    });
    let submit: Operation<Result<bool, RequestError<StorageError>>> = Box::new(move |storage| {
        RequestService::new(storage, || NOW_MS)
            .submit_response(reply)
            .map(|_| true)
    });
    let [withdrawn, submitted] = concurrent_pair(&fixture.database, [withdraw, submit]);
    assert_ne!(withdrawn.is_ok(), submitted.is_ok());
    if withdrawn.is_ok() {
        assert!(matches!(
            submitted,
            Err(RequestError::Response(ResponseRejection::Withdrawn))
        ));
        assert_eq!(count_rows(&fixture.database, "request_responses"), 0);
    } else {
        assert!(matches!(
            withdrawn,
            Err(RequestError::Withdrawal(WithdrawalRejection::AlreadyFinal))
        ));
        assert_eq!(count_rows(&fixture.database, "request_responses"), 1);
    }
}

#[test]
fn withdrawn_requests_cannot_reserve_timeout_or_reply_notifications() {
    let mut fixture = Fixture::new();
    let owner = fixture.identity_id.clone();
    enqueue(
        &mut fixture,
        "notice",
        Originator::Explicit(owner.clone()),
        RequestKind::Request,
    );
    service(&mut fixture)
        .enable_notifications(
            "notice",
            tmt_core::request::notification::NotificationPolicy {
                deadline_ms: NOW_MS + 1,
                timeout_ms: 1,
                waiter: None,
            },
        )
        .unwrap();
    let notification = service(&mut fixture).notification("notice").unwrap();
    service(&mut fixture)
        .withdraw_request(&owner, "notice", "obsolete")
        .unwrap();
    fixture.set_now(NOW_MS + 2);
    assert!(
        service(&mut fixture)
            .claim_timeout_hint("notice")
            .unwrap()
            .is_none()
    );
    assert!(
        service(&mut fixture)
            .finish_wait("attempt-notice", false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        service(&mut fixture).notification("notice").unwrap(),
        notification
    );
    let reply = response(&fixture, "notice");
    assert!(matches!(
        service(&mut fixture).submit_response_with_hint(reply, None),
        Err(RequestError::Response(ResponseRejection::Withdrawn))
    ));
    assert_eq!(
        service(&mut fixture).notification("notice").unwrap(),
        notification
    );
}
