//! Account pace over fixed two-hour horizons, independent of chart aggregation.
use super::{
    QuotaWindow, Trend,
    observation::{Point, Report},
};

pub(in crate::board) const HORIZON_MS: u64 = 2 * 3_600_000;
const MIN_SPACING_MS: u64 = 30 * 60_000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::board) struct Pace {
    pub latest: Option<Point>,
    pub per_hour: Option<f64>,
    pub trend: Option<Trend>,
    pub exhausts_at_ms: Option<u64>,
}
impl Pace {
    pub fn early_ms(self) -> Option<u64> {
        let reset = self.latest?.resets_at_ms;
        let exhausted = self.exhausts_at_ms?;
        reset.checked_sub(exhausted).filter(|ms| *ms > 0)
    }
}

/// All admitted points are retained until this projection: intermediate drops,
/// resets and conflicts must not disappear behind endpoint selection or dedupe.
pub(in crate::board) fn project<'a>(
    reports: impl IntoIterator<Item = &'a Report>,
    kind: QuotaWindow,
    now: u64,
) -> Pace {
    let reports: Vec<_> = reports
        .into_iter()
        .filter(|report| report.observed_at_ms <= now)
        .collect();
    let Some(newest_at) = reports.iter().map(|report| report.observed_at_ms).max() else {
        return Pace::default();
    };
    if reports
        .iter()
        .filter(|report| report.observed_at_ms == newest_at)
        .any(|report| report.window(kind).is_none())
    {
        return Pace::default();
    }
    let mut points: Vec<_> = reports
        .into_iter()
        .filter_map(|report| report.window(kind))
        .collect();
    points.sort_by_key(|point| point.observed_at_ms);
    points.dedup(); // Repeated account evidence from several seats is not summed.
    let mut pace = Pace::default();
    let Some(latest) = points.last().copied().filter(|point| {
        now.saturating_sub(point.observed_at_ms) <= HORIZON_MS && point.resets_at_ms > now
    }) else {
        return pace;
    };
    // A conflicting newest timestamp cannot supply even a truthful balance.
    if points
        .iter()
        .rev()
        .take_while(|point| point.observed_at_ms == latest.observed_at_ms)
        .count()
        != 1
    {
        return pace;
    }
    pace.latest = Some(latest);
    let Some(rate) = horizon(&points, now) else {
        return pace;
    };
    pace.per_hour = Some(rate);
    if let Some(before) = horizon(&points, now.saturating_sub(HORIZON_MS)) {
        pace.trend = Some(
            if (rate - before).abs() <= f64::EPSILON * 64.0 * rate.abs().max(before.abs()).max(1.0)
            {
                Trend::Same
            } else if rate > before {
                Trend::Up
            } else if rate < before {
                Trend::Down
            } else {
                Trend::Same
            },
        );
    }
    if rate > 0.0 {
        let remaining_ms = (100.0 - latest.used_percent) / rate * 3_600_000.0;
        if remaining_ms.is_finite() && remaining_ms >= 0.0 && remaining_ms < u64::MAX as f64 {
            pace.exhausts_at_ms = latest
                .observed_at_ms
                .checked_add(remaining_ms.ceil() as u64);
        }
    }
    pace
}

fn horizon(points: &[Point], through: u64) -> Option<f64> {
    let start = through.saturating_sub(HORIZON_MS);
    let mut first: Option<Point> = None;
    let mut last: Option<Point> = None;
    let mut blocked_at = None;
    for &point in points
        .iter()
        .filter(|p| (start..=through).contains(&p.observed_at_ms))
    {
        if blocked_at == Some(point.observed_at_ms) {
            continue;
        }
        if let Some(previous) = last {
            if point.observed_at_ms == previous.observed_at_ms {
                // Same-time conflicts are an unknown boundary, never a chosen seat.
                first = None;
                last = None;
                blocked_at = Some(point.observed_at_ms);
                continue;
            }
            if point.window_minutes != previous.window_minutes
                || point.resets_at_ms != previous.resets_at_ms
                || point.observed_at_ms >= previous.resets_at_ms
                || point.used_percent < previous.used_percent
            {
                first = None;
            }
        }
        first.get_or_insert(point);
        last = Some(point);
    }
    let (first, last) = (first?, last?);
    let elapsed = last.observed_at_ms.checked_sub(first.observed_at_ms)?;
    if elapsed < MIN_SPACING_MS || last.resets_at_ms <= through {
        return None;
    }
    let rate = (last.used_percent - first.used_percent) * 3_600_000.0 / elapsed as f64;
    rate.is_finite().then_some(rate)
}

#[cfg(test)]
mod tests;
