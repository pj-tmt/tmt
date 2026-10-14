//! Admission and one detail formatter over Rate's exact interval evidence.
use super::model::{Bucket, Coverage, Selection, Series};
use crate::board::rate::series::{Interval, Series as Observed};

pub(in crate::board) fn display(
    observed: &Observed,
    selection: Selection,
) -> Result<Series, &'static str> {
    let buckets = observed
        .buckets
        .iter()
        .map(|reading| {
            let tokens = reading.tokens.and_then(|tokens| u64::try_from(tokens).ok());
            let coverage = if tokens.is_none() {
                Coverage::Unknown
            } else if reading.complete() {
                Coverage::Complete
            } else if reading.identities == 1 && reading.duration_identities == 1 {
                u64::try_from(reading.source_identity_ms)
                    .ok()
                    .filter(|covered| *covered <= (reading.to_ms - reading.from_ms) as u64)
                    .map_or(Coverage::Unknown, |covered_ms| Coverage::Partial {
                        covered_ms,
                    })
            } else {
                // Different identities' reported durations do not disclose the
                // intersection of their covered clock time. Do not infer it.
                Coverage::Unknown
            };
            Bucket {
                from_ms: reading.from_ms,
                to_ms: reading.to_ms,
                tokens,
                coverage,
            }
        })
        .collect();
    Series::new(selection, buckets)
}

pub(in crate::board) fn range_label(observed: &Observed, minutes: u8) -> String {
    let known = observed
        .buckets
        .iter()
        .any(|bucket| bucket.tokens.is_some());
    let total = observed
        .buckets
        .iter()
        .filter_map(|bucket| bucket.tokens)
        .try_fold(0u128, u128::checked_add);
    let reading = if known {
        total.map_or("unavailable".into(), |total| {
            format!("observed {total} tok")
        })
    } else {
        "unavailable".into()
    };
    let partial = known && observed.buckets.iter().any(|bucket| !bucket.complete());
    format!(
        "−{minutes}m → now · {reading}{}",
        if partial { " · partial" } else { "" }
    )
}

fn timestamp(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms).map_or_else(
        |_| format!("{ms} ms"),
        |time| time.strftime("%Y-%m-%d %H:%M:%S UTC").to_string(),
    )
}

/// Click, keyboard inspection and supported hover use this same evidence.
pub(in crate::board) fn details(reading: &Interval) -> Vec<String> {
    let mut lines = vec![
        format!(
            "Observed tokens: {}{}",
            reading
                .tokens
                .map_or("unavailable".into(), |tokens| tokens.to_string()),
            if reading.tokens.is_some() && !reading.complete() {
                " · partial"
            } else {
                ""
            }
        ),
        format!(
            "Coverage: {} · {}/{} agents with observed counts",
            if reading.complete() {
                "complete"
            } else if reading.observed_identities == 0 {
                "unavailable"
            } else {
                "partial"
            },
            reading.observed_identities,
            reading.identities,
        ),
    ];
    lines.insert(
        1,
        format!(
            "{} – {}",
            timestamp(reading.from_ms),
            timestamp(reading.to_ms)
        ),
    );
    lines.insert(
        2,
        format!("Exact range: [{}, {}) ms", reading.from_ms, reading.to_ms),
    );
    if !reading.scope_complete {
        lines.push("Roster coverage is partial, stale or unavailable".into());
    }
    if reading.duration_identities == 0 {
        lines.push("Coverage duration: unavailable; receipt counts are not a duration".into());
    } else if reading.identities == 1 {
        lines.push(format!(
            "Reported coverage: {} / {} ms",
            reading.source_identity_ms,
            reading.to_ms - reading.from_ms,
        ));
    } else {
        lines.push(format!(
            "Reported coverage: {} agent-ms across {} agents",
            reading.source_identity_ms, reading.duration_identities,
        ));
        if !reading.complete() {
            lines.push("Joint covered time is unavailable".into());
        }
    }
    if reading.boundary_cut {
        lines.push("Partial: source counts crossing an interval boundary are not split".into());
    }
    if reading.overflow
        || reading
            .tokens
            .is_some_and(|tokens| tokens > u128::from(u64::MAX))
    {
        lines.push("Plot unavailable for this total; no value is rounded down".into());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::rate::series::Grid;

    fn observed() -> Observed {
        Observed {
            grid: Grid::new(300_000, 3_600_000, 3_600_000).unwrap(),
            buckets: (0..12)
                .map(|index| Interval {
                    from_ms: index * 300_000,
                    to_ms: (index + 1) * 300_000,
                    tokens: Some(index as u128),
                    identities: 1,
                    observed_identities: 1,
                    complete_identities: 1,
                    source_identity_ms: 300_000,
                    duration_identities: 1,
                    boundary_cut: false,
                    overflow: false,
                    scope_complete: true,
                })
                .collect(),
        }
    }

    #[test]
    fn joint_partial_coverage_never_becomes_an_invented_duration() {
        let mut source = observed();
        source.buckets[0].identities = 2;
        source.buckets[0].source_identity_ms = 100_000;
        let shown = display(&source, Selection::default()).unwrap();
        assert_eq!(shown.buckets[0].tokens, Some(0));
        assert_eq!(shown.buckets[0].coverage, Coverage::Unknown);
        assert!(
            details(&source.buckets[0])
                .iter()
                .any(|line| line == "Joint covered time is unavailable")
        );
        source.buckets[0].identities = 1;
        source.buckets[0].complete_identities = 0;
        assert_eq!(
            display(&source, Selection::default()).unwrap().buckets[0].coverage,
            Coverage::Partial {
                covered_ms: 100_000
            }
        );
    }

    #[test]
    fn huge_exact_total_is_retained_in_details_without_wrapping_the_plot() {
        let mut source = observed();
        let exact = u128::from(u64::MAX) + 1;
        source.buckets[0].tokens = Some(exact);
        let shown = display(&source, Selection::default()).unwrap();
        assert_eq!(shown.buckets[0].tokens, None);
        assert_eq!(shown.buckets[0].coverage, Coverage::Unknown);
        assert!(
            details(&source.buckets[0])
                .iter()
                .any(|line| line.contains(&exact.to_string()))
        );
        assert!(
            display(
                &source,
                Selection {
                    each_minutes: 60,
                    range_minutes: 15
                }
            )
            .is_err()
        );
    }
}
