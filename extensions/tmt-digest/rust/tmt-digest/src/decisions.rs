//! `tmt digest decisions`: the tick decisions Digest recorded, listed as they
//! were, or with `--report` the savings those deliveries made. Nothing is
//! delivered here; only `--recompute` estimates, and says so.

mod report;
use report::Recompute;

/// What the command prints: the recorded decisions, or the savings report.
pub enum Mode {
    List,
    Report(Option<Alternative>),
}

/// An interval to recompute with; the flush count defaults to the configured one.
pub struct Alternative {
    pub interval_ms: u64,
    pub flush_count: Option<u64>,
}

use crate::{
    core::{Core, Error},
    history::{Decision, Store},
    settings::DigestConfig,
    status::inventory,
    usage::Since,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub fn execute(since: Since, mode: Mode) -> Result<Value, Error> {
    let core = Core::discover()?.until(Instant::now() + Duration::from_secs(15));
    let root = core.data_root()?;
    let now = crate::tick::now_ms()?;
    // Names are a courtesy: a roster that cannot be read leaves them null.
    let names: BTreeMap<String, String> = core
        .json(&["identity", "ls"])
        .and_then(|roster| inventory(&roster))
        .map(|rows| rows.into_iter().collect())
        .unwrap_or_default();
    let from = now.saturating_sub(since.window_ms());
    let mut store = Store::open(&root, false)?;
    let Mode::Report(alternative) = mode else {
        let decisions = match &mut store {
            Some(store) => store.decisions(from, now)?,
            None => vec![],
        };
        return Ok(document(since, now, &decisions, &names));
    };
    let recompute = alternative
        .map(|alternative| -> Result<Recompute, Error> {
            Ok(Recompute {
                interval_ms: alternative.interval_ms,
                flush_count: match alternative.flush_count {
                    Some(count) => count,
                    None => u64::try_from(DigestConfig::load(&core.settings_path()?)?.flush_count)
                        .map_err(|_| {
                            Error::new("DIGEST_SETTINGS_INVALID", "flushCount must be positive")
                        })?,
                },
            })
        })
        .transpose()?;
    let (settlements, decisions, coverage) = match &mut store {
        Some(store) => (
            store.settlements(from, now)?,
            store.decisions(from.saturating_sub(120_000), now.saturating_add(120_000))?,
            store.coverage(from, now)?,
        ),
        None => Default::default(),
    };
    Ok(report::document(&report::Input {
        since: since.text(),
        from_ms: from,
        through_ms: now,
        settlements: &settlements,
        decisions: &decisions,
        coverage: &coverage,
        names: &names,
        recompute,
    }))
}

fn document(
    since: Since,
    now: u64,
    decisions: &[Decision],
    names: &BTreeMap<String, String>,
) -> Value {
    json!({
        "version": 1,
        "since": since.text(),
        "fromMs": now.saturating_sub(since.window_ms()),
        "throughMs": now,
        "decisions": decisions.iter().map(|decision| json!({
            "atMs": decision.at_ms,
            "identityId": decision.identity_id,
            "name": names.get(&decision.identity_id),
            "mode": decision.mode(),
            "intervalMs": decision.interval_ms,
            "flushCount": decision.flush_count,
            "rule": decision.rule.text(),
            "heldCount": decision.held_count,
            "oldestHeldAgeMs": decision.oldest_held_age_ms,
            "outcome": decision.outcome,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests;
