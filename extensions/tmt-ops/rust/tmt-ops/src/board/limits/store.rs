//! Sampled readings kept across board restarts, so a rate can be estimated from
//! more than one session of the board. A disposable cache: losing or
//! corrupting it only costs the estimate.
use super::{Provider, Reading, Window};
use crate::cache;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io, path::PathBuf};

const FILE: &str = "samples.json";
const VERSION: u64 = 1;
/// Samples kept per provider: a full weekly cycle and a day.
const KEEP_MS: u64 = 8 * 24 * 3_600_000;
/// One kept sample per bucket; a newer reading in the same bucket replaces it.
const BUCKET_MS: u64 = 10 * 60_000;
/// Rewriting the file for every replaced sample would be wasted I/O.
const SAVE_EVERY_MS: u64 = 5 * 60_000;
const READ_LIMIT: u64 = 1024 * 1024;
/// Providers kept; a driver list is short, and the file is untrusted.
const MAX_PROVIDERS: usize = 8;

pub(in crate::board) struct Store {
    path: Option<PathBuf>,
    history: BTreeMap<Provider, Vec<Reading>>,
    changed: bool,
    saved_at_ms: u64,
}

impl Store {
    /// Reads the cache; a missing, oversized or malformed file starts empty.
    pub(in crate::board) fn load(directory: Option<PathBuf>) -> Self {
        let path = directory.map(|directory| directory.join(FILE));
        let history = path.as_deref().and_then(read).unwrap_or_default();
        Self {
            path,
            history,
            changed: false,
            saved_at_ms: 0,
        }
    }

    pub(in crate::board) fn history(&self, provider: &Provider) -> &[Reading] {
        self.history.get(provider).map_or(&[], Vec::as_slice)
    }

    /// Providers with at least one kept reading.
    pub(in crate::board) fn providers(&self) -> impl Iterator<Item = Provider> + '_ {
        self.history
            .iter()
            .filter(|(_, samples)| !samples.is_empty())
            .map(|(provider, _)| provider.clone())
    }

    /// Keeps `reading` unless it is invalid or not newer than the last one.
    pub(in crate::board) fn record(&mut self, provider: &Provider, reading: Reading) {
        if !reading.valid()
            || (self.history.len() >= MAX_PROVIDERS && !self.history.contains_key(provider))
        {
            return;
        }
        let samples = self.history.entry(provider.clone()).or_default();
        if let Some(last) = samples.last()
            && reading.observed_at_ms <= last.observed_at_ms
        {
            return;
        }
        if samples.last().is_some_and(|last| {
            last.observed_at_ms / BUCKET_MS == reading.observed_at_ms / BUCKET_MS
        }) {
            samples.pop();
        }
        samples.push(reading);
        let oldest = reading.observed_at_ms.saturating_sub(KEEP_MS);
        samples.retain(|sample| sample.observed_at_ms >= oldest);
        self.changed = true;
    }

    /// Writes the cache when something changed and the last write is old enough.
    pub(in crate::board) fn save_if_due(&mut self, now_ms: u64) {
        if !self.changed || now_ms.saturating_sub(self.saved_at_ms) < SAVE_EVERY_MS {
            return;
        }
        let Some(path) = &self.path else { return };
        // A failed write keeps the samples in memory and retries later.
        if cache::replace(path, document(&self.history).to_string().as_bytes()).is_ok() {
            self.changed = false;
            self.saved_at_ms = now_ms;
        }
    }
}

fn document(history: &BTreeMap<Provider, Vec<Reading>>) -> Value {
    let providers = history
        .iter()
        .map(|(provider, samples)| {
            let rows = samples
                .iter()
                .map(|r| {
                    json!([
                        r.observed_at_ms,
                        r.weekly.left,
                        r.weekly.resets_at_ms,
                        r.short.map(|w| w.left),
                        r.short.map(|w| w.resets_at_ms),
                    ])
                })
                .collect::<Vec<_>>();
            (provider.id().to_owned(), Value::from(rows))
        })
        .collect::<serde_json::Map<_, _>>();
    json!({ "version": VERSION, "providers": providers })
}

fn read(path: &std::path::Path) -> Option<BTreeMap<Provider, Vec<Reading>>> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > READ_LIMIT {
        return None;
    }
    let value: Value = serde_json::from_reader(io::BufReader::new(file)).ok()?;
    if value["version"].as_u64() != Some(VERSION) {
        return None;
    }
    let mut history = BTreeMap::new();
    for (key, rows) in value["providers"].as_object()? {
        if history.len() >= MAX_PROVIDERS {
            break;
        }
        let Some(provider) = Provider::from_driver(key) else {
            continue;
        };
        let mut samples: Vec<Reading> = Vec::new();
        for row in rows.as_array().into_iter().flatten() {
            // One bad row drops itself; order and validity are re-established.
            let Some(sample) = sample(row) else { continue };
            if samples
                .last()
                .is_none_or(|last| last.observed_at_ms < sample.observed_at_ms)
            {
                samples.push(sample);
            }
        }
        history.insert(provider, samples);
    }
    Some(history)
}

fn sample(row: &Value) -> Option<Reading> {
    let row = row.as_array().filter(|row| row.len() == 5)?;
    let window = |left: &Value, reset: &Value| {
        Some(Window {
            left: left.as_f64()?,
            resets_at_ms: reset.as_u64()?,
        })
    };
    let short = match (&row[3], &row[4]) {
        (Value::Null, Value::Null) => None,
        (left, reset) => Some(window(left, reset)?),
    };
    let reading = Reading {
        observed_at_ms: row[0].as_u64()?,
        weekly: window(&row[1], &row[2])?,
        short,
    };
    reading.valid().then_some(reading)
}

#[cfg(test)]
mod tests;
