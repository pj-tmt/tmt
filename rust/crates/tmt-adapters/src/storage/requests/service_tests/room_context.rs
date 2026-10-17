use super::support::{Fixture, NOW_MS, count_rows, service};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        Originator, PrepareRequest, RequestError, RequestKind, RequestRoute, ResponseProof,
        SubmitResponse, correlation,
    },
    room::{MembershipChange, RoomRepository, RoomWrite},
};

const DESIGN: &str = "11111111-1111-4111-8111-111111111111";
const REVIEW: &str = "22222222-2222-4222-8222-222222222222";

#[test]
fn preparation_rechecks_room_membership_before_any_request_or_cadence_effect() {
    for retire in [false, true] {
        check_preparation_after_room_change(retire);
    }
}

fn check_preparation_after_room_change(retire: bool) {
    use super::support::{endpoint, prepare_input};
    for inbox in [false, true] {
        let mut fixture = Fixture::new();
        let id = fixture.identity_id.clone();
        fixture
            .storage
            .save_meeting_room(
                DESIGN,
                RoomWrite {
                    expected_revision: 0,
                    name: "Design".into(),
                    member_ids: vec![id.clone()],
                },
            )
            .unwrap();
        let make_request = |request_id| {
            if inbox {
                input(&id, &id, request_id, Some(DESIGN))
            } else {
                let mut request = prepare_input(
                    &fixture,
                    request_id,
                    endpoint("%1", 81),
                    false,
                    NOW_MS + 3_600_000,
                    Originator::Explicit(id.clone()),
                    true,
                );
                request.room_id = Some(DESIGN.into());
                request
            }
        };
        let request = make_request("accepted");
        let stale_request = make_request("rejected");
        let positive = if inbox {
            service(&mut fixture).enqueue(request, "accepted-attempt".into(), 90)
        } else {
            service(&mut fixture).prepare(request, "accepted-attempt".into(), 90)
        };
        positive.unwrap();
        let oracle = rusqlite::Connection::open(&fixture.database).unwrap();
        let attention_revision =
            || {
                oracle.query_row(
            "SELECT latest_revision FROM request_attention_identities WHERE identity_id=?",
            [&id], |row| row.get::<_, i64>(0),
        ).unwrap()
            };
        // Self-addressed inbox publication advances both sender and recipient
        // attention; pane preparation has only the sender event.
        let before = attention_revision();
        assert_eq!(before, if inbox { 2 } else { 1 });
        // The caller retains an earlier valid selection while membership changes.
        if retire {
            fixture.storage.retire_meeting_room(DESIGN, 1).unwrap();
        } else {
            fixture
                .storage
                .change_meeting_membership(DESIGN, &id, MembershipChange::Leave)
                .unwrap();
        }
        let rejected = if inbox {
            service(&mut fixture).enqueue(stale_request, "rejected-attempt".into(), 90)
        } else {
            service(&mut fixture).prepare(stale_request, "rejected-attempt".into(), 90)
        };
        assert!(matches!(
            rejected,
            Err(RequestError::RoomRecipientNotMember)
        ));
        assert_eq!(count_rows(&fixture.database, "request_attempts"), 1);
        assert_eq!(attention_revision(), before);
        let cadence: i64 = oracle
            .query_row(
                "SELECT COALESCE(SUM(reserved_count),0) FROM preamble_counters",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cadence, if inbox { 0 } else { 1 });
        assert!(
            service(&mut fixture)
                .get_context("rejected")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            service(&mut fixture)
                .get_context("accepted")
                .unwrap()
                .unwrap()
                .attempt
                .room_id
                .as_deref(),
            Some(DESIGN)
        );
    }
}

fn input(sender: &str, receiver: &str, id: &str, room: Option<&str>) -> PrepareRequest {
    PrepareRequest {
        origin: tmt_core::request::RequestOrigin::Person,
        kind: RequestKind::Request,
        room_id: room.map(str::to_owned),
        request_id: id.into(),
        message: format!("Review {id}"),
        route: RequestRoute::Inbox {
            recipient_identity_id: receiver.into(),
        },
        wait: false,
        expires_at_ms: NOW_MS + 3_600_000,
        originator: Originator::Explicit(sender.into()),
        recipient_identity_id: Some(receiver.into()),
        preamble: None,
    }
}

