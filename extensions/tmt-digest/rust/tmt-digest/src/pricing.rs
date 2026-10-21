//! The price table Digest quotes with: standard base-input list prices, an
//! explicit uncached-input assumption, never historical bills. One public file,
//! `extensions/tmt-digest/pricing/pricing-v1.json`, is embedded as the built-in
//! table and refetched anonymously by `status` (see the contract's Pricing file).

mod cache;
mod fetch;
#[cfg(test)]
mod tests;

use crate::history::MAX;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{OnceLock, mpsc},
    time::Duration,
};

const BUILT_IN: &str = include_str!("../../../pricing/pricing-v1.json");
const SCHEMA: &str = "tmt-digest-pricing-v1";
const BASIS: &str = "standard-base-input";
const MAX_MODELS: usize = 64;
const MAX_FILE_BYTES: usize = 16 * 1024;
/// Attempts are at least this far apart; a failed attempt counts.
const ATTEMPT_EVERY_MS: u64 = 30_000;
/// The fetch has its own deadline; this only bounds the wait for its thread.
const WAIT: Duration = Duration::from_millis(2_500);

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Rate {
    model_id: String,
    usd_micros_per_million_input_tokens: u64,
    source: String,
}

/// One validated price file. Equal `version`s always mean equal tables.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Table {
    schema: String,
    version: String,
    as_of: String,
    basis: String,
    models: Vec<Rate>,
}

impl Table {
    /// Strict parse: UTF-8 JSON with exactly the file's keys and valid values.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_FILE_BYTES {
            return None;
        }
        let table: Self = serde_json::from_slice(bytes).ok()?;
        table.valid().then_some(table)
    }

    fn valid(&self) -> bool {
        self.schema == SCHEMA
            && date(&self.version)
            && date(&self.as_of)
            && self.basis == BASIS
            && (1..=MAX_MODELS).contains(&self.models.len())
            && self.models.iter().enumerate().all(|(index, rate)| {
                rate.valid()
                    && !self.models[..index]
                        .iter()
                        .any(|other| other.model_id == rate.model_id)
            })
    }

    /// May this table be used next to `known`? Never an older version, and a
    /// version already known is the same table: a price change takes a new version.
    fn follows(&self, known: &Self) -> bool {
        self.version > known.version || self == known
    }

    pub fn quote(&self, model: &str, observed_at: u64, tokens: u64) -> Option<Value> {
        if observed_at > MAX || tokens > MAX {
            return None;
        }
        let rate = self.models.iter().find(|rate| rate.model_id == model)?;
        let per_million = rate.usd_micros_per_million_input_tokens;
        let micros = (u128::from(tokens) * u128::from(per_million) + 500_000) / 1_000_000;
        let micros = u64::try_from(micros).ok().filter(|n| *n <= MAX)?;
        Some(json!({
            "usdMicros":micros,"modelId":model,"modelObservedAtMs":observed_at,
            "pricingVersion":self.version,"usdMicrosPerMillionInputTokens":per_million,
            "assumption":"uncached-input"
        }))
    }

    fn document(&self) -> Value {
        json!({"version":self.version,"asOf":self.as_of,"basis":self.basis,
            "models":self.models.iter().map(|rate| json!({"modelId":rate.model_id,
                "usdMicrosPerMillionInputTokens":rate.usd_micros_per_million_input_tokens,
                "source":rate.source})).collect::<Vec<_>>()})
    }
}

impl Rate {
    fn valid(&self) -> bool {
        let id = self.model_id.as_bytes();
        (1..=64).contains(&id.len())
            && id
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            && self.usd_micros_per_million_input_tokens <= MAX
            && self.source.starts_with("https://")
            && self.source.len() <= 256
            && self.source.bytes().all(|c| (0x21..=0x7e).contains(&c))
    }
}

/// A real `YYYY-MM-DD` date, as the file's `version` and `asOf` are.
fn date(text: &str) -> bool {
    let number = |range: std::ops::Range<usize>| {
        text.get(range)
            .filter(|part| part.bytes().all(|c| c.is_ascii_digit()))
            .and_then(|part| part.parse::<u32>().ok())
    };
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year >= 2000 && (1..=days).contains(&day)
}

