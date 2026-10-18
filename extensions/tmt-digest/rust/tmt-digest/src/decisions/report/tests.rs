use super::*;
use crate::history::Rule;

const A: &str = "10000000-0000-4000-8000-000000000001";
const B: &str = "10000000-0000-4000-8000-000000000002";

/// A settled checklist whose messages arrived at `arrivals`; `complete` false keeps
/// two of its messages out of the retained items.
fn checklist(
    sequence: u64,
    claimed: u64,
    settled: u64,
    arrivals: &[u64],
    outcome: &str,
    complete: bool,
) -> ChecklistSettlement {
    let sealed = arrivals.len() as u64 + if complete { 0 } else { 2 };
    serde_json::from_value(json!({
        "checklistId": format!("20000000-0000-4000-8000-{sequence:012}"),
        "settlementSequence": sequence, "claimedAtMs": claimed, "settledAtMs": settled,
        "outcome": outcome, "opportunity": "idle",
        "sealedCount": sealed, "retainedCount": arrivals.len(),
        "membershipComplete": complete, "nextItemAfter": null,
        "items": arrivals.iter().enumerate().map(|(index, at)| json!({
            "sequence": index + 1, "createdAtMs": at,
            "contextTokensAtArrival": null, "contextObservedAtMs": null
        })).collect::<Vec<_>>(),
    }))
    .unwrap()
}

fn delivered(at_ms: u64, rule: Rule) -> Decision {
    Decision {
        at_ms,
        identity_id: A.into(),
        interval_ms: matches!(
            rule,
            Rule::IntervalDeadline | Rule::FlushCount | Rule::NotDue
        )
        .then_some(20_000),
        flush_count: 10,
        rule,
        held_count: 2,
        oldest_held_age_ms: Some(1_000),
        outcome: Some("delivered"),
    }
}

fn run(
    settlements: &[(String, ChecklistSettlement)],
    decisions: &[Decision],
    recompute: Option<Recompute>,
) -> Value {
    document(&Input {
        since: "24h",
        from_ms: 0,
        through_ms: 1_000_000,
        settlements,
        decisions,
        coverage: &BTreeMap::from([(
            A.to_owned(),
            Coverage {
                started: 0,
                through: 900_000,
                raw_from: 0,
                gap: None,
            },
        )]),
        names: &BTreeMap::from([(A.to_owned(), "worker".to_owned())]),
        recompute,
    })
}

fn rows(member: &str, items: Vec<ChecklistSettlement>) -> Vec<(String, ChecklistSettlement)> {
    items
        .into_iter()
        .map(|item| (member.to_owned(), item))
        .collect()
}

#[test]
fn every_report_carries_the_baseline_and_the_turns_saved_beside_the_actual_delay() {
    // Three deliveries took 6 messages: delays 40/30/20 s, 5 s, 15/10 s.
    let settlements = rows(
        A,
        vec![
            checklist(
                1,
                50_000,
                60_000,
                &[20_000, 30_000, 40_000],
                "delivered",
                true,
            ),
            checklist(2, 95_000, 100_000, &[95_000], "delivered", true),
            checklist(3, 140_000, 150_000, &[135_000, 140_000], "delivered", true),
        ],
    );
    let document = run(&settlements, &[], None);
    assert_eq!(document["baseline"], "each message delivered on its own");
    let member = &document["members"][0];
    assert_eq!(member["id"], A);
    assert_eq!(member["name"], "worker");
    assert_eq!(member["messages"], 6);
    assert_eq!(member["baseline"], json!({"deliveries": 6}));
    assert_eq!(member["actual"]["deliveries"], 3);
    assert_eq!(member["turnsSaved"], 3);
    // Delays in ms, sorted: 5000 10000 15000 20000 30000 40000.
    assert_eq!(
        member["actual"]["delayMs"],
        json!({"median": 15_000, "p90": 40_000, "max": 40_000})
    );
    assert_eq!(document["total"]["turnsSaved"], 3);
    assert_eq!(document["total"]["baseline"], json!({"deliveries": 6}));
    // Without an alternative nothing is recomputed or labelled as an estimate.
    assert!(member.get("recomputed").is_none());
    assert!(document.get("recompute").is_none());
}

