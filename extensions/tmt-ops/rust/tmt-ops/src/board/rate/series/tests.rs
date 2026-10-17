use super::*;

fn project<'a>(rates: impl IntoIterator<Item = &'a Rate>, grid: Grid) -> Series {
    super::project(rates, grid, None)
}
use crate::board::rate::{history, tests::input};

const HOUR: u64 = 3_600_000;

fn retained_seed(request: rollup::Request, through: u64) -> rollup::Seed {
    let from = through - request.range_ms();
    rollup::Seed {
        limits: Vec::new(),
        request,
        through,
        fine_through: through,
        sampled: Some(through - SLOT_MS),
        latest: input(100).resumes["a"].clone(),
        buckets: (0..request.buckets())
            .map(|index| history::Slice {
                from_ms: from + index as u64 * request.each_ms(),
                to_ms: from + (index as u64 + 1) * request.each_ms(),
                input_tokens: index as u64 + 1,
                output_tokens: 1,
                cached_input_tokens: 0,
                covered_ms: request.each_ms(),
                complete: true,
                gap: false,
                discontinuous: false,
            })
            .collect(),
    }
}

fn recent_at(through: u64, gap: bool) -> Rate {
    let mut seed = history_seed(false, false);
    let offset = through - HOUR;
    seed.from += offset;
    seed.through += offset;
    seed.available = Some(offset);
    seed.sampled = Some(through - SLOT_MS);
    for slice in &mut seed.buckets {
        slice.from_ms += offset;
        slice.to_ms += offset;
    }
    if gap {
        let last = seed.buckets.last_mut().unwrap();
        last.complete = false;
        last.gap = true;
        last.covered_ms = 0;
    }
    let mut rate = Rate::default();
    rate.seed(&input(100), &[("a".into(), Some(seed))].into(), through);
    rate
}

#[test]
fn retained_ranges_use_one_tier_and_leave_recent_readings_and_ring_size_unchanged() {
    let through = 31 * 86_400_000;
    for (range, each) in [(360, 15), (1_440, 60), (10_080, 360), (43_200, 1_440)] {
        let request = rollup::Request::new(each * 60_000, range * 60_000).unwrap();
        let mut rate = recent_at(through, false);
        let before = rate.reading(through, TokenWindow::HOUR);
        let ring = rate.members["a"].buckets.len();
        rate.seed_retained(
            &input(100),
            &[("a".into(), Some(retained_seed(request, through)))].into(),
            through,
        );
        assert_eq!(rate.reading(through, TokenWindow::HOUR), before);
        assert_eq!(rate.members["a"].buckets.len(), ring);
        assert_eq!(ring, 720);
        let grid = Grid::new(request.each_ms(), request.range_ms(), through).unwrap();
        let series = project([&rate], grid);
        assert_eq!(series.buckets.len(), request.buckets());
        let expected = (request.buckets() * (request.buckets() + 3) / 2) as u128;
        assert_eq!(
            series
                .buckets
                .iter()
                .map(|b| b.tokens.unwrap())
                .sum::<u128>(),
            expected
        );
        assert!(series.buckets.iter().all(Interval::complete));
        assert!(rate.members["a"].retained.as_ref().unwrap().buckets.len() <= 48);
        assert_eq!(series.buckets.last().unwrap().to_ms, through as i64);
    }
}

#[test]
fn retained_provider_suffix_requires_the_matching_fine_bridge_and_stops_at_discontinuity() {
    let through = 31 * 86_400_000;
    let request = rollup::Request::new(21_600_000, 604_800_000).unwrap();
    for (fine_gap, same_watermark) in [(false, true), (true, true), (false, false)] {
        let fine_through = through + 20_000;
        let mut rate = recent_at(fine_through, fine_gap);
        let mut seed = retained_seed(request, through);
        seed.fine_through = fine_through;
        seed.sampled = Some(fine_through - SLOT_MS);
        seed.buckets[10].discontinuous = true;
        seed.buckets[10].complete = false;
        seed.buckets[10].gap = true;
        if !same_watermark {
            seed.latest = input(101).resumes["a"].clone();
        }
        rate.seed_retained(
            &input(100),
            &[("a".into(), Some(seed))].into(),
            fine_through,
        );
        let series = project(
            [&rate],
            Grid::new(request.each_ms(), request.range_ms(), fine_through).unwrap(),
        );
        for (index, bucket) in series.buckets.iter().enumerate() {
            if !fine_gap && same_watermark && index > 10 {
                assert_eq!(bucket.providers.get("claude"), bucket.tokens.as_ref());
            } else {
                assert!(bucket.providers.is_empty());
            }
        }
    }
}

