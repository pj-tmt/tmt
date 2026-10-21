//! Independent value vectors and durable minute coverage, never provider files.

use super::*;
use crate::history::store::test_support::{Fixture, ID, event};
use crate::history::{Window, usage::UsagePage};
use std::fs;

fn document(through: u64) -> Value {
    let from = through.saturating_sub(600_000);
    json!({"asOfMs":through+123,"throughMs":through,"resolutionMs":5000,
        "retainedFromMs":through.saturating_sub(7_200_000),"identities":[{
        "id":ID,"found":true,"reporting":true,
        "latest":{"consumption":{"modelId":"gpt-6.1-sol","observedAtMs":through-5000}},
        "windows":[{"windowMs":600000,"fromMs":from,"toMs":through,"bucketMs":5000,
        "buckets":(from..through).step_by(5000).map(|time|json!({
            "fromMs":time,"toMs":time+5000,"inputTokens":10,"outputTokens":100,
            "cachedInputTokens":8,"cacheWriteTokens":2,"coveredMs":5000,
            "complete":true,"gap":false,"discontinuous":false})).collect::<Vec<_>>()}]}]})
}
fn page(through: u64) -> UsagePage {
    UsagePage::parse(&document(through), ID).unwrap()
}

#[test]
fn normative_quotes_and_weighted_ratios_have_independent_values() {
    let vectors: Value = serde_json::from_str(
        &fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/vectors/valuation-v1.json"
        ))
        .unwrap(),
    )
    .unwrap();
    for case in vectors["quotes"].as_array().unwrap() {
        assert_eq!(
            crate::pricing::built_in()
                .quote(
                    case["model"].as_str().unwrap(),
                    case["observedAtMs"].as_u64().unwrap(),
                    case["tokens"].as_u64().unwrap()
                )
                .unwrap_or(Value::Null),
            case["expected"],
            "{}",
            case["name"]
        );
    }
    for case in vectors["ratios"].as_array().unwrap() {
        assert_eq!(
            ratio(
                case["avoided"].as_u64().unwrap(),
                case["observed"].as_u64().unwrap(),
                60000,
                120000
            )
            .unwrap_or(Value::Null),
            case["expected"],
            "{}",
            case["name"]
        );
    }
    // The producer prices every quote under its one table version and never sums, so the
    // vector's cost sums are for consumers: each must keep two distinct versions and be
    // unavailable, or the case no longer proves the rule.
    for case in vectors["costSums"].as_array().unwrap() {
        let versions: std::collections::BTreeSet<_> = case["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| member["pricingVersion"].as_str().unwrap())
            .collect();
        assert!(versions.len() > 1, "{}", case["name"]);
        assert_eq!(case["expected"], Value::Null, "{}", case["name"]);
        assert_eq!(case["pricesDiffer"], true, "{}", case["name"]);
    }
    let table = crate::pricing::Resolved::built_in().document();
    assert_eq!(table["models"].as_array().unwrap().len(), 4);
    assert_eq!(table["version"], "2026-10-14");
    assert!(
        table["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["source"].as_str().unwrap().starts_with("https://"))
    );
}

#[test]
fn full_minutes_exclude_shifted_edges_and_cache_output_counts() {
    let through = 1_200_000 + 15_000;
    let parsed = page(through);
    assert_eq!(parsed.minutes.len(), 9);
    assert_eq!(parsed.minutes[0].start, 660000);
    assert_eq!(parsed.minutes.last().unwrap().start, 1140000);
    assert!(
        parsed
            .minutes
            .iter()
            .all(|minute| minute.input == Some(120))
    );
    let mut bad = document(through);
    bad["identities"][0]["windows"][0]["buckets"][20]["complete"] = json!(false);
    bad["identities"][0]["windows"][0]["buckets"][20]["gap"] = json!(true);
    bad["identities"][0]["windows"][0]["buckets"][20]["coveredMs"] = json!(0);
    let parsed = UsagePage::parse(&bad, ID).unwrap();
    assert_eq!(
        parsed
            .minutes
            .iter()
            .filter(|minute| minute.input.is_none())
            .count(),
        1
    );
    bad["identities"][0]["latest"]["consumption"]["modelId"] = Value::Null;
    bad["identities"][0]["latest"]["model"] = json!("gpt-6.1-sol");
    assert!(UsagePage::parse(&bad, ID).unwrap().model.is_none());
}

#[test]
fn malformed_source_bounds_and_false_coverage_are_refused() {
    let good = document(1_200_000);
    for (pointer, value) in [
        ("/identities/0/id", json!("wrong")),
        ("/throughMs", json!(1_200_001)),
        ("/asOfMs", json!(1_199_999)),
        ("/resolutionMs", json!(30000)),
        ("/identities/0/windows/0/bucketMs", json!(30000)),
        ("/identities/0/windows/0/buckets/0/fromMs", json!(600001)),
        ("/identities/0/windows/0/buckets/0/coveredMs", json!(1)),
        (
            "/identities/0/windows/0/buckets/0/inputTokens",
            json!(MAX + 1),
        ),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(UsagePage::parse(&bad, ID).is_err(), "{pointer}");
    }
    let mut missing = good.clone();
    missing["identities"][0]["windows"][0]["buckets"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(UsagePage::parse(&missing, ID).is_err());
    let mut reset = good;
    reset["identities"][0]["windows"][0]["buckets"][0]["discontinuous"] = json!(true);
    assert!(
        UsagePage::parse(&reset, ID).unwrap().minutes[0]
            .input
            .is_none()
    );
}

fn recorded_day(store: &mut Store, start: u64) {
    for through in ((start + 600000)..=(start + super::super::super::DAY)).step_by(600000) {
        store.record_usage(ID, &page(through)).unwrap();
    }
}

#[test]
fn matched_period_restart_overlap_gaps_and_profile_changes_are_observable() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let start = 10 * crate::history::DAY;
    let through = start + crate::history::DAY + 321;
    store.baseline(ID, start, 0, 0).unwrap();
    store
        .record(
            ID,
            &event(1, start + 1000, &[Some(1000), Some(100)]),
            through,
        )
        .unwrap();
    store
        .record(
            ID,
            &event(2, start + MINUTE + 1000, &[Some(1000), Some(2700)]),
            through,
        )
        .unwrap();
    store.checkpoint(ID, through, 2, 2).unwrap();
    recorded_day(&mut store, start);
    let expected = json!({"basisPoints":154,"estimatedInputTokensAvoided":2700,"observedInputTokens":172680,"fromMs":start+MINUTE,"throughMs":start+crate::history::DAY});
    let status = store
        .status_built_in(ID, Window::Day, through, Some(2))
        .unwrap();
    assert_eq!(status["estimatedInputTokensAvoided"], 2800);
    assert_eq!(status["cost"]["usdMicros"], 5600);
    assert_eq!(status["percent"], expected); // The edge event is not the ratio numerator.
    assert!(
        store
            .status_built_in(ID, Window::Lifetime, through, Some(2))
            .unwrap()["percent"]
            .is_null()
    );
    drop(store);
    let mut store = fixture.store();
    for _ in 0..3 {
        store
            .record_usage(ID, &page(start + crate::history::DAY))
            .unwrap();
    }
    assert_eq!(
        store
            .status_built_in(ID, Window::Day, through, Some(2))
            .unwrap()["percent"],
        expected
    );
    assert_eq!(
        store
            .read(|tx| tx
                .query_row("SELECT COUNT(*) FROM usage_minutes", [], |r| number(r, 0))
                .map_err(io))
            .unwrap(),
        1440
    );
    let mut bad = document(start + crate::history::DAY);
    bad["identities"][0]["windows"][0]["buckets"][119]["complete"] = json!(false);
    bad["identities"][0]["windows"][0]["buckets"][119]["gap"] = json!(true);
    bad["identities"][0]["windows"][0]["buckets"][119]["coveredMs"] = json!(0);
    store
        .record_usage(ID, &UsagePage::parse(&bad, ID).unwrap())
        .unwrap();
    let partial = store
        .status_built_in(ID, Window::Day, through, Some(2))
        .unwrap();
    assert!(partial["percent"].is_null());
    assert_eq!(partial["cost"]["usdMicros"], 5600);
    assert_eq!(partial["complete"], true);
    let mut improved = document(start + crate::history::DAY);
    improved["identities"][0]["latest"]["consumption"]["modelId"] = json!("claude-opus-5-5");
    store
        .record_usage(ID, &UsagePage::parse(&improved, ID).unwrap())
        .unwrap();
    let changed = store
        .status_built_in(ID, Window::Day, through, Some(2))
        .unwrap();
    assert_eq!(changed["percent"], expected);
    assert_eq!(changed["cost"]["usdMicros"], 11200);
    assert_eq!(changed["cost"]["modelId"], "claude-opus-5-5");
    improved["identities"][0]["latest"]["consumption"]["modelId"] = Value::Null;
    store
        .record_usage(ID, &UsagePage::parse(&improved, ID).unwrap())
        .unwrap();
    assert!(
        store
            .status_built_in(ID, Window::Day, through, Some(2))
            .unwrap()["cost"]
            .is_null()
    );
    store
        .write(|tx| {
            tx.execute(
                "DELETE FROM usage_minutes WHERE minute=?1",
                [(start + MINUTE) as i64],
            )
            .map_err(io)
        })
        .unwrap();
    assert!(
        store
            .status_built_in(ID, Window::Day, through, Some(2))
            .unwrap()["percent"]
            .is_null()
    );
}

#[test]
fn interrupted_usage_transaction_preserves_rows_watermark_and_model() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let start = 10 * crate::history::DAY;
    store.baseline(ID, start, 0, 0).unwrap();
    store.record_usage(ID, &page(start + 600000)).unwrap();
    let before: (u64, u64) = store
        .read(|tx| {
            tx.query_row(
                "SELECT COUNT(*),SUM(input_tokens) FROM usage_minutes",
                [],
                |r| Ok((number(r, 0)?, number(r, 1)?)),
            )
            .map_err(io)
        })
        .unwrap();
    store.write(|tx| tx.execute_batch(&format!("CREATE TRIGGER fail_usage BEFORE INSERT ON usage_minutes WHEN NEW.minute={} BEGIN SELECT RAISE(ABORT,'fixture interrupted'); END;",start+600000+MINUTE)).map_err(io)).unwrap();
    let mut next = document(start + 1200000);
    next["identities"][0]["latest"]["consumption"]["modelId"] = json!("claude-opus-5-5");
    assert!(
        store
            .record_usage(ID, &UsagePage::parse(&next, ID).unwrap())
            .is_err()
    );
    assert_eq!(
        store
            .read(|tx| tx
                .query_row(
                    "SELECT COUNT(*),SUM(input_tokens) FROM usage_minutes",
                    [],
                    |r| Ok((number(r, 0)?, number(r, 1)?))
                )
                .map_err(io))
            .unwrap(),
        before
    );
    assert_eq!(
        store
            .read(|tx| tx
                .query_row("SELECT through,model_id FROM usage_state", [], |r| Ok((
                    number(r, 0)?,
                    r.get::<_, String>(1)?
                )))
                .map_err(io))
            .unwrap(),
        (start + 600000, "gpt-6.1-sol".into())
    );
}