#[test]
fn members_total_by_pooling_and_what_cannot_be_measured_is_counted_not_hidden() {
    let mut settlements = rows(
        A,
        vec![
            checklist(1, 0, 20_000, &[10_000, 15_000], "delivered", true),
            checklist(2, 0, 30_000, &[25_000], "uncertain", true),
            checklist(3, 0, 40_000, &[35_000], "delivered", false),
            // A message that arrived after the checklist settled is a clock fault.
            checklist(4, 0, 50_000, &[60_000], "delivered", true),
        ],
    );
    settlements.extend(rows(
        B,
        vec![checklist(5, 0, 45_000, &[40_000], "delivered", true)],
    ));
    let document = run(&settlements, &[], None);
    let a = &document["members"][0];
    assert_eq!(a["messages"], 2);
    assert_eq!(
        a["excluded"],
        json!({"notDelivered": 1, "incomplete": 1, "clock": 1})
    );
    // B has no coverage row and no name.
    let b = &document["members"][1];
    assert_eq!(
        (b["id"].as_str(), b["name"].clone()),
        (Some(B), Value::Null)
    );
    assert_eq!(b["coverage"], Value::Null);
    let total = &document["total"];
    assert_eq!(total["messages"], 3);
    assert_eq!(total["actual"]["deliveries"], 2);
    assert_eq!(total["turnsSaved"], 1);
    assert_eq!(
        total["excluded"],
        json!({"notDelivered": 1, "incomplete": 1, "clock": 1})
    );
    // Delays pool across members: 10 s, 5 s, 5 s.
    assert_eq!(total["actual"]["delayMs"]["max"], 10_000);
}

#[test]
fn nothing_delivered_is_zero_turns_saved_and_no_delay_not_a_guess() {
    let document = run(
        &[],
        &[],
        Some(Recompute {
            interval_ms: 60_000,
            flush_count: 10,
        }),
    );
    assert_eq!(document["members"], json!([]));
    let total = &document["total"];
    assert_eq!(total["messages"], 0);
    assert_eq!(total["actual"]["delayMs"], Value::Null);
    assert_eq!(total["recomputed"]["delayMs"], Value::Null);
    assert_eq!(total["turnsSaved"], 0);
}

#[test]
fn a_recomputed_interval_batches_by_arrival_and_is_labelled_an_estimate() {
    let arrivals = [0, 10_000, 20_000, 100_000, 105_000];
    let settlements = rows(
        A,
        vec![checklist(1, 0, 110_000, &arrivals, "delivered", true)],
    );
    let document = run(
        &settlements,
        &[],
        Some(Recompute {
            interval_ms: 30_000,
            flush_count: 10,
        }),
    );
    // 30 s after the first arrival takes 0, 10 and 20 s together; 100 s starts the next.
    let member = &document["members"][0];
    assert_eq!(member["recomputed"]["deliveries"], 2);
    assert_eq!(member["recomputed"]["turnsSaved"], 3);
    // Delays: 30 20 10 and 30 25 s.
    assert_eq!(
        member["recomputed"]["delayMs"],
        json!({"median": 25_000, "p90": 30_000, "max": 30_000})
    );
    // The same messages were actually delivered at once, 110 s after the first.
    assert_eq!(member["actual"]["deliveries"], 1);
    assert_eq!(member["actual"]["delayMs"]["max"], 110_000);
    let recompute = &document["recompute"];
    assert_eq!(recompute["method"], METHOD);
    assert_eq!(recompute["estimate"], true);
    assert_eq!(recompute["intervalMs"], 30_000);
    assert!(
        recompute["notes"][0]
            .as_str()
            .unwrap()
            .contains("lower bound")
    );
}

