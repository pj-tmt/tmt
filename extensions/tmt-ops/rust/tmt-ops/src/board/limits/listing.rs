//! Reads provider limits out of one `ls --json` document. Core reports the
//! windows a driver's own files carry on each session's consumption
//! observation; a driver whose sessions report usage but no windows can only
//! show them in a pane's statusline footer, so the listing supplies the panes
//! worth reading. Providers are the driver IDs the listing names.
use super::{Provider, Reading, Window};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Panes tried per footer read, freshest first.
const MAX_CANDIDATES: usize = 3;
/// The header labels the windows `7d` and `5h`, so only windows of about those
/// lengths are shown; any other limit is left out rather than mislabelled.
const WEEKLY_MINUTES: std::ops::RangeInclusive<u64> = 6 * 24 * 60..=8 * 24 * 60;
const SHORT_MINUTES: std::ops::RangeInclusive<u64> = 4 * 60..=6 * 60;

/// A listed session whose pane may show the account's limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) struct Candidate {
    pub name: String,
    /// When Core last observed this session's usage; the statusline cannot be
    /// newer than the session's own last activity.
    pub observed_at_ms: u64,
}

fn rows(listed: &Value) -> impl Iterator<Item = &Value> {
    listed["identities"].as_array().into_iter().flatten()
}

fn driver(row: &Value) -> Option<Provider> {
    Provider::from_driver(row["resume"]["driver"].as_str()?)
}

/// Providers with at least one listed session.
pub(in crate::board) fn present(listed: &Value) -> BTreeSet<Provider> {
    rows(listed).filter_map(driver).collect()
}

/// The newest limits each provider's listed sessions report themselves.
pub(in crate::board) fn reported(listed: &Value) -> BTreeMap<Provider, Reading> {
    let mut newest: BTreeMap<Provider, Reading> = BTreeMap::new();
    for row in rows(listed) {
        let (Some(provider), Some(reading)) = (
            driver(row),
            reading(&row["resume"]["consumption"]["rateLimits"]),
        ) else {
            continue;
        };
        match newest.get(&provider) {
            Some(known) if known.observed_at_ms >= reading.observed_at_ms => {}
            _ => {
                newest.insert(provider, reading);
            }
        }
    }
    newest
}

/// Active sessions that report usage, per provider, newest usage first: the
/// panes whose footer may show limits Core does not carry.
pub(in crate::board) fn footer_candidates(listed: &Value) -> BTreeMap<Provider, Vec<Candidate>> {
    let mut found: BTreeMap<Provider, Vec<Candidate>> = BTreeMap::new();
    for row in rows(listed).filter(|row| row["presence"] == "active") {
        let (Some(provider), Some(name), Some(observed_at_ms)) = (
            driver(row),
            row["name"].as_str().filter(|name| !name.is_empty()),
            row["resume"]["usage"]["observedAtMs"]
                .as_u64()
                .filter(|ms| *ms > 0),
        ) else {
            continue;
        };
        found.entry(provider).or_default().push(Candidate {
            name: name.to_owned(),
            observed_at_ms,
        });
    }
    for candidates in found.values_mut() {
        candidates.sort_by(|a, b| {
            b.observed_at_ms
                .cmp(&a.observed_at_ms)
                .then(a.name.cmp(&b.name))
        });
        candidates.truncate(MAX_CANDIDATES);
    }
    found
}

/// The public `rateLimits` document as a reading, in percent left: the window
/// of about a week and, when present, the one of about five hours.
fn reading(limits: &Value) -> Option<Reading> {
    let observed_at_ms = limits["observedAtMs"].as_u64().filter(|ms| *ms > 0)?;
    let windows = limits["windows"]
        .as_array()?
        .iter()
        .map(|window| {
            let minutes = window["windowMinutes"].as_u64().filter(|m| *m > 0)?;
            let used = window["usedPercent"]
                .as_f64()
                .filter(|p| (0.0..=100.0).contains(p))?;
            let resets_at_ms = window["resetsAtMs"].as_u64().filter(|ms| *ms > 0)?;
            Some((
                minutes,
                Window {
                    left: 100.0 - used,
                    resets_at_ms,
                },
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    let window_of = |range: &std::ops::RangeInclusive<u64>| {
        windows
            .iter()
            .find(|(minutes, _)| range.contains(minutes))
            .map(|(_, window)| *window)
    };
    let weekly = window_of(&WEEKLY_MINUTES)?;
    let short = window_of(&SHORT_MINUTES);
    let reading = Reading {
        observed_at_ms,
        weekly,
        short,
    };
    reading.valid().then_some(reading)
}

#[cfg(test)]
mod tests;