#[test]
fn retained_zero_missing_stale_and_other_selections_do_not_borrow_fine_totals() {
    let through = 31 * 86_400_000;
    let request = rollup::Request::new(86_400_000, rollup::RETENTION_MS).unwrap();
    let mut rate = recent_at(through, false);
    let mut seed = retained_seed(request, through);
    seed.latest = serde_json::Value::Null;
    seed.sampled = None;
    seed.buckets[0].input_tokens = 0;
    seed.buckets[0].output_tokens = 0;
    seed.buckets[1].input_tokens = 0;
    seed.buckets[1].output_tokens = 0;
    seed.buckets[1].covered_ms = 0;
    seed.buckets[1].complete = false;
    seed.buckets[1].gap = true;
    rate.seed_retained(&input(100), &[("a".into(), Some(seed))].into(), through);
    let grid = Grid::new(request.each_ms(), request.range_ms(), through).unwrap();
    let known = project([&rate], grid);
    assert_eq!(known.buckets[0].tokens, Some(0));
    assert!(known.buckets[0].complete());
    assert_eq!(known.buckets[1].tokens, None);
    assert!(!known.buckets[1].complete());
    assert!(
        known
            .buckets
            .iter()
            .all(|bucket| bucket.providers.is_empty())
    );
    for stale in [
        Grid::new(
            request.each_ms(),
            request.range_ms(),
            through + rollup::RESOLUTION_MS,
        )
        .unwrap(),
        Grid::new(2 * request.each_ms(), request.range_ms(), through).unwrap(),
    ] {
        assert!(
            project([&rate], stale)
                .buckets
                .iter()
                .all(|bucket| bucket.tokens.is_none())
        );
    }
    let before = rate.reading(through, TokenWindow::HOUR);
    rate.seed_retained(&input(100), &[("a".into(), None)].into(), through);
    assert!(
        project([&rate], grid)
            .buckets
            .iter()
            .all(|bucket| bucket.tokens.is_none())
    );
    assert_eq!(rate.reading(through, TokenWindow::HOUR), before);
}

#[test]
fn retained_duplicates_choose_one_whole_uuid_and_scope_keeps_missing_pool_identity() {
    let through = 31 * 86_400_000;
    let request = rollup::Request::new(21_600_000, 604_800_000).unwrap();
    let mut strong = recent_at(through, false);
    strong.seed_retained(
        &input(100),
        &[("a".into(), Some(retained_seed(request, through)))].into(),
        through,
    );
    let weak = recent_at(through, false);
    let grid = Grid::new(request.each_ms(), request.range_ms(), through).unwrap();
    let one = project([&strong], grid);
    assert_eq!(project([&weak, &strong, &strong], grid), one);
    let expected = BTreeSet::from(["a", "pool"]);
    let scoped = super::project([&strong], grid, Some(&expected));
    for (bucket, known) in scoped.buckets.iter().zip(one.buckets) {
        assert_eq!(bucket.tokens, known.tokens);
        assert_eq!(bucket.identities, 2);
        assert_eq!(bucket.observed_identities, 1);
        assert!(!bucket.complete());
    }
}

#[test]
fn retained_provider_names_survive_recent_ring_expiry_and_live_driver_changes() {
    let through = 31 * 86_400_000;
    let request = rollup::Request::new(21_600_000, 604_800_000).unwrap();
    let mut rate = recent_at(through, false);
    rate.seed_retained(
        &input(100),
        &[("a".into(), Some(retained_seed(request, through)))].into(),
        through,
    );
    let grid = Grid::new(request.each_ms(), request.range_ms(), through).unwrap();
    let before = project([&rate], grid);
    assert!(
        before
            .buckets
            .iter()
            .all(|bucket| bucket.providers.contains_key("claude"))
    );
    // Reclaim every fine cell, then admit enough different verified drivers to
    // exercise the compact ID allocator. Retained provenance must stay Claude.
    for index in 0..20 {
        let mut next = input(200 + index);
        next.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!(format!("driver{index}"));
        rate.sample(&next, through + HOUR + (index + 1) * SLOT_MS);
    }
    let after = project([&rate], grid);
    assert_eq!(after.buckets, before.buckets);
    assert_eq!(after.grid, before.grid);
    assert_eq!(
        after.drivers,
        BTreeSet::from(["claude".into(), "driver19".into()])
    );
    rate.seed_retained(
        &input(300),
        &[("a".into(), None)].into(),
        through + 2 * HOUR,
    );
    assert!(
        project([&rate], grid)
            .buckets
            .iter()
            .all(|bucket| bucket.tokens.is_none())
    );
}