/// The table compiled into this build; the tests prove it parses.
pub fn built_in() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        Table::parse(BUILT_IN.as_bytes()).expect("the embedded price file is valid")
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Fetched,
    Cached,
    BuiltIn,
}

/// The table in use for one status document and where it came from.
#[derive(Clone, Debug)]
pub struct Resolved {
    table: Table,
    origin: Origin,
    fetched_at_ms: Option<u64>,
}

impl Resolved {
    pub fn built_in() -> Self {
        Self {
            table: built_in().clone(),
            origin: Origin::BuiltIn,
            fetched_at_ms: None,
        }
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The status document's `pricing` object.
    pub fn document(&self) -> Value {
        let mut document = self.table.document();
        document["origin"] = match self.origin {
            Origin::Fetched => "fetched",
            Origin::Cached => "cached",
            Origin::BuiltIn => "built-in",
        }
        .into();
        if let Some(time) = self.fetched_at_ms {
            document["fetchedAtMs"] = time.into();
        }
        document
    }

    /// The cached file, when it still follows the built-in table.
    fn cached(state: &cache::State) -> Option<Self> {
        let table = state
            .file
            .as_ref()
            .filter(|file| file.follows(built_in()))?;
        Some(Self {
            table: table.clone(),
            origin: Origin::Cached,
            fetched_at_ms: state.fetched_at_ms,
        })
    }
}

/// One fetch started alongside the Core read and joined before prices are used.
pub struct Lookup {
    fallback: Resolved,
    pending: Option<mpsc::Receiver<Option<Resolved>>>,
}

impl Lookup {
    pub fn start(root: &Path, now: u64) -> Self {
        Self::begin(root, now, fetch::target())
    }

    fn begin(root: &Path, now: u64, target: Option<fetch::Target>) -> Self {
        let state = cache::read(root);
        let fallback = state
            .as_ref()
            .and_then(Resolved::cached)
            .unwrap_or_else(Resolved::built_in);
        let due = state.as_ref().is_none_or(|state| state.due(now));
        let Some(target) = target.filter(|_| due) else {
            return Self {
                fallback,
                pending: None,
            };
        };
        let (sender, receiver) = mpsc::channel();
        let root = root.to_owned();
        std::thread::spawn(move || {
            let _ = sender.send(attempt(&root, now, &target, state));
        });
        Self {
            fallback,
            pending: Some(receiver),
        }
    }

    /// The fetched table, or the cache and then the built-in table when the fetch
    /// failed, was dropped or is still running after its deadline.
    pub fn finish(self) -> Resolved {
        self.pending
            .and_then(|pending| pending.recv_timeout(WAIT).ok())
            .flatten()
            .unwrap_or(self.fallback)
    }
}

fn attempt(
    root: &Path,
    now: u64,
    target: &fetch::Target,
    state: Option<cache::State>,
) -> Option<Resolved> {
    let known = state.as_ref().and_then(|state| state.file.as_ref());
    let fetched = fetch::get(target)
        .and_then(|bytes| Table::parse(&bytes))
        .filter(|table| {
            table.follows(built_in()) && known.is_none_or(|known| table.follows(known))
        });
    let next = match &fetched {
        Some(table) => cache::State {
            attempted_at_ms: now,
            fetched_at_ms: Some(now),
            file: Some(table.clone()),
        },
        // A failed attempt keeps the last file that still follows the built-in table.
        None => {
            let kept = state.as_ref().filter(|state| {
                state
                    .file
                    .as_ref()
                    .is_some_and(|file| file.follows(built_in()))
            });
            cache::State {
                attempted_at_ms: now,
                fetched_at_ms: kept.and_then(|state| state.fetched_at_ms),
                file: kept.and_then(|state| state.file.clone()),
            }
        }
    };
    // Best effort: without a cache the next status simply asks again.
    let _ = cache::write(root, &next);
    fetched.map(|table| Resolved {
        table,
        origin: Origin::Fetched,
        fetched_at_ms: Some(now),
    })
}
