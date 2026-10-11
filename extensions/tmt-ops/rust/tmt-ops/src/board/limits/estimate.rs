//! What a provider's sampled history says now: the time to reset, the burn rate
//! of the weekly window and whether it empties before it refills. Pure; the
//! clock is a parameter.
use super::{Reading, Window};

const HOUR_MS: u64 = 3_600_000;
/// A weekly window predicted to empty within this interval needs intervention.
const URGENT_EMPTY_MS: u64 = 12 * HOUR_MS;
/// How far back from the newest sample the burn is averaged.
const BURN_SPAN_MS: u64 = 6 * HOUR_MS;
/// Samples spanning less than this say nothing about a rate.
const MIN_SPAN_MS: u64 = HOUR_MS;
const MIN_TREND_SPAN_MS: u64 = 5 * HOUR_MS;
const RECENT_SPAN_MS: u64 = 2 * HOUR_MS;
/// Precision is not retained; assume whole percent.
const READING_STEP_PERCENT: f64 = 1.0;
/// A reading older than this is no longer shown as a number.
const STALE_MS: u64 = 2 * HOUR_MS;
/// A reset time later than this starts a new cycle; less is rounding in the
/// source's own countdown.
const NEW_CYCLE_MS: u64 = 30 * 60_000;

/// The one refill/reset/time fence shared by the displayed pace and its trend.
fn current_cycle(history: &[Reading]) -> &[Reading] {
    let Some(newest) = history.last() else {
        return history;
    };
    let mut start = history.len() - 1;
    for (index, pair) in history.windows(2).enumerate().rev() {
        let (older, newer) = (&pair[0], &pair[1]);
        let refilled = older.weekly.left < newer.weekly.left;
        let next_cycle = newer.weekly.resets_at_ms > older.weekly.resets_at_ms + NEW_CYCLE_MS;
        if refilled || next_cycle || newest.observed_at_ms - older.observed_at_ms > BURN_SPAN_MS {
            break;
        }
        start = index;
    }
    &history[start..]
}

fn rate(history: &[Reading], minimum_span: u64) -> Option<(f64, f64)> {
    let (first, newest) = (history.first()?, history.last()?);
    let span = newest.observed_at_ms - first.observed_at_ms;
    let hours = span as f64 / HOUR_MS as f64;
    (span >= minimum_span).then(|| {
        (
            (first.weekly.left - newest.weekly.left).max(0.0) / hours,
            hours,
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    Up,
    Same,
    Down,
}

fn compare_rates(baseline: f64, recent: f64, recent_hours: f64) -> Trend {
    let floor = 0.1_f64
        .max(READING_STEP_PERCENT / recent_hours)
        .max(0.2 * baseline);
    // The same strict delta threshold, arranged without cancellation when an
    // admitted fractional-hour span lands exactly on one reading step.
    if recent > baseline + floor {
        Trend::Up
    } else if baseline > recent + floor {
        Trend::Down
    } else {
        Trend::Same
    }
}

/// Endpoint pace over the actual elapsed time, including gaps. The recent
/// 1–2-hour span is inside the 5–6-hour baseline on purpose: the arrow compares
/// recent pace with the pace displayed beside it, not with a disjoint window.
pub(super) fn pace(history: &[Reading]) -> (Option<f64>, Option<Trend>) {
    let cycle = current_cycle(history);
    let burn = rate(cycle, MIN_SPAN_MS).map(|(rate, _)| rate);
    let trend = (|| {
        let (baseline, _) = rate(cycle, MIN_TREND_SPAN_MS)?;
        if !baseline.is_finite() || baseline <= 0.0 {
            return None;
        }
        let newest = cycle.last()?;
        let start = cycle
            .iter()
            .position(|reading| newest.observed_at_ms - reading.observed_at_ms <= RECENT_SPAN_MS)?;
        let (recent, hours) = rate(&cycle[start..], MIN_SPAN_MS)?;
        recent
            .is_finite()
            .then(|| compare_rates(baseline, recent, hours))
    })();
    (burn, trend)
}

/// A window as of `now`: the share left and how long until it refills.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Left {
    pub percent: f64,
    pub resets_in_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Figures {
    pub weekly: Left,
    pub short: Option<Left>,
    /// Percent of the weekly window per hour; `None` before enough samples.
    pub burn: Option<f64>,
    pub trend: Option<Trend>,
    /// The burn empties the weekly window before it resets.
    pub runs_out_before_reset: bool,
}

impl Figures {
    /// No countdown is inferred without a finite positive measured rate.
    pub fn empties_in_ms(&self) -> Option<u64> {
        time_to_empty(self.weekly, self.burn)
    }

    /// Depletion is urgent only before reset, at or within the 12-hour boundary.
    pub fn empties_soon(&self) -> bool {
        self.runs_out_before_reset && self.empties_in_ms().is_some_and(|ms| ms <= URGENT_EMPTY_MS)
    }
}

fn time_to_empty(weekly: Left, burn: Option<f64>) -> Option<u64> {
    let rate = burn.filter(|rate| rate.is_finite() && *rate > 0.0)?;
    Some((weekly.percent / rate * HOUR_MS as f64) as u64)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outlook {
    /// No usable reading: never sampled, or the last one is too old to quote.
    Unknown {
        age_ms: Option<u64>,
    },
    Known(Figures),
}

/// The newest sample as of `now_ms`. An old number is never quoted: past
/// `STALE_MS`, or once its window has already reset, only its age remains.
pub fn outlook(
    latest: Option<&Reading>,
    burn: Option<f64>,
    trend: Option<Trend>,
    now_ms: u64,
) -> Outlook {
    let Some(reading) = latest else {
        return Outlook::Unknown { age_ms: None };
    };
    let age_ms = now_ms.saturating_sub(reading.observed_at_ms);
    let left = |window: &Window| {
        window
            .resets_at_ms
            .checked_sub(now_ms)
            .filter(|ms| *ms > 0)
            .map(|resets_in_ms| Left {
                percent: window.left,
                resets_in_ms,
            })
    };
    let weekly = left(&reading.weekly);
    let Some(weekly) = weekly.filter(|_| age_ms <= STALE_MS) else {
        return Outlook::Unknown {
            age_ms: Some(age_ms),
        };
    };
    let runs_out_before_reset =
        time_to_empty(weekly, burn).is_some_and(|ms| ms < weekly.resets_in_ms);
    Outlook::Known(Figures {
        weekly,
        short: reading.short.as_ref().and_then(left),
        burn,
        trend: trend.filter(|_| burn.is_some_and(|rate| rate.is_finite() && rate > 0.0)),
        runs_out_before_reset,
    })
}

#[cfg(test)]
mod tests;
