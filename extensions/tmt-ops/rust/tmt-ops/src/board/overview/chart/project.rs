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
    provider_rows(series, None)
}
pub(in crate::board) fn inspected_rows(
    series: &Series,
    reading: &Interval,
) -> crate::board::spark_row::Data {
    provider_rows(series, Some(reading))
}
fn provider_rows(series: &Series, reading: Option<&Interval>) -> crate::board::spark_row::Data {
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
    let selected =
        reading.and_then(|bucket| bucket.tokens.map(|total| (total, bucket.providers.clone())));
    let totals = if reading.is_some() {
        selected.as_ref()
    } else {
        series.totals.as_ref()
    };
    let partial = reading.is_some_and(|bucket| bucket.tokens.is_some() && !bucket.complete());
    let number = |text: String| if partial { format!("~{text}") } else { text };
    let rows = drivers
        .into_iter()
        .map(|driver| {
            let amount = totals.map(|(total, parts)| {
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
                                || total.saturating_sub(bucket.providers.values().sum::<u64>()),
                                |driver| bucket.providers.get(driver).copied().unwrap_or(0),
                            )
                        })
                    })
                    .collect(),
                total: amount.map_or("—".into(), |amount| number(super::format::tokens(amount))),
                share: amount
                    .zip(totals)
                    .map_or("—".into(), |(amount, (total, _))| {
                        number(super::format::share(amount, *total))
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
        axis: super::format::axis(series.selection.range_minutes),
        buckets: series.buckets.len(),
        no_data: series.totals.is_none(),
    }
}

/// The total remains a lower bound whenever any roster/bucket evidence is partial.
pub(in crate::board) fn summary(observed: &Observed, minutes: u32) -> String {
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
    format!(
        "Usage · last {} · {reading} tok",
        super::format::range(minutes)
    )
}
/// The fixed readout uses the exact interval projection, including lower bounds.
pub(in crate::board) fn readout(reading: &Interval) -> String {
    let stamp = |ms, pattern: &str| {
        jiff::Timestamp::from_millisecond(ms)
            .map(|time| time.strftime(pattern).to_string())
            .unwrap_or_else(|_| "—".into())
    };
    let amount = reading.tokens.map_or("—".into(), |n| {
        format!(
            "{}{}",
            if reading.complete() { "" } else { "~" },
            super::format::tokens(n)
        )
    });
    format!(
        "{}–{} · {amount} tok",
        stamp(reading.from_ms, "%b %-d %H:%M"),
        stamp(
            reading.to_ms,
            if reading.from_ms.div_euclid(86_400_000) == reading.to_ms.div_euclid(86_400_000) {
                "%H:%M"
            } else {
                "%b %-d %H:%M"
            }
        )
    )
}

pub(super) fn label(driver: &str) -> String {
    let mut chars = driver.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
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
    fn readout_has_only_span_total_and_honest_partial_or_missing_numbers() {
        let mut source = observed();
        let bucket = &mut source.buckets[0];
        bucket.from_ms = "2026-10-17T14:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        bucket.to_ms = bucket.from_ms + 300_000;
        bucket.tokens = Some(100);
        assert_eq!(readout(bucket), "Oct 17 14:00–14:05 · 100 tok");
        bucket.complete_identities = 0;
        assert_eq!(readout(bucket), "Oct 17 14:00–14:05 · ~100 tok");
        bucket.tokens = None;
        assert_eq!(readout(bucket), "Oct 17 14:00–14:05 · — tok");
        assert_eq!(
            (bucket.from_ms, bucket.to_ms),
            (1_792_245_600_000, 1_792_245_900_000)
        );
        bucket.to_ms = bucket.from_ms + 86_400_000;
        bucket.tokens = Some(0);
        assert_eq!(readout(bucket), "Oct 17 14:00–Oct 18 14:00 · ~0 tok");
    }

    #[test]
    fn inspected_provider_values_use_driver_identity_when_display_labels_collide() {
        let mut source = observed();
        for bucket in &mut source.buckets {
            bucket.tokens = Some(0);
        }
        source.buckets[0].tokens = Some(100);
        source.buckets[0].providers = [("Foo".into(), 30), ("foo".into(), 60)].into();
        let display = display(&source, Selection::default()).unwrap();
        let rows = inspected_rows(&display, &source.buckets[0]);
        assert_eq!(
            rows.rows
                .iter()
                .map(|row| (row.name.as_str(), row.total.as_str(), row.share.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Foo", "30", "30%"),
                ("Foo", "60", "60%"),
                ("unknown", "10", "10%")
            ]
        );
        source.buckets[0].complete_identities = 0;
        let partial = inspected_rows(&display, &source.buckets[0]);
        assert_eq!(partial.rows[1].total, "~60");
        assert_eq!(partial.rows[2].share, "~10%");
        source.buckets[0].tokens = None;
        let missing = inspected_rows(&display, &source.buckets[0]);
        assert!(
            missing
                .rows
                .iter()
                .all(|row| row.total == "—" && row.share == "—")
        );
        assert_eq!(missing.rows.len(), 3);
    }

    #[test]
    fn joint_partial_coverage_never_becomes_an_invented_duration() {
        let mut source = observed();
        source.buckets[0].identities = 2;
        source.buckets[0].source_identity_ms = 100_000;
        let shown = display(&source, Selection::default()).unwrap();
        assert_eq!(shown.buckets[0].tokens, Some(0));
        assert_eq!(shown.buckets[0].coverage, Coverage::Unknown);
        assert_eq!(
            (shown.buckets[0].from_ms, shown.buckets[0].to_ms),
            (0, 300_000)
        );
        assert_eq!(source.buckets[0].source_identity_ms, 100_000);
        assert_eq!(source.buckets[0].identities, 2);
        assert!(!source.buckets[0].complete());
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
    fn huge_exact_total_is_retained_in_projection_without_wrapping_the_plot() {
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
        assert_eq!(source.buckets[0].tokens, Some(exact));
        assert_eq!(source.buckets[0].providers["driver-future"], exact);
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
    fn provider_shares_keep_unknown_in_the_denominator_and_typed_evidence() {
        let mut source = observed();
        for bucket in &mut source.buckets {
            bucket.tokens = Some(0);
        }
        source.buckets[0].tokens = Some(100);
        source.buckets[0].providers = [("claude".into(), 61), ("codex".into(), 30)].into();
        assert_eq!(summary(&source, 60), "Usage · last 1h · 100 tok");
        let rows = spark_rows(&display(&source, Selection::default()).unwrap());
        assert_eq!(
            rows.rows
                .iter()
                .map(|row| (row.name.as_str(), row.total.as_str(), row.share.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Claude", "61", "61%"),
                ("Codex", "30", "30%"),
                ("unknown", "9", "9%")
            ]
        );
        source.buckets[0].providers = [("claude".into(), 60), ("codex".into(), 40)].into();
        let rows = spark_rows(&display(&source, Selection::default()).unwrap());
        assert_eq!(rows.rows.len(), 2);
        assert_eq!(rows.rows[1].share, "40%");
        source.buckets[0].tokens = Some(0);
        source.buckets[0].providers.clear();
        assert_eq!(summary(&source, 60), "Usage · last 1h · 0 tok");
        for bucket in &mut source.buckets {
            bucket.tokens = None;
            bucket.providers.clear();
        }
        assert_eq!(summary(&source, 60), "Usage · last 1h · — tok");
    }
}
