//! A request with a remote route has an originator and no local recipient (#4536). The readers
//! that key on a recipient must neither fail on it nor list it as someone's work.
use super::support::{Fixture, NOW_MS, count_rows, service};
use crate::storage::Storage;
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        AttemptStatus, Originator, PrepareRequest, RequestError, RequestKind, RequestOrigin,
        RequestRoute, ResponseLookup, ResponseProof, ResponseRejection, Settlement, SubmitResponse,
        attention::FinalState,
        correlation::response_token,
        history::{AnswersQuery, HistoryQuery, HistoryScope},
    },
};

const REQUEST: &str = "req_55555555-5555-4555-8555-555555555555";

fn remote_route() -> RequestRoute {
    RequestRoute::Remote {
        remote: "bob".into(),
        target_id: Some("7f0c2f2e-2b1a-4c1e-9d2a-0a1b2c3d4e5f".into()),
        target_name: "reviewer".into(),
        sender_name: "Ben".into(),
    }
}

#[test]
fn a_remote_request_belongs_to_its_originator_alone_and_every_reader_copes() {
    let mut f = Fixture::new();
    let ben = create_or_resolve(&mut f.storage, "Ben", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    let other = f.identity_id.clone();
    let prepared = service(&mut f)
        .prepare(
            PrepareRequest {
                origin: RequestOrigin::Person,
                room_id: None,
                kind: RequestKind::Request,
                request_id: REQUEST.into(),
                message: "hello across machines".into(),
                route: remote_route(),
                wait: false,
                expires_at_ms: NOW_MS + 3_600_001,
                originator: Originator::Explicit(ben.clone()),
                recipient_identity_id: None,
                preamble: None,
            },
            "attempt-remote".into(),
            7,
        )
        .expect("prepare a remote request");

    // The route round-trips through storage, exactly, and nobody is its recipient.
    let detail = service(&mut f).request_detail(REQUEST).unwrap();
    assert_eq!(detail.item.recipient_identity_id, None);
    assert_eq!(detail.item.delivery, AttemptStatus::Prepared);
    assert_eq!(detail.item.recipient_acknowledged, None);

    service(&mut f).begin_send(&prepared.attempt_id).unwrap();
    service(&mut f)
        .settle(&prepared.attempt_id, Settlement::Sent)
        .unwrap();
    assert_eq!(
        service(&mut f)
            .request_detail(REQUEST)
            .unwrap()
            .item
            .delivery,
        AttemptStatus::Sent
    );

    // It is the originator's work: it shows in their exchanges, and nowhere else.
    let exchanges = service(&mut f).list_exchanges(&ben, None, None).unwrap();
    assert_eq!(exchanges.items.len(), 1);
    assert!(service(&mut f).show_exchange(&ben, REQUEST).is_ok());
    assert!(
        service(&mut f)
            .list_incoming(&other, None, None, None)
            .unwrap()
            .items
            .is_empty()
    );
    for identity in [&ben, &other] {
        let history = service(&mut f)
            .request_history(HistoryQuery {
                scope: HistoryScope::Recipient {
                    identity_id: identity.clone(),
                    room_id: None,
                },
                before: None,
                limit: 20,
            })
            .unwrap();
        assert!(history.items.is_empty(), "no recipient history lists it");
    }
    // It has no answer yet, so neither the results nor the answers view lists it.
    let results = service(&mut f)
        .request_history(HistoryQuery {
            scope: HistoryScope::OriginatorResults {
                originator_id: ben.clone(),
                recipient_id: None,
            },
            before: None,
            limit: 20,
        })
        .unwrap();
    assert!(results.items.is_empty());
    let answers = service(&mut f)
        .request_answers(AnswersQuery {
            originator_id: ben.clone(),
            before: None,
            limit: 20,
        })
        .unwrap();
    assert!(answers.items.is_empty());
    f.storage.close().unwrap();
}

#[test]
fn a_remote_request_needs_a_confirmed_sender_a_request_kind_and_a_canonical_target() {
    let mut f = Fixture::new();
    let ben = create_or_resolve(&mut f.storage, "Ben", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    let mut refused = |kind: RequestKind, originator: Originator, target_id: &str| {
        let route = RequestRoute::Remote {
            remote: "bob".into(),
            target_id: Some(target_id.into()),
            target_name: "reviewer".into(),
            sender_name: "Ben".into(),
        };
        service(&mut f)
            .prepare(
                PrepareRequest {
                    origin: RequestOrigin::Person,
                    room_id: None,
                    kind,
                    request_id: REQUEST.into(),
                    message: "hello".into(),
                    route,
                    wait: false,
                    expires_at_ms: NOW_MS + 3_600_001,
                    originator,
                    recipient_identity_id: None,
                    preamble: None,
                },
                "attempt-refused".into(),
                7,
            )
            .is_err()
    };
    let canonical = "7f0c2f2e-2b1a-4c1e-9d2a-0a1b2c3d4e5f";
    assert!(refused(
        RequestKind::Request,
        Originator::Unknown,
        canonical
    ));
    assert!(refused(
        RequestKind::Announcement,
        Originator::Explicit(ben.clone()),
        canonical
    ));
    assert!(refused(
        RequestKind::Request,
        Originator::Explicit(ben.clone()),
        "7F0C2F2E-2B1A-4C1E-9D2A-0A1B2C3D4E5F"
    ));
    assert!(!refused(
        RequestKind::Request,
        Originator::Verified(ben),
        canonical
    ));
    f.storage.close().unwrap();
}

#[test]
fn a_remote_request_takes_one_final_with_its_receipt_and_every_reader_lists_it() {
    let mut f = Fixture::new();
    let ben = create_or_resolve(&mut f.storage, "Ben", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    let prepared = service(&mut f)
        .prepare(
            PrepareRequest {
                origin: RequestOrigin::Person,
                room_id: None,
                kind: RequestKind::Request,
                request_id: REQUEST.into(),
                message: "hello across machines".into(),
                route: remote_route(),
                wait: false,
                expires_at_ms: NOW_MS + 3_600_001,
                originator: Originator::Explicit(ben.clone()),
                recipient_identity_id: None,
                preamble: None,
            },
            "attempt-remote".into(),
            7,
        )
        .unwrap();
    service(&mut f).begin_send(&prepared.attempt_id).unwrap();
    service(&mut f)
        .settle(&prepared.attempt_id, Settlement::Sent)
        .unwrap();
    let submission = |token: [u8; 16], body: &str| SubmitResponse {
        request_id: REQUEST.into(),
        proof: ResponseProof::Compact(token),
        body: body.into(),
    };
    let receipt = response_token(REQUEST, &prepared.attempt_id, &remote_route());

    // Another request's receipt is not this one's.
    let foreign = response_token(REQUEST, "another-attempt", &remote_route());
    assert!(matches!(
        service(&mut f).submit_response(submission(foreign, "no")),
        Err(RequestError::Response(ResponseRejection::ReceiptMismatch))
    ));
    assert_eq!(count_rows(&f.database, "request_responses"), 0);

    let first = service(&mut f)
        .submit_response(submission(receipt, "peer answer"))
        .unwrap();
    assert_eq!(first.route, remote_route());
    assert_eq!(count_rows(&f.database, "request_responses"), 1);

    // The stored final keeps the route, so a retry after a restart still proves its receipt.
    f.storage.close().unwrap();
    f.storage = Storage::open(&f.database).unwrap();
    let ResponseLookup::Available(stored) = service(&mut f).get_response(REQUEST).unwrap() else {
        panic!("the final is retained");
    };
    assert_eq!(stored.route, remote_route());
    assert_eq!(stored.body, "peer answer");
    assert_eq!(
        service(&mut f)
            .submit_response(submission(receipt, "peer answer"))
            .unwrap(),
        first
    );
    assert!(matches!(
        service(&mut f).submit_response(submission(receipt, "another answer")),
        Err(RequestError::Response(ResponseRejection::Conflict))
    ));
    assert!(matches!(
        service(&mut f).submit_response(submission(foreign, "peer answer")),
        Err(RequestError::Response(ResponseRejection::ReceiptMismatch))
    ));

    // Result, exchanges, results and the answers view all carry it.
    let detail = service(&mut f).request_detail(REQUEST).unwrap();
    assert_eq!(detail.item.recipient_identity_id, None);
    let exchange = service(&mut f).show_exchange(&ben, REQUEST).unwrap();
    assert!(matches!(
        exchange.exchange.final_state,
        FinalState::Retained { body_bytes: 11, .. }
    ));
    let results = service(&mut f)
        .request_history(HistoryQuery {
            scope: HistoryScope::OriginatorResults {
                originator_id: ben.clone(),
                recipient_id: None,
            },
            before: None,
            limit: 20,
        })
        .unwrap();
    assert_eq!(results.items.len(), 1);
    let answers = service(&mut f)
        .request_answers(AnswersQuery {
            originator_id: ben,
            before: None,
            limit: 20,
        })
        .unwrap();
    assert_eq!(answers.items.len(), 1);
    f.storage.close().unwrap();
}

#[test]
fn an_expired_remote_final_whose_attempt_is_gone_reads_as_absent() {
    let mut f = Fixture::new();
    let ben = create_or_resolve(&mut f.storage, "Ben", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    let prepared = service(&mut f)
        .prepare(
            PrepareRequest {
                origin: RequestOrigin::Person,
                room_id: None,
                kind: RequestKind::Request,
                request_id: REQUEST.into(),
                message: "hello across machines".into(),
                route: remote_route(),
                wait: false,
                expires_at_ms: NOW_MS + 3_600_001,
                originator: Originator::Explicit(ben),
                recipient_identity_id: None,
                preamble: None,
            },
            "attempt-remote".into(),
            7,
        )
        .unwrap();
    service(&mut f).begin_send(&prepared.attempt_id).unwrap();
    service(&mut f)
        .settle(&prepared.attempt_id, Settlement::Sent)
        .unwrap();
    service(&mut f)
        .submit_response(SubmitResponse {
            request_id: REQUEST.into(),
            proof: ResponseProof::Compact(response_token(
                REQUEST,
                &prepared.attempt_id,
                &remote_route(),
            )),
            body: "peer answer".into(),
        })
        .unwrap();

    // Housekeeping lets an attempt go once its finals have expired, even if a batch limit left
    // an expired final behind; the final then has no route to read.
    f.storage.close().unwrap();
    let oracle = rusqlite::Connection::open(&f.database).unwrap();
    oracle
        .execute(
            "DELETE FROM request_attempts WHERE request_id = ?",
            [REQUEST],
        )
        .unwrap();
    drop(oracle);
    f.storage = Storage::open(&f.database).unwrap();
    assert_eq!(count_rows(&f.database, "request_responses"), 1);
    assert_eq!(
        service(&mut f).get_response(REQUEST).unwrap(),
        ResponseLookup::Unavailable
    );
    f.storage.close().unwrap();
}
