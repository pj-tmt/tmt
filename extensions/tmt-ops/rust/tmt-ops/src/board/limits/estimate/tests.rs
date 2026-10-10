use super::*;

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
    let figures = known(outlook(Some(&reading), Some(2.1), at(1.0)));
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
        outlook(None, None, at(1.0)),
        Outlook::Unknown { age_ms: None }
    );
    let reading = latest(0.0, 44.0);
    assert!(matches!(
        outlook(Some(&reading), None, at(2.0)),
        Outlook::Known(_)
    ));
    assert_eq!(
        outlook(Some(&reading), None, at(2.0) + 1),
        Outlook::Unknown {
            age_ms: Some(2 * HOUR + 1)
        }
    );
}

#[test]
fn a_reading_whose_week_already_reset_is_not_quoted() {
    let reading = reading_with_reset(0.0, 44.0, at(1.0));
    assert_eq!(
        outlook(Some(&reading), Some(1.0), at(1.0)),
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
    let figures = known(outlook(Some(&reading), None, at(1.0)));
    assert_eq!(figures.short, None);
}

#[test]
fn running_out_means_the_burn_empties_the_week_before_it_resets() {
    // 30% left at 2%/h lasts 15 h.
    let before_reset = |hours_to_reset: f64, burn: Option<f64>| {
        let reading = reading_with_reset(0.0, 30.0, at(hours_to_reset));
        known(outlook(Some(&reading), burn, at(0.0))).runs_out_before_reset
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
