//! Observable formula, private persistence and window evidence, independent inputs.

use super::test_support::{Fixture, ID, event};
use super::*;
use crate::history::{Decision, Rule, collect, model};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
fn vectors() -> Value {
    serde_json::from_str(
        &fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/vectors/savings-v1.json"
        ))
        .expect("the supplier's frozen savings vectors"),
    )
    .unwrap()
}
#[test]
fn accepted_counterfactual_and_unknowns_have_independent_expected_values() {
    let vectors: Value = vectors();
    for vector in vectors["events"].as_array().unwrap() {
        let event: model::ChecklistSettlement =
            serde_json::from_value(vector["input"].clone()).unwrap();
        event.validate().unwrap();
        let result = event.estimate();
        assert_eq!(
            json!({"tokens":result.tokens,"avoided":result.avoided,"gap":result.gap}),
            vector["expected"],
            "{}",
            vector["name"]
        );
    }
}
#[test]
fn restart_deduplication_atomic_rollback_and_read_only_projection() {
    let fixture = Fixture::new();
    let start = 10 * DAY;
    let mut store = fixture.store();
    store.baseline(ID, start, 5, 11).unwrap();
    let batch = event(6, start + 100, &[Some(1000), Some(1200), Some(1500)]);
    store.record(ID, &batch, start + 200).unwrap();
    store.record(ID, &batch, start + 200).unwrap();
    store.checkpoint(ID, start + 200, 7, 12).unwrap();
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, start + 200, Some(12))
            .unwrap(),
        json!({"method":METHOD,"window":"lifetime","fromMs":start,"throughMs":start+200,"estimatedInputTokensAvoided":2700,"avoidedSeparateDeliveries":2,"complete":true,"gap":null,"cost":null,"percent":null})
    );
    store.write(|tx| tx.execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;").map_err(io)).unwrap();
    assert!(
        store
            .record(ID, &event(8, start + 250, &[None]), start + 300)
            .is_err()
    );
    assert_eq!(store.cursor(ID).unwrap(), Some(7));
    store
        .write(|tx| tx.execute_batch("DROP TRIGGER fail_event").map_err(io))
        .unwrap();
    drop(store);
    let file = fixture.0.join("digest/history/history.sqlite");
    let before = fs::read(&file).unwrap();
    let mut read = Store::open(&fixture.0, false).unwrap().unwrap();
    assert_eq!(read.cursor(ID).unwrap(), Some(7));
    assert_eq!(
        read.status_built_in(ID, Window::Lifetime, start + 200, Some(12))
            .unwrap()["estimatedInputTokensAvoided"],
        2700
    );
    assert_eq!(fs::read(file).unwrap(), before);
    assert_eq!(
        fs::read_dir(fixture.0.join("digest/history"))
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn windows_distinguish_before_collection_known_zero_missing_context_and_counter_gaps() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let start = 10 * DAY;
    store.baseline(ID, start, 5, 0).unwrap();
    assert_eq!(
        store
            .status_built_in(ID, Window::Day, start, Some(0))
            .unwrap()["gap"],
        "before_collection"
    );
    store.checkpoint(ID, start + DAY, 6, 0).unwrap();
    let empty = store
        .status_built_in(ID, Window::Day, start + DAY, Some(0))
        .unwrap();
    assert_eq!(empty["estimatedInputTokensAvoided"], 0);
    assert_eq!(empty["complete"], true);
    store
        .record(
            ID,
            &event(7, start + DAY + 10, &[None, None]),
            start + DAY + 20,
        )
        .unwrap();
    store.checkpoint(ID, start + DAY + 20, 8, 1).unwrap();
    let unknown = store
        .status_built_in(ID, Window::Day, start + DAY + 20, Some(1))
        .unwrap();
    assert_eq!(unknown["estimatedInputTokensAvoided"], Value::Null);
    assert_eq!(unknown["avoidedSeparateDeliveries"], 1);
    assert_eq!(unknown["gap"], "missing_context");
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, start + DAY + 20, Some(2))
            .unwrap()["gap"],
        "uncollected_deliveries"
    );
    store.checkpoint(ID, start + DAY + 30, 10, 3).unwrap();
    assert_eq!(
        store
            .status_built_in(ID, Window::Day, start + DAY + 30, Some(3))
            .unwrap()["gap"],
        "missing_deliveries"
    );
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, start + DAY + 30, Some(3))
            .unwrap()["complete"],
        false
    );
    store.checkpoint(ID, start + 9 * DAY, 11, 3).unwrap();
    assert_eq!(
        store
            .status_built_in(ID, Window::Week, start + 9 * DAY, Some(3))
            .unwrap()["estimatedInputTokensAvoided"],
        0
    );
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, start + 9 * DAY, Some(3))
            .unwrap()["estimatedInputTokensAvoided"],
        Value::Null
    );
}
#[test]
fn raw_retention_and_capacity_eviction_preserve_lifetime_totals() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let start = 10 * DAY;
    store.baseline(ID, start, 1, 0).unwrap();
    store
        .record(ID, &event(2, start + 10, &[Some(7), Some(9)]), start + 20)
        .unwrap();
    store.checkpoint(ID, start + 20, 3, 1).unwrap();
    {
        store
            .write(|tx| {
                prune(tx, start + 20, 0).unwrap();
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, start + 20, Some(1))
            .unwrap()["estimatedInputTokensAvoided"],
        9
    );
    assert_eq!(
        store
            .status_built_in(ID, Window::Day, start + 20, Some(1))
            .unwrap()["complete"],
        false
    );
    store
        .record(ID, &event(4, start + 30, &[None]), start + 40)
        .unwrap();
    store.maintain(start + RETENTION + DAY).unwrap();
    assert_eq!(
        store
            .status_built_in(ID, Window::Day, start + RETENTION + DAY, Some(2))
            .unwrap()["complete"],
        false
    );
    store.checkpoint(ID, start + RETENTION + DAY, 5, 2).unwrap();
    assert_eq!(
        store
            .read(|tx| tx
                .query_row("SELECT COUNT(*) FROM events", [], |r| number(r, 0))
                .map_err(io))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, start + RETENTION + DAY, Some(2))
            .unwrap()["estimatedInputTokensAvoided"],
        9
    );
    assert_eq!(
        store
            .status_built_in(ID, Window::Week, start + RETENTION + DAY, Some(2))
            .unwrap()["complete"],
        true
    );
}
#[test]
fn clock_rollback_and_store_admission_never_create_zero_evidence() {
    let fixture = Fixture::new();
    assert!(Store::open(&fixture.0, false).unwrap().is_none());
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
    let mut store = fixture.store();
    store.baseline(ID, 1000, 1, 0).unwrap();
    store.record(ID, &event(2, 900, &[None]), 1001).unwrap();
    store.checkpoint(ID, 1001, 3, 1).unwrap();
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, 1001, Some(1))
            .unwrap()["gap"],
        "clock_discontinuity"
    );
    store
        .write(|tx| tx.pragma_update(None, "user_version", 4).map_err(io))
        .unwrap();
    drop(store);
    assert!(Store::open(&fixture.0, true).is_err());
    let before = fs::read(fixture.0.join("digest/history/history.sqlite")).unwrap();
    assert!(Store::open(&fixture.0, false).is_err());
    assert_eq!(
        fs::read(fixture.0.join("digest/history/history.sqlite")).unwrap(),
        before
    );
    fs::set_permissions(
        fixture.0.join("digest/history"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(Store::open(&fixture.0, true).is_err());
    let other = Fixture::new();
    symlink(fixture.0.join("digest"), other.0.join("digest")).unwrap();
    assert!(Store::open(&other.0, true).is_err());
}

#[test]
fn capacity_failure_rolls_back_event_cursor_and_lifetime_totals() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.baseline(ID, 1000, 1, 0).unwrap();
    let pages: u64 = store
        .read(|tx| {
            tx.pragma_query_value(None, "page_count", |r| number(r, 0))
                .map_err(io)
        })
        .unwrap();
    store
        .write(|tx| {
            tx.pragma_update(None, "max_page_count", pages as i64)
                .map_err(io)
        })
        .unwrap();
    let batch = event(2, 1100, &vec![Some(12345); 512]);
    assert!(store.record(ID, &batch, 1200).is_err());
    assert_eq!(store.cursor(ID).unwrap(), Some(1));
    assert_eq!(
        store
            .read(|tx| tx
                .query_row("SELECT COUNT(*) FROM events", [], |r| number(r, 0))
                .map_err(io))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, 1200, Some(0))
            .unwrap()["estimatedInputTokensAvoided"],
        0
    );
}

