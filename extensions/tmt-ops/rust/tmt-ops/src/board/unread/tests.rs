use super::*;
use serde_json::json;

fn reply(request: &str, member: &str, at: u64) -> Value {
    json!({"requestId":request,"recipientId":member,"sender":{"identityId":"you"},
        "kind":"request","answeredAtMs":at,"answer":"final","revision":1,"acknowledged":false})
}

fn turn(request: &str, member: &str, at: u64) -> Value {
    json!({"requestId":request,"recipientId":member,"sender":{"identityId":"you"},
        "kind":"request","answeredAtMs":at,"answer":"turn","revision":1,"acknowledged":false})
}

fn page(items: Vec<Value>, more: bool) -> Value {
    let cursor = if more {
        json!({"answeredAtMs":items.last().unwrap()["answeredAtMs"],
            "requestId":items.last().unwrap()["requestId"]})
    } else {
        Value::Null
    };
    json!({"items":items,"nextBefore":cursor})
}

#[test]
fn core_acknowledgment_owns_the_boundary_and_known_zero_has_no_unread_label() {
    let mut acknowledged = reply("old-a", "a", 100);
    acknowledged["acknowledged"] = json!(true);
    let observed = Page::read(
        "you",
        &page(vec![reply("new-b", "b", 110), acknowledged], true),
    )
    .unwrap();
    assert_eq!(observed.count("a").total, 0);
    assert_eq!(observed.count("a").label(), None);
    assert_eq!(observed.count("b").label().as_deref(), Some("1+ unread"));
    let unobserved = observed.count("c");
    assert!(!unobserved.more);
    assert_eq!(unobserved.label(), None);
    let mut aggregate = observed.count("b");
    aggregate.add(unobserved);
    assert_eq!(aggregate.label().as_deref(), Some("1+ unread"));
}

#[test]
fn core_acknowledgment_counts_one_request_once_across_room_duplicates() {
    let same = reply("same", "a", 100);
    let observed = Page::read(
        "you",
        &page(vec![reply("new", "a", 110), same.clone(), same], false),
    )
    .unwrap();
    assert_eq!(observed.count("a").total, 2);
    assert_eq!(observed.count("a").label().as_deref(), Some("2 unread"));
}

#[test]
fn malformed_foreign_and_over_limit_pages_cannot_claim_complete_counts() {
    let valid = page(vec![reply("request", "a", 100)], false);
    assert!(Page::read("you", &valid).is_some());
    let mut missing_boundary = valid.clone();
    missing_boundary
        .as_object_mut()
        .unwrap()
        .remove("nextBefore");
    let mut wrong_actor = valid.clone();
    wrong_actor["items"][0]["sender"]["identityId"] = json!("someone-else");
    let mut unknown_answer = valid.clone();
    unknown_answer["items"][0]["answer"] = json!("withdrawn");
    let mut invalid_time = valid.clone();
    invalid_time["items"][0]["answeredAtMs"] = json!(0);
    let mut results_cursor = page(vec![reply("request", "a", 100)], true);
    results_cursor["nextBefore"] = json!({"submittedAtMs":100,"requestId":"request"});
    let conflict = page(
        vec![reply("same", "a", 100), reply("same", "b", 100)],
        false,
    );
    let too_many = page(
        (0..51).map(|i| reply(&format!("r{i}"), "a", 100)).collect(),
        false,
    );
    for invalid in [
        missing_boundary,
        wrong_actor,
        unknown_answer,
        invalid_time,
        results_cursor,
        conflict,
        too_many,
    ] {
        assert!(Page::read("you", &invalid).is_none());
    }
}

#[test]
fn a_final_and_a_captured_turn_count_alike_and_one_request_counts_once() {
    let observed = Page::read(
        "you",
        &page(
            vec![
                turn("direct", "a", 130),
                reply("asked", "a", 120),
                turn("elsewhere", "b", 110),
            ],
            false,
        ),
    )
    .unwrap();
    assert_eq!(observed.count("a").total, 2);
    assert_eq!(observed.count("a").label().as_deref(), Some("2 unread"));
    assert_eq!(observed.count("b").total, 1);
    let repeated = Page::read(
        "you",
        &page(vec![turn("same", "a", 100), turn("same", "a", 100)], false),
    )
    .unwrap();
    assert_eq!(repeated.count("a").total, 1);
}

#[test]
fn core_acknowledgment_is_authoritative_for_finals_and_turns() {
    let mut final_answer = reply("asked", "a", 100);
    final_answer["revision"] = json!(3);
    final_answer["acknowledged"] = json!(true);
    let mut captured = turn("direct", "a", 110);
    captured["revision"] = json!(4);
    captured["acknowledged"] = json!(false);
    let observed = Page::read("you", &page(vec![captured, final_answer], false)).unwrap();
    assert_eq!(
        observed.count("a").total,
        1,
        "Core acknowledged finals stay read"
    );
}

