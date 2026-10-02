use super::*;
use serde_json::json;

pub(crate) fn input(tokens: u64) -> Input {
    let resume = json!({"driver":"claude", "session":"redacted-session", "consumption": {
        "inputTokens":tokens, "outputTokens":tokens / 2, "cachedInputTokens":tokens / 4,
        "epoch":"00000000-0000-4000-8000-000000000001", "sequence": tokens + 1,
        "observedAtMs":tokens + 1, "complete":true, "gap":false }});
    Input {
        room: "redacted-room".into(),
        resumes: BTreeMap::from([("a".into(), resume)]),
    }
}

#[test]
fn no_data_warmup_and_measured_zero_are_distinct() {
    let mut rate = Rate::default();
    let mut absent = input(100);
    absent.resumes.insert("a".into(), Value::Null);
    rate.sample(&absent, 0);
    assert_eq!(rate.reading(0, TokenWindow::Minute), None);
    rate.sample(&input(100), 5_000);
    assert_eq!(rate.reading(5_000, TokenWindow::Five), None);
    rate.sample(&input(100), 10_000);
    assert_eq!(rate.reading(10_000, TokenWindow::Minute), None);
    assert_eq!(rate.reading(10_000, TokenWindow::Five).unwrap().rate, 0.0);
    rate.sample(&input(100), 15_000);
    let zero = rate.reading(15_000, TokenWindow::Minute).unwrap();
    assert_eq!(zero.rate, 0.0);
    assert!(!zero.partial);
    assert_eq!(zero.span, 10_000);
}

#[test]
fn four_windows_count_cached_input_once_and_average_covered_span() {
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    rate.sample(&input(200), 10_000);
    for window in [
        TokenWindow::Minute,
        TokenWindow::HalfHour,
        TokenWindow::Hour,
    ] {
        let reading = rate.reading(10_000, window).unwrap();
        assert_eq!(reading.rate, 15.0);
        assert_eq!(reading.span, 10_000);
        assert!(!reading.partial);
    }
    assert_eq!(rate.reading(10_000, TokenWindow::Five).unwrap().rate, 30.0);
    rate.sample(&input(300), 60_000);
    assert_eq!(rate.reading(60_000, TokenWindow::Minute).unwrap().rate, 5.0);
    rate.sample(&input(300), 70_000);
    assert_eq!(rate.reading(70_000, TokenWindow::Minute).unwrap().rate, 2.5);
    assert_eq!(
        rate.reading(70_000, TokenWindow::HalfHour).unwrap().rate,
        300.0 / 70.0
    );
    assert_eq!(rate.reading(70_000, TokenWindow::Five).unwrap().rate, 0.0);
}

#[test]
fn never_reporting_members_are_excluded_but_lost_reporters_remain_partial() {
    let mut rate = Rate::default();
    let mut sample = input(100);
    sample.resumes.insert("never".into(), Value::Null);
    rate.sample(&sample, 0);
    sample = input(200);
    sample.resumes.insert("never".into(), Value::Null);
    rate.sample(&sample, 10_000);
    assert!(!rate.reading(10_000, TokenWindow::Minute).unwrap().partial);
    assert!(!rate.reporter("never"));
    let mut missing = sample.clone();
    missing.resumes.insert("a".into(), Value::Null);
    rate.sample(&missing, 15_000);
    assert!(rate.reading(15_000, TokenWindow::Minute).unwrap().partial);
    rate.sample(&input(300), 20_000);
    rate.sample(&input(400), 25_000);
    assert!(
        rate.reading(25_000, TokenWindow::Minute).unwrap().partial,
        "recovery cannot erase historic gaps"
    );
    rate.sample(&input(400), 80_000);
    assert!(!rate.reading(80_000, TokenWindow::Minute).unwrap().partial);
    assert!(rate.reading(80_000, TokenWindow::HalfHour).unwrap().partial);
    rate.sample(&input(400), 1_820_000);
    assert!(
        !rate
            .reading(1_820_000, TokenWindow::HalfHour)
            .unwrap()
            .partial
    );
    assert!(rate.reading(1_820_000, TokenWindow::Hour).unwrap().partial);
    rate.sample(&input(400), 3_620_000);
    assert!(!rate.reading(3_620_000, TokenWindow::Hour).unwrap().partial);
}

