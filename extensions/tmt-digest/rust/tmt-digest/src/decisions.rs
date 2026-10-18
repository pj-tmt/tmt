//! `tmt digest decisions`: the tick decisions Digest recorded, listed as they
//! were. Nothing is delivered, replayed or estimated here.

use crate::{
    core::{Core, Error},
    history::{Decision, Store},
    status::inventory,
    usage::Since,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub fn execute(since: Since) -> Result<Value, Error> {
    let core = Core::discover()?.until(Instant::now() + Duration::from_secs(15));
    let root = core.data_root()?;
    let now = crate::tick::now_ms()?;
    // Names are a courtesy: a roster that cannot be read leaves them null.
    let names: BTreeMap<String, String> = core
        .json(&["identity", "ls"])
        .and_then(|roster| inventory(&roster))
        .map(|rows| rows.into_iter().collect())
        .unwrap_or_default();
    let decisions = match Store::open(&root, false)? {
        Some(store) => store.decisions(now.saturating_sub(since.window_ms()), now)?,
        None => vec![],
    };
    Ok(document(since, now, &decisions, &names))
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
