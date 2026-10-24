//! One delivery state per sent message (#4378), read from the same rows the request
//! history and `x show` read: no pane text and no client-side guess.
use super::{
    digest::{held, identity, mark_user, policy, request, start},
    passthrough::{claimed_passthrough, identity as person, mark_user as mark_person},
    support::{Fixture, NOW_MS, service},
};
use crate::storage::requests::passthrough::{capture, settle_turn};
use tmt_core::{
    operation::new_operation_id,
    request::{
        RequestError, WakeState,
        delivery::{DeliveryState, DeliveryView},
        digest::{
            DeliveryPolicy, DigestOpportunity, DigestPolicyWrite, DigestRejection, DigestState,
        },
        passthrough::CaptureState,
    },
    responses::FinalText,
};

fn view(f: &mut Fixture, id: &str) -> DeliveryView {
    service(f).request_detail(id).unwrap().item.delivery_view
}

fn shown(f: &mut Fixture, id: &str) -> (&'static str, &'static str, bool) {
    let view = view(f, id);
    (view.state.as_str(), view.reason.code(), view.can_send_now)
}

/// The tick's own write: the hold window plus the rule's limit.
fn record_hold(f: &mut Fixture, target: &str, owner: &str, revision: u64, hold: Option<u64>) {
    service(f)
        .write_digest(DigestPolicyWrite {
            max_hold_ms: hold,
            ..policy(target, owner, revision, NOW_MS + 600_000)
        })
        .unwrap();
}

/// A bound session of `identity` whose driver last reported `state` at `since_ms`.
fn bind_with_activity(f: &Fixture, identity: &str, state: &str, since_ms: u64) {
    bind_driver_with_activity(f, identity, "claude", state, since_ms);
}

fn bind_driver_with_activity(
    f: &Fixture,
    identity: &str,
    driver: &str,
    state: &str,
    since_ms: u64,
) {
    let db = rusqlite::Connection::open(&f.database).unwrap();
    db.execute(
        "INSERT INTO bindings(id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at,runtime_state,runtime_pid,runtime_start_identity,observed_provider_session_id,launch_owner_pid,launch_owner_start_identity)
         VALUES('binding',?1,'tmux','%1','server','/fixture',10,'server-start',11,'original','original','running',12,'runtime-start','session',13,'owner-start')",
        [identity],
    )
    .unwrap();
    let activity = serde_json::json!({"activity": {
        "session": "session", "pid": 12, "start": "runtime-start", "state": state,
        "turn": null, "sinceMs": since_ms, "lastActivityMs": since_ms,
    }});
    db.execute(
        "INSERT INTO identity_session_preferences(identity_id,remembered_harness,runtime_mode,provider_session_id,driver_state_version,driver_state)
         VALUES(?1,?3,'default','session',3,?2)",
        rusqlite::params![identity, activity.to_string(), driver],
    )
    .unwrap();
}

#[test]
fn a_held_message_counts_down_to_the_oldest_held_plus_the_recorded_hold() {
    let (mut f, target, owner, sender) = start();
    held(&mut f, &target, &sender, "first");
    // A policy written before the limit was recorded can state no deadline.
    let unknown = view(&mut f, "first");
    assert_eq!(
        (unknown.state, unknown.latest_ms, unknown.remaining_ms),
        (DeliveryState::Batched, None, None)
    );
    record_hold(&mut f, &target, &owner, 1, Some(300_000));
    f.set_now(NOW_MS + 100_000);
    held(&mut f, &target, &sender, "second");
    // Every held message goes out together, so each carries the oldest one's deadline.
    for id in ["first", "second"] {
        let held = view(&mut f, id);
        assert_eq!(
            (
                held.state,
                held.latest_ms,
                held.remaining_ms,
                held.can_send_now
            ),
            (
                DeliveryState::Batched,
                Some(NOW_MS + 300_000),
                Some(200_000),
                true
            ),
            "{id}"
        );
        assert_eq!(held.reason.code(), "held_to_save_usage");
    }
    f.set_now(NOW_MS + 400_000);
    assert_eq!(view(&mut f, "first").remaining_ms, Some(0));
    f.storage.close().unwrap();
}

