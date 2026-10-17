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
    project(points, QuotaWindow::Weekly, NOW, NOW)
}
#[test]
fn rate_uses_actual_endpoint_spacing_and_trend_uses_the_preceding_two_hours() {
    let points = [
        report(NOW - 4 * HOUR, 10.0, RESET),
        report(NOW - 3 * HOUR, 11.0, RESET),
        report(NOW - 2 * HOUR, 20.0, RESET),
        report(NOW - HOUR / 2, 23.0, RESET),
    ];
    let pace = project_week(&points);
    assert_eq!(pace.per_hour, Some(2.0)); // 3 percentage points in 90 minutes.
    assert_eq!(pace.trend, Some(Trend::Up));
    assert_eq!(pace.latest.unwrap().balance().left, 77.0);
    assert_eq!(
        project(&points, QuotaWindow::Short, NOW, NOW).per_hour,
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
    assert_eq!(
        project(&points, QuotaWindow::Weekly, NOW + 3 * HOUR, NOW + 3 * HOUR),
        Pace::default()
    );
    assert_eq!(
        project(&points, QuotaWindow::Weekly, RESET, RESET),
        Pace::default()
    );
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
fn newest_open_observation_cannot_hide_a_reset_or_drop_after_closed_history() {
    let points = [
        report(NOW - 2 * HOUR, 10.0, RESET),
        report(NOW - HOUR, 20.0, RESET),
        report(NOW + 60_000, 30.0, RESET),
        report(NOW + 120_000, 25.0, RESET),
    ];
    let pace = project(&points, QuotaWindow::Weekly, NOW, NOW + 120_000);
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
