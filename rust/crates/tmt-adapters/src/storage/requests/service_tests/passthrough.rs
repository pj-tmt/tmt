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

pub(super) fn identity(f: &mut Fixture, name: &str) -> String {
    create_or_resolve(&mut f.storage, name, Lifetime::Saved)
        .unwrap()
        .identity
        .id
}

pub(super) fn mark_user(f: &Fixture, id: &str) {
    rusqlite::Connection::open(&f.database)
        .unwrap()
        .execute(
            "INSERT INTO identity_metadata(identity_id, key, value) VALUES(?1, 'user', 'true')",
            [id],
        )
        .unwrap();
}

pub(super) fn request(target: &str, sender: &str, id: &str) -> PrepareRequest {
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

pub(super) fn policy(mode: DeliveryMode) -> DeliveryPolicy {
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

/// A passthrough prepared as the CLI does and claimed for its one delivery attempt.
pub(super) fn claimed_passthrough(
    f: &mut Fixture,
    agent: &str,
    person: &str,
    id: &str,
    text: &str,
) {
    let mut typed = request(agent, person, id);
    typed.message = text.into();
    let policy = DeliveryPolicy {
        automatic: true,
        ..policy(DeliveryMode::Passthrough)
    };
    service(f)
        .enqueue_delivery(typed, format!("attempt-{id}"), 7, policy)
        .unwrap();
    assert!(service(f).claim_wake(id).unwrap().claimed);
}

/// A detached question from `agent` to `person`: nobody waits, so its final may notify.
fn ask(f: &mut Fixture, person: &str, agent: &str, id: &str) {
    let mut question = request(person, agent, id);
    question.wait = false;
    service(f)
        .enqueue_delivery(
            question,
            format!("attempt-{id}"),
            7,
            policy(DeliveryMode::Request),
        )
        .unwrap();
}

fn open_ids(f: &mut Fixture, person: &str) -> Vec<String> {
    service(f)
        .open_requests(person, None, None)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.request_id)
        .collect()
}

fn answer_of(f: &mut Fixture, id: &str) -> Option<String> {
    match service(f).get_response(id).unwrap() {
        tmt_core::request::ResponseLookup::Available(response) => Some(response.body),
        _ => None,
    }
}

#[test]
fn typed_text_answers_only_the_oldest_request_the_agent_has_open_to_the_person() {
    use tmt_core::request::{
        ResponseProof, SubmitResponse, WakeState, correlation, notification::NotificationPolicy,
    };
    let mut f = Fixture::new();
    let agent = f.identity_id.clone();
    let other = identity(&mut f, "Other");
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person);
    // Two questions from the agent, oldest first, and one from another agent.
    for (at, sender, id) in [
        (0, &agent, "q-old"),
        (1, &other, "q-other"),
        (2, &agent, "q-new"),
    ] {
        f.set_now(NOW_MS + at * 1000);
        ask(&mut f, &person, sender, id);
    }
    for question in ["q-old", "q-new"] {
        service(&mut f)
            .enable_notifications(
                question,
                NotificationPolicy {
                    deadline_ms: NOW_MS + 600_000,
                    timeout_ms: 1000,
                    waiter: None,
                },
            )
            .unwrap();
    }
    f.set_now(NOW_MS + 3000);
    claimed_passthrough(&mut f, &agent, &person, "typed", "Yes, go ahead!");
    assert_eq!(open_ids(&mut f, &person), ["q-old", "q-other", "q-new"]);

    service(&mut f)
        .settle_request_delivery("typed", WakeState::Sent)
        .unwrap();

    // The text as written, not as typed, is the final of the oldest question from this agent.
    assert_eq!(
        answer_of(&mut f, "q-old").as_deref(),
        Some("Yes, go ahead!")
    );
    assert_eq!(open_ids(&mut f, &person), ["q-other", "q-new"]);
    assert_eq!(answer_of(&mut f, "q-new"), None);
    assert_eq!(answer_of(&mut f, "q-other"), None);
    // The passthrough itself still takes no final.
    assert!(matches!(
        service(&mut f).get_response("typed").unwrap(),
        tmt_core::request::ResponseLookup::NotRequired
    ));
    // The agent was handed the text in its pane, so no notice about the answer is claimed...
    assert_eq!(
        service(&mut f)
            .notification("q-old")
            .unwrap()
            .unwrap()
            .reply,
        WakeState::NotAttempted
    );
    // ...unlike an ordinary reply to a question that asked for one.
    let proof = ResponseProof::Compact(correlation::response_token(
        "q-new",
        "attempt-q-new",
        &RequestRoute::Inbox {
            recipient_identity_id: person.clone(),
        },
    ));
    let (_, hint) = service(&mut f)
        .submit_response_with_hint(
            SubmitResponse {
                request_id: "q-new".into(),
                proof,
                body: "a reply".into(),
            },
            None,
        )
        .unwrap();
    assert!(
        hint.is_some(),
        "an ordinary reply claims the originator's notice"
    );
    f.storage.close().unwrap();
}

#[test]
fn typed_text_with_nothing_open_records_no_answer() {
    use tmt_core::request::WakeState;
    let mut f = Fixture::new();
    let agent = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person);
    claimed_passthrough(&mut f, &agent, &person, "typed", "Hello");
    service(&mut f)
        .settle_request_delivery("typed", WakeState::Sent)
        .unwrap();
    assert!(open_ids(&mut f, &person).is_empty());
    assert_eq!(answer_of(&mut f, "typed"), None);
    f.storage.close().unwrap();
}

