use super::*;

fn project<'a>(rates: impl IntoIterator<Item = &'a Rate>, grid: Grid) -> Series {
    super::project(rates, grid, None)
}
use crate::board::rate::{history, tests::input};

const HOUR: u64 = 3_600_000;

fn seeded(zero: bool, partial: bool) -> Rate {
    let seed = history::Seed {
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
    };
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