fn history_seed(zero: bool, partial: bool) -> history::Seed {
    history::Seed {
        from: 0,
        through: HOUR,
        available: Some(0),
        sampled: Some(HOUR - SLOT_MS),
        latest: input(100).resumes["a"].clone(),
        reporting: true,
        buckets: (0..120)
            .map(|index| history::Slice {
                from_ms: index * 30_000,
                to_ms: (index + 1) * 30_000,
                input_tokens: if zero { 0 } else { index + 1 },
                output_tokens: u64::from(!zero),
                cached_input_tokens: if zero { 0 } else { index.div_ceil(2) },
                covered_ms: if partial { 10_000 } else { 30_000 },
                complete: !partial,
                gap: partial,
                discontinuous: false,
            })
            .collect(),
    }
}
fn seeded(zero: bool, partial: bool) -> Rate {
    let seed = history_seed(zero, partial);
    let mut rate = Rate::default();
    rate.seed(
        &input(100),
        &BTreeMap::from([("a".into(), Some(seed))]),
        HOUR,
    );
    rate
}

#[test]
fn every_allowed_pair_conserves_exact_totals_over_equal_source_coverage() {
    let rate = seeded(false, false);
    let mut pairs = 0;
    for each in [5, 15, 30, 60] {
        for range in [15, 30, 60] {
            if each > range {
                assert!(Grid::new(each * 60_000, range * 60_000, HOUR).is_err());
                continue;
            }
            pairs += 1;
            let grid = Grid::new(each * 60_000, range * 60_000, HOUR).unwrap();
            let series = project([&rate], grid);
            assert_eq!(series.buckets.len(), (range / each) as usize);
            let first_slice = 120 - range * 2;
            let expected: u128 = (first_slice..120).map(|i| u128::from(i + 2)).sum();
            assert_eq!(
                series
                    .buckets
                    .iter()
                    .map(|b| b.tokens.unwrap())
                    .sum::<u128>(),
                expected
            );
            for bucket in &series.buckets {
                assert_eq!(bucket.to_ms - bucket.from_ms, (each * 60_000) as i64);
                assert!(bucket.complete());
                assert_eq!(bucket.source_identity_ms, u128::from(each * 60_000));
            }
        }
    }
    assert_eq!(pairs, 9);
    assert_eq!(
        project([&rate], Grid::new(300_000, HOUR, HOUR).unwrap())
            .buckets
            .len(),
        12
    );
}

#[test]
fn measured_zero_missing_and_partial_durations_remain_distinct() {
    let grid = Grid::new(300_000, HOUR, HOUR).unwrap();
    let zero = seeded(true, false);
    assert!(
        project([&zero], grid)
            .buckets
            .iter()
            .all(|b| b.tokens == Some(0) && b.complete())
    );
    let mut missing = Rate::default();
    let mut no_report = input(100);
    no_report
        .resumes
        .insert("a".into(), serde_json::Value::Null);
    missing.sample(&no_report, HOUR);
    assert!(
        project([&missing], grid)
            .buckets
            .iter()
            .all(|b| b.tokens.is_none() && !b.complete())
    );
    let partial = seeded(false, true);
    for bucket in project([&partial], grid).buckets {
        assert!(bucket.tokens.is_some());
        assert!(!bucket.complete());
        assert_eq!(bucket.source_identity_ms, 100_000);
        assert_eq!(bucket.duration_identities, 1);
    }
}

#[test]
fn source_cut_is_disclosed_and_never_prorated() {
    let rate = seeded(false, false);
    let closed = project([&rate], Grid::new(300_000, 900_000, HOUR).unwrap());
    let shifted = project(
        [&rate],
        Grid::new(300_000, 900_000, HOUR + SLOT_MS).unwrap(),
    );
    assert!(closed.buckets.iter().all(Interval::complete));
    assert!(
        shifted
            .buckets
            .iter()
            .all(|b| b.boundary_cut && !b.complete())
    );
    assert!(shifted.buckets[0].tokens.unwrap() < closed.buckets[0].tokens.unwrap());
    assert!(
        shifted
            .buckets
            .iter()
            .all(|b| b.source_identity_ms < 300_000)
    );
}