#[test]
fn resets_invalid_order_and_gap_recovery_never_invent_tokens() {
    for kind in [
        "session",
        "epoch",
        "decrease",
        "gap",
        "incomplete",
        "invalid",
        "old",
        "equal",
    ] {
        let mut rate = Rate::default();
        rate.sample(&input(100), 0);
        rate.sample(&input(200), 10_000);
        let mut changed = input(300);
        let value = changed.resumes.get_mut("a").unwrap();
        match kind {
            "session" => value["session"] = json!("new-session"),
            "epoch" => {
                value["consumption"]["epoch"] = json!("00000000-0000-4000-8000-000000000002")
            }
            "decrease" => value["consumption"]["inputTokens"] = json!(100),
            "gap" => {
                value["consumption"]["gap"] = json!(true);
                value["consumption"]["complete"] = json!(false);
            }
            "incomplete" => value["consumption"]["complete"] = json!(false),
            "invalid" => value["consumption"]["cachedInputTokens"] = json!(999),
            "old" => value["consumption"]["observedAtMs"] = json!(1),
            "equal" => value["consumption"]["sequence"] = json!(201),
            _ => unreachable!(),
        }
        rate.sample(&changed, 15_000);
        assert_eq!(
            rate.buckets
                .iter()
                .map(|bucket| bucket.tokens)
                .sum::<u128>(),
            150,
            "{kind}"
        );
        assert!(rate.reading(15_000, TokenWindow::Minute).unwrap().partial);
        let mut recovery = input(400);
        let value = recovery.resumes.get_mut("a").unwrap();
        if kind == "session" {
            value["session"] = changed.resumes["a"]["session"].clone();
        }
        if kind == "epoch" {
            value["consumption"]["epoch"] = changed.resumes["a"]["consumption"]["epoch"].clone();
        }
        rate.sample(&recovery, 20_000);
        if matches!(kind, "gap" | "incomplete" | "invalid" | "old" | "equal") {
            assert_eq!(
                rate.buckets
                    .iter()
                    .map(|bucket| bucket.tokens)
                    .sum::<u128>(),
                150,
                "recovery {kind}"
            );
        }
    }
}

#[test]
fn failed_reads_grace_rebaseline_and_keep_gap_evidence() {
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    rate.sample(&input(200), 10_000);
    rate.failed(15_000, 5_000);
    assert!(!rate.reading(15_000, TokenWindow::Minute).unwrap().partial);
    rate.failed(20_000, 5_000);
    assert!(rate.reading(20_000, TokenWindow::Minute).unwrap().partial);
    rate.sample(&input(1_000), 25_000);
    rate.sample(&input(1_100), 30_000);
    assert_eq!(
        rate.buckets
            .iter()
            .map(|bucket| bucket.tokens)
            .sum::<u128>(),
        300
    );
    assert!(rate.reading(30_000, TokenWindow::Minute).unwrap().partial);
}

#[test]
fn one_hour_ring_is_bounded_under_churn_and_long_pauses() {
    assert!(std::mem::size_of::<Bucket>() <= 64);
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    for step in 1..=2_000 {
        let mut sample = input(100 + step);
        if step % 50 == 0 {
            sample.resumes.insert(format!("churn-{step}"), Value::Null);
        }
        rate.sample(&sample, step * 5_000);
    }
    assert_eq!(rate.buckets.len(), 720);
    assert!(rate.members.len() <= 2);
    assert_eq!(
        rate.reading(10_000_000, TokenWindow::Hour).unwrap().span,
        3_600_000
    );
    rate.sample(&input(2_100), 50_000_000);
    assert_eq!(
        rate.reading(50_000_000, TokenWindow::Hour).unwrap().rate,
        0.0
    );
}

#[test]
fn trend_distinguishes_no_evidence_zero_and_nonzero() {
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    assert_eq!(rate.trend(0, TokenWindow::Five), [None; 8]);
    rate.sample(&input(100), 5_000);
    assert_eq!(rate.trend(5_000, TokenWindow::Five)[7], Some(0.0));
    rate.sample(&input(200), 10_000);
    assert_eq!(rate.trend(10_000, TokenWindow::Five)[7], Some(30.0));
    assert_eq!(rate.trend(10_000, TokenWindow::Minute)[7], Some(15.0));
}

#[test]
fn recorded_public_projection_fixture_matches_expected_rates() {
    let records: Value =
        serde_json::from_str(include_str!("fixtures/completed-requests.json")).unwrap();
    let roster = Input {
        room: "fixture-room".into(),
        resumes: BTreeMap::from([
            ("claude-id".into(), Value::Null),
            ("codex-id".into(), Value::Null),
        ]),
    };
    let mut rate = Rate::default();
    for record in records.as_array().unwrap() {
        let sample = roster.listed(&record["document"]);
        let now = record["receiptMs"].as_u64().unwrap();
        rate.sample(&sample, now);
        let actual = rate.reading(now, TokenWindow::Minute);
        assert_eq!(actual.map(|reading| reading.rate), record["rate"].as_f64());
        assert!(actual.is_none_or(|reading| !reading.partial));
    }
}

#[test]
fn partial_member_zero_is_data_not_absence() {
    let mut rate = Rate::default();
    let mut first = input(100);
    first.resumes.insert("b".into(), first.resumes["a"].clone());
    rate.sample(&first, 0);
    let mut next = input(100);
    next.resumes.insert("b".into(), Value::Null);
    rate.sample(&next, 10_000);
    let reading = rate.reading(10_000, TokenWindow::Minute).unwrap();
    assert_eq!(reading.rate, 0.0);
    assert!(reading.partial);
}