#[test]
fn public_history_overflow_collection_is_idempotent_and_never_delivers() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let baseline = json!({"identityId":ID,"observedAtMs":1000,"throughSequence":5,"deliveredDigests":10,"checklists":[],"nextAfter":null});
    let mut first = event(6, 1050, &[None, Some(1200)]);
    first.sealed_count = Some(3);
    first.retained_count = 3;
    first.next_item_after = Some(2);
    let mut rest = event(6, 1050, &[Some(1500)]);
    rest.items[0].sequence = 3;
    rest.sealed_count = Some(3);
    rest.retained_count = 3;
    for (index, events) in [(2, vec![first]), (3, vec![rest])] {
        fs::write(fixture.0.join(format!("page.{index}")),serde_json::to_vec(&json!({"identityId":ID,"observedAtMs":1100,"throughSequence":8,"deliveredDigests":11,"checklists":events,"nextAfter":null})).unwrap()).unwrap();
    }
    fs::write(
        fixture.0.join("page.1"),
        serde_json::to_vec(&baseline).unwrap(),
    )
    .unwrap();
    fs::write(fixture.0.join("page.4"),serde_json::to_vec(&json!({"identityId":ID,"observedAtMs":1200,"throughSequence":9,"deliveredDigests":11,"checklists":[],"nextAfter":null})).unwrap()).unwrap();
    let script = format!(
        "#!/bin/sh\ncd '{}' || exit 9\ninput=$(cat)\nprintf '%s\\n' \"$input\" >> calls\nn=$(wc -l < calls | tr -d ' ')\ncat page.$n\n",
        fixture.0.display()
    );
    fs::write(fixture.0.join("core"), script).unwrap();
    fs::set_permissions(fixture.0.join("core"), fs::Permissions::from_mode(0o700)).unwrap();
    let core = crate::core::Core::fixture(fixture.0.join("core"));
    for _ in 0..3 {
        collect(
            &core,
            &mut store,
            ID,
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        )
        .unwrap();
    }
    assert_eq!(store.cursor(ID).unwrap(), Some(9));
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, 1200, Some(11))
            .unwrap()["estimatedInputTokensAvoided"],
        2700
    );
    assert_eq!(
        store
            .read(|tx| tx
                .query_row("SELECT COUNT(*) FROM events", [], |r| number(r, 0))
                .map_err(io))
            .unwrap(),
        1
    );
    let calls: Vec<Value> = fs::read_to_string(fixture.0.join("calls"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        calls
            .iter()
            .all(|call| call["operation"] == "digest.checklist.history")
    );
    assert_eq!(calls[2]["input"]["after"], 2);
    assert_eq!(
        calls[2]["input"]["checklistId"],
        "20000000-0000-4000-8000-000000000006"
    );
    assert_eq!(calls[3]["input"]["after"], 8);
}

