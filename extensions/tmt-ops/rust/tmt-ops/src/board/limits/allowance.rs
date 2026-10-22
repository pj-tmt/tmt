//! The numbers of the Digest Allowance block: how fast a weekly window empties
//! over several spans, what each rate leaves at the reset, and the even burn
//! that would last exactly until it. Pure; the clock is a parameter, so painting
//! runs no timer and reads nothing.
use super::{Provider, Reading, Standing, estimate};

const HOUR_MS: u64 = 3_600_000;
/// Samples spanning less than this say nothing about a rate.
const MIN_SPAN_MS: u64 = HOUR_MS;
/// A reset time later than this starts a new cycle; less is rounding in the
/// source's own countdown.
const NEW_CYCLE_MS: u64 = 30 * 60_000;

/// One projection line of the block: the rate it holds to the reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Hours3,
    Hours6,
    Hours24,
    /// The average since the window last refilled.
    SinceReset,
}

impl Line {
    pub const ALL: [Self; 4] = [Self::Hours3, Self::Hours6, Self::Hours24, Self::SinceReset];

    pub fn caption(self) -> &'static str {
        match self {
            Self::Hours3 => "3h",
            Self::Hours6 => "6h",
            Self::Hours24 => "1d",
            Self::SinceReset => "SINCE RESET",
        }
    }
}

/// Percent of the weekly window used per hour, for the lines read off the
/// sampled history. `None` where the history does not cover the span.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rates {
    hours3: Option<f64>,
    hours6: Option<f64>,
    hours24: Option<f64>,
    since_reset: Option<f64>,
}

impl Rates {
    /// Rates for a test that needs no history behind them.
    #[cfg(test)]
    pub(in crate::board) fn measured(
        hours3: Option<f64>,
        hours6: Option<f64>,
        hours24: Option<f64>,
        since_reset: Option<f64>,
    ) -> Self {
        Self {
            hours3,
            hours6,
            hours24,
            since_reset,
        }
    }

    pub fn get(&self, line: Line) -> Option<f64> {
        match line {
            Line::Hours3 => self.hours3,
            Line::Hours6 => self.hours6,
            Line::Hours24 => self.hours24,
            Line::SinceReset => self.since_reset,
        }
    }
}

/// The readings of the window's current cycle: back from the newest until the
/// window refilled or a new cycle began.
fn cycle(history: &[Reading]) -> &[Reading] {
    let Some(_) = history.last() else {
        return history;
    };
    let mut start = history.len() - 1;
    for (index, pair) in history.windows(2).enumerate().rev() {
        let (older, newer) = (&pair[0], &pair[1]);
        let refilled = older.weekly.left < newer.weekly.left;
        let next_cycle = newer.weekly.resets_at_ms > older.weekly.resets_at_ms + NEW_CYCLE_MS;
        if refilled || next_cycle {
            break;
        }
        start = index;
    }
    &history[start..]
}

/// Endpoint rate of `readings` when they span at least `minimum` milliseconds.
fn rate(readings: &[Reading], minimum: u64) -> Option<f64> {
    let (first, newest) = (readings.first()?, readings.last()?);
    let span = newest.observed_at_ms.checked_sub(first.observed_at_ms)?;
    (span >= minimum)
        .then(|| (first.weekly.left - newest.weekly.left).max(0.0) / (span as f64 / HOUR_MS as f64))
}

/// The measured rates of one provider's sampled history.
pub(super) fn rates(history: &[Reading]) -> Rates {
    let cycle = cycle(history);
    let Some(newest) = cycle.last() else {
        return Rates::default();
    };
    // A span line needs the history to cover at least half of it, and never less
    // than an hour, so a few close samples are not read as a rate.
    let over = |span: u64| {
        let from = cycle
            .iter()
            .position(|reading| newest.observed_at_ms - reading.observed_at_ms <= span)?;
        rate(&cycle[from..], MIN_SPAN_MS.max(span / 2))
    };
    Rates {
        hours3: over(3 * HOUR_MS),
        hours6: over(6 * HOUR_MS),
        hours24: over(24 * HOUR_MS),
        since_reset: rate(cycle, MIN_SPAN_MS),
    }
}

