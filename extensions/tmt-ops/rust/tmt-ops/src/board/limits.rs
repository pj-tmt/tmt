//! Provider usage limits for the HOME header. An account reports a weekly
//! window and sometimes a short one; the board keeps a bounded history of those
//! readings and estimates the burn rate from it. Sampling is the refresh
//! worker's job and runs only while the board is open: a reading is only as
//! recent as its source, and the header says how old it is. This module takes
//! the clock and the pane capture as parameters; painting lives with the HOME
//! header. Providers are whatever drivers the listing names; nothing here
//! spells a driver.
mod allowance;
mod estimate;
mod footer;
pub(super) mod history;
mod listing;
pub(in crate::board) mod observation;
pub(in crate::board) mod pace;
mod store;

pub use allowance::{Line, NEED_CAP, Need, Outcome, Rates, Row, Spent, Weekly, rows};
pub use estimate::{Figures, Left, Outlook, Trend, outlook};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use store::Store;

/// An account whose limits the board shows, identified by its driver ID as
/// Core's listing reports it and, when Core names one, the provider account
/// (`rateLimits.account.key`, #5274). Two accounts of one driver are two
/// providers; a driver with no named account is the plain driver, exactly as
/// before. Identity is the driver ID and the account key alone: the label is
/// display text, so a changed label keeps the same provider.
#[derive(Debug, Clone)]
pub struct Provider {
    driver: String,
    account: Option<ProviderAccount>,
    /// The plain driver beside two or more named accounts: its sessions report no account, so it
    /// is shown as `unknown account` rather than reading as the total (#5275).
    unknown_account: bool,
}

/// The account part of a [`Provider`]: the grouping key, the last label Core
/// reported for it, and whether the board shows the label (only when the driver
/// has more than one account).
#[derive(Debug, Clone)]
struct ProviderAccount {
    key: String,
    label: String,
    shown: bool,
}

impl PartialEq for Provider {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Provider {}
impl PartialOrd for Provider {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Provider {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.driver.cmp(&other.driver).then_with(|| {
            self.account
                .as_ref()
                .map(|account| &account.key)
                .cmp(&other.account.as_ref().map(|account| &account.key))
        })
    }
}

/// One shared quota choice, independent of the token chart's aggregation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::board) enum QuotaWindow {
    #[default]
    Weekly,
    Short,
}

impl QuotaWindow {
    pub const ALL: [Self; 2] = [Self::Weekly, Self::Short];

    pub fn key(self) -> &'static str {
        match self {
            Self::Weekly => "7d",
            Self::Short => "5h",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|window| window.key() == value)
    }

    /// Match the same provider window lengths used by the held limits source.
    pub fn accepts(self, minutes: u64) -> bool {
        match self {
            Self::Weekly => (6 * 24 * 60..=8 * 24 * 60).contains(&minutes),
            Self::Short => (4 * 60..=6 * 60).contains(&minutes),
        }
    }
}

impl Provider {
    /// The driver ID from a listing row, if it is a plain identifier. The ID
    /// becomes a cache key, so anything else is not accepted.
    pub(super) fn from_driver(id: &str) -> Option<Self> {
        let plain = !id.is_empty()
            && id.len() <= 24
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        plain.then(|| Self {
            driver: id.to_owned(),
            account: None,
            unknown_account: false,
        })
    }

    /// The same driver for the account Core named.
    pub(super) fn with_account(mut self, account: &observation::Account) -> Self {
        self.account = Some(ProviderAccount {
            key: account.key.clone(),
            label: account.label.clone(),
            shown: false,
        });
        self
    }

    /// The driver ID, shared by all of its accounts.
    pub(super) fn id(&self) -> &str {
        &self.driver
    }

    /// What names this provider where only a string fits (the cache file, a chart
    /// row): the driver ID, or the driver ID and the account key.
    pub(super) fn key(&self) -> String {
        match &self.account {
            None => self.driver.clone(),
            Some(account) => format!("{}.{}", self.driver, account.key),
        }
    }

    /// What names this provider's row on the board: `driver.key` when the driver has several
    /// accounts, the driver ID otherwise (one account is the whole driver).
    pub(super) fn board_name(&self) -> String {
        match &self.account {
            Some(account) if account.shown => self.key(),
            _ => self.driver.clone(),
        }
    }