#[test]
fn the_recorded_hold_round_trips_and_only_a_set_may_carry_one() {
    let (mut f, target, owner, _) = start();
    for bad in [Some(0), Some(9_007_199_254_740_992)] {
        assert!(
            matches!(
                service(&mut f).write_digest(DigestPolicyWrite {
                    max_hold_ms: bad,
                    ..policy(&target, &owner, 1, NOW_MS + 5000)
                }),
                Err(RequestError::Digest(DigestRejection::Invalid))
            ),
            "{bad:?}"
        );
    }
    // A clear records nothing.
    assert!(matches!(
        service(&mut f).write_digest(DigestPolicyWrite {
            max_hold_ms: Some(60_000),
            ..policy(&target, &owner, 1, 0)
        }),
        Err(RequestError::Digest(DigestRejection::Invalid))
    ));
    record_hold(&mut f, &target, &owner, 1, Some(60_000));
    let stored = |f: &mut Fixture| {
        service(f)
            .digest_policies(std::slice::from_ref(&target))
            .unwrap()
            .remove(0)
            .policy
            .unwrap()
            .max_hold_ms
    };
    assert_eq!(stored(&mut f), Some(60_000));
    // The next write replaces it, even with none.
    record_hold(&mut f, &target, &owner, 2, None);
    assert_eq!(stored(&mut f), None);
    f.storage.close().unwrap();
}

#[test]
fn a_person_and_a_plain_message_are_sent_and_can_go_now() {
    let (mut f, target, _, sender) = start();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person, "true");
    service(&mut f)
        .enqueue_delivery(
            request(&target, Some(&person), "typed"),
            "attempt-typed".into(),
            7,
            DeliveryPolicy::default(),
        )
        .unwrap();
    assert_eq!(
        shown(&mut f, "typed"),
        ("sent", "waiting_for_delivery", true)
    );
    held(&mut f, &target, &sender, "agent");
    assert_eq!(
        shown(&mut f, "agent"),
        ("batched", "held_to_save_usage", true)
    );
    f.storage.close().unwrap();
}

#[test]
fn a_checklist_moves_its_messages_from_batched_to_sent_to_delivered_or_unconfirmed() {
    let (mut f, target, _, sender) = start();
    held(&mut f, &target, &sender, "a");
    let claim = |f: &mut Fixture| {
        service(f)
            .claim_digest_checklist(
                &target,
                new_operation_id(),
                new_operation_id(),
                DigestOpportunity::TurnBoundary,
                None,
            )
            .unwrap()
            .unwrap()
    };
    let batch = claim(&mut f);
    assert_eq!(
        shown(&mut f, "a"),
        ("sent", "waiting_for_delivery", false),
        "in flight: nothing to press"
    );
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &batch.id,
            &batch.attempt_token,
            DigestState::Unsent,
        )
        .unwrap();
    assert_eq!(shown(&mut f, "a"), ("batched", "held_to_save_usage", true));
    let batch = claim(&mut f);
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &batch.id,
            &batch.attempt_token,
            DigestState::Delivered,
        )
        .unwrap();
    assert_eq!(
        shown(&mut f, "a"),
        ("delivered", "reached_the_agent", false)
    );

    held(&mut f, &target, &sender, "b");
    let batch = claim(&mut f);
    service(&mut f)
        .settle_digest_checklist(
            &target,
            &batch.id,
            &batch.attempt_token,
            DigestState::Uncertain,
        )
        .unwrap();
    assert_eq!(shown(&mut f, "b"), ("unconfirmed", "handed_over", false));
    f.storage.close().unwrap();
}

#[test]
fn a_delivered_message_is_working_only_for_a_turn_the_bound_session_began_after_it() {
    let mut f = Fixture::new();
    let target = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person, "true");
    let send = |f: &mut Fixture, id: &str| {
        service(f)
            .enqueue_delivery(
                request(&target, Some(&person), id),
                format!("attempt-{id}"),
                7,
                DeliveryPolicy::default(),
            )
            .unwrap();
        assert!(service(f).claim_wake(id).unwrap().claimed);
        service(f)
            .settle_request_delivery(id, WakeState::Sent)
            .unwrap();
    };
    // Without a bound session nothing can say the agent started.
    send(&mut f, "early");
    assert_eq!(
        shown(&mut f, "early"),
        ("delivered", "reached_the_agent", false)
    );

    bind_with_activity(&f, &target, "working", NOW_MS - 1);
    assert_eq!(
        shown(&mut f, "early"),
        ("delivered", "reached_the_agent", false),
        "a turn that began before the hand-over is not this message's"
    );
    f.set_now(NOW_MS + 10);
    send(&mut f, "later");
    // The driver reports a new turn: both timestamps move together.
    let began = NOW_MS + 20;
    rusqlite::Connection::open(&f.database)
        .unwrap()
        .execute(
            "UPDATE identity_session_preferences SET driver_state = json_set(driver_state,
                '$.activity.sinceMs', ?1, '$.activity.lastActivityMs', ?1)",
            [began as i64],
        )
        .unwrap();
    assert_eq!(shown(&mut f, "later"), ("working", "agent_working", false));
    // An idle session has not started on anything.
    rusqlite::Connection::open(&f.database)
        .unwrap()
        .execute(
            "UPDATE identity_session_preferences SET driver_state = replace(driver_state, 'working', 'idle')",
            [],
        )
        .unwrap();
    assert_eq!(
        shown(&mut f, "later"),
        ("delivered", "reached_the_agent", false)
    );
    f.storage.close().unwrap();
}