#[test]
fn schema_one_read_is_unchanged_and_write_migration_preserves_savings() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.baseline(ID, 1000, 0, 0).unwrap();
    store
        .record(ID, &event(1, 2000, &[Some(1), Some(2700)]), 3000)
        .unwrap();
    store.checkpoint(ID, 4000, 1, 1).unwrap();
    store
        .write(|tx| {
            tx.execute_batch(
                "DROP TABLE usage_state; DROP TABLE usage_minutes; DROP TABLE decisions; PRAGMA user_version=1;",
            )
            .map_err(io)
        })
        .unwrap();
    drop(store);
    let file = fixture.0.join("digest/history/history.sqlite");
    let before = fs::read(&file).unwrap();
    let mut reader = Store::open(&fixture.0, false).unwrap().unwrap();
    let old = reader
        .status_built_in(ID, Window::Lifetime, 4000, Some(1))
        .unwrap();
    assert_eq!(old["estimatedInputTokensAvoided"], 2700);
    assert!(old["cost"].is_null() && old["percent"].is_null());
    drop(reader);
    assert_eq!(fs::read(&file).unwrap(), before);
    let mut writer = fixture.store();
    assert_eq!(
        writer
            .read(|tx| tx
                .pragma_query_value(None, "user_version", |r| number(r, 0))
                .map_err(io))
            .unwrap(),
        3
    );
    assert_eq!(writer.cursor(ID).unwrap(), Some(1));
    assert_eq!(
        writer
            .status_built_in(ID, Window::Lifetime, 4000, Some(1))
            .unwrap()["estimatedInputTokensAvoided"],
        2700
    );
}

