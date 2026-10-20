use super::*;
use crate::export::conversations::{Comment, Head, Thread};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("../../../contracts/vectors/needs-you-v1.json")).unwrap()
}
fn comment(id: &str, at: &str, body: &str, deleted: bool) -> Comment {
    Comment {
        writer: "10000000-0000-4000-8000-000000000101".into(),
        id: id.into(),
        revision: "1".into(),
        sequence: "1".into(),
        deleted,
        body: body.into(),
        device_name: "  Agent   One ".into(),
        at: at.into(),
        attachments: Vec::new(),
    }
}
fn thread(id: &str, deleted: bool, comments: Vec<Comment>) -> Thread {
    Thread {
        writer: "10000000-0000-4000-8000-000000000101".into(),
        id: id.into(),
        revision: "1".into(),
        anchor: None,
        resolved: false,
        deleted,
        device_name: "Fixture".into(),
        at: "1".into(),
        comments,
        proposal: None,
        decision: None,
        status: None,
        notifications: Vec::new(),
    }
}
fn view(threads: Vec<Thread>) -> Conversations {
    Conversations {
        format: "fixture",
        version: 1,
        space_id: "fixture".into(),
        page_id: "fixture".into(),
        title: "Page".into(),
        epoch: "1".into(),
        membership_head: Head {
            revision: "1".into(),
            statement_hash: String::new(),
        },
        threads,
        asks: Vec::new(),
    }
}

