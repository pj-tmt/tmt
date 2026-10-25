use super::*;
use serde_json::json;
const HOUR: u64 = 3_600_000;
const NOW: u64 = 100 * HOUR;
const RESET: u64 = NOW + 10 * HOUR;

fn report(at: u64, used: f64, reset: u64) -> Report {
    Report::read(&json!({"observedAtMs":at,"windows":[
        {"windowMinutes":300,"usedPercent":used / 2.0,"resetsAtMs":reset},
        {"windowMinutes":10080,"usedPercent":used,"resetsAtMs":reset}
    ]}))
    .unwrap()
}
fn project_week(points: &[Report]) -> Pace {
    project(points, QuotaWindow::Weekly, NOW)
}
#[test]
fn rate_uses_actual_endpoint_spacing_and_trend_uses_the_preceding_two_hours() {
    let points = [
        report(NOW - 4 * HOUR, 18.0, RESET),
        report(NOW - 3 * HOUR, 19.0, RESET),
        report(NOW - 2 * HOUR, 20.0, RESET),
        report(NOW - HOUR / 2, 23.0, RESET),
    ];
    let pace = project_week(&points);
    assert_eq!(pace.per_hour, Some(2.0)); // 3 percentage points in 90 minutes.
    assert_eq!(pace.trend, Some(Trend::Up));
    assert_eq!(pace.latest.point().unwrap().balance().left, 77.0);
    assert_eq!(
        project(&points, QuotaWindow::Short, NOW).per_hour,
        Some(1.0)
    );
    assert_eq!(pace.early_ms(), None);
}
#[test]
fn duplicate_account_seats_do_not_sum_and_missing_previous_horizon_has_no_arrow() {
    let a = report(NOW - 2 * HOUR, 60.0, RESET);
    let b = report(NOW - HOUR, 70.0, RESET);
    let once = project_week(&[a.clone(), b.clone()]);
    assert_eq!(once.per_hour, Some(10.0));
    assert_eq!(once.trend, None);
    assert_eq!(once.exhausts_at_ms, Some(NOW + 2 * HOUR));
    assert_eq!(once.early_ms(), Some(8 * HOUR));
    assert_eq!(once, project_week(&[b.clone(), a.clone(), b, a]));
}
#[test]
fn sparse_zero_future_stale_and_conflicting_evidence_are_honest() {
    let first = report(NOW - HOUR, 10.0, RESET);
    for points in [
        vec![],
        vec![first.clone()],
        vec![first.clone(), report(NOW - HOUR + 29 * 60_000, 12.0, RESET)],
        vec![
            first.clone(),
            report(NOW - HOUR / 2, 12.0, RESET),
            report(NOW - HOUR / 2, 13.0, RESET),
        ],
    ] {
        let pace = project_week(&points);
        assert_eq!(pace.per_hour, None);
        assert_eq!(pace.trend, None);
        assert_eq!(pace.early_ms(), None);
    }
    let points = [first.clone(), report(NOW - HOUR / 2, 10.0, RESET)];
    let zero = project_week(&points);
    assert_eq!(zero.per_hour, Some(0.0));
    assert_eq!(zero.exhausts_at_ms, None);
    let mut future = points.to_vec();
    future.push(report(NOW + HOUR, 90.0, RESET));
    assert_eq!(project_week(&future), zero);
    for through in [NOW + 3 * HOUR, RESET] {
        let old = project(&points, QuotaWindow::Weekly, through);
        assert_eq!(old.latest, zero.latest);
        assert_eq!(old.per_hour, None);
        assert_eq!(old.trend, None);
        assert_eq!(old.exhausts_at_ms, None);
    }
}
#[test]
fn every_intermediate_drop_or_reset_restarts_the_suffix() {
    for reset in [RESET, RESET + HOUR] {
        let points = [
            report(NOW - 2 * HOUR, 10.0, RESET),
            report(NOW - 90 * 60_000, 30.0, RESET),
            report(NOW - HOUR, 20.0, reset),
            report(NOW - HOUR / 2, 21.0, reset),
        ];
        assert_eq!(project_week(&points).per_hour, Some(2.0));
        assert_eq!(project_week(&points[..3]).per_hour, None);
    }
    let crossed = [
        report(NOW - 2 * HOUR, 10.0, NOW - HOUR),
        report(NOW - HOUR / 2, 20.0, RESET),
    ];
    assert_eq!(project_week(&crossed).per_hour, None);
}
#[test]
fn latest_admitted_drop_cannot_reuse_the_estimate_before_the_drop() {
    let points = [
        report(NOW - 2 * HOUR, 10.0, RESET),
        report(NOW - HOUR, 20.0, RESET),
        report(NOW + 60_000, 30.0, RESET),
        report(NOW + 120_000, 25.0, RESET),
    ];
    let pace = project(&points, QuotaWindow::Weekly, NOW + 120_000);
    assert_eq!(pace.per_hour, None);
    assert_eq!(pace.early_ms(), None);
}