    /// The provider a [`Provider::key`] names, if it is one.
    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key.split_once('.') {
            None => Self::from_driver(key),
            Some((driver, account)) => {
                let named = observation::Account::read(&serde_json::json!({
                    "key": account, "label": "unnamed"
                }))?;
                Some(Self::from_driver(driver)?.with_account(&named))
            }
        }
    }

    /// The driver alone, whichever account it is.
    pub(super) fn plain(&self) -> Self {
        Self {
            driver: self.driver.clone(),
            account: None,
            unknown_account: false,
        }
    }

    /// Whether the shown name carries an account part (the driver has several accounts).
    pub(super) fn names_account(&self) -> bool {
        self.account.as_ref().is_some_and(|account| account.shown) || self.unknown_account
    }

    /// The account key, when Core named one.
    pub(super) fn account_key(&self) -> Option<&str> {
        self.account.as_ref().map(|account| account.key.as_str())
    }

    /// The account label Core last reported.
    pub(super) fn account_label(&self) -> Option<&str> {
        self.account.as_ref().map(|account| account.label.as_str())
    }

    /// Shows the account label in [`Provider::label`]. Set only when the driver has more than
    /// one account; a single account reads exactly like the plain driver.
    pub(super) fn showing_account(mut self, shown: bool) -> Self {
        if let Some(account) = self.account.as_mut() {
            account.shown = shown;
        }
        self
    }

    /// Marks the plain driver as the sessions that name no account.
    pub(super) fn showing_unknown_account(mut self, shown: bool) -> Self {
        if self.account.is_none() {
            self.unknown_account = shown;
        }
        self
    }

    /// Replaces the account label with the one Core reported most recently.
    pub(super) fn labelled(mut self, label: &str) -> Self {
        if let Some(account) = self.account.as_mut() {
            account.label = label.to_owned();
        }
        self
    }

    /// The shown name: the driver ID with a capital, and the account label after it when the
    /// driver has more than one account.
    pub(super) fn label(&self) -> String {
        let mut letters = self.driver.chars();
        let name: String = letters
            .next()
            .map(|first| first.to_uppercase().chain(letters).collect())
            .unwrap_or_default();
        match &self.account {
            Some(account) if account.shown => format!("{name} · {}", account.label),
            None if self.unknown_account => format!("{name} · unknown account"),
            _ => name,
        }
    }

    #[cfg(test)]
    pub(super) fn named(id: &str) -> Self {
        Self::from_driver(id).expect("a plain driver ID")
    }
}

/// One limit window as an account reported it: the share still available and
/// when it refills.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    /// Percent left, 0 through 100.
    pub left: f64,
    pub resets_at_ms: u64,
}

impl Window {
    fn valid(&self) -> bool {
        (0.0..=100.0).contains(&self.left) && self.resets_at_ms > 0
    }
}

/// What one account showed at `observed_at_ms`, the time its source last
/// refreshed it, never the time the board read it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub observed_at_ms: u64,
    pub weekly: Window,
    pub short: Option<Window>,
}

impl Reading {
    fn valid(&self) -> bool {
        self.observed_at_ms > 0 && self.weekly.valid() && self.short.is_none_or(|w| w.valid())
    }
}

/// Reading a pane costs a subprocess, and its footer changes only when the
/// session does.
const FOOTER_READ_EVERY: Duration = Duration::from_secs(60);
/// The statusline is the pane's last lines.
pub const CAPTURE_LINES: &str = "12";

/// One provider's newest reading and its weekly burn, ready for the header.
/// The header derives everything time-dependent from the clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    pub provider: Provider,
    pub latest: Option<Reading>,
    pub burn: Option<f64>,
    pub trend: Option<Trend>,
    /// The weekly rates the Digest Allowance block projects.
    pub rates: Rates,
}

impl Standing {
    pub(super) fn outlook(&self, now_ms: u64) -> Outlook {
        outlook(self.latest.as_ref(), self.burn, self.trend, now_ms)
    }
}

/// Where each provider stood when the worker sampled, in the shown order. Ages
/// come from each reading's own time against the clock at paint.
pub type Snapshot = Vec<Standing>;

/// The refresh worker's sampling state. The history cache is read on the first
/// sample, off the paint path.
pub struct Sampler {
    directory: Option<std::path::PathBuf>,
    store: Option<Store>,
    footers: BTreeMap<Provider, FooterRead>,
}

/// The last time a provider's panes were read for their footer.
struct FooterRead {
    at: Instant,
    /// The newest session activity those panes had then.
    newest_ms: u64,
}

impl Sampler {
    pub(super) fn new(directory: Option<std::path::PathBuf>) -> Self {
        Self {
            directory,
            store: None,
            footers: BTreeMap::new(),
        }
    }