#[test]
fn normative_status_vectors_cover_selected_windows_and_unknowns() {
    let vectors: Value = vectors();
    for vector in vectors["status"].as_array().unwrap() {
        let fixture = Fixture::new();
        let mut store = fixture.store();
        let given = &vector["given"];
        let start = given["startedAtMs"].as_u64().unwrap();
        let through = given["throughMs"].as_u64().unwrap();
        let baseline = given["baselineCounter"].as_u64().unwrap();
        store.baseline(ID, start, 5, baseline).unwrap();
        let mut deliveries = baseline;
        if let Some(name) = given["event"].as_str() {
            let source = vectors["events"]
                .as_array()
                .unwrap()
                .iter()
                .find(|event| event["name"] == name)
                .unwrap();
            let event: model::ChecklistSettlement =
                serde_json::from_value(source["input"].clone()).unwrap();
            store.record(ID, &event, through).unwrap();
            deliveries += u64::from(event.outcome == "delivered");
        }
        store.checkpoint(ID, through, 7, deliveries).unwrap();
        assert_eq!(
            store
                .status_built_in(
                    ID,
                    Window::parse(given["window"].as_str().unwrap()).unwrap(),
                    through,
                    given["counter"].as_u64()
                )
                .unwrap(),
            vector["expected"],
            "{}",
            vector["name"]
        );
    }
}