#[test]
fn a_delivery_that_did_not_reach_the_pane_leaves_the_question_open() {
    use tmt_core::request::WakeState;
    let mut f = Fixture::new();
    let agent = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person);
    for (state, question, typed) in [
        (WakeState::Unavailable, "q-refused", "typed-refused"),
        (WakeState::Uncertain, "q-unsure", "typed-unsure"),
    ] {
        enqueue(&mut f, &person, &agent, question, DeliveryMode::Request).unwrap();
        claimed_passthrough(&mut f, &agent, &person, typed, "Not delivered");
        service(&mut f)
            .settle_request_delivery(typed, state)
            .unwrap();
        assert_eq!(answer_of(&mut f, question), None, "{state:?}");
    }
    assert_eq!(open_ids(&mut f, &person), ["q-refused", "q-unsure"]);
    f.storage.close().unwrap();
}

#[test]
fn a_sent_passthrough_starts_a_capture_the_history_shows_and_an_unsent_one_does_not() {
    use tmt_core::request::{
        WakeState,
        passthrough::{BIND_WINDOW_MS, CaptureState},
    };
    let mut f = Fixture::new();
    let agent = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person);
    claimed_passthrough(&mut f, &agent, &person, "typed", "Use main!");
    claimed_passthrough(&mut f, &agent, &person, "refused", "Not typed");
    // Nothing is captured before the text is in the pane.
    assert!(
        service(&mut f)
            .request_detail("typed")
            .unwrap()
            .item
            .capture
            .is_none()
    );
    service(&mut f)
        .settle_request_delivery("typed", WakeState::Sent)
        .unwrap();
    service(&mut f)
        .settle_request_delivery("refused", WakeState::Unavailable)
        .unwrap();

    let capture = service(&mut f)
        .request_detail("typed")
        .unwrap()
        .item
        .capture
        .unwrap();
    assert_eq!(capture.state, CaptureState::Pending);
    assert_eq!(capture.delivered_at_ms, NOW_MS);
    assert!(capture.answer.is_none());
    assert!(
        service(&mut f)
            .request_detail("refused")
            .unwrap()
            .item
            .capture
            .is_none(),
        "a refused delivery has no turn to wait for"
    );
    // A request is never a passthrough: it has no capture.
    enqueue(&mut f, &agent, &person, "asked", DeliveryMode::Request).unwrap();
    assert!(
        service(&mut f)
            .request_detail("asked")
            .unwrap()
            .item
            .capture
            .is_none()
    );

    // Past the binding window the read says no answer, though nothing wrote that.
    f.set_now(NOW_MS + BIND_WINDOW_MS + 1);
    let late = service(&mut f)
        .request_detail("typed")
        .unwrap()
        .item
        .capture
        .unwrap();
    assert_eq!(late.state, CaptureState::NoAnswer);
    f.storage.close().unwrap();
}

#[test]
fn whether_an_identity_can_take_typed_input_is_read_from_stored_facts_only() {
    use crate::passthrough::{Can, Reason, can_identity};
    let mut f = Fixture::new();
    let target = identity(&mut f, "worker");
    let can = |f: &Fixture| can_identity(&f.storage, &target).unwrap();
    // Unknown identity: nothing to say.
    assert_eq!(
        can_identity(&f.storage, "00000000-0000-4000-8000-000000000000").unwrap(),
        None
    );
    // Saved but unbound.
    assert_eq!(can(&f), Some(Can::no(Reason::NoPane)));

    let db = rusqlite::Connection::open(&f.database).unwrap();
    db.execute(
        "INSERT INTO bindings(id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at,runtime_state,runtime_pid,runtime_start_identity,observed_provider_session_id,launch_owner_pid,launch_owner_start_identity)
         VALUES('binding',?1,'tmux','%1','server','/fixture',10,'server-start',11,'original','original','running',12,'runtime-start','session',13,'owner-start')",
        [&target],
    )
    .unwrap();
    // Running with no reported activity yet: still offered; the send verifies it.
    assert_eq!(can(&f), Some(Can::YES));

    let report = |state: &str| {
        let activity = serde_json::json!({"activity": {
            "session": "session", "pid": 12, "start": "runtime-start", "state": state,
            "turn": null, "sinceMs": 1, "lastActivityMs": 1,
        }});
        db.execute(
            "INSERT OR REPLACE INTO identity_session_preferences(identity_id,remembered_harness,runtime_mode,provider_session_id,driver_state_version,driver_state)
             VALUES(?1,'claude','default','session',3,?2)",
            rusqlite::params![target, activity.to_string()],
        )
        .unwrap();
    };
    for state in ["working", "idle"] {
        report(state);
        assert_eq!(can(&f), Some(Can::YES), "{state}");
    }
    // Waiting on its user: the delivery would be refused anyway.
    report("waiting");
    assert_eq!(can(&f), Some(Can::no(Reason::AwaitingApproval)));
    // An ended agent keeps its last phase, which is no evidence.
    db.execute("UPDATE bindings SET runtime_state='ended'", [])
        .unwrap();
    assert_eq!(can(&f), Some(Can::no(Reason::NotRunning)));
    // A runtime nobody observed yet is offered, as the command does; the send verifies it.
    db.execute("UPDATE bindings SET runtime_state='unknown'", [])
        .unwrap();
    assert_eq!(can(&f), Some(Can::no(Reason::AwaitingApproval)));
}
