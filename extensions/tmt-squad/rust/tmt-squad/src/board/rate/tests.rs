use super::*;
use serde_json::json;

pub(crate) fn input(tokens: u64) -> Input {
    let resume = json!({"driver":"claude", "session":"redacted-session", "consumption": {
        "inputTokens":tokens, "outputTokens":tokens / 2, "cachedInputTokens":tokens / 4,
        "epoch":"00000000-0000-4000-8000-000000000001", "sequence": tokens + 1,
        "observedAtMs":tokens + 1, "complete":true, "gap":false }});
    Input {
        room: "redacted-room".into(),
        names: BTreeMap::from([("a".into(), "worker".into())]),
        resumes: BTreeMap::from([("a".into(), resume)]),
    }
}

#[test]
fn sampling_keeps_observed_names_when_live_presence_is_missing() {
    let mut roster = input(100);
    roster.names.insert("lead-id".into(), "design-lead".into());
    roster.resumes.insert("lead-id".into(), Value::Null);
    let sample = roster.listed(&json!({"identities": [
        {"id": "a", "name": "worker", "resume": roster.resumes["a"]},
        {"id": "unrelated", "name": "outside-roster", "resume": null}
    ]}));
    assert_eq!(sample.names, roster.names);
    assert_eq!(sample.names["lead-id"], "design-lead");
    assert_eq!(sample.resumes["lead-id"], Value::Null);
    assert!(!sample.resumes.contains_key("unrelated"));
}

#[test]
fn no_data_warmup_and_measured_zero_are_distinct() {
    let mut rate = Rate::default();
    let mut absent = input(100);
    absent.resumes.insert("a".into(), Value::Null);
    rate.sample(&absent, 0);
    assert_eq!(rate.reading(0, TokenWindow::MINUTE), None);
    rate.sample(&input(100), 5_000);
    assert_eq!(rate.reading(5_000, TokenWindow::MINUTE), None);
    rate.sample(&input(100), 10_000);
    assert_eq!(rate.reading(10_000, TokenWindow::MINUTE).unwrap().tokens, 0);
    rate.sample(&input(100), 15_000);
    let zero = rate.reading(15_000, TokenWindow::MINUTE).unwrap();
    assert_eq!(zero.tokens, 0);
    assert!(zero.partial, "window exceeds observed uptime");
    assert_eq!(zero.span, 10_000);
}

#[test]
fn windows_count_cached_input_once_without_dividing_by_covered_span() {
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    rate.sample(&input(200), 10_000);
    for window in [
        TokenWindow::MINUTE,
        TokenWindow::FIVE_MINUTES,
        TokenWindow::HOUR,
    ] {
        let reading = rate.reading(10_000, window).unwrap();
        assert_eq!(reading.tokens, 150);
        assert_eq!(reading.span, 10_000);
        assert!(reading.partial);
    }
    rate.sample(&input(300), 60_000);
    assert_eq!(
        rate.reading(60_000, TokenWindow::MINUTE).unwrap().tokens,
        300
    );
    rate.sample(&input(300), 70_000);
    assert_eq!(
        rate.reading(70_000, TokenWindow::MINUTE).unwrap().tokens,
        150
    );
    assert_eq!(
        rate.reading(70_000, TokenWindow::FIVE_MINUTES)
            .unwrap()
            .tokens,
        300
    );
}