#[test]
fn interrupted_reconciliation_exposes_no_total_beyond_its_coverage_watermark() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.baseline(ID, 1000, 1, 0).unwrap();
    store
        .record(ID, &event(2, 1100, &[Some(100), Some(200)]), 1200)
        .unwrap();
    let interrupted = store
        .status_built_in(ID, Window::Lifetime, 1200, None)
        .unwrap();
    assert_eq!(interrupted["throughMs"], 1000);
    assert_eq!(interrupted["estimatedInputTokensAvoided"], Value::Null);
    assert_eq!(interrupted["gap"], "uncollected_deliveries");
    drop(store);
    let mut resumed = fixture.store();
    resumed.checkpoint(ID, 1200, 3, 1).unwrap();
    assert_eq!(
        resumed
            .status_built_in(ID, Window::Lifetime, 1200, Some(1))
            .unwrap()["estimatedInputTokensAvoided"],
        200
    );
}

fn decision(at_ms: u64, rule: Rule, outcome: Option<&'static str>) -> Decision {
    let interval = matches!(
        rule,
        Rule::IntervalDeadline | Rule::FlushCount | Rule::NotDue
    );
    Decision {
        at_ms,
        identity_id: ID.into(),
        interval_ms: interval.then_some(20_000),
        flush_count: 10,
        rule,
        held_count: 3,
        oldest_held_age_ms: Some(5_000),
        outcome,
    }
}

#[test]
fn decisions_round_trip_in_time_order_inside_an_open_closed_window() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let rows = [
        decision(30, Rule::FlushCount, Some("delivered")),
        decision(10, Rule::NotDue, None),
        decision(20, Rule::IntervalDeadline, Some("not_idle")),
        decision(40, Rule::AutoIdlePoint, Some("uncertain")),
        decision(50, Rule::Off, Some("nothing_due")),
        decision(60, Rule::AutoHold, Some("not_idle")),
        decision(70, Rule::AutoCap, Some("delivered")),
    ];
    for row in &rows {
        store.record_decision(row).unwrap();
    }
    let mut at = |from, through| -> Vec<u64> {
        store
            .decisions(from, through)
            .unwrap()
            .iter()
            .map(|row| row.at_ms)
            .collect()
    };
    assert_eq!(at(0, 100), [10, 20, 30, 40, 50, 60, 70]);
    assert_eq!(at(10, 40), [20, 30, 40], "the window is (from, through]");
    assert_eq!(store.decisions(0, 100).unwrap()[0], rows[1]);
    // A read-only open sees the same rows.
    drop(store);
    let mut reader = Store::open(&fixture.0, false).unwrap().unwrap();
    assert_eq!(reader.decisions(0, 100).unwrap().len(), 7);
    // Every rule round-trips through its name, and the auto ones are auto rows without an interval.
    for rule in [Rule::AutoIdlePoint, Rule::AutoHold, Rule::AutoCap] {
        assert_eq!(Rule::parse(rule.text()), Some(rule));
        let row = decision(1, rule, Some("delivered"));
        assert_eq!((row.mode(), row.interval_ms), ("auto", None));
    }
    assert_eq!(Rule::parse("auto_hold_cap"), None);
    assert!(reader.decisions(100, 0).is_err());
}

#[test]
fn a_store_from_before_decision_records_lists_none_and_never_gains_them_on_read() {
    let fixture = Fixture::new();
    drop(fixture.store());
    let path = fixture.0.join("digest/history/history.sqlite");
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE decisions; PRAGMA user_version=2;")
        .unwrap();
    drop(old);
    let mut reader = Store::open(&fixture.0, false).unwrap().unwrap();
    assert!(reader.decisions(0, 100).unwrap().is_empty());
    drop(reader);
    // The next tick adds the table and the new schema version.
    let mut store = fixture.store();
    store
        .record_decision(&decision(5, Rule::NotDue, None))
        .unwrap();
    assert_eq!(store.decisions(0, 100).unwrap().len(), 1);
}