#[test]
fn duplicates_choose_one_whole_observation_and_unreported_uuid_is_not_zero() {
    let strong = seeded(false, false);
    let mut weak = Rate::default();
    weak.sample(&input(100), HOUR - 10_000);
    weak.sample(&input(101), HOUR - SLOT_MS);
    let grid = Grid::new(300_000, HOUR, HOUR).unwrap();
    let one = project([&strong], grid);
    assert_eq!(project([&weak, &strong], grid), one);
    assert_eq!(project([&strong, &strong], grid), one);
    let mut silent = Rate::default();
    let mut absent = input(100);
    absent.resumes = BTreeMap::from([("silent".into(), serde_json::Value::Null)]);
    silent.sample(&absent, HOUR);
    for (mixed, known) in project([&strong, &silent], grid)
        .buckets
        .iter()
        .zip(&one.buckets)
    {
        assert_eq!(mixed.tokens, known.tokens);
        assert_eq!(mixed.identities, 2);
        assert_eq!(mixed.observed_identities, 1);
        assert_eq!(mixed.complete_identities, 1);
        assert!(
            !mixed.complete(),
            "one UUID never grants joint completeness"
        );
    }
}

#[test]
fn receipt_evidence_is_not_reconstructed_as_source_duration() {
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    rate.sample(&input(100), SLOT_MS);
    let series = project([&rate], Grid::new(300_000, 900_000, SLOT_MS * 2).unwrap());
    let latest = series.buckets.last().unwrap();
    assert_eq!(latest.tokens, Some(0));
    assert_eq!(latest.observed_identities, 1);
    assert_eq!(latest.source_identity_ms, 0);
    assert_eq!(latest.duration_identities, 0);
    assert!(!latest.complete());
}

#[test]
fn invalid_grids_and_empty_roster_never_create_measured_zero() {
    for (each, range) in [
        (0, HOUR),
        (60_000, HOUR),
        (HOUR, 900_000),
        (300_000, HOUR + 1),
    ] {
        assert!(Grid::new(each, range, HOUR).is_err());
    }
    assert!(Grid::new(300_000, HOUR, u64::MAX).is_err());
    let series = project([], Grid::new(300_000, HOUR, 0).unwrap());
    assert_eq!(series.buckets.len(), 12);
    assert!(
        series
            .buckets
            .iter()
            .all(|b| b.tokens.is_none() && !b.complete())
    );
}

#[test]
fn canonical_scope_counts_missing_pool_members_and_excludes_stale_ids() {
    let rate = seeded(false, false);
    let grid = Grid::new(300_000, HOUR, HOUR).unwrap();
    let expected = BTreeSet::from(["a", "pool"]);
    let series = super::project([&rate], grid, Some(&expected));
    assert!(
        series
            .buckets
            .iter()
            .all(|b| b.identities == 2 && b.observed_identities == 1 && !b.complete())
    );
    let only_pool = BTreeSet::from(["pool"]);
    assert!(
        super::project([&rate], grid, Some(&only_pool))
            .buckets
            .iter()
            .all(|b| b.identities == 1 && b.tokens.is_none())
    );
}

#[test]
fn historical_provider_suffix_stops_at_gaps_boundaries_and_an_unusable_watermark() {
    let grid = Grid::new(300_000, HOUR, HOUR).unwrap();
    for boundary in ["gap", "discontinuous", "watermark", "sample", "driver"] {
        let mut seed = history_seed(false, false);
        seed.latest["driver"] = serde_json::json!("codex");
        match boundary {
            "gap" => {
                seed.buckets[59].gap = true;
                seed.buckets[59].complete = false;
            }
            "discontinuous" => seed.buckets[59].discontinuous = true,
            "sample" => seed.sampled = None,
            "driver" => seed.latest["driver"] = serde_json::json!("invalid\nname"),
            _ => seed.latest = serde_json::Value::Null,
        }
        let mut rate = Rate::default();
        rate.seed(&input(100), &[("a".into(), Some(seed))].into(), HOUR);
        let series = project([&rate, &rate], grid);
        assert!(
            series.drivers.contains("claude"),
            "current driver label survives a different historical watermark"
        );
        assert_eq!(
            series
                .buckets
                .iter()
                .filter_map(|bucket| bucket.tokens)
                .sum::<u128>(),
            7380
        );
        for (index, bucket) in series.buckets.iter().enumerate() {
            assert_eq!(
                bucket.providers.get("claude").copied().unwrap_or(0),
                0,
                "latest Codex never rewrites older Claude/unknown history"
            );
            assert_eq!(
                bucket.providers.get("codex").copied().unwrap_or(0),
                if index >= 6 && matches!(boundary, "gap" | "discontinuous") {
                    bucket.tokens.unwrap()
                } else {
                    0
                },
                "{boundary} interval {index}"
            );
        }
    }
}