#[test]
fn reaching_the_flush_count_delivers_at_that_arrival() {
    let (deliveries, delays) = batch(
        &[0, 1_000, 2_000, 3_000],
        Recompute {
            interval_ms: 3_600_000,
            flush_count: 3,
        },
    );
    // 0, 1 and 2 s go at the third arrival; the fourth waits out the interval.
    assert_eq!(deliveries, 2);
    assert_eq!(delays, [2_000, 1_000, 0, 3_600_000]);
    // A flush count of one delivers every message as it arrives.
    let (deliveries, delays) = batch(
        &[0, 5, 5, 9],
        Recompute {
            interval_ms: 1_000,
            flush_count: 1,
        },
    );
    assert_eq!((deliveries, delays), (3, vec![0, 0, 0, 0]));
    // No messages is no delivery.
    assert_eq!(
        batch(
            &[],
            Recompute {
                interval_ms: 1,
                flush_count: 1
            }
        ),
        (0, vec![])
    );
}

#[test]
fn each_delivery_is_matched_to_one_recorded_rule_or_counted_unrecorded() {
    let settlements = rows(
        A,
        vec![
            checklist(1, 10_000, 20_000, &[9_000], "delivered", true),
            checklist(2, 30_000, 40_000, &[29_000], "delivered", true),
            checklist(3, 50_000, 60_000, &[49_000], "delivered", true),
        ],
    );
    let decisions = [
        delivered(20_500, Rule::IntervalDeadline),
        // Another member's row and a row long after are not this member's delivery.
        Decision {
            identity_id: B.into(),
            ..delivered(40_500, Rule::Off)
        },
        delivered(40_900, Rule::FlushCount),
    ];
    let document = run(&settlements, &decisions, None);
    assert_eq!(
        document["members"][0]["actual"]["byRule"],
        json!({"flush_count": 1, "interval_deadline": 1, "unrecorded": 1})
    );
    // A row is used once: two deliveries cannot share it.
    let twice = rows(
        A,
        vec![
            checklist(1, 10_000, 20_000, &[9_000], "delivered", true),
            checklist(2, 11_000, 21_000, &[10_000], "delivered", true),
        ],
    );
    let document = run(&twice, &[delivered(21_500, Rule::Off)], None);
    assert_eq!(
        document["members"][0]["actual"]["byRule"],
        json!({"off": 1, "unrecorded": 1})
    );
}

#[test]
fn coverage_says_where_the_evidence_begins_and_whether_it_has_a_gap() {
    let settlements = rows(
        A,
        vec![checklist(1, 0, 20_000, &[10_000], "delivered", true)],
    );
    let cover = |coverage: Coverage| {
        document(&Input {
            since: "24h",
            from_ms: 100,
            through_ms: 1_000_000,
            settlements: &settlements,
            decisions: &[],
            coverage: &BTreeMap::from([(A.to_owned(), coverage)]),
            names: &BTreeMap::new(),
            recompute: None,
        })["members"][0]["coverage"]
            .clone()
    };
    let whole = cover(Coverage {
        started: 50,
        through: 900_000,
        raw_from: 0,
        gap: None,
    });
    assert_eq!(whole["startsAtMs"], 100);
    assert_eq!(whole["complete"], true);
    // Collection began inside the window, or older settlements were evicted.
    let late = cover(Coverage {
        started: 400,
        through: 900_000,
        raw_from: 0,
        gap: None,
    });
    assert_eq!(
        (late["startsAtMs"].clone(), late["complete"].clone()),
        (json!(400), json!(false))
    );
    let evicted = cover(Coverage {
        started: 0,
        through: 900_000,
        raw_from: 700,
        gap: None,
    });
    assert_eq!(evicted["startsAtMs"], 700);
    let gap = cover(Coverage {
        started: 0,
        through: 900_000,
        raw_from: 0,
        gap: Some("missing_deliveries".into()),
    });
    assert_eq!(
        (gap["complete"].clone(), gap["gap"].clone()),
        (json!(false), json!("missing_deliveries"))
    );
    assert_eq!(gap["collectedThroughMs"], 900_000);
}