#[test]
fn an_inconsistent_decision_is_refused_and_a_corrupt_row_fails_the_read() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let refused = |row: Decision| {
        let mut other = Store::open(&fixture.0, true).unwrap().unwrap();
        other.record_decision(&row).is_err()
    };
    // A delivery rule needs Core's answer; not_due has none.
    assert!(refused(decision(1, Rule::IntervalDeadline, None)));
    assert!(refused(decision(1, Rule::NotDue, Some("delivered"))));
    assert!(refused(decision(1, Rule::Off, Some("maybe"))));
    // The mode follows the rule: an interval only with an interval rule.
    let mut auto = decision(1, Rule::AutoIdlePoint, Some("delivered"));
    auto.interval_ms = Some(1);
    assert!(refused(auto));
    let mut interval = decision(1, Rule::FlushCount, Some("delivered"));
    interval.interval_ms = None;
    assert!(refused(interval));
    // An age exists exactly when something is held.
    let mut empty = decision(1, Rule::Off, Some("delivered"));
    empty.held_count = 0;
    assert!(refused(empty));
    let mut unsafe_time = decision(1, Rule::Off, Some("delivered"));
    unsafe_time.at_ms = MAX + 1;
    assert!(refused(unsafe_time));
    let mut stranger = decision(1, Rule::Off, Some("delivered"));
    stranger.identity_id = "worker".into();
    assert!(refused(stranger));
    assert!(store.decisions(0, 100).unwrap().is_empty());
    store.write(|tx| {
        tx.execute("INSERT INTO decisions(at_ms,identity_id,flush_count,rule,held,oldest_age_ms,outcome) VALUES(5,?1,10,'sometimes',1,1,'delivered')", [ID]).map_err(io)?;
        Ok(())
    }).unwrap();
    assert!(store.decisions(0, 100).is_err());
}

#[test]
fn decisions_keep_thirty_days_and_yield_to_a_full_budget_oldest_first() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let start = 40 * DAY;
    for (offset, rule, outcome) in [
        (0, Rule::NotDue, None),
        (10, Rule::IntervalDeadline, Some("delivered")),
        (20, Rule::FlushCount, Some("delivered")),
    ] {
        store
            .record_decision(&decision(start + offset, rule, outcome))
            .unwrap();
    }
    // Older than the retention goes on the next maintenance.
    store.maintain(start + RETENTION + 5).unwrap();
    let kept: Vec<u64> = store
        .decisions(0, MAX)
        .unwrap()
        .iter()
        .map(|row| row.at_ms - start)
        .collect();
    assert_eq!(kept, [10, 20]);
    // A budget that fits two rows drops only the older of three rows.
    store
        .record_decision(&decision(
            start + RETENTION + 6,
            Rule::Off,
            Some("delivered"),
        ))
        .unwrap();
    store
        .write(|tx| prune(tx, start + RETENTION + 7, 2 * decision::ROW_BYTES))
        .unwrap();
    let kept: Vec<u64> = store
        .decisions(0, MAX)
        .unwrap()
        .iter()
        .map(|row| row.at_ms - start)
        .collect();
    assert_eq!(kept, [20, RETENTION + 6]);
}

#[test]
fn the_newest_decision_per_member_is_found_by_time() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    assert_eq!(store.last_decision(ID).unwrap(), None);
    store
        .record_decision(&decision(20_000, Rule::FlushCount, Some("delivered")))
        .unwrap();
    store
        .record_decision(&decision(10_000, Rule::NotDue, None))
        .unwrap();
    let mut other = decision(30_000, Rule::Off, Some("delivered"));
    other.identity_id = "10000000-0000-4000-8000-000000000002".into();
    store.record_decision(&other).unwrap();
    assert_eq!(store.last_decision(ID).unwrap().unwrap().at_ms, 20_000);
    assert_eq!(
        store
            .last_decision(ID)
            .unwrap()
            .unwrap()
            .oldest_held_at_ms(),
        Some(15_000)
    );
}

