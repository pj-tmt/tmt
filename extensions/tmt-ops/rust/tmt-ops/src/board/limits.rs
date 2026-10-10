//! Provider usage limits for the HOME header. An account reports a weekly
//! window and sometimes a short one; the board keeps a bounded history of those
//! readings and estimates the burn rate from it. Sampling is the refresh
//! worker's job and runs only while the board is open: a reading is only as
//! recent as its source, and the header says how old it is. This module takes
//! the clock and the pane capture as parameters; painting lives with the HOME
//! header. Providers are whatever drivers the listing names; nothing here
//! spells a driver.
mod estimate;
mod footer;
mod listing;
mod store;

pub use estimate::{Figures, Left, Outlook, outlook};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use store::Store;

/// An account whose limits the board shows, identified by its driver ID as
/// Core's listing reports it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Provider(String);

impl Provider {
    /// The driver ID from a listing row, if it is a plain identifier. The ID
    /// becomes a cache key, so anything else is not accepted.
    pub(super) fn from_driver(id: &str) -> Option<Self> {
        let plain = !id.is_empty()
            && id.len() <= 24
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        plain.then(|| Self(id.to_owned()))
    }

    pub(super) fn id(&self) -> &str {
        &self.0
    }

    /// The shown name: the driver ID with a capital.
    pub(super) fn label(&self) -> String {
        let mut letters = self.0.chars();
        letters
            .next()
            .map(|first| first.to_uppercase().chain(letters).collect())
            .unwrap_or_default()
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
}

impl Standing {
    pub(super) fn outlook(&self, now_ms: u64) -> Outlook {
        outlook(self.latest.as_ref(), self.burn, now_ms)
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
            if !reported.contains_key(&provider) {
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
        providers
            .into_iter()
            .map(|provider| {
                let history = store.history(&provider);
                Standing {
                    latest: history.last().copied(),
                    burn: estimate::burn_per_hour(history),
                    provider,
                }
            })
            .collect()
    }
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
