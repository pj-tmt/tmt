//! A request with a remote route has an originator and no local recipient (#4536). The readers
//! that key on a recipient must neither fail on it nor list it as someone's work.
use super::support::{Fixture, NOW_MS, service};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        AttemptStatus, Originator, PrepareRequest, RequestKind, RequestOrigin, RequestRoute,
        Settlement,
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