#[test]
fn unreported_members_are_excluded_and_totals_disclose_incomplete_coverage() {
    let mut rate = Rate::default();
    let mut sample = input(100);
    sample.resumes.insert("never".into(), Value::Null);
    rate.sample(&sample, 0);
    sample = input(200);
    sample.resumes.insert("never".into(), Value::Null);
    rate.sample(&sample, 10_000);
    assert!(rate.reading(10_000, TokenWindow::MINUTE).unwrap().partial);
    assert!(!rate.reporter("never"));
    let mut missing = sample.clone();
    missing.resumes.insert("a".into(), Value::Null);
    rate.sample(&missing, 15_000);
    assert!(rate.reading(15_000, TokenWindow::MINUTE).unwrap().partial);
    rate.sample(&input(300), 20_000);
    rate.sample(&input(400), 25_000);
    assert!(
        rate.reading(25_000, TokenWindow::MINUTE).unwrap().partial,
        "recovery cannot erase historic gaps"
    );
    rate.sample(&input(400), 80_000);
    assert!(!rate.reading(80_000, TokenWindow::MINUTE).unwrap().partial);
    assert!(
        rate.reading(80_000, TokenWindow::FIVE_MINUTES)
            .unwrap()
            .partial
    );
    rate.sample(&input(400), 1_820_000);
    assert!(
        !rate
            .reading(1_820_000, TokenWindow::FIVE_MINUTES)
            .unwrap()
            .partial
    );
    assert!(rate.reading(1_820_000, TokenWindow::HOUR).unwrap().partial);
    rate.sample(&input(400), 3_620_000);
    assert!(!rate.reading(3_620_000, TokenWindow::HOUR).unwrap().partial);
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
            rate.members["a"]
                .buckets
                .iter()
                .map(|bucket| bucket.tokens)
                .sum::<u128>(),
            150,
            "{kind}"
        );
        assert!(rate.reading(15_000, TokenWindow::MINUTE).unwrap().partial);
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
                rate.members["a"]
                    .buckets
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
    rate.sample(&input(200), 60_000);
    rate.failed(65_000, 5_000);
    assert!(!rate.reading(65_000, TokenWindow::MINUTE).unwrap().partial);
    rate.failed(70_000, 5_000);
    assert!(rate.reading(70_000, TokenWindow::MINUTE).unwrap().partial);
    rate.sample(&input(1_000), 75_000);
    rate.sample(&input(1_100), 80_000);
    assert_eq!(
        rate.members["a"]
            .buckets
            .iter()
            .map(|bucket| bucket.tokens)
            .sum::<u128>(),
        300
    );
    assert!(rate.reading(80_000, TokenWindow::MINUTE).unwrap().partial);
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
    assert_eq!(rate.members["a"].buckets.len(), 720);
    assert!(rate.members.len() <= 2);
    assert_eq!(
        rate.reading(10_000_000, TokenWindow::HOUR).unwrap().span,
        3_600_000
    );
    rate.sample(&input(2_100), 50_000_000);
    assert_eq!(
        rate.reading(50_000_000, TokenWindow::HOUR).unwrap().tokens,
        0
    );
}

#[test]
fn trend_distinguishes_no_evidence_zero_and_nonzero() {
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    assert_eq!(rate.trend(0, TokenWindow::MINUTE), [None; 8]);
    rate.sample(&input(100), 5_000);
    assert_eq!(rate.trend(5_000, TokenWindow::MINUTE)[7], Some(0.0));
    rate.sample(&input(200), 10_000);
    assert_eq!(rate.trend(10_000, TokenWindow::MINUTE)[7], Some(150.0));
}

#[test]
fn recorded_public_projection_fixture_preserves_observed_counter_deltas() {
    let records: Value =
        serde_json::from_str(include_str!("fixtures/completed-requests.json")).unwrap();
    let roster = Input {
        room: "fixture-room".into(),
        names: BTreeMap::from([
            ("claude-id".into(), "claude-worker".into()),
            ("codex-id".into(), "codex-worker".into()),
        ]),
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
        let actual = rate.reading(now, TokenWindow::MINUTE);
        assert_eq!(
            actual.map(|reading| reading.tokens),
            record["tokens"].as_u64().map(u128::from)
        );
        assert_eq!(
            actual.map(|r| r.partial),
            (now >= 5_000).then_some(now < 60_000)
        );
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
    let reading = rate.reading(10_000, TokenWindow::MINUTE).unwrap();
    assert_eq!(reading.tokens, 0);
    assert!(reading.partial);
}

#[test]
fn member_windows_models_and_removal_share_one_bounded_history() {
    let mut rate = Rate::new(TokenWindow::parse("24h").unwrap());
    let mut first = input(100);
    first.resumes.get_mut("a").unwrap()["model"] = json!("old");
    first.resumes.insert("b".into(), first.resumes["a"].clone());
    first.resumes.insert("never".into(), Value::Null);
    rate.sample(&first, 0);
    let mut next = input(200);
    next.resumes.get_mut("a").unwrap()["model"] = json!("new");
    next.resumes
        .insert("b".into(), input(300).resumes["a"].clone());
    next.resumes.insert("never".into(), Value::Null);
    rate.sample(&next, 60_000);
    assert_eq!(
        rate.member("a", 60_000, TokenWindow::MINUTE)
            .unwrap()
            .tokens,
        150
    );
    assert_eq!(
        rate.member("b", 60_000, TokenWindow::MINUTE)
            .unwrap()
            .tokens,
        300
    );
    assert_eq!(
        rate.reading(60_000, TokenWindow::MINUTE).unwrap().tokens,
        450
    );
    assert!(rate.reading(60_000, TokenWindow::MINUTE).unwrap().partial);
    assert_eq!(rate.model("a"), Some("new")); // Retained tokens use current model.
    assert!(rate.member("never", 60_000, TokenWindow::MINUTE).is_none());
    rate.sample(&next, 120_000);
    assert_eq!(
        rate.member("a", 120_000, TokenWindow::MINUTE)
            .unwrap()
            .tokens,
        0
    );
    assert_eq!(
        rate.member("a", 120_000, TokenWindow::FIVE_MINUTES)
            .unwrap()
            .tokens,
        150
    );
    assert_eq!(rate.members["a"].buckets.len(), 17_280);
    next.resumes.remove("b");
    rate.sample(&next, 125_000);
    assert!(rate.member("b", 125_000, TokenWindow::HOUR).is_none());
    assert_eq!(
        rate.reading(125_000, TokenWindow::HOUR).unwrap().tokens,
        150
    );
}

pub(crate) fn history_fixture() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../contracts/consumption-history-v1.json"
    )))
    .unwrap()
}

