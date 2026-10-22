//! A passthrough is a person's own text typed into the agent's pane (#4240). It is a request
//! only for its sender: the recipient agent has no open request, attention or reply to give.
use super::support::{Fixture, NOW_MS, service};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{
        Originator, PrepareRequest, RequestError, RequestKind, RequestRoute,
        digest::{DeliveryMode, DeliveryPolicy},
    },
};

fn identity(f: &mut Fixture, name: &str) -> String {
    create_or_resolve(&mut f.storage, name, Lifetime::Saved)
        .unwrap()
        .identity
        .id
}

fn mark_user(f: &Fixture, id: &str) {
    rusqlite::Connection::open(&f.database)
        .unwrap()
        .execute(
            "INSERT INTO identity_metadata(identity_id, key, value) VALUES(?1, 'user', 'true')",
            [id],
        )
        .unwrap();
}

fn request(target: &str, sender: &str, id: &str) -> PrepareRequest {
    PrepareRequest {
        origin: tmt_core::request::RequestOrigin::Person,
        room_id: None,
        kind: RequestKind::Request,
        request_id: id.into(),
        message: format!("typed text for {id}"),
        route: RequestRoute::Inbox {
            recipient_identity_id: target.into(),
        },
        wait: true,
        expires_at_ms: NOW_MS + 3_600_000,
        originator: Originator::Explicit(sender.into()),
        recipient_identity_id: Some(target.into()),
        preamble: None,
    }
}

fn policy(mode: DeliveryMode) -> DeliveryPolicy {
    DeliveryPolicy {
        mode,
        ..DeliveryPolicy::default()
    }
}

fn enqueue(
    f: &mut Fixture,
    target: &str,
    sender: &str,
    id: &str,
    mode: DeliveryMode,
) -> Result<tmt_core::request::PreparedRequest, RequestError<crate::storage::StorageError>> {
    service(f).enqueue_delivery(
        request(target, sender, id),
        format!("attempt-{id}"),
        7,
        policy(mode),
    )
}

#[test]
fn only_a_person_may_pass_text_through_and_only_to_an_identified_recipient() {
    let mut f = Fixture::new();
    let target = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    let agent = identity(&mut f, "Agent");
    mark_user(&f, &person);
    assert!(matches!(
        enqueue(
            &mut f,
            &target,
            &agent,
            "from-agent",
            DeliveryMode::Passthrough
        ),
        Err(RequestError::Invalid(_))
    ));
    // Nobody is anonymous: an unknown originator is not a person either.
    let mut anonymous = request(&target, &person, "anonymous");
    anonymous.originator = Originator::Unknown;
    assert!(matches!(
        service(&mut f).enqueue_delivery(
            anonymous,
            "attempt-anonymous".into(),
            7,
            policy(DeliveryMode::Passthrough)
        ),
        Err(RequestError::Invalid(_))
    ));
    // An announcement is not typed text.
    let mut announcement = request(&target, &person, "announcement");
    announcement.kind = RequestKind::Announcement;
    assert!(matches!(
        service(&mut f).enqueue_delivery(
            announcement,
            "attempt-announcement".into(),
            7,
            policy(DeliveryMode::Passthrough)
        ),
        Err(RequestError::Invalid(_))
    ));
    // The person's passthrough is accepted, and its mode is stored with the request.
    let prepared = enqueue(&mut f, &target, &person, "typed", DeliveryMode::Passthrough).unwrap();
    assert_eq!(prepared.digest_until_ms, None);
    assert_eq!(
        service(&mut f)
            .request_delivery_policy("typed")
            .unwrap()
            .mode,
        DeliveryMode::Passthrough
    );
    // A passthrough never waits for, or frames, a reply.
    let attempt = service(&mut f)
        .get_attempt(&prepared.attempt_id)
        .unwrap()
        .unwrap();
    assert!(!attempt.wait_active);
    assert!(attempt.preamble_every.is_none());
    f.storage.close().unwrap();
}

#[test]
fn a_passthrough_is_not_a_request_to_the_recipient_but_stays_the_senders_exchange() {
    let mut f = Fixture::new();
    let target = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person);
    // A request and a passthrough to the same agent: only the request is the agent's.
    enqueue(&mut f, &target, &person, "asked", DeliveryMode::Request).unwrap();
    enqueue(&mut f, &target, &person, "typed", DeliveryMode::Passthrough).unwrap();

    let open = service(&mut f).open_requests(&target, None, None).unwrap();
    let ids: Vec<_> = open
        .items
        .iter()
        .map(|item| item.request_id.as_str())
        .collect();
    assert_eq!(ids, ["asked"], "the passthrough is not an open request");

    let incoming = service(&mut f)
        .list_incoming(&target, None, None, None)
        .unwrap();
    let incoming: Vec<_> = incoming
        .items
        .iter()
        .map(|item| item.exchange.request_id.as_str())
        .collect();
    assert_eq!(
        incoming,
        ["asked"],
        "the passthrough publishes no attention"
    );

    // The sender's own history still has it, flagged by mode through the policy.
    let sent = service(&mut f).request_detail("typed").unwrap();
    assert_eq!(sent.item.delivery_policy.mode, DeliveryMode::Passthrough);
    assert_eq!(
        service(&mut f)
            .request_detail("asked")
            .unwrap()
            .item
            .delivery_policy
            .mode,
        DeliveryMode::Request
    );
    f.storage.close().unwrap();
}

#[test]
fn a_passthrough_takes_no_reply_and_requires_no_final() {
    use tmt_core::request::{
        ResponseLookup, ResponseProof, ResponseRejection, SubmitResponse, attention::FinalState,
        correlation,
    };
    let mut f = Fixture::new();
    let target = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person);
    let prepared = enqueue(&mut f, &target, &person, "typed", DeliveryMode::Passthrough).unwrap();
    // Even a caller that can derive the proof (an explicit request id) is refused.
    let proof = ResponseProof::Compact(correlation::response_token(
        "typed",
        &prepared.attempt_id,
        &RequestRoute::Inbox {
            recipient_identity_id: target.clone(),
        },
    ));
    assert!(matches!(
        service(&mut f).submit_response(SubmitResponse {
            request_id: "typed".into(),
            proof,
            body: "a reply".into(),
        }),
        Err(RequestError::Response(ResponseRejection::NotRequired))
    ));
    assert!(matches!(
        service(&mut f).get_response("typed").unwrap(),
        ResponseLookup::NotRequired
    ));
    assert!(matches!(
        service(&mut f)
            .request_detail("typed")
            .unwrap()
            .item
            .final_state,
        FinalState::NotRequired
    ));
    // The control: the same request in request mode still waits for its final.
    enqueue(&mut f, &target, &person, "asked", DeliveryMode::Request).unwrap();
    assert!(matches!(
        service(&mut f)
            .request_detail("asked")
            .unwrap()
            .item
            .final_state,
        FinalState::NotSubmitted
    ));
    f.storage.close().unwrap();
}