#[test]
fn a_stored_refused_start_explains_delivery_without_reusing_the_old_working_turn() {
    let mut f = Fixture::new();
    let target = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person, "true");
    service(&mut f)
        .enqueue_delivery(
            request(&target, Some(&person), "unbound"),
            "attempt-unbound".into(),
            7,
            DeliveryPolicy::default(),
        )
        .unwrap();
    assert!(service(&mut f).claim_wake("unbound").unwrap().claimed);
    service(&mut f)
        .settle_request_delivery("unbound", WakeState::Sent)
        .unwrap();
    bind_with_activity(&f, &target, "working", NOW_MS + 1);
    assert_eq!(
        shown(&mut f, "unbound"),
        ("working", "agent_working", false)
    );
    let db = rusqlite::Connection::open(&f.database).unwrap();
    let before: String = db
        .query_row(
            "SELECT driver_state FROM identity_session_preferences WHERE identity_id=?1",
            [&target],
            |row| row.get(0),
        )
        .unwrap();
    db.execute("UPDATE bindings SET runtime_start_refusal=?1 WHERE identity_id=?2",
        rusqlite::params![serde_json::json!({"driver":"claude", "session":"unadmitted", "reason":"previous_session_live", "atMs":NOW_MS + 2}).to_string(), target]).unwrap();
    assert_eq!(
        shown(&mut f, "unbound"),
        ("delivered", "session_not_bound", false)
    );
    let name: String = db
        .query_row(
            "SELECT name FROM identities WHERE id=?1",
            [&target],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        view(&mut f, "unbound").reason_text,
        format!(
            "session not bound · the old session is still running; move it with: tmt resume --move -- {}",
            tmt_core::names::command_word(&name)
        )
    );
    db.execute("UPDATE bindings SET runtime_start_refusal=?1 WHERE identity_id=?2",
        rusqlite::params![serde_json::json!({"driver":"claude", "session":"unadmitted", "reason":"driver_mismatch", "atMs":NOW_MS + 3}).to_string(), target]).unwrap();
    assert_eq!(
        view(&mut f, "unbound").reason_text,
        format!(
            "session not bound · exit the agent in its pane, then run: tmt resume -- {}",
            tmt_core::names::command_word(&name)
        )
    );
    let after: String = db
        .query_row(
            "SELECT driver_state FROM identity_session_preferences WHERE identity_id=?1",
            [&target],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after, before,
        "reading the diagnostic cannot rewrite admitted activity"
    );
    drop(db);
    f.storage.close().unwrap();
}

/// The submitted reply is the end of the road (#4910): a busy agent, a hold that never cleared and
/// a paste that never happened all read answered once the final exists.
#[test]
fn a_submitted_reply_reads_answered_whatever_the_agent_is_doing() {
    use tmt_core::request::{RequestRoute, ResponseProof, SubmitResponse, correlation};
    let (mut f, target, _, sender) = start();
    let prepared = held(&mut f, &target, &sender, "a");
    bind_with_activity(&f, &target, "working", NOW_MS + 5);
    assert_ne!(shown(&mut f, "a").0, "answered");
    service(&mut f)
        .submit_response(SubmitResponse {
            request_id: "a".into(),
            proof: ResponseProof::Compact(correlation::response_token(
                "a",
                &prepared.attempt_id,
                &RequestRoute::Inbox {
                    recipient_identity_id: target.clone(),
                },
            )),
            body: "done".into(),
        })
        .unwrap();
    assert_eq!(shown(&mut f, "a"), ("answered", "answered_by_reply", false));
    f.storage.close().unwrap();
}