    /// Records what `listed` (an `ls --json` document) and the panes of
    /// providers that report no limits themselves show, then returns where each
    /// provider stands. `capture` returns the recent output of the named pane
    /// without side effects; it runs at most three times a minute per provider.
    pub(super) fn sample(
        &mut self,
        listed: &Value,
        now_ms: u64,
        now: Instant,
        capture: &mut dyn FnMut(&str) -> Option<String>,
    ) -> Snapshot {
        let directory = &self.directory;
        let store = self
            .store
            .get_or_insert_with(|| Store::load(directory.clone()));
        let reported = listing::reported(listed);
        for (provider, reading) in &reported {
            store.record(provider, *reading);
        }
        for (provider, candidates) in listing::footer_candidates(listed) {
            // A driver whose sessions name an account reports its limits; a footer is for the
            // ones that do not.
            if !reported.keys().any(|known| known.id() == provider.id()) {
                read_footer(
                    store,
                    &mut self.footers,
                    &provider,
                    &candidates,
                    now,
                    capture,
                );
            }
        }
        store.save_if_due(now_ms);
        let mut providers = listing::present(listed);
        providers.extend(store.providers());
        let providers = listing::fold_single_account(providers);
        // The newest label of each account: the kept one, then what the listing reported now.
        let mut labels: BTreeMap<Provider, String> = BTreeMap::new();
        for provider in store.providers().chain(reported.keys().cloned()) {
            if let Some(label) = provider.account_label() {
                labels.insert(provider.clone(), label.to_owned());
            }
        }
        // An account is named only where a driver has more than one, so a single account (or
        // none) reads exactly like the plain driver. Then the plain driver, whose sessions name
        // no account, reads as the unknown one rather than as the total.
        let accounts = |driver: &str| {
            providers
                .iter()
                .filter(|other| other.id() == driver && other.account_key().is_some())
                .count()
        };
        let named: Vec<Provider> = providers
            .iter()
            .map(|provider| {
                let several = accounts(provider.id()) > 1;
                let provider = match labels.get(provider) {
                    Some(label) => provider.clone().labelled(label),
                    None => provider.clone(),
                };
                provider
                    .showing_account(several)
                    .showing_unknown_account(several)
            })
            .collect();
        named
            .into_iter()
            .map(|provider| {
                // A driver with one account folds the readings it kept before accounts were named
                // into that account, so its pace survives the upgrade.
                let history = if provider.account_key().is_some() {
                    folded(store.history(&provider.plain()), store.history(&provider))
                } else {
                    store.history(&provider).to_vec()
                };
                let history = history.as_slice();
                let (burn, trend) = estimate::pace(history);
                Standing {
                    latest: history.last().copied(),
                    burn,
                    trend,
                    rates: allowance::rates(history),
                    provider,
                }
            })
            .collect()
    }
}

/// A single account's history with the readings kept for its plain driver before accounts were
/// named, oldest first. A timestamp both hold keeps the account's.
pub(in crate::board) fn folded(plain: &[Reading], account: &[Reading]) -> Vec<Reading> {
    let mut merged: Vec<Reading> = plain
        .iter()
        .filter(|reading| {
            account
                .iter()
                .all(|kept| kept.observed_at_ms != reading.observed_at_ms)
        })
        .chain(account)
        .copied()
        .collect();
    merged.sort_by_key(|reading| reading.observed_at_ms);
    merged
}

/// Reads the freshest pane of `provider` that shows the footer, if its sessions
/// did anything since the last reading or attempt.
fn read_footer(
    store: &mut Store,
    footers: &mut BTreeMap<Provider, FooterRead>,
    provider: &Provider,
    candidates: &[listing::Candidate],
    now: Instant,
    capture: &mut dyn FnMut(&str) -> Option<String>,
) {
    let Some(first) = candidates.first() else {
        return;
    };
    let stored = store
        .history(provider)
        .last()
        .map_or(0, |reading| reading.observed_at_ms);
    let last = footers.get(provider);
    let tried = last.map_or(0, |read| read.newest_ms);
    // Nothing happened in any session since the last reading or attempt, so the
    // footers cannot have changed; a provider with no footer costs one attempt
    // per burst of activity, not one per tick.
    if first.observed_at_ms <= stored.max(tried)
        || last.is_some_and(|read| now.saturating_duration_since(read.at) < FOOTER_READ_EVERY)
    {
        return;
    }
    footers.insert(
        provider.clone(),
        FooterRead {
            at: now,
            newest_ms: first.observed_at_ms,
        },
    );
    for candidate in candidates {
        if let Some(reading) =
            capture(&candidate.name).and_then(|text| footer::read(&text, candidate.observed_at_ms))
        {
            store.record(provider, reading);
            return;
        }
    }
}

#[cfg(test)]
mod tests;
