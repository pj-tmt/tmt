use super::*;
use serde_json::json;

fn reply(request: &str, member: &str, at: u64) -> Value {
    json!({"requestId":request,"recipientId":member,"sender":{"identityId":"you"},
        "kind":"request","recipientAcknowledged":true,
        "final":{"status":"retained","submittedAtMs":at}})
}

fn page(items: Vec<Value>, more: bool) -> Value {
    let cursor = if more {
        json!({"submittedAtMs":items.last().unwrap()["final"]["submittedAtMs"],
            "requestId":items.last().unwrap()["requestId"]})
    } else {
        Value::Null
    };
    json!({"items":items,"nextBefore":cursor})
}

#[test]
fn each_marker_owns_the_boundary_and_known_zero_has_no_unread_label() {
    let observed = Page::read(
        "you",
        &page(
            vec![reply("new-b", "b", 110), reply("old-a", "a", 100)],
            true,
        ),
    )
    .unwrap();
    assert_eq!(observed.count("a", 200), Count::default());
    assert_eq!(
        observed.count("b", 10).label().as_deref(),
        Some("1+ unread")
    );
    assert_eq!(
        observed.count("b", 100).label().as_deref(),
        Some("1 unread")
    );
    let unobserved = observed.count("c", 10);
    assert!(unobserved.more);
    assert_eq!(unobserved.label(), None);
    assert_eq!(observed.newest("b"), Some(110));
    assert_eq!(observed.newest("c"), None);
    let mut aggregate = observed.count("b", 100);
    aggregate.add(unobserved);
    assert_eq!(aggregate.label().as_deref(), Some("1+ unread"));
}

#[test]
fn only_newer_submissions_count_independently_of_core_ack_and_room_duplicates() {
    let same = reply("same", "a", 100);
    let observed = Page::read(
        "you",
        &page(vec![reply("new", "a", 110), same.clone(), same], false),
    )
    .unwrap();
    assert_eq!(observed.count("a", 0).total, 2);
    assert_eq!(
        observed.count("a", 100).label().as_deref(),
        Some("1 unread")
    );
    assert_eq!(observed.count("a", 110), Count::default());
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
    let mut withdrawn = valid.clone();
    withdrawn["items"][0]["final"]["status"] = json!("withdrawn");
    let mut invalid_time = valid.clone();
    invalid_time["items"][0]["final"]["submittedAtMs"] = json!(0);
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
        withdrawn,
        invalid_time,
        conflict,
        too_many,
    ] {
        assert!(Page::read("you", &invalid).is_none());
    }
}