#[test]
fn historical_room_scope_filters_before_pagination_and_survives_leave_and_reply() {
    let mut fixture = Fixture::new();
    let sender = fixture.identity_id.clone();
    let receiver = create_or_resolve(&mut fixture.storage, "Receiver", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    for (id, name) in [(DESIGN, "Design"), (REVIEW, "Review")] {
        fixture
            .storage
            .save_meeting_room(
                id,
                RoomWrite {
                    expected_revision: 0,
                    name: name.into(),
                    member_ids: vec![receiver.clone()],
                },
            )
            .unwrap();
    }
    for (id, room) in [
        ("a1", Some(DESIGN)),
        ("b", Some(REVIEW)),
        ("a2", Some(DESIGN)),
        ("plain", None),
    ] {
        service(&mut fixture)
            .enqueue(
                input(&sender, &receiver, id, room),
                format!("attempt-{id}"),
                90,
            )
            .unwrap();
    }
    fixture
        .storage
        .change_meeting_membership(DESIGN, &receiver, MembershipChange::Leave)
        .unwrap();
    let first = service(&mut fixture)
        .list_incoming(&receiver, Some(DESIGN), Some(1), None)
        .unwrap();
    assert_eq!(first.items[0].exchange.request_id, "a1");
    let next = service(&mut fixture)
        .list_incoming(&receiver, Some(DESIGN), Some(1), first.next_after)
        .unwrap();
    assert_eq!(next.items.len(), 1);
    assert_eq!(next.items[0].exchange.request_id, "a2");
    assert_eq!(next.items[0].exchange.room_id.as_deref(), Some(DESIGN));
    assert_eq!(next.next_after, None);
    assert_eq!(
        service(&mut fixture)
            .incoming_watermark(&receiver, Some(DESIGN))
            .unwrap(),
        next.items[0].exchange.revision
    );
    assert!(
        service(&mut fixture)
            .incoming_watermark(&receiver, None)
            .unwrap()
            > next.items[0].exchange.revision
    );
    for item in first.items.iter().chain(next.items.iter()) {
        service(&mut fixture)
            .acknowledge_incoming_request(
                &receiver,
                &item.exchange.request_id,
                item.exchange.revision,
            )
            .unwrap();
    }
    assert_eq!(
        service(&mut fixture)
            .incoming_watermark(&receiver, Some(DESIGN))
            .unwrap(),
        0
    );
    assert!(
        service(&mut fixture)
            .list_incoming(&receiver, Some(DESIGN), None, None)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        service(&mut fixture)
            .list_incoming(&receiver, None, None, None)
            .unwrap()
            .items
            .len(),
        2
    );

    let context = service(&mut fixture).get_context("a1").unwrap().unwrap();
    assert_eq!(context.attempt.room_id.as_deref(), Some(DESIGN));
    service(&mut fixture)
        .submit_response(SubmitResponse {
            request_id: "a1".into(),
            proof: ResponseProof::Compact(correlation::response_token(
                "a1",
                "attempt-a1",
                &context.attempt.route,
            )),
            body: "Review complete after leaving.".into(),
        })
        .unwrap();
    let replies = service(&mut fixture)
        .list_incoming(&sender, Some(DESIGN), None, None)
        .unwrap();
    assert_eq!(replies.items.len(), 1);
    assert_eq!(
        replies.items[0].kind,
        tmt_core::request::attention::IncomingKind::Response
    );
    assert_eq!(replies.items[0].exchange.room_id.as_deref(), Some(DESIGN));
    assert_eq!(
        service(&mut fixture)
            .incoming_watermark(&sender, Some(DESIGN))
            .unwrap(),
        replies.items[0].exchange.revision
    );
    assert_eq!(
        service(&mut fixture)
            .incoming_watermark(&sender, Some(REVIEW))
            .unwrap(),
        0
    );
    assert!(
        service(&mut fixture)
            .list_incoming(&sender, Some(REVIEW), None, None)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn invalid_room_scope_rejects_without_creating_attention_or_requests() {
    let mut fixture = Fixture::new();
    let identity = fixture.identity_id.clone();
    for invalid in ["Design", "", "00000000-0000-0000-0000-000000000000"] {
        assert!(matches!(
            service(&mut fixture).enqueue(
                input(&identity, &identity, "invalid", Some(invalid)),
                "invalid-attempt".into(),
                90
            ),
            Err(RequestError::Invalid(_))
        ));
        assert!(
            service(&mut fixture)
                .incoming_watermark(&identity, Some(invalid))
                .is_err()
        );
        assert!(
            service(&mut fixture)
                .list_incoming(&identity, Some(invalid), None, None)
                .is_err()
        );
    }
    assert_eq!(count_rows(&fixture.database, "request_attempts"), 0);
    assert_eq!(
        count_rows(&fixture.database, "request_attention_identities"),
        0
    );
}
