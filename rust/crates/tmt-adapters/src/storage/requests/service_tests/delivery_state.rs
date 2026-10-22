//! One delivery state per sent message (#4378), read from the same rows the request
//! history and `x show` read: no pane text and no client-side guess.
use super::{
    digest::{held, identity, mark_user, policy, request, start},
    support::{Fixture, NOW_MS, service},
};
use tmt_core::{
    operation::new_operation_id,
    request::{
        RequestError, WakeState,
        delivery::{DeliveryState, DeliveryView},
        digest::{
            DeliveryPolicy, DigestOpportunity, DigestPolicyWrite, DigestRejection, DigestState,
        },
    },
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
         VALUES(?1,'claude','default','session',3,?2)",
        rusqlite::params![identity, activity.to_string()],
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
