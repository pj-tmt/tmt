use super::*;

fn burn_per_hour(history: &[Reading]) -> Option<f64> {
    pace(history).0
}

const HOUR: u64 = HOUR_MS;
const T0: u64 = 1_791_600_000_000;
const RESET: u64 = T0 + 100 * HOUR;

fn reading(hours: f64, left: f64) -> Reading {
    reading_with_reset(hours, left, RESET)
}

fn reading_with_reset(hours: f64, left: f64, resets_at_ms: u64) -> Reading {
    Reading {
        observed_at_ms: T0 + (hours * HOUR as f64) as u64,
        weekly: Window { left, resets_at_ms },
        short: None,
    }
}

#[test]
fn burn_is_the_drop_in_percent_left_over_the_span_in_hours() {
    let history = [reading(0.0, 80.0), reading(1.0, 79.0), reading(3.0, 77.0)];
    assert_eq!(burn_per_hour(&history), Some(1.0));
    assert_eq!(
        burn_per_hour(&[reading(0.0, 80.0), reading(2.0, 79.0)]),
        Some(0.5)
    );
}

#[test]
fn burn_needs_an_hour_of_samples_and_never_goes_negative() {
    assert_eq!(burn_per_hour(&[]), None);
    assert_eq!(burn_per_hour(&[reading(0.0, 80.0)]), None);
    assert_eq!(
        burn_per_hour(&[reading(0.0, 80.0), reading(0.9, 70.0)]),
        None
    );
    assert_eq!(
        burn_per_hour(&[reading(0.0, 80.0), reading(1.0, 79.0)]),
        Some(1.0)
    );
    assert_eq!(
        burn_per_hour(&[reading(0.0, 80.0), reading(2.0, 80.0)]),
        Some(0.0)
    );
}

#[test]
fn a_refill_starts_a_new_cycle_and_older_samples_do_not_count() {
    let history = [
        reading(0.0, 50.0),
        reading(1.0, 40.0),
        reading(2.0, 90.0),
        reading(4.0, 88.0),
    ];
    assert_eq!(burn_per_hour(&history), Some(1.0));
    // Right after the refill there is no hour of the new cycle yet.
    assert_eq!(burn_per_hour(&history[..3]), None);
}

#[test]
fn a_later_reset_time_starts_a_new_cycle_but_rounding_does_not() {
    let next_week = RESET + 168 * HOUR;
    let moved = [
        reading_with_reset(0.0, 10.0, RESET),
        // The percent did not rise, but the window is a new one.
        reading_with_reset(1.0, 9.0, next_week),
        reading_with_reset(3.0, 7.0, next_week),
    ];
    assert_eq!(burn_per_hour(&moved), Some(1.0));
    let rounded = [
        reading_with_reset(0.0, 10.0, RESET),
        reading_with_reset(3.0, 7.0, RESET + 10 * 60_000),
    ];
    assert_eq!(burn_per_hour(&rounded), Some(1.0));
}

#[test]
fn only_the_last_six_hours_before_the_newest_sample_count() {
    let history = [reading(0.0, 90.0), reading(5.0, 85.0), reading(8.0, 80.0)];
    // The sample 8 h back is out; 5 -> 8 h is 5 points over 3 h.
    assert_eq!(burn_per_hour(&history), Some(5.0 / 3.0));
}

#[test]
fn a_gap_between_samples_still_yields_the_average_across_it() {
    // The board was closed in between; the percent still dropped.
    assert_eq!(
        burn_per_hour(&[reading(0.0, 70.0), reading(5.0, 60.0)]),
        Some(2.0)
    );
}

fn latest(hours: f64, left: f64) -> Reading {
    reading(hours, left)
}

fn at(hours: f64) -> u64 {
    T0 + (hours * HOUR as f64) as u64
}

fn known(outlook: Outlook) -> Figures {
    match outlook {
        Outlook::Known(figures) => figures,
        other => panic!("expected figures, got {other:?}"),
    }
}

#[test]
fn a_fresh_reading_reports_percent_left_and_time_to_reset_as_of_now() {
    let mut reading = latest(0.0, 44.0);
    reading.short = Some(Window {
        left: 97.0,
        resets_at_ms: at(3.0),
    });
    let figures = known(outlook(Some(&reading), Some(2.1), None, at(1.0)));
    assert_eq!(figures.weekly.percent, 44.0);
    assert_eq!(figures.weekly.resets_in_ms, 99 * HOUR);
    assert_eq!(
        figures.short,
        Some(Left {
            percent: 97.0,
            resets_in_ms: 2 * HOUR
        })
    );
    assert_eq!(figures.burn, Some(2.1));
}

