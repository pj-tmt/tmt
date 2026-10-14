//! Observable formula, private persistence and window evidence, independent inputs.

use super::test_support::{Fixture, ID, event};
use super::*;
use crate::history::{collect, model};
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
            .status(ID, Window::Lifetime, start + 200, Some(12))
            .unwrap(),
        json!({"method":METHOD,"window":"lifetime","fromMs":start,"throughMs":start+200,"estimatedInputTokensAvoided":2700,"avoidedSeparateDeliveries":2,"complete":true,"gap":null,"cost":null,"percent":null})
    );
    store.0.execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;").unwrap();
    assert!(
        store
            .record(ID, &event(8, start + 250, &[None]), start + 300)
            .is_err()
    );
    assert_eq!(store.cursor(ID).unwrap(), Some(7));
    store.0.execute_batch("DROP TRIGGER fail_event").unwrap();
    drop(store);
    let file = fixture.0.join("digest/history/history.sqlite");
    let before = fs::read(&file).unwrap();
    let read = Store::open(&fixture.0, false).unwrap().unwrap();
    assert_eq!(read.cursor(ID).unwrap(), Some(7));
    assert_eq!(
        read.status(ID, Window::Lifetime, start + 200, Some(12))
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
        store.status(ID, Window::Day, start, Some(0)).unwrap()["gap"],
        "before_collection"
    );
    store.checkpoint(ID, start + DAY, 6, 0).unwrap();
    let empty = store.status(ID, Window::Day, start + DAY, Some(0)).unwrap();
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
        .status(ID, Window::Day, start + DAY + 20, Some(1))
        .unwrap();
    assert_eq!(unknown["estimatedInputTokensAvoided"], Value::Null);
    assert_eq!(unknown["avoidedSeparateDeliveries"], 1);
    assert_eq!(unknown["gap"], "missing_context");
    assert_eq!(
        store
            .status(ID, Window::Lifetime, start + DAY + 20, Some(2))
            .unwrap()["gap"],
        "uncollected_deliveries"
    );
    store.checkpoint(ID, start + DAY + 30, 10, 3).unwrap();
    assert_eq!(
        store
            .status(ID, Window::Day, start + DAY + 30, Some(3))
            .unwrap()["gap"],
        "missing_deliveries"
    );
    assert_eq!(
        store
            .status(ID, Window::Lifetime, start + DAY + 30, Some(3))
            .unwrap()["complete"],
        false
    );
    store.checkpoint(ID, start + 9 * DAY, 11, 3).unwrap();
    assert_eq!(
        store
            .status(ID, Window::Week, start + 9 * DAY, Some(3))
            .unwrap()["estimatedInputTokensAvoided"],
        0
    );
    assert_eq!(
        store
            .status(ID, Window::Lifetime, start + 9 * DAY, Some(3))
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
        let tx = store.0.transaction().unwrap();
        prune(&tx, start + 20, 0).unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(
        store
            .status(ID, Window::Lifetime, start + 20, Some(1))
            .unwrap()["estimatedInputTokensAvoided"],
        9
    );
    assert_eq!(
        store.status(ID, Window::Day, start + 20, Some(1)).unwrap()["complete"],
        false
    );
    store
        .record(ID, &event(4, start + 30, &[None]), start + 40)
        .unwrap();
    store.maintain(start + RETENTION + DAY).unwrap();
    assert_eq!(
        store
            .status(ID, Window::Day, start + RETENTION + DAY, Some(2))
            .unwrap()["complete"],
        false
    );
    store.checkpoint(ID, start + RETENTION + DAY, 5, 2).unwrap();
    assert_eq!(
        store
            .0
            .query_row("SELECT COUNT(*) FROM events", [], |r| number(r, 0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .status(ID, Window::Lifetime, start + RETENTION + DAY, Some(2))
            .unwrap()["estimatedInputTokensAvoided"],
        9
    );
    assert_eq!(
        store
            .status(ID, Window::Week, start + RETENTION + DAY, Some(2))
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
        store.status(ID, Window::Lifetime, 1001, Some(1)).unwrap()["gap"],
        "clock_discontinuity"
    );
    store.0.pragma_update(None, "user_version", 3).unwrap();
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
        .0
        .pragma_query_value(None, "page_count", |r| number(r, 0))
        .unwrap();
    store
        .0
        .pragma_update(None, "max_page_count", pages as i64)
        .unwrap();
    let batch = event(2, 1100, &vec![Some(12345); 512]);
    assert!(store.record(ID, &batch, 1200).is_err());
    assert_eq!(store.cursor(ID).unwrap(), Some(1));
    assert_eq!(
        store
            .0
            .query_row("SELECT COUNT(*) FROM events", [], |r| number(r, 0))
            .unwrap(),
        0
    );
    assert_eq!(
        store.status(ID, Window::Lifetime, 1200, Some(0)).unwrap()["estimatedInputTokensAvoided"],
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
        store.status(ID, Window::Lifetime, 1200, Some(11)).unwrap()["estimatedInputTokensAvoided"],
        2700
    );
    assert_eq!(
        store
            .0
            .query_row("SELECT COUNT(*) FROM events", [], |r| number(r, 0))
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
                .status(
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
    let interrupted = store.status(ID, Window::Lifetime, 1200, None).unwrap();
    assert_eq!(interrupted["throughMs"], 1000);
    assert_eq!(interrupted["estimatedInputTokensAvoided"], Value::Null);
    assert_eq!(interrupted["gap"], "uncollected_deliveries");
    drop(store);
    let mut resumed = fixture.store();
    resumed.checkpoint(ID, 1200, 3, 1).unwrap();
    assert_eq!(
        resumed.status(ID, Window::Lifetime, 1200, Some(1)).unwrap()["estimatedInputTokensAvoided"],
        200
    );
}