/// What one line leaves when its rate holds to the reset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// No rate to hold, or no window to hold it against.
    Unknown,
    /// Percent still left at the reset.
    Lasts(f64),
    /// The window empties first, at this time.
    RunsOut(u64),
}

impl Outcome {
    /// Orders outcomes worst first: the earliest empty time, then the least left.
    fn severity(self) -> Option<(u8, f64)> {
        match self {
            Self::Unknown => None,
            Self::RunsOut(at) => Some((0, at as f64)),
            Self::Lasts(left) => Some((1, left)),
        }
    }
}

/// `left` percent that refills in `resets_in_ms`, spent at `rate` percent per
/// hour from `now_ms`.
pub fn project(left: f64, resets_in_ms: u64, now_ms: u64, rate: Option<f64>) -> Outcome {
    let Some(rate) = rate.filter(|rate| rate.is_finite() && *rate >= 0.0) else {
        return Outcome::Unknown;
    };
    let remaining = left - rate * (resets_in_ms as f64 / HOUR_MS as f64);
    if rate > 0.0 && remaining <= 0.0 {
        Outcome::RunsOut(now_ms.saturating_add((left / rate * HOUR_MS as f64) as u64))
    } else {
        Outcome::Lasts(remaining.max(0.0))
    }
}

/// The even-burn figure beyond which the display only says "more than this".
pub const NEED_CAP: f64 = 99.0;

/// What the NEED %/h cell shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Need {
    /// Inside the last hour before the reset the figure only grows without meaning.
    Hidden,
    /// More than [`NEED_CAP`] percent an hour.
    Over,
    Rate(f64),
}

/// A weekly window as of now, with its projection lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weekly {
    pub left: f64,
    pub resets_at_ms: u64,
    /// Percent per hour that lasts exactly until the reset.
    pub need_per_hour: f64,
    pub lines: [Outcome; 4],
}

impl Weekly {
    /// The NEED %/h display at `now_ms`: hidden in the last hour before the reset, capped
    /// above [`NEED_CAP`].
    pub fn need(&self, now_ms: u64) -> Need {
        if self.resets_at_ms.saturating_sub(now_ms) < HOUR_MS {
            Need::Hidden
        } else if self.need_per_hour > NEED_CAP {
            Need::Over
        } else {
            Need::Rate(self.need_per_hour)
        }
    }

    /// The worst of the lines, for a row with no room for all five.
    pub fn worst(&self) -> Outcome {
        self.lines
            .iter()
            .copied()
            .filter_map(|outcome| outcome.severity().map(|key| (key, outcome)))
            .min_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map_or(Outcome::Unknown, |(_, outcome)| outcome)
    }
}

/// One source of the block. `weekly` is `None` while no usable reading stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub provider: Provider,
    pub weekly: Option<Weekly>,
}

/// The rows of `standings`: every provider that has been sampled, in the shown
/// order.
pub fn rows(standings: &[Standing], now_ms: u64) -> Vec<Row> {
    standings
        .iter()
        .filter(|standing| standing.latest.is_some())
        .map(|standing| Row {
            provider: standing.provider.clone(),
            weekly: weekly(standing, now_ms),
        })
        .collect()
}

fn weekly(standing: &Standing, now_ms: u64) -> Option<Weekly> {
    let estimate::Outlook::Known(figures) = standing.outlook(now_ms) else {
        return None;
    };
    let left = figures.weekly;
    let hours = left.resets_in_ms as f64 / HOUR_MS as f64;
    let lines = Line::ALL.map(|line| {
        project(
            left.percent,
            left.resets_in_ms,
            now_ms,
            standing.rates.get(line),
        )
    });
    Some(Weekly {
        left: left.percent,
        resets_at_ms: now_ms.saturating_add(left.resets_in_ms),
        need_per_hour: left.percent / hours,
        lines,
    })
}

#[cfg(test)]
mod tests;