#[test]
fn decisions_never_exceed_their_row_cap_whatever_the_flapping() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let start = 40 * DAY;
    store.write(|tx| {
        for n in 0..decision::MAX_ROWS + 5 {
            tx.execute("INSERT INTO decisions(at_ms,identity_id,interval_ms,flush_count,rule,held,oldest_age_ms) VALUES(?1,?2,20000,10,'not_due',1,1)", params![(start + n) as i64, ID]).unwrap();
        }
        Ok(())
    }).unwrap();
    store.maintain(start + decision::MAX_ROWS + 5).unwrap();
    let kept = store.decisions(0, MAX).unwrap();
    assert_eq!(kept.len() as u64, decision::MAX_ROWS);
    // The oldest five went.
    assert_eq!(kept[0].at_ms, start + 5);
}

#[test]
fn settlements_and_coverage_are_read_back_for_a_window_without_changing_the_store() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let start = 10 * DAY;
    store.baseline(ID, start, 1, 0).unwrap();
    store
        .record(ID, &event(2, start + 10, &[Some(7)]), start + 20)
        .unwrap();
    store
        .record(ID, &event(3, start + 30, &[Some(1), Some(2)]), start + 40)
        .unwrap();
    store.checkpoint(ID, start + 40, 3, 2).unwrap();
    let mut times = |from: u64, through: u64| -> Vec<u64> {
        store
            .settlements(from, through)
            .unwrap()
            .iter()
            .map(|(_, event)| event.settled_at_ms.unwrap() - start)
            .collect()
    };
    assert_eq!(times(0, MAX), [10, 30]);
    assert_eq!(
        times(start + 10, start + 30),
        [30],
        "the window is (from, through]"
    );
    assert_eq!(store.settlements(0, MAX).unwrap()[0].0, ID);
    assert!(store.settlements(5, 4).is_err());
    let coverage = store.coverage(start, start + 40).unwrap();
    let held = &coverage[ID];
    assert_eq!(
        (held.started, held.through, held.gap.clone()),
        (start, start + 40, None)
    );
    // A recorded gap inside the window is named.
    store
        .write(|tx| add_gap(tx, ID, start + 5, start + 15, "missing_deliveries"))
        .unwrap();
    assert_eq!(
        store.coverage(start, start + 40).unwrap()[ID]
            .gap
            .as_deref(),
        Some("missing_deliveries")
    );
    assert_eq!(
        store.coverage(start + 20, start + 40).unwrap()[ID].gap,
        None
    );
}

#[test]
fn a_refused_window_names_every_accepted_value_and_each_one_parses_back() {
    for given in ["", "1h", "24H", "week"] {
        assert_eq!(
            Window::parse(given).unwrap_err(),
            "Use 24h, 7d or lifetime",
            "{given:?}"
        );
    }
    for window in Window::ALL {
        assert_eq!(Window::parse(window.text()).unwrap().text(), window.text());
        assert!(Window::parse("x").unwrap_err().contains(window.text()));
    }
    assert_eq!(
        Window::ALL.map(Window::text),
        ["24h", "7d", "lifetime"],
        "the CLI lists the windows in this order"
    );
}

#[test]
fn a_history_writer_waits_for_the_admitted_transaction_and_keeps_both_changes() {
    use std::{sync::mpsc, time::Duration};

    let fixture = Fixture::new();
    let mut waiting = fixture.store();
    let file = fixture.0.join("digest/history/history.sqlite");
    // Open both connections before taking a lock: closing another descriptor for
    // this inode while SQLite owns a POSIX lock could invalidate the fixture.
    let mut blocker = Connection::open(&file).unwrap();
    let held = blocker
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    held.execute("INSERT INTO members(id,started,through,cursor,counter,raw_from) VALUES('other',1000,1000,1,0,1000)", []).unwrap();
    let (sent, received) = mpsc::channel();
    let (ready, started) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready.send(()).unwrap();
        let result = waiting.baseline(ID, 1000, 2, 0);
        sent.send(result).unwrap();
        waiting
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let early = received.recv_timeout(Duration::from_millis(300));
    let waited = matches!(early, Err(mpsc::RecvTimeoutError::Timeout));
    held.commit().unwrap();
    let result = match early {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            received.recv_timeout(Duration::from_secs(5)).unwrap()
        }
        Err(error) => panic!("writer exited without a result: {error}"),
    };
    let mut store = worker.join().unwrap();
    assert!(waited, "writer must wait for the admitted transaction");
    result.unwrap();
    assert_eq!(store.cursor(ID).unwrap(), Some(2));
    assert_eq!(store.cursor("other").unwrap(), Some(1));
}