pub(crate) fn fixture_seeds(fixture: &Value) -> history::Seeds {
    let response = &fixture["response"];
    BTreeMap::from([(
        "a".into(),
        history::Seed::read(response, &response["identities"][0], 15_000),
    )])
}

pub(crate) fn historical_input(fixture: &Value, live: bool) -> Input {
    let mut value = input(100);
    value.resumes.insert(
        "a".into(),
        if live {
            fixture["nextLive"].clone()
        } else {
            fixture["response"]["identities"][0]["latest"].clone()
        },
    );
    value
}

#[test]
fn closed_shared_seed_and_open_live_delta_never_recount_cumulative_tokens() {
    let fixture = history_fixture();
    let seed = fixture_seeds(&fixture);
    let input = historical_input(&fixture, true);
    let mut rate = Rate::default();
    rate.seed(&input, &seed, 22_500);
    assert_eq!(
        rate.member("a", 22_500, TokenWindow::MINUTE)
            .unwrap()
            .tokens,
        21
    );
    rate.sample(&input, 22_500);
    assert_eq!(
        rate.member("a", 22_500, TokenWindow::MINUTE)
            .unwrap()
            .tokens,
        30
    );
    rate.sample(&input, 25_000);
    assert_eq!(
        rate.reading(25_000, TokenWindow::MINUTE).unwrap().tokens,
        30
    );
    // Reentry rebuilds recent core history, then extends only beyond latest again.
    rate.failed(26_000, 5_000);
    rate.seed(&input, &seed, 27_500);
    rate.sample(&input, 27_500);
    assert_eq!(
        rate.reading(27_500, TokenWindow::MINUTE).unwrap().tokens,
        30
    );
    assert!(rate.reading(27_500, TokenWindow::HOUR).unwrap().partial);
}

#[test]
fn history_coverage_zero_and_missing_watermarks_have_distinct_meanings() {
    let fixture = history_fixture();
    let input = historical_input(&fixture, true);
    let mut seed = fixture_seeds(&fixture);
    let row = seed.get_mut("a").unwrap().as_mut().unwrap();
    for bucket in &mut row.buckets {
        bucket.input_tokens = 0;
        bucket.output_tokens = 0;
        bucket.cached_input_tokens = 0;
    }
    let mut rate = Rate::default();
    rate.seed(&input, &seed, 22_500);
    assert_eq!(rate.reading(22_500, TokenWindow::MINUTE).unwrap().tokens, 0);
    let row = seed.get_mut("a").unwrap().as_mut().unwrap();
    for bucket in &mut row.buckets {
        bucket.covered_ms = 0;
        bucket.complete = false;
        bucket.gap = true;
    }
    rate.seed(&input, &seed, 22_500);
    assert_eq!(rate.reading(22_500, TokenWindow::MINUTE), None);
    seed = fixture_seeds(&fixture);
    let row = seed.get_mut("a").unwrap().as_mut().unwrap();
    row.latest = Value::Null;
    row.sampled = None;
    rate.seed(&input, &seed, 22_500);
    rate.sample(&input, 22_500);
    assert_eq!(
        rate.reading(22_500, TokenWindow::MINUTE).unwrap().tokens,
        21,
        "null seed cannot bridge to current cumulative counters"
    );
}