#[test]
fn every_pending_item_of_the_shared_vectors_is_named_review_first_and_in_page_order_within_a_kind()
{
    for case in vectors()["cases"].as_array().unwrap() {
        let conversations = crate::summary_pending::tests::view(case);
        let writers = case["deviceWriters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let (report, anchors) =
            crate::page_check::summary(case["source"].as_str().unwrap(), &conversations.threads)
                .unwrap();
        let pending = crate::summary_pending::items(&report, &anchors, &conversations, &writers);
        let named = needs(&report, &conversations, &pending);
        let name = case["name"].as_str().unwrap();
        // The same targets as the count, none lost and none added.
        let mut counted: Vec<_> = pending.iter().map(|item| item.id.clone()).collect();
        let mut shown: Vec<_> = named.iter().map(|need| need.id.clone()).collect();
        counted.sort();
        shown.sort();
        assert_eq!(shown, counted, "{name}");
        assert!(
            named
                .windows(2)
                .all(|pair| pair[0].kind.rank() <= pair[1].kind.rank()),
            "{name}: review, question, reading, proposal"
        );
        assert!(
            named
                .iter()
                .all(|need| !need.title.is_empty() && need.title.chars().count() <= 200),
            "{name}"
        );
    }
}

#[test]
fn a_question_group_is_named_by_its_title_and_a_reading_carries_its_reason() {
    let cases = vectors();
    let case = &cases["cases"][0];
    let conversations = crate::summary_pending::tests::view(case);
    let writers = case["deviceWriters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let (report, anchors) =
        crate::page_check::summary(case["source"].as_str().unwrap(), &conversations.threads)
            .unwrap();
    let pending = crate::summary_pending::items(&report, &anchors, &conversations, &writers);
    let named = needs(&report, &conversations, &pending);
    let by_kind = |kind| named.iter().find(|need| need.kind == kind).unwrap();
    assert_eq!(by_kind(NeedKind::Question).title, "Decisions");
    assert_eq!(
        by_kind(NeedKind::Question).agent.as_deref(),
        Some("Agent 1")
    );
    assert_eq!(by_kind(NeedKind::Reading).title, "Read plan");
    assert_eq!(
        by_kind(NeedKind::Reading).why.as_deref(),
        Some("Read first")
    );
    assert_eq!(by_kind(NeedKind::Proposal).title, "Choose release");
    assert_eq!(
        by_kind(NeedKind::Proposal).agent.as_deref(),
        Some("Agent 1")
    );
    assert_eq!(by_kind(NeedKind::Review).title, "Review scope");
    assert_eq!(by_kind(NeedKind::Review).agent.as_deref(), Some("Agent 1"));
}

#[test]
fn only_the_first_twenty_by_rank_are_named_while_the_count_stays_whole() {
    let case = &vectors()["cases"][0];
    let proposal = |n: usize| {
        let mut row = thread(
            &format!("10000000-0000-4000-8000-{n:012}"),
            false,
            Vec::new(),
        );
        row.proposal = Some(
            serde_json::from_value(serde_json::json!({
                "proposalId": format!("10000000-0000-4000-8000-{n:012}"),
                "title": format!("Proposal {n}"),
                "body": "Please decide",
                "proposer": {
                    "machineId": "10000000-0000-4000-8000-000000000101",
                    "agentId": "10000000-0000-4000-8000-000000000101",
                    "label": "Agent 1"
                }
            }))
            .unwrap(),
        );
        row
    };
    let conversations = view((1..=25).map(proposal).collect());
    let (report, anchors) =
        crate::page_check::summary("<p>Plain</p>", &conversations.threads).unwrap();
    let pending =
        crate::summary_pending::items(&report, &anchors, &conversations, &Default::default());
    assert_eq!(pending.len(), 25, "{}", case["name"]);
    let named = needs(&report, &conversations, &pending);
    assert_eq!(named.len(), NEEDS_PASSAGES);
    assert_eq!(named[0].title, "Proposal 1", "page order inside a kind");
}

#[test]
fn the_latest_comment_is_the_newest_live_one_that_is_not_a_control_record() {
    let conversations = view(vec![
        thread(
            "10000000-0000-4000-8000-000000000001",
            false,
            vec![
                comment("a", "100", "First  thought", false),
                comment("b", "300", "Deleted later words", true),
                comment("c", "200", "Confirmed \u{201c}Plan\u{201d}", false),
            ],
        ),
        thread(
            "10000000-0000-4000-8000-000000000002",
            false,
            vec![comment("d", "250", "A reply\nover  two lines", false)],
        ),
        thread(
            "10000000-0000-4000-8000-000000000003",
            true,
            vec![comment("e", "900", "Inside a deleted thread", false)],
        ),
    ]);
    let latest = latest(&conversations).unwrap();
    assert_eq!(
        latest.text, "A reply over two lines",
        "folded whitespace, newest by clock"
    );
    assert_eq!(latest.at_ms, 250);
    assert_eq!(latest.who.as_deref(), Some("Agent One"));
    // Only control records and deleted words: no excerpt, never an empty one.
    let none = view(vec![thread(
        "10000000-0000-4000-8000-000000000004",
        false,
        vec![
            comment("f", "5", "Read \u{201c}Plan\u{201d}\nReading: x", false),
            comment("g", "6", "   ", false),
            comment("h", "7", "Unparsable clock", false),
        ],
    )]);
    let mut none = none;
    none.threads[0].comments[2].at = "later".into();
    assert_eq!(super::latest(&none), None);
}

#[test]
fn a_long_comment_is_clipped_to_the_excerpt_bound_in_code_points() {
    let long = "界".repeat(LATEST_POINTS + 40);
    let conversations = view(vec![thread(
        "10000000-0000-4000-8000-000000000001",
        false,
        vec![comment("a", "1", &long, false)],
    )]);
    assert_eq!(
        latest(&conversations).unwrap().text.chars().count(),
        LATEST_POINTS
    );
}

#[test]
fn control_characters_never_reach_the_browser_and_an_empty_label_is_left_out() {
    let mut row = comment("a", "1", "bell\u{7}ring\u{0}text", false);
    row.device_name = "\u{7}".into();
    let conversations = view(vec![thread(
        "10000000-0000-4000-8000-000000000001",
        false,
        vec![row],
    )]);
    let latest = latest(&conversations).unwrap();
    assert_eq!(latest.text, "bell ring text");
    assert_eq!(latest.who, None);
    assert_eq!(
        serde_json::to_value(&latest).unwrap(),
        serde_json::json!({"atMs":1,"text":"bell ring text"})
    );
}