#[test]
fn core_acknowledgment_requires_a_revision_and_boolean() {
    for (field, value) in [
        ("revision", Value::Null),
        ("revision", json!(0)),
        ("revision", json!(9_007_199_254_740_992_u64)),
        ("acknowledged", Value::Null),
        ("acknowledged", json!("false")),
    ] {
        let mut invalid = page(vec![reply("asked", "a", 100)], false);
        invalid["items"][0][field] = value;
        assert!(
            Page::read("you", &invalid).is_none(),
            "missing or malformed Core ack evidence must not replace a held page"
        );
    }
}
#[test]
fn acknowledgments_are_actor_and_revision_fenced_and_late_pages_cannot_regress_them() {
    use super::publish::{Completed, Outcome};
    let document = page(vec![reply("asked", "a", 100)], false);
    let mut held = Page::read("you", &document).unwrap();
    let answer = held.answer("a", "asked").unwrap().clone();
    let ack = Completed {
        actor: "you".into(),
        items: vec![(answer.clone(), Outcome::Acknowledged)],
    };
    let wrong = Completed {
        actor: "someone else".into(),
        items: ack.items.clone(),
    };
    held.acknowledged(&wrong);
    assert_eq!(held.count("a").total, 1);
    held.acknowledged(&ack);
    assert_eq!(held.count("a").total, 0);
    let mut late = Page::read("you", &document).unwrap();
    late.retain_acknowledged(&held);
    assert_eq!(late.count("a").total, 0);
    let mut newer = document.clone();
    newer["items"][0]["revision"] = json!(2);
    let mut newer = Page::read("you", &newer).unwrap();
    newer.acknowledged(&ack);
    assert_eq!(
        newer.count("a").total,
        1,
        "an old in-flight ack cannot settle a new revision"
    );
    let conflict = Completed {
        actor: "you".into(),
        items: vec![(answer, Outcome::Conflict(2))],
    };
    let mut held = Page::read("you", &document).unwrap();
    held.acknowledged(&conflict);
    assert_eq!(held.answer("a", "asked").unwrap().revision, 2);
    assert_eq!(
        held.answer("a", "asked").unwrap().kind,
        None,
        "no body source was acquired for this revision"
    );
    let mut late = Page::read("you", &document).unwrap();
    late.retain_acknowledged(&held);
    assert_eq!(late.answer("a", "asked").unwrap().revision, 2);
    assert_eq!(late.count("a").total, 1);
}

#[test]
fn concurrent_ack_old_batch_adopts_highest_revision() {
    use super::publish::{Completed, Outcome};
    let document = page(vec![reply("asked", "a", 100)], false);
    let sent = Page::read("you", &document)
        .unwrap()
        .answer("a", "asked")
        .unwrap()
        .clone();
    let mut revision_two = document.clone();
    revision_two["items"][0]["revision"] = json!(2);
    let mut held = Page::read("you", &revision_two).unwrap();
    let conflict = Completed {
        actor: "you".into(),
        items: vec![(sent, Outcome::Conflict(3))],
    };
    held.acknowledged(&conflict);
    assert_eq!(held.answer("a", "asked").unwrap().revision, 3);
    assert_eq!(held.answer("a", "asked").unwrap().kind, None);
    assert_eq!(held.count("a").total, 1);
    let mut revision_four = document;
    revision_four["items"][0]["revision"] = json!(4);
    let mut fresh = Page::read("you", &revision_four).unwrap();
    fresh.acknowledged(&conflict);
    assert_eq!(fresh.answer("a", "asked").unwrap().revision, 4);
    assert_eq!(
        fresh.answer("a", "asked").unwrap().kind,
        Some(BodyKind::Final)
    );
}

#[test]
fn review_partial_counts_follow_each_members_oldest_answer() {
    let items = (0..50)
        .map(|i| {
            let mut item = reply(
                &format!("r{i:02}"),
                if i % 2 == 0 { "a" } else { "b" },
                1000 - i,
            );
            if i == 48 {
                item["acknowledged"] = json!(true);
            }
            item
        })
        .collect();
    let mut observed = Page::read("you", &page(items, true)).unwrap();
    assert_eq!(observed.count("a").label().as_deref(), Some("24 unread"));
    assert_eq!(observed.count("b").label().as_deref(), Some("25+ unread"));
    assert!(!observed.count("absent").more);
    let oldest_b = observed.answer("b", "r49").unwrap().clone();
    observed.acknowledged(&publish::Completed {
        actor: "you".into(),
        items: vec![(oldest_b, publish::Outcome::Acknowledged)],
    });
    assert_eq!(observed.count("b").label().as_deref(), Some("24 unread"));
}
