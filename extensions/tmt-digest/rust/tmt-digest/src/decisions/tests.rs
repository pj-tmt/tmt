use super::*;
use crate::history::{Decision, Rule};

const ID: &str = "10000000-0000-4000-8000-000000000001";

fn row(at_ms: u64, rule: Rule, outcome: Option<&'static str>) -> Decision {
    let interval = !matches!(rule, Rule::AutoIdlePoint | Rule::Off);
    Decision {
        at_ms,
        identity_id: ID.into(),
        interval_ms: interval.then_some(20_000),
        flush_count: 10,
        rule,
        held_count: 2,
        oldest_held_age_ms: Some(1_500),
        outcome,
    }
}

#[test]
fn a_recorded_day_lists_every_row_with_its_rule_and_outcome() {
    let names = BTreeMap::from([(ID.to_owned(), "worker".to_owned())]);
    let rows = [
        row(1_000, Rule::NotDue, None),
        row(2_000, Rule::IntervalDeadline, Some("delivered")),
        row(3_000, Rule::AutoIdlePoint, Some("not_idle")),
    ];
    let document = document(Since::Day, 90_000_000, &rows, &names);
    assert_eq!(
        document,
        json!({
            "version": 1, "since": "24h", "fromMs": 3_600_000, "throughMs": 90_000_000,
            "decisions": [
                {"atMs":1000,"identityId":ID,"name":"worker","mode":"interval","intervalMs":20000,
                 "flushCount":10,"rule":"not_due","heldCount":2,"oldestHeldAgeMs":1500,"outcome":null},
                {"atMs":2000,"identityId":ID,"name":"worker","mode":"interval","intervalMs":20000,
                 "flushCount":10,"rule":"interval_deadline","heldCount":2,"oldestHeldAgeMs":1500,"outcome":"delivered"},
                {"atMs":3000,"identityId":ID,"name":"worker","mode":"auto","intervalMs":null,
                 "flushCount":10,"rule":"auto_idle_point","heldCount":2,"oldestHeldAgeMs":1500,"outcome":"not_idle"},
            ]
        })
    );
    // Every delivery names its rule; only a postponement has no answer.
    for row in document["decisions"].as_array().unwrap() {
        assert!(row["rule"].is_string());
        assert_eq!(row["outcome"].is_null(), row["rule"] == "not_due");
    }
}

#[test]
fn a_member_missing_from_the_roster_is_listed_without_a_name_and_an_empty_day_is_empty() {
    let listed = document(
        Since::Week,
        10 * 86_400_000,
        &[row(1, Rule::Off, Some("delivered"))],
        &BTreeMap::new(),
    );
    assert_eq!(listed["decisions"][0]["name"], Value::Null);
    assert_eq!(listed["fromMs"], 3 * 86_400_000);
    assert_eq!(
        document(Since::Month, 0, &[], &BTreeMap::new())["decisions"],
        json!([])
    );
}