#[test]
fn live_driver_changes_and_repeated_sequences_never_relabel_or_double_count() {
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    rate.sample(&input(200), 5_000);
    rate.sample(&input(200), 10_000);
    let mut codex = input(900);
    codex.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!("codex");
    rate.sample(&codex, 300_000); // New driver establishes a baseline, not 1,050 tokens.
    let mut next = input(1_000);
    next.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!("codex");
    rate.sample(&next, 305_000);
    rate.sample(&next, 310_000);
    let observed = project(
        [&rate, &rate],
        Grid::new(300_000, 600_000, 600_000).unwrap(),
    );
    assert_eq!(
        observed
            .buckets
            .iter()
            .map(|bucket| bucket.tokens.unwrap_or(0))
            .sum::<u128>(),
        300
    );
    assert_eq!(observed.buckets[0].providers.get("claude"), Some(&150));
    assert_eq!(observed.buckets[1].providers.get("codex"), Some(&150));
}

#[test]
fn a_same_slot_driver_switch_preserves_both_proven_increments_and_reseed_replaces_them() {
    let mut rate = Rate::default();
    rate.sample(&input(100), 0);
    rate.sample(&input(200), 5_000);
    let mut codex = input(900);
    codex.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!("codex");
    rate.sample(&codex, 6_000);
    let mut next = input(1_000);
    next.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!("codex");
    rate.sample(&next, 7_000);
    rate.sample(&next, 8_000);
    let grid = Grid::new(300_000, 300_000, 300_000).unwrap();
    let observed = project([&rate, &rate], grid);
    assert_eq!(observed.buckets[0].tokens, Some(300));
    assert_eq!(
        observed.buckets[0].providers,
        [("claude".into(), 150), ("codex".into(), 150)].into()
    );
    assert_eq!(rate.members["a"].provider_splits.len(), 1);
    let mut seed = history_seed(false, false);
    seed.latest = serde_json::Value::Null;
    rate.seed(&input(100), &[("a".into(), Some(seed))].into(), HOUR);
    assert!(
        rate.members["a"].provider_splits.is_empty(),
        "authoritative seeds revoke the old slot's split"
    );
    assert!(
        project([&rate], Grid::new(300_000, HOUR, HOUR).unwrap())
            .buckets
            .iter()
            .all(|bucket| bucket.providers.values().all(|value| *value == 0))
    );
}

#[test]
fn arbitrary_driver_ids_keep_three_same_slot_parts_and_recycle_only_expired_names() {
    let mut rate = Rate::default();
    for (driver, baseline, at) in [
        ("driver-first", 100, 0),
        ("driver-second", 900, 6_000),
        ("driver-third", 2_000, 8_000),
    ] {
        let mut before = input(baseline);
        before.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!(driver);
        rate.sample(&before, at);
        let mut after = input(baseline + 100);
        after.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!(driver);
        rate.sample(&after, if at == 0 { 5_000 } else { at + 1_000 });
    }
    let grid = Grid::new(300_000, 300_000, 300_000).unwrap();
    let shown = project([&rate, &rate], grid);
    assert_eq!(shown.buckets[0].tokens, Some(450));
    assert_eq!(
        shown.buckets[0].providers,
        [
            ("driver-first".into(), 150),
            ("driver-second".into(), 150),
            ("driver-third".into(), 150)
        ]
        .into()
    );
    let mut last = input(2_100);
    last.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!("driver-third");
    rate.sample(&last, HOUR + 10_000);
    assert!(rate.members["a"].provider_splits.is_empty());
    let mut next = input(5_000);
    next.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!("driver-new");
    rate.sample(&next, HOUR + 11_000);
    next = input(5_100);
    next.resumes.get_mut("a").unwrap()["driver"] = serde_json::json!("driver-new");
    rate.sample(&next, HOUR + 12_000);
    assert_eq!(
        rate.members["a"].drivers.len(),
        3,
        "expired slots permit arena reuse"
    );
    let shown = project([&rate], Grid::new(300_000, 300_000, HOUR + 15_000).unwrap());
    assert_eq!(shown.buckets[0].providers.get("driver-new"), Some(&150));
    assert!(!shown.drivers.contains("driver-first"));
}