/// A passthrough earns its answer from the turn it started; a capture that ended without one
/// reads unconfirmed and says why, and neither reads `working` once it is over.
#[test]
fn a_passthrough_reads_the_outcome_of_its_capture_and_never_working_after_it_ends() {
    let mut f = Fixture::new();
    let ben = person(&mut f, "Ben");
    mark_person(&f, &ben);
    let target = f.identity_id.clone();
    for id in [
        "pending",
        "answered",
        "never-bound",
        "bound-no-answer",
        "lapsed",
    ] {
        claimed_passthrough(&mut f, &target, &ben, id, "typed text");
        service(&mut f)
            .settle_request_delivery(id, WakeState::Sent)
            .unwrap();
    }
    bind_with_activity(&f, &target, "working", NOW_MS + 5);
    let set = |sql: &str, id: &str| {
        let changed = rusqlite::Connection::open(&f.database)
            .unwrap()
            .execute(sql, rusqlite::params![id])
            .unwrap();
        assert_eq!(changed, 1, "{id} has a capture");
    };
    set(
        "UPDATE request_passthrough SET state='answered', bound_session='s', bound_at_ms=1, answer_text='an answer', answer_truncated=0, answer_provider='claude', answer_ended_ms=2, settled_at_ms=2 WHERE request_id=?1",
        "answered",
    );
    set(
        "UPDATE request_passthrough SET state='no_answer', settled_at_ms=2 WHERE request_id=?1",
        "never-bound",
    );
    set(
        "UPDATE request_passthrough SET state='no_answer', bound_session='s', bound_at_ms=1, settled_at_ms=2 WHERE request_id=?1",
        "bound-no-answer",
    );
    // Stored as still pending, but past its binding window: it reads `no_answer` before anything
    // has written that.
    set(
        "UPDATE request_passthrough SET delivered_at_ms=1 WHERE request_id=?1",
        "lapsed",
    );

    // Still inside its window, the agent's current turn is the only evidence: unchanged.
    assert_eq!(
        shown(&mut f, "pending"),
        ("working", "agent_working", false)
    );
    assert_eq!(
        shown(&mut f, "answered"),
        ("answered", "answered_by_turn", false)
    );
    assert_eq!(
        shown(&mut f, "never-bound"),
        ("unconfirmed", "no_turn_started", false)
    );
    assert_eq!(
        shown(&mut f, "bound-no-answer"),
        ("unconfirmed", "turn_ended_without_answer", false)
    );
    assert_eq!(
        shown(&mut f, "lapsed"),
        ("unconfirmed", "no_turn_started", false)
    );
    f.storage.close().unwrap();
}

/// One passthrough typed into `agent`'s pane while its bound session's driver last reported
/// `state` at `since_ms` (`None`: no binding at all). The send began at `NOW_MS`.
fn typed_while(state: Option<(&str, u64)>) -> Fixture {
    typed_while_driver("claude", state)
}

fn typed_while_driver(driver: &str, state: Option<(&str, u64)>) -> Fixture {
    let mut f = Fixture::new();
    let ben = person(&mut f, "Ben");
    mark_person(&f, &ben);
    let target = f.identity_id.clone();
    if let Some((state, since_ms)) = state {
        bind_driver_with_activity(&f, &target, driver, state, since_ms);
    }
    claimed_passthrough(&mut f, &target, &ben, "typed", "also do the other thing");
    service(&mut f)
        .settle_request_delivery("typed", WakeState::Sent)
        .unwrap();
    f
}

#[test]
fn a_passthrough_typed_into_a_running_turn_binds_to_it_and_the_turns_end_answers_it() {
    // The turn ran for five seconds before the send began: no prompt will announce the text.
    let mut f = typed_while(Some(("working", NOW_MS - 5_000)));
    let db = rusqlite::Connection::open(&f.database).unwrap();
    let folded = capture(&db, "typed").unwrap().unwrap();
    assert_eq!(folded.state, CaptureState::Bound);
    assert!(folded.folded);
    assert_eq!(folded.bound_session.as_deref(), Some("session"));
    assert_eq!(
        folded.bound_turn, None,
        "the fixture's provider names no turn"
    );
    assert_eq!(folded.bound_at_ms, Some(folded.delivered_at_ms));

    let target = f.identity_id.clone();
    let text = FinalText::new("Both done.").unwrap();
    assert!(
        settle_turn(
            &db,
            &target,
            "session",
            None,
            Some((&text, "claude")),
            NOW_MS + 10_000
        )
        .unwrap()
    );
    let answered = capture(&db, "typed").unwrap().unwrap();
    assert_eq!(answered.answer.unwrap().text, "Both done.");
    assert_eq!(
        shown(&mut f, "typed"),
        ("answered", "answered_by_folded_turn", false)
    );
    f.storage.close().unwrap();
}

