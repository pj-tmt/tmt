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
                providers: if tokens.is_some() {
                    reading
                        .providers
                        .iter()
                        .map(|(driver, n)| {
                            (
                                driver.clone(),
                                u64::try_from(*n).expect("provider is part of admitted total"),
                            )
                        })
                        .collect()
                } else {
                    Default::default()
                },
                coverage,
            }
        })
        .collect();
    let mut display = Series::new(selection, buckets)?;
    display.drivers.extend(observed.drivers.iter().cloned());
    let mut total = None;
    let mut providers = std::collections::BTreeMap::new();
    for reading in &observed.buckets {
        if let Some(tokens) = reading.tokens {
            *total.get_or_insert(0u128) += tokens;
        }
        for (driver, tokens) in &reading.providers {
            *providers.entry(driver.clone()).or_insert(0u128) += *tokens;
        }
    }
    display.drivers.extend(providers.keys().cloned());
    display.totals = total.map(|total| (total, providers));
    Ok(display)
}

/// One adapter into shared SparkRow; neither the component nor another tab re-derives
/// provenance. Future consumers pass their own rows and captions to the same module.
pub(in crate::board) fn spark_rows(series: &Series) -> crate::board::spark_row::Data {
    use crate::board::spark_row::{Data, Row};
    let mut drivers: Vec<_> = series
        .drivers
        .iter()
        .map(|driver| Some(driver.as_str()))
        .collect();
    if series
        .totals
        .as_ref()
        .is_some_and(|(total, parts)| *total > parts.values().sum())
    {
        drivers.push(None);
    }
    let rows = drivers
        .into_iter()
        .map(|driver| {
            let amount = series.totals.as_ref().map(|(total, parts)| {
                driver.map_or_else(
                    || *total - parts.values().sum::<u128>(),
                    |driver| parts.get(driver).copied().unwrap_or(0),
                )
            });
            Row {
                name: driver.map_or_else(|| "unknown".into(), label),
                values: series
                    .buckets
                    .iter()
                    .map(|bucket| {
                        bucket.tokens.map(|total| {
                            driver.map_or_else(
                                || total - bucket.providers.values().sum::<u64>(),
                                |driver| bucket.providers.get(driver).copied().unwrap_or(0),
                            )
                        })
                    })
                    .collect(),
                total: amount.map_or("—".into(), super::format::tokens),
                share: amount
                    .zip(series.totals.as_ref())
                    .map_or("—".into(), |(amount, (total, _))| {
                        super::format::share(amount, *total)
                    }),
                muted: driver.is_none(),
                quota: driver.is_some(),
            }
        })
        .collect();
    Data {
        rows,
        name_width: 8,
        share_caption: "% of all".into(),
        show_axis: true,
        range_minutes: u64::from(series.selection.range_minutes),
        buckets: series.buckets.len(),
        no_data: series.totals.is_none(),
    }
}

/// The total remains a lower bound whenever any roster/bucket evidence is partial.
pub(in crate::board) fn summary(observed: &Observed, minutes: u8) -> String {
    let total = observed
        .buckets
        .iter()
        .filter_map(|bucket| bucket.tokens)
        .try_fold(0u128, u128::checked_add);
    let reading = total
        .filter(|_| {
            observed
                .buckets
                .iter()
                .any(|bucket| bucket.tokens.is_some())
        })
        .map_or("—".into(), |total| {
            format!(
                "{}{}",
                if observed.buckets.iter().any(|bucket| !bucket.complete()) {
                    "~"
                } else {
                    ""
                },
                super::format::tokens(total)
            )
        });
    format!("Usage · last {minutes}m · {reading} tok")
}
fn legend(total: u128, providers: &std::collections::BTreeMap<String, u128>) -> Vec<String> {
    let mut rows: Vec<_> = providers
        .iter()
        .map(|(driver, amount)| {
            format!(
                "■ {} {}",
                label(driver),
                super::format::share(*amount, total)
            )
        })
        .collect();
    let unknown = total.saturating_sub(providers.values().sum());
    if unknown > 0 {
        rows.push(format!(
            "░ unknown {}",
            super::format::share(unknown, total)
        ));
    }
    rows
}
pub(super) fn label(driver: &str) -> String {
    let mut chars = driver.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
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
    if let Some(total) = reading.tokens {
        lines.push(
            legend(total, &reading.providers)
                .into_iter()
                .collect::<Vec<_>>()
                .join(" · "),
        );
    }
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
            drivers: Default::default(),
            grid: Grid::new(300_000, 3_600_000, 3_600_000).unwrap(),
            buckets: (0..12)
                .map(|index| Interval {
                    from_ms: index * 300_000,
                    to_ms: (index + 1) * 300_000,
                    tokens: Some(index as u128),
                    providers: Default::default(),
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
        source.buckets[0].providers = [("driver-future".into(), exact)].into();
        let shown = display(&source, Selection::default()).unwrap();
        assert_eq!(shown.buckets[0].tokens, None);
        assert_eq!(shown.buckets[0].coverage, Coverage::Unknown);
        assert_eq!(shown.totals.as_ref().unwrap().0, exact + 66);
        assert_eq!(shown.totals.as_ref().unwrap().1["driver-future"], exact);
        assert!(shown.drivers.contains("driver-future"));
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
    #[test]
    fn provider_shares_keep_unknown_in_the_denominator_and_exact_details() {
        let mut source = observed();
        for bucket in &mut source.buckets {
            bucket.tokens = Some(0);
        }
        source.buckets[0].tokens = Some(100);
        source.buckets[0].providers = [("claude".into(), 61), ("codex".into(), 30)].into();
        assert_eq!(summary(&source, 60), "Usage · last 60m · 100 tok");
        assert!(
            details(&source.buckets[0])
                .iter()
                .any(|line| line == "■ Claude 61% · ■ Codex 30% · ░ unknown 9%")
        );
        source.buckets[0].providers = [("claude".into(), 60), ("codex".into(), 40)].into();
        assert_eq!(legend(100, &source.buckets[0].providers).len(), 2);
        source.buckets[0].tokens = Some(0);
        source.buckets[0].providers.clear();
        assert_eq!(summary(&source, 60), "Usage · last 60m · 0 tok");
        for bucket in &mut source.buckets {
            bucket.tokens = None;
            bucket.providers.clear();
        }
        assert_eq!(summary(&source, 60), "Usage · last 60m · — tok");
    }
}