#[test]
fn no_sample_and_an_old_sample_are_unknown_with_their_age() {
    assert_eq!(
        outlook(None, None, None, at(1.0)),
        Outlook::Unknown { age_ms: None }
    );
    let reading = latest(0.0, 44.0);
    assert!(matches!(
        outlook(Some(&reading), None, None, at(2.0)),
        Outlook::Known(_)
    ));
    assert_eq!(
        outlook(Some(&reading), None, None, at(2.0) + 1),
        Outlook::Unknown {
            age_ms: Some(2 * HOUR + 1)
        }
    );
}

#[test]
fn a_reading_whose_week_already_reset_is_not_quoted() {
    let reading = reading_with_reset(0.0, 44.0, at(1.0));
    assert_eq!(
        outlook(Some(&reading), Some(1.0), None, at(1.0)),
        Outlook::Unknown { age_ms: Some(HOUR) }
    );
}

#[test]
fn an_expired_short_window_drops_out_while_the_week_stays() {
    let mut reading = latest(0.0, 44.0);
    reading.short = Some(Window {
        left: 97.0,
        resets_at_ms: at(0.5),
    });
    let figures = known(outlook(Some(&reading), None, None, at(1.0)));
    assert_eq!(figures.short, None);
}

#[test]
fn running_out_means_the_burn_empties_the_week_before_it_resets() {
    // 30% left at 2%/h lasts 15 h.
    let before_reset = |hours_to_reset: f64, burn: Option<f64>| {
        let reading = reading_with_reset(0.0, 30.0, at(hours_to_reset));
        known(outlook(Some(&reading), burn, None, at(0.0))).runs_out_before_reset
    };
    assert!(before_reset(20.0, Some(2.0)));
    assert!(!before_reset(10.0, Some(2.0)));
    assert!(
        !before_reset(15.0, Some(2.0)),
        "emptying exactly at the reset is not before it"
    );
    assert!(!before_reset(20.0, Some(0.0)));
    assert!(!before_reset(20.0, None));
}

#[test]
fn urgent_depletion_includes_the_twelve_hour_boundary_only_before_reset() {
    for (duration, urgent) in [
        (URGENT_EMPTY_MS - 1, true),
        (URGENT_EMPTY_MS, true),
        (URGENT_EMPTY_MS + 1, false),
    ] {
        let reading = reading_with_reset(0.0, duration as f64 / HOUR as f64, RESET);
        let figures = known(outlook(Some(&reading), Some(1.0), None, T0));
        assert_eq!(figures.empties_in_ms(), Some(duration));
        assert_eq!(figures.empties_soon(), urgent, "{duration}");
    }
    let reading = reading_with_reset(0.0, 12.0, T0 + URGENT_EMPTY_MS);
    let figures = known(outlook(Some(&reading), Some(1.0), None, T0));
    assert!(
        !figures.empties_soon(),
        "reset at depletion is not a warning"
    );
}

#[test]
fn depletion_requires_a_positive_finite_rate_and_a_current_reading() {
    let reading = latest(0.0, 0.0);
    for burn in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        let figures = known(outlook(Some(&reading), burn, None, T0));
        assert_eq!(figures.empties_in_ms(), None);
        assert!(!figures.runs_out_before_reset);
        assert!(!figures.empties_soon());
    }
    let figures = known(outlook(Some(&reading), Some(1.0), None, T0));
    assert_eq!(figures.empties_in_ms(), Some(0));
    assert!(figures.empties_soon());
    assert!(matches!(
        outlook(Some(&reading), Some(1.0), None, T0 + STALE_MS + 1),
        Outlook::Unknown { .. }
    ));
    assert!(matches!(
        outlook(None, Some(1.0), None, T0),
        Outlook::Unknown { .. }
    ));
}

#[test]
fn trend_compares_the_recent_actual_span_with_the_displayed_baseline() {
    // Six-hour pace is 2 pp/h; the recent two-hour pace is 4 pp/h.
    let up = [reading(0.0, 90.0), reading(4.0, 86.0), reading(6.0, 78.0)];
    assert_eq!(pace(&up), (Some(2.0), Some(Trend::Up)));
    let down = [reading(0.0, 90.0), reading(4.0, 79.0), reading(6.0, 78.0)];
    assert_eq!(pace(&down), (Some(2.0), Some(Trend::Down)));
    let same = [reading(0.0, 90.0), reading(4.0, 82.0), reading(6.0, 78.0)];
    assert_eq!(pace(&same), (Some(2.0), Some(Trend::Same)));
    // A gap leaves the recent actual span at 1.5 h; no interpolation to 2 h.
    let gap = [reading(0.0, 90.0), reading(4.5, 84.0), reading(6.0, 78.0)];
    assert_eq!(pace(&gap), (Some(2.0), Some(Trend::Up)));
}