#[test]
fn preceding_horizon_uses_its_own_boundary_and_an_expired_prior_cycle_has_no_arrow() {
    let points = [
        report(NOW - 4 * HOUR, 10.0, NOW - 2 * HOUR - 1),
        report(NOW - 3 * HOUR, 20.0, NOW - 2 * HOUR - 1),
        report(NOW - 2 * HOUR, 30.0, RESET),
        report(NOW - HOUR, 40.0, RESET),
    ];
    let pace = project_week(&points);
    assert_eq!(pace.per_hour, Some(10.0));
    assert_eq!(pace.trend, None);
}

#[test]
fn actual_clock_admits_the_current_observation_after_the_last_closed_bucket() {
    let points = [
        report(NOW - 110 * 60_000, 20.0, RESET),
        report(NOW - 10 * 60_000, 21.0, RESET),
        report(NOW + 60_000, 24.0, RESET),
    ];
    let value = project(&points, QuotaWindow::Weekly, NOW + 3 * 60_000);
    assert!((value.per_hour.unwrap() - 80.0 / 37.0).abs() < 1e-10);
}

#[test]
fn actual_clock_excludes_points_that_have_left_the_two_hour_horizon() {
    let points = [
        report(NOW - 119 * 60_000, 10.0, RESET),
        report(NOW - 110 * 60_000, 20.0, RESET),
        report(NOW - 10 * 60_000, 21.0, RESET),
    ];
    let value = project(&points, QuotaWindow::Weekly, NOW + 3 * 60_000);
    assert!((value.per_hour.unwrap() - 0.6).abs() < 1e-10);
}

#[test]
fn actual_clock_includes_an_observation_at_the_horizon_endpoint() {
    let now = NOW + 3 * 60_000;
    let points = [
        report(NOW - 110 * 60_000, 20.0, RESET),
        report(NOW - 10 * 60_000, 21.0, RESET),
        report(now, 24.0, RESET),
    ];
    let value = project(&points, QuotaWindow::Weekly, now);
    assert!((value.per_hour.unwrap() - 240.0 / 113.0).abs() < 1e-10);
}

#[test]
fn last_known_api_balance_survives_its_rate_horizon() {
    let at = NOW - 40 * HOUR;
    let pace = project_week(&[report(at, 17.0, RESET)]);
    assert_eq!(
        pace.latest
            .point()
            .map(|p| (p.balance().left, p.observed_at_ms)),
        Some((83.0, at))
    );
    assert_eq!(pace.per_hour, None);
    assert_eq!(pace.trend, None);
    assert_eq!(pace.exhausts_at_ms, None);
}

#[test]
fn absence_conflict_and_future_evidence_are_distinct_from_an_old_balance() {
    assert_eq!(project_week(&[]).latest, Evidence::Absent);
    assert_eq!(
        project_week(&[report(NOW + 1, 17.0, RESET)]).latest,
        Evidence::Unavailable
    );
    let first = report(NOW - 40 * HOUR, 17.0, RESET);
    let conflict = report(NOW - 40 * HOUR, 18.0, RESET);
    assert_eq!(
        project_week(&[first.clone(), conflict]).latest,
        Evidence::Unavailable
    );
    let once = project_week(std::slice::from_ref(&first));
    assert_eq!(once, project_week(&[first.clone(), first.clone()]));
    let short = Report::read(&json!({"observedAtMs":NOW,"windows":[
        {"windowMinutes":300,"usedPercent":5,"resetsAtMs":RESET}
    ]}))
    .unwrap();
    assert_eq!(
        project_week(&[first, short.clone()]).latest,
        Evidence::Absent
    );
    assert_eq!(
        project_week(&[report(NOW, 17.0, RESET), short]).latest,
        Evidence::Unavailable
    );
}

#[test]
fn freshness_and_reset_fences_keep_the_balance_but_never_a_stale_forecast() {
    let reports = [report(NOW - HOUR, 10.0, RESET), report(NOW, 17.0, RESET)];
    let boundary = project(&reports, QuotaWindow::Weekly, NOW + 2 * HOUR);
    assert!(boundary.latest.point().is_some());
    for at in [NOW + 2 * HOUR + 1, RESET] {
        let value = project(&reports, QuotaWindow::Weekly, at);
        assert_eq!(value.latest, boundary.latest);
        assert_eq!(value.per_hour, None);
        assert_eq!(value.trend, None);
        assert_eq!(value.exhausts_at_ms, None);
    }
}