#[test]
fn seed_failure_and_counter_boundaries_preserve_known_history_without_inventing_tokens() {
    let fixture = history_fixture();
    let seed = fixture_seeds(&fixture);
    for kind in ["epoch", "session", "gap", "decrease"] {
        let mut input = historical_input(&fixture, true);
        let value = input.resumes.get_mut("a").unwrap();
        match kind {
            "epoch" => {
                value["consumption"]["epoch"] = json!("00000000-0000-4000-8000-000000000002")
            }
            "session" => value["session"] = json!("replacement"),
            "gap" => {
                value["consumption"]["gap"] = json!(true);
                value["consumption"]["complete"] = json!(false);
            }
            "decrease" => value["consumption"]["inputTokens"] = json!(100),
            _ => unreachable!(),
        }
        let mut rate = Rate::default();
        rate.seed(&input, &seed, 22_500);
        rate.sample(&input, 22_500);
        assert_eq!(
            rate.reading(22_500, TokenWindow::MINUTE).unwrap().tokens,
            21,
            "{kind}"
        );
    }
    let input = historical_input(&fixture, true);
    let mut rate = Rate::default();
    rate.seed(&input, &seed, 22_500);
    rate.sample(&input, 22_500);
    rate.seed(&input, &BTreeMap::from([("a".into(), None)]), 27_500);
    let mut later = input.clone();
    later.resumes.get_mut("a").unwrap()["consumption"]["inputTokens"] = json!(1_000);
    rate.sample(&later, 27_500);
    assert_eq!(
        rate.reading(27_500, TokenWindow::MINUTE).unwrap().tokens,
        30
    );
    rate.retain(&Input {
        room: input.room,
        names: BTreeMap::new(),
        resumes: BTreeMap::new(),
    });
    assert!(rate.members.is_empty());
}

#[test]
fn rolled_history_is_not_prorated_at_a_short_window_boundary() {
    let mut seeds = fixture_seeds(&history_fixture());
    let row = seeds.get_mut("a").unwrap().as_mut().unwrap();
    row.from = 0;
    row.through = 60_000;
    row.available = Some(0);
    row.buckets = [0, 30_000]
        .into_iter()
        .map(|from| history::Slice {
            from_ms: from,
            to_ms: from + 30_000,
            input_tokens: 20,
            output_tokens: 10,
            cached_input_tokens: 5,
            covered_ms: 30_000,
            complete: true,
            gap: false,
            discontinuous: false,
        })
        .collect();
    let mut rate = Rate::default();
    rate.seed(&input(100), &seeds, 75_000);
    let reading = rate.member("a", 75_000, TokenWindow::MINUTE).unwrap();
    assert_eq!(
        reading.tokens, 30,
        "only the whole second rollup is covered by this window"
    );
    assert!(reading.partial);
    assert_eq!(
        rate.trend(75_000, TokenWindow::MINUTE)
            .into_iter()
            .flatten()
            .sum::<f64>(),
        30.0
    );
    assert_eq!(
        rate.member("a", 75_000, TokenWindow::HOUR).unwrap().tokens,
        60
    );
}

#[test]
fn reseeding_preserves_older_observations_in_the_same_long_window_ring() {
    let mut rate = Rate::new(TokenWindow::parse("2h").unwrap());
    rate.sample(&input(100), 0);
    rate.sample(&input(110), 10_000);
    let mut seeds = fixture_seeds(&history_fixture());
    let row = seeds.get_mut("a").unwrap().as_mut().unwrap();
    row.from += 3_600_000;
    row.through += 3_600_000;
    row.available = row.available.map(|at| at + 3_600_000);
    row.sampled = row.sampled.map(|at| at + 3_600_000);
    for bucket in &mut row.buckets {
        bucket.from_ms += 3_600_000;
        bucket.to_ms += 3_600_000;
    }
    rate.seed(&input(100), &seeds, 3_622_500);
    let reading = rate
        .reading(3_622_500, TokenWindow::parse("2h").unwrap())
        .unwrap();
    assert_eq!(reading.tokens, 36);
    assert!(reading.partial);
    rate.seed(&input(100), &seeds, 3_625_000);
    assert_eq!(
        rate.reading(3_625_000, TokenWindow::parse("2h").unwrap())
            .unwrap()
            .tokens,
        36
    );
}

#[test]
fn malformed_history_is_not_consumption_evidence() {
    let fixture = history_fixture();
    let original = fixture["response"].clone();
    assert!(history::Seed::read(&original, &original["identities"][0], 15_000).is_some());
    for kind in [
        "clock",
        "cache",
        "coverage",
        "bounds",
        "latest_time",
        "complete_gap",
        "missing",
    ] {
        let mut value = original.clone();
        match kind {
            "clock" => value["throughMs"] = json!(25_000),
            "cache" => {
                value["identities"][0]["windows"][0]["buckets"][1]["cachedInputTokens"] = json!(99)
            }
            "coverage" => {
                value["identities"][0]["windows"][0]["buckets"][1]["coveredMs"] = json!(6_000)
            }
            "bounds" => value["identities"][0]["windows"][0]["buckets"][1]["fromMs"] = json!(5_000),
            "latest_time" => value["identities"][0]["lastSampleAtMs"] = json!(21_000),
            "complete_gap" => {
                value["identities"][0]["windows"][0]["buckets"][1]["gap"] = json!(true)
            }
            "missing" => value["identities"][0]["found"] = json!(false),
            _ => unreachable!(),
        }
        assert!(
            history::Seed::read(&value, &value["identities"][0], 15_000).is_none(),
            "{kind}"
        );
    }
}