#[test]
fn a_folded_passthrough_whose_turn_ends_without_a_final_reads_unconfirmed_not_working() {
    let mut f = typed_while(Some(("working", NOW_MS - 5_000)));
    let db = rusqlite::Connection::open(&f.database).unwrap();
    let target = f.identity_id.clone();
    assert!(settle_turn(&db, &target, "session", None, None, NOW_MS + 10_000).unwrap());
    assert_eq!(
        shown(&mut f, "typed"),
        ("unconfirmed", "turn_ended_without_answer", false)
    );
    f.storage.close().unwrap();
}

#[test]
fn a_driver_that_does_not_fold_queued_text_keeps_a_pending_prompt_matched_capture() {
    // The same running turn that folds a Claude passthrough: Codex is unmeasured, so its text
    // stays pending and is matched by the prompt of whichever turn runs it.
    for driver in ["codex", "gemini"] {
        let mut f = typed_while_driver(driver, Some(("working", NOW_MS - 5_000)));
        let db = rusqlite::Connection::open(&f.database).unwrap();
        let pending = capture(&db, "typed").unwrap().unwrap();
        assert_eq!(pending.state, CaptureState::Pending, "{driver}");
        assert!(!pending.folded, "{driver}");
        // The running turn ending settles nothing for it.
        let target = f.identity_id.clone();
        let text = FinalText::new("The earlier turn's final.").unwrap();
        assert!(
            !settle_turn(
                &db,
                &target,
                "session",
                None,
                Some((&text, "claude")),
                NOW_MS + 10_000
            )
            .unwrap(),
            "{driver}"
        );
        assert_eq!(
            shown(&mut f, "typed"),
            ("delivered", "reached_the_agent", false),
            "{driver}"
        );
        f.storage.close().unwrap();
    }
}

#[test]
fn a_passthrough_is_folded_only_into_a_turn_the_verified_binding_shows_running_before_the_send() {
    for (case, state) in [
        // The turn began after the send did: it may be this text's own prompt.
        (
            "a turn that began after the send",
            Some(("working", NOW_MS + 5)),
        ),
        ("an idle agent", Some(("idle", NOW_MS - 5_000))),
        ("an unbound agent", None),
    ] {
        let mut f = typed_while(state);
        let db = rusqlite::Connection::open(&f.database).unwrap();
        let pending = capture(&db, "typed").unwrap().unwrap();
        assert_eq!(pending.state, CaptureState::Pending, "{case}");
        assert!(!pending.folded, "{case}");
        assert_eq!(pending.bound_session, None, "{case}");
        f.storage.close().unwrap();
    }
}

#[test]
fn a_message_stops_reading_working_when_the_turn_recorded_for_it_ends() {
    let mut f = Fixture::new();
    let target = f.identity_id.clone();
    let person = identity(&mut f, "Ben");
    mark_user(&f, &person, "true");
    service(&mut f)
        .enqueue_delivery(
            request(&target, Some(&person), "m"),
            "attempt-m".into(),
            7,
            DeliveryPolicy::default(),
        )
        .unwrap();
    assert!(service(&mut f).claim_wake("m").unwrap().claimed);
    service(&mut f)
        .settle_request_delivery("m", WakeState::Sent)
        .unwrap();
    let began = NOW_MS + 20;
    bind_with_activity(&f, &target, "working", began);
    let db = rusqlite::Connection::open(&f.database).unwrap();
    // Nothing recorded: the activity alone says the agent started.
    assert_eq!(shown(&mut f, "m"), ("working", "agent_working", false));
    db.execute(
        "INSERT INTO request_turns(request_id,started_ms) VALUES('m',?1)",
        [began as i64],
    )
    .unwrap();
    assert_eq!(shown(&mut f, "m"), ("working", "agent_working", false));
    // The turn ended and a later one is running: the message is not that turn's.
    db.execute(
        "UPDATE request_turns SET ended_ms=?1 WHERE request_id='m'",
        [(began + 50) as i64],
    )
    .unwrap();
    db.execute(
        "UPDATE identity_session_preferences SET driver_state = json_set(driver_state,
            '$.activity.sinceMs', ?1, '$.activity.lastActivityMs', ?1)",
        [(began + 90) as i64],
    )
    .unwrap();
    assert_eq!(
        shown(&mut f, "m"),
        ("unconfirmed", "turn_ended_without_reply", false)
    );
    f.storage.close().unwrap();
}
