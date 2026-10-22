use super::*;
use crate::board::limits::Window;

const NOW: u64 = 1_791_600_000_000;
const HOUR: u64 = HOUR_MS;

/// A history sampled every 30 minutes that ends `now`, left dropping by
/// `per_hour` percent an hour from `start` percent, in a window that refills at
/// `resets_at`.
fn history(hours: u64, start: f64, per_hour: f64, resets_at: u64) -> Vec<Reading> {
    (0..=hours * 2)
        .map(|step| {
            let at = NOW - hours * HOUR + step * HOUR / 2;
            Reading {
                observed_at_ms: at,
                weekly: Window {
                    left: start - per_hour * (step as f64 / 2.0),
                    resets_at_ms: resets_at,
                },
                short: None,
            }
        })
        .collect()
}

fn standing(history: &[Reading]) -> Standing {
    let (burn, trend) = estimate::pace(history);
    Standing {
        provider: Provider::named("claude"),
        latest: history.last().copied(),
        burn,
        trend,
        rates: rates(history),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn each_span_averages_the_drop_over_its_own_hours() {
    // Faster for the last 3 hours (2 pt/h) than before (1 pt/h).
    let mut readings = history(24, 90.0, 1.0, NOW + 40 * HOUR);
    let drop_at = NOW - 3 * HOUR;
    let mut left = readings
        .iter()
        .find(|r| r.observed_at_ms == drop_at)
        .unwrap()
        .weekly
        .left;
    for reading in readings.iter_mut().filter(|r| r.observed_at_ms > drop_at) {
        left -= 1.0 / 2.0 * 2.0;
        reading.weekly.left = left + 1.0 / 2.0 * 0.0;
    }
    let rates = rates(&readings);
    let measured = |line| rates.get(line).unwrap();
    assert!(measured(Line::Hours3) > measured(Line::Hours6));
    assert!(measured(Line::Hours6) > measured(Line::Hours24));
    assert!(measured(Line::Hours24) > measured(Line::SinceReset) * 0.99);
}

#[test]
fn a_steady_burn_gives_the_same_rate_on_every_line() {
    let rates = rates(&history(26, 80.0, 0.75, NOW + 40 * HOUR));
    // The tokens line has no spent tokens here: its own tests cover it.
    for line in [Line::Hours3, Line::Hours6, Line::Hours24, Line::SinceReset] {
        assert!(close(rates.get(line).unwrap(), 0.75), "{line:?}");
    }
}

#[test]
fn a_span_the_history_does_not_cover_has_no_rate() {
    // Four hours of history: the 24 h line needs twelve, the 6 h line three.
    let rates = rates(&history(4, 50.0, 1.0, NOW + 40 * HOUR));
    assert!(rates.get(Line::Hours3).is_some());
    assert!(rates.get(Line::Hours6).is_some());
    assert_eq!(rates.get(Line::Hours24), None);
    assert!(rates.get(Line::SinceReset).is_some());
}

#[test]
fn a_refill_starts_the_average_since_reset_over() {
    let mut readings = history(10, 40.0, 1.0, NOW + 40 * HOUR);
    // The window refilled 2 hours ago and has burnt 0.5 pt/h since.
    let refill = NOW - 2 * HOUR;
    for reading in readings.iter_mut().filter(|r| r.observed_at_ms >= refill) {
        reading.weekly.left =
            100.0 - 0.5 * ((reading.observed_at_ms - refill) as f64 / HOUR as f64);
        reading.weekly.resets_at_ms = NOW + 160 * HOUR;
    }
    let rates = rates(&readings);
    assert!(close(rates.get(Line::SinceReset).unwrap(), 0.5));
    assert!(close(rates.get(Line::Hours3).unwrap_or(0.5), 0.5));
}

#[test]
fn too_little_history_has_no_rate_at_all() {
    let rates = rates(&history(0, 70.0, 1.0, NOW + 40 * HOUR));
    assert_eq!(rates, Rates::default());
    assert_eq!(super::rates(&[]), Rates::default());
}

#[test]
fn a_rate_that_holds_leaves_the_rest_at_the_reset() {
    // 26 % left, resets in 40 h: 0.2 pt/h leaves 18 %; 0 leaves all.
    assert_eq!(
        project(26.0, 40 * HOUR, NOW, Some(0.2)),
        Outcome::Lasts(18.0)
    );
    assert_eq!(
        project(26.0, 40 * HOUR, NOW, Some(0.0)),
        Outcome::Lasts(26.0)
    );
}

#[test]
fn a_rate_that_runs_out_names_the_time() {
    // 26 % left at 1 pt/h empties in 26 h, before the reset in 40 h.
    assert_eq!(
        project(26.0, 40 * HOUR, NOW, Some(1.0)),
        Outcome::RunsOut(NOW + 26 * HOUR)
    );
    // Exactly empty at the reset still counts as running out.
    assert_eq!(
        project(40.0, 40 * HOUR, NOW, Some(1.0)),
        Outcome::RunsOut(NOW + 40 * HOUR)
    );
}

#[test]
fn no_rate_is_unknown_never_zero() {
    for rate in [None, Some(f64::NAN), Some(-1.0), Some(f64::INFINITY)] {
        assert_eq!(
            project(50.0, 10 * HOUR, NOW, rate),
            Outcome::Unknown,
            "{rate:?}"
        );
    }
}

#[test]
fn a_row_needs_the_even_burn_and_projects_every_line() {
    let readings = history(26, 80.0, 0.75, NOW + 40 * HOUR);
    let standing = standing(&readings);
    let left = standing.latest.unwrap().weekly.left;
    let rows = rows(&[standing], NOW, &|_| None);
    let weekly = rows[0].weekly.unwrap();
    assert!(close(weekly.left, left));
    assert!(close(weekly.need_per_hour, left / 40.0));
    assert_eq!(weekly.resets_at_ms, NOW + 40 * HOUR);
    // 0.75 pt/h from `left` over 40 h, on every line.
    let expected = project(left, 40 * HOUR, NOW, Some(0.75));
    // The tokens line has no spent tokens here: its own tests cover it.
    for line in [Line::Hours3, Line::Hours6, Line::Hours24, Line::SinceReset] {
        let at = Line::ALL.iter().position(|l| *l == line).unwrap();
        assert_eq!(weekly.lines[at], expected, "{line:?}");
    }
}

#[test]
fn the_worst_line_is_the_earliest_empty_time_else_the_least_left() {
    let weekly = |lines| Weekly {
        left: 30.0,
        resets_at_ms: NOW + 40 * HOUR,
        need_per_hour: 0.75,
        lines,
    };
    use Outcome::*;
    assert_eq!(
        weekly([
            Lasts(20.0),
            RunsOut(NOW + 30 * HOUR),
            RunsOut(NOW + 20 * HOUR),
            Unknown,
            Lasts(5.0)
        ])
        .worst(),
        RunsOut(NOW + 20 * HOUR)
    );
    assert_eq!(
        weekly([Lasts(20.0), Unknown, Lasts(5.0), Unknown, Lasts(9.0)]).worst(),
        Lasts(5.0)
    );
    assert_eq!(weekly([Unknown; 5]).worst(), Unknown);
}

#[test]
fn a_stale_or_unsampled_provider_has_a_row_without_numbers_or_no_row() {
    let mut old = history(26, 80.0, 0.75, NOW + 40 * HOUR);
    old.iter_mut().for_each(|r| r.observed_at_ms -= 5 * HOUR);
    // The newest sample is older than the two hours a number is quoted for.
    let stale = standing(&old);
    let unsampled = Standing {
        provider: Provider::named("muse"),
        latest: None,
        burn: None,
        trend: None,
        rates: Rates::default(),
    };
    let rows = rows(&[stale, unsampled], NOW, &|_| None);
    assert_eq!(rows.len(), 1, "a provider never sampled has no row");
    assert_eq!(rows[0].weekly, None, "an old reading is not quoted");
}

fn spent(last_3h: u64, day: u64) -> Option<Spent> {
    Some(Spent { last_3h, day })
}

#[test]
fn tokens_rate_is_the_three_hour_tokens_at_the_points_a_token_cost_over_a_day() {
    // 2.4 points over 24 h = 24 000 tokens: 0.0001 points a token. 3 000 tokens in the
    // last 3 h are 1 000 an hour: 0.1 points an hour.
    let rates = Rates::measured(None, None, Some(0.1), None);
    let rate = rates.tokens(spent(3_000, 24_000)).unwrap();
    assert!(close(rate, 0.1), "{rate}");
    // A quiet last 3 h is a real zero, not a dash.
    assert_eq!(rates.tokens(spent(0, 24_000)), Some(0.0));
    // Missing sides are unknown: no tokens in the day, none reported, no 24 h rate.
    assert_eq!(rates.tokens(spent(0, 0)), None);
    assert_eq!(rates.tokens(None), None);
    assert_eq!(Rates::default().tokens(spent(3_000, 24_000)), None);
}

#[test]
fn the_tokens_line_projects_like_the_others_and_stays_unknown_without_tokens() {
    let readings = history(26, 80.0, 0.75, NOW + 40 * HOUR);
    let mut standing = standing(&readings);
    standing.rates = Rates::measured(Some(0.75), Some(0.75), Some(0.75), Some(0.75));
    let left = standing.latest.unwrap().weekly.left;
    // 0.75 points an hour over 24 h = 18 points for 18 000 tokens: 0.001 points a token. 6 000
    // tokens in 3 h are 2 000 an hour: 2 points an hour.
    let weekly = rows(&[standing.clone()], NOW, &|_| spent(6_000, 18_000))[0]
        .weekly
        .unwrap();
    let at = Line::ALL.iter().position(|l| *l == Line::Tokens).unwrap();
    assert_eq!(weekly.lines[at], project(left, 40 * HOUR, NOW, Some(2.0)));
    let none = rows(&[standing], NOW, &|_| None)[0].weekly.unwrap();
    assert_eq!(none.lines[at], Outcome::Unknown);
}