#[test]
fn trend_requires_five_hours_of_baseline_and_one_hour_of_recent_history() {
    assert_eq!(pace(&[]), (None, None));
    assert_eq!(pace(&[reading(0.0, 90.0)]), (None, None));
    for history in [
        vec![reading(0.0, 90.0), reading(2.9, 87.0), reading(4.9, 84.0)],
        vec![reading(0.0, 90.0), reading(4.1, 87.0), reading(5.0, 84.0)],
        vec![reading(0.0, 90.0), reading(2.9, 87.0), reading(5.0, 84.0)],
    ] {
        assert!(pace(&history).0.is_some());
        assert_eq!(pace(&history).1, None);
    }
    let boundary = [reading(0.0, 90.0), reading(4.0, 86.0), reading(5.0, 85.0)];
    assert_eq!(pace(&boundary), (Some(1.0), Some(Trend::Same)));
}

#[test]
fn trend_obeys_the_same_refill_reset_and_six_hour_fences_as_burn() {
    let refill = [
        reading(0.0, 90.0),
        reading(4.0, 89.0),
        reading(5.0, 99.0),
        reading(6.0, 98.0),
    ];
    assert_eq!(pace(&refill), (Some(1.0), None));
    let reset = [
        reading(0.0, 90.0),
        reading(4.0, 86.0),
        reading_with_reset(5.0, 85.0, RESET + NEW_CYCLE_MS + 1),
        reading_with_reset(6.0, 84.0, RESET + NEW_CYCLE_MS + 1),
    ];
    assert_eq!(pace(&reset), (Some(1.0), None));
    let old = [reading(0.0, 90.0), reading(4.0, 86.0), reading(6.1, 84.0)];
    assert_eq!(pace(&old).1, None);
    let rounding = [
        reading(0.0, 90.0),
        reading(4.0, 86.0),
        reading_with_reset(6.0, 84.0, RESET + NEW_CYCLE_MS),
    ];
    assert_eq!(pace(&rounding), (Some(1.0), Some(Trend::Same)));
}

#[test]
fn whole_percent_floor_does_not_flip_on_one_tick_but_can_on_two() {
    for hours in [1.0, 2.0] {
        let one_tick = READING_STEP_PERCENT / hours;
        assert_eq!(compare_rates(1.0, 1.0 + one_tick, hours), Trend::Same);
        assert_eq!(compare_rates(1.0 + one_tick, 1.0, hours), Trend::Same);
        assert_eq!(compare_rates(1.0, 1.0 + 2.0 * one_tick, hours), Trend::Up);
        assert_eq!(compare_rates(1.0 + 2.0 * one_tick, 1.0, hours), Trend::Down);
    }
    assert_eq!(compare_rates(0.0, 0.0, 2.0), Trend::Same);
    assert_eq!(
        compare_rates(0.25, 0.0, 2.0),
        Trend::Same,
        "low baselines conservatively cannot produce a down arrow"
    );
}

#[test]
fn trend_thresholds_are_strict_including_the_absolute_and_relative_floors() {
    // A >2 h span cannot be admitted today; exercise the absolute floor itself
    // so changing the observation window cannot silently weaken it later.
    assert_eq!(compare_rates(0.0, 0.1, 20.0), Trend::Same);
    assert_eq!(compare_rates(0.0, 0.1001, 20.0), Trend::Up);
    assert_eq!(compare_rates(10.0, 12.0, 2.0), Trend::Same);
    assert_eq!(compare_rates(10.0, 12.01, 2.0), Trend::Up);
    assert_eq!(compare_rates(10.0, 8.0, 2.0), Trend::Same);
    assert_eq!(compare_rates(10.0, 7.99, 2.0), Trend::Down);
    // Eight points in six hours versus three in the latest 1.5 h is exactly
    // one reading step apart; subtracting rounded rates must not flip it.
    // Pins the IEEE result: 8/6 + 1/1.5 equals 2.0 exactly in f64.
    let fractional = [reading(0.0, 90.0), reading(4.5, 85.0), reading(6.0, 82.0)];
    assert_eq!(pace(&fractional), (Some(8.0 / 6.0), Some(Trend::Same)));
}

#[test]
fn trend_is_omitted_without_a_displayed_pace_or_a_current_reading() {
    let flat = [reading(0.0, 90.0), reading(4.0, 90.0), reading(6.0, 90.0)];
    assert_eq!(pace(&flat), (Some(0.0), None));
    let reading = latest(0.0, 44.0);
    for burn in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        assert_eq!(
            known(outlook(Some(&reading), burn, Some(Trend::Up), T0)).trend,
            None
        );
    }
    assert_eq!(
        known(outlook(Some(&reading), Some(1.0), Some(Trend::Up), T0)).trend,
        Some(Trend::Up)
    );
    assert!(matches!(
        outlook(
            Some(&reading),
            Some(1.0),
            Some(Trend::Up),
            T0 + STALE_MS + 1
        ),
        Outlook::Unknown { .. }
    ));
    let reset = reading_with_reset(0.0, 44.0, at(1.0));
    assert!(matches!(
        outlook(Some(&reset), Some(1.0), Some(Trend::Up), at(1.0)),
        Outlook::Unknown { .. }
    ));
}