#[test]
fn usage_shares_raw_budget_and_expiry_preserves_settlement_totals() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let start = 10 * crate::history::DAY;
    store.baseline(ID, start, 0, 0).unwrap();
    store
        .record(
            ID,
            &event(1, start + 1, &[Some(1), Some(2700)]),
            start + 600000,
        )
        .unwrap();
    store.record_usage(ID, &page(start + 600000)).unwrap();
    store.checkpoint(ID, start + 600000, 1, 1).unwrap();
    store
        .write(|tx| {
            // Both owners contribute to one budget. All minutes precede the event's
            // collection timestamp, so they must be evicted before its raw inputs.
            let event_bytes: u64 = tx
                .query_row("SELECT length(inputs) FROM events", [], |r| number(r, 0))
                .unwrap();
            prune(tx, start + 600000, event_bytes + 2 * ROW_BYTES).unwrap();
            assert_eq!(
                tx.query_row("SELECT COUNT(*) FROM usage_minutes", [], |r| number(r, 0))
                    .unwrap(),
                2
            );
            assert_eq!(
                tx.query_row("SELECT MIN(minute) FROM usage_minutes", [], |r| number(
                    r, 0
                ))
                .unwrap(),
                start + 480000
            );
            prune(tx, start + 600000, 0).unwrap();
            assert_eq!(
                tx.query_row("SELECT COUNT(*) FROM events", [], |r| number(r, 0))
                    .unwrap(),
                0
            );
            assert_eq!(
                tx.query_row("SELECT COUNT(*) FROM usage_minutes", [], |r| number(r, 0))
                    .unwrap(),
                0
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        store
            .status_built_in(ID, Window::Lifetime, start + 600000, Some(1))
            .unwrap()["estimatedInputTokensAvoided"],
        2700
    );
    store.record_usage(ID, &page(start + 1200000)).unwrap();
    store
        .maintain(start + 1200000 + super::super::RETENTION + MINUTE)
        .unwrap();
    assert_eq!(
        store
            .read(|tx| tx
                .query_row("SELECT COUNT(*) FROM usage_minutes", [], |r| number(r, 0))
                .map_err(io))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .status_built_in(
                ID,
                Window::Lifetime,
                start + 1200000 + super::super::RETENTION + MINUTE,
                Some(1)
            )
            .unwrap()["estimatedInputTokensAvoided"],
        2700
    );
}
