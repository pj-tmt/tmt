//! Reads provider limits out of one `ls --json` document. Core reports the
//! windows a driver's own files carry on each session's consumption
//! observation; a driver whose sessions report usage but no windows can only
//! show them in a pane's statusline footer, so the listing supplies the panes
//! worth reading. Providers are the driver IDs the listing names.
use super::{Provider, QuotaWindow, Reading};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Panes tried per footer read, freshest first.
const MAX_CANDIDATES: usize = 3;
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

/// Providers with at least one listed session: the driver, or the driver and the account its
/// session's limits name (#5274). A driver with exactly one named account is that account alone:
/// sessions that name none are the same account's and fold into it.
pub(in crate::board) fn present(listed: &Value) -> BTreeSet<Provider> {
    let mut found: BTreeSet<Provider> = BTreeSet::new();
    for row in rows(listed) {
        let Some(plain) = driver(row) else { continue };
        let named = reading(&row["resume"]["consumption"]["rateLimits"])
            .and_then(|(_, account)| account)
            .map(|account| plain.clone().with_account(&account));
        found.insert(named.unwrap_or(plain));
    }
    fold_single_account(found)
}

/// Drops the plain driver where exactly one account of it is present: one account is the whole
/// driver, so the board reads exactly as it did before accounts were named. Two or more accounts
/// keep the plain driver beside them, as the sessions whose account is unknown.
pub(in crate::board) fn fold_single_account(providers: BTreeSet<Provider>) -> BTreeSet<Provider> {
    let accounts = |driver: &str| {
        providers
            .iter()
            .filter(|provider| provider.id() == driver && provider.account_key().is_some())
            .count()
    };
    providers
        .iter()
        .filter(|provider| provider.account_key().is_some() || accounts(provider.id()) != 1)
        .cloned()
        .collect()
}

/// The newest limits each provider's listed sessions report themselves. Two accounts of one
/// driver are two providers; the key carries the label of the newest reading.
pub(in crate::board) fn reported(listed: &Value) -> BTreeMap<Provider, Reading> {
    let mut newest: BTreeMap<Provider, Reading> = BTreeMap::new();
    for row in rows(listed) {
        let (Some(plain), Some((reading, account))) = (
            driver(row),
            reading(&row["resume"]["consumption"]["rateLimits"]),
        ) else {
            continue;
        };
        let provider = match account {
            Some(account) => plain.with_account(&account),
            None => plain,
        };
        match newest.get(&provider) {
            Some(known) if known.observed_at_ms >= reading.observed_at_ms => {}
            _ => {
                // The map keeps the key it first saw, so replace it to carry the newest label.
                newest.remove(&provider);
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
/// of about a week and, when present, the one of about five hours, with the account it names.
fn reading(limits: &Value) -> Option<(Reading, Option<super::observation::Account>)> {
    let report = super::observation::Report::read(limits)?;
    let window_of = |kind: QuotaWindow| report.window(kind).map(|point| point.balance());
    let weekly = window_of(QuotaWindow::Weekly)?;
    let short = window_of(QuotaWindow::Short);
    let reading = Reading {
        observed_at_ms: report.observed_at_ms,
        weekly,
        short,
    };
    reading.valid().then_some((reading, report.account))
}

#[cfg(test)]
mod tests;