#[test]
fn adopting_history_keeps_its_schema_budget_and_rollback_durability() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .read(|tx| {
            assert_eq!(
                tx.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
                    .unwrap(),
                "delete"
            );
            assert_eq!(
                tx.pragma_query_value(None, "synchronous", |r| number(r, 0))
                    .unwrap(),
                2
            );
            assert_eq!(
                tx.pragma_query_value(None, "max_page_count", |r| number(r, 0))
                    .unwrap(),
                PAGES
            );
            assert_eq!(
                tx.pragma_query_value(None, "user_version", |r| number(r, 0))
                    .unwrap(),
                3
            );
            assert_eq!(
                tx.query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name='_tmt_state_migrations'",
                    [],
                    |r| number(r, 0)
                )
                .unwrap(),
                0
            );
            Ok(())
        })
        .unwrap();
    store.baseline(ID, 1000, 1, 0).unwrap();
    drop(store);
    let file = fixture.0.join("digest/history/history.sqlite");
    let before = fs::read(&file).unwrap();
    let mut read = Store::open(&fixture.0, false).unwrap().unwrap();
    assert_eq!(read.cursor(ID).unwrap(), Some(1));
    let error = read
        .write::<()>(|_| panic!("a read-only store must never enter a write callback"))
        .unwrap_err();
    assert_eq!(error.code, "DIGEST_HISTORY_INVALID");
    drop(read);
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(
        fs::read_dir(fixture.0.join("digest/history"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn a_blocked_rollback_commit_keeps_the_timeout_consequence_and_never_replays() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.baseline(ID, 1000, 1, 0).unwrap();
    let file = fixture.0.join("digest/history/history.sqlite");
    let mut blocker = Connection::open(&file).unwrap();
    let held = blocker.transaction().unwrap();
    assert_eq!(
        held.query_row("SELECT cursor FROM members WHERE id=?1", [ID], |r| number(
            r, 0
        ))
        .unwrap(),
        1
    );
    let mut calls = 0;
    let error = store
        .write(|tx| {
            calls += 1;
            tx.execute("UPDATE members SET cursor=2 WHERE id=?1", [ID])
                .map_err(io)?;
            Ok(())
        })
        .unwrap_err();
    held.commit().unwrap();
    assert_eq!(calls, 1);
    assert_eq!(error.code, "DIGEST_HISTORY_IO");
    assert!(
        error.message.contains("commit write timed out"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("the change may not be saved"),
        "{}",
        error.message
    );
    assert!(
        error.message.ends_with("Retry: tmt digest tick"),
        "{}",
        error.message
    );
    assert_eq!(store.cursor(ID).unwrap(), Some(1));
}

#[test]
fn decision_and_settlement_reads_never_take_the_write_lock() {
    let fixture = Fixture::new();
    let mut writer = fixture.store();
    writer.baseline(ID, 1_000, 1, 0).unwrap();
    let recorded = decision(2_000, Rule::NotDue, None);
    writer.record_decision(&recorded).unwrap();
    // Admit every connection before holding the lock, so closing a newly opened
    // descriptor cannot invalidate a same-process POSIX lock in the fixture.
    let mut reader = Store::open(&fixture.0, false).unwrap().unwrap();
    let mut blocker = Connection::open(fixture.0.join("digest/history/history.sqlite")).unwrap();
    let held = blocker
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    for store in [&mut writer, &mut reader] {
        assert_eq!(
            store.decisions(0, 3_000).unwrap().as_slice(),
            std::slice::from_ref(&recorded)
        );
        assert_eq!(store.last_decision(ID).unwrap(), Some(recorded.clone()));
        assert!(store.settlements(0, 3_000).unwrap().is_empty());
        assert_eq!(store.coverage(0, 3_000).unwrap()[ID].started, 1_000);
    }
    held.rollback().unwrap();
}
