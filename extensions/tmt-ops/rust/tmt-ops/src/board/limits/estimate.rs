//! What a provider's sampled history says now: the time to reset, the burn rate
//! of the weekly window and whether it empties before it refills. Pure; the
//! clock is a parameter.
use super::{Reading, Window};

const HOUR_MS: u64 = 3_600_000;
/// How far back from the newest sample the burn is averaged.
const BURN_SPAN_MS: u64 = 6 * HOUR_MS;
/// Samples spanning less than this say nothing about a rate.
const MIN_SPAN_MS: u64 = HOUR_MS;
/// A reading older than this is no longer shown as a number.
const STALE_MS: u64 = 2 * HOUR_MS;
/// A reset time later than this starts a new cycle; less is rounding in the
/// source's own countdown.
const NEW_CYCLE_MS: u64 = 30 * 60_000;

/// Percent of the weekly window used per hour, averaged over the last
/// `BURN_SPAN_MS` of the current cycle. `history` is ascending by time. `None`
/// until the cycle's samples span `MIN_SPAN_MS`.
pub(super) fn burn_per_hour(history: &[Reading]) -> Option<f64> {
    let newest = history.last()?;
    let mut first = newest;
    for pair in history.windows(2).rev() {
        let (older, newer) = (&pair[0], &pair[1]);
        let refilled = older.weekly.left < newer.weekly.left;
        let next_cycle = newer.weekly.resets_at_ms > older.weekly.resets_at_ms + NEW_CYCLE_MS;
        if refilled || next_cycle || newest.observed_at_ms - older.observed_at_ms > BURN_SPAN_MS {
            break;
        }
        first = older;
    }
    let span = newest.observed_at_ms - first.observed_at_ms;
    (span >= MIN_SPAN_MS)
        .then(|| (first.weekly.left - newest.weekly.left).max(0.0) / (span as f64 / HOUR_MS as f64))
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
    /// The burn empties the weekly window before it resets.
    pub runs_out_before_reset: bool,
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
pub fn outlook(latest: Option<&Reading>, burn: Option<f64>, now_ms: u64) -> Outlook {
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
    let runs_out_before_reset = burn.is_some_and(|rate| {
        rate > 0.0 && weekly.percent / rate * (HOUR_MS as f64) < weekly.resets_in_ms as f64
    });
    Outlook::Known(Figures {
        weekly,
        short: reading.short.as_ref().and_then(left),
        burn,
        runs_out_before_reset,
    })
}

#[cfg(test)]
mod tests;
