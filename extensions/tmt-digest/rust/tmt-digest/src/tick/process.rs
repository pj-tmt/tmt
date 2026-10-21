//! Public Core calls and current settings; no terminal or Core storage access.

use super::{HOLD_FLOOR_MS, TickObservation, TickPort};
use crate::{
    core::{Core, Error},
    history::{Decision, Rule},
    settings::{DigestConfig, Mode, canonical_uuid},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

const MAX_MS: u64 = 9_007_199_254_740_991;
/// An `auto` member's backlog is offered again after this long while it stays held.
const REOFFER: Duration = Duration::from_secs(15);

pub struct TickProcess {
    core: Core,
    history: Option<crate::history::Store>,
    settings_path: PathBuf,
    synchronized: BTreeMap<String, Mode>,
    failed: BTreeSet<String>,
    /// The newest recorded `(rule, outcome, oldest held arrival)` per member, so
    /// the once-a-second loop records a change of decision and not each repeat.
    decided: BTreeMap<String, (Rule, Option<&'static str>, u64)>,
    /// What an `auto` member held when its backlog was last offered, and when: an
    /// offer waits for new arrivals or `REOFFER`, so a busy member is not asked every second.
    offered: BTreeMap<String, ((u64, Option<u64>), Instant)>,
    errors: Vec<String>,
    boundary_ms: u64,
    boundary: Instant,
}
impl TickProcess {
    pub fn new(core: Core, settings_path: PathBuf, boundary_ms: u64, boundary: Instant) -> Self {
        Self {
            core,
            history: None,
            settings_path,
            synchronized: BTreeMap::new(),
            failed: BTreeSet::new(),
            decided: BTreeMap::new(),
            offered: BTreeMap::new(),
            errors: vec![],
            boundary_ms,
            boundary,
        }
    }
    fn within_minute(&self) -> Result<bool, Error> {
        Ok(Instant::now() < self.boundary && super::now_ms()? < self.boundary_ms)
    }
    fn synchronize(
        &mut self,
        id: &str,
        settings: &DigestConfig,
        policy: &Value,
    ) -> Result<(), Error> {
        let mode = settings.effective(id)?;
        let revision = number(policy, "revision")?;
        let active = policy["active"].as_bool().ok_or_else(invalid)?;
        let changed = self.synchronized.get(id) != Some(&mode);
        // A hold from before anyone was recorded names the member as its own owner;
        // it adopts the setter once one is recorded, and any other owner stays pinned.
        let recorded = settings.recorded_setter(id).filter(|setter| *setter != id);
        let identity = |key: &str| -> Result<String, Error> {
            if revision == 0 {
                Ok(recorded.unwrap_or(id).into())
            } else {
                let pinned = policy[key]
                    .as_str()
                    .filter(|id| canonical_uuid(id))
                    .ok_or_else(invalid)?;
                Ok(match recorded {
                    Some(setter) if pinned == id => setter,
                    _ => pinned,
                }
                .into())
            }
        };
        let horizon = match &mode {
            Mode::Interval { milliseconds, .. } => Some((*milliseconds).max(HOLD_FLOOR_MS)),
            Mode::Auto => Some(HOLD_FLOOR_MS),
            Mode::Off => None,
        };
        let adopts = revision > 0
            && recorded.is_some()
            && ["ownerIdentityId", "setterIdentityId"]
                .iter()
                .any(|key| policy[*key].as_str() == Some(id));
        let operation = match horizon {
            Some(_) if changed || !active || adopts => Some("digest.policy.set"),
            None if active => Some("digest.policy.clear"),
            _ => None,
        };
        if let Some(operation) = operation {
            if !self.within_minute()? {
                return Ok(());
            }
            let mut input = json!({"identityId": id, "expectedRevision": revision,
                "ownerIdentityId": identity("ownerIdentityId")?,
                "setterIdentityId": identity("setterIdentityId")?});
            if let Some(horizon) = horizon {
                // A missed next minute does not immediately release a configured long interval.
                input["untilMs"] = json!(super::now_ms()?.saturating_add(horizon).min(MAX_MS));
            }
            self.core.api(operation, input)?;
        }
        self.synchronized.insert(id.into(), mode);
        Ok(())
    }
    pub(super) fn attach_history(&mut self, result: Result<Option<crate::history::Store>, Error>) {
        match result {
            Ok(history) => self.history = history,
            Err(error) => self
                .errors
                .push(format!("{} ({})", error.message, error.code)),
        }
    }
    fn collect_history(&mut self, id: &str) {
        if let Some(store) = &mut self.history
            && let Err(error) = crate::history::collect(&self.core, store, id, self.boundary)
        {
            // Collection cannot mark a member delivery-failed or authorize a retry.
            let message = format!("{id}: {} ({})", error.message, error.code);
            if !self.errors.contains(&message) {
                self.errors.push(message);
            }
        }
        if let Some(store) = &mut self.history
            && let Err(error) = crate::history::collect_usage(&self.core, store, id, self.boundary)
        {
            let message = format!("{id}: {} ({})", error.message, error.code);
            if !self.errors.contains(&message) {
                self.errors.push(message);
            }
        }
    }
    /// Append one decision record unless it repeats the member's newest one: the
    /// same rule and answer for the same oldest held message. That newest record
    /// is read from the store once per member and tick, so a state that lasts
    /// across minutes stays one row. A store failure is reported and never blocks
    /// or retries delivery.
    fn record(&mut self, row: &TickObservation, rule: Rule, outcome: Option<&'static str>) {
        let Some(oldest) = row.oldest_held_at_ms.filter(|_| row.held_count > 0) else {
            return;
        };
        let key = (rule, outcome, oldest);
        let id = row.identity_id.as_str();
        let known = match (self.decided.get(id), &mut self.history) {
            (Some(known), _) => Some(*known),
            (None, Some(store)) => match store.last_decision(id) {
                Ok(last) => {
                    last.and_then(|last| Some((last.rule, last.outcome, last.oldest_held_at_ms()?)))
                }
                Err(error) => {
                    self.fail(id, &error);
                    return;
                }
            },
            (None, None) => None,
        };
        self.decided.insert(id.into(), key);
        if known == Some(key) {
            return;
        }
        let Ok(at_ms) = super::now_ms() else { return };
        let Some(store) = &mut self.history else {
            return;
        };
        let decision = Decision {
            at_ms,
            identity_id: id.into(),
            interval_ms: match row.mode {
                Mode::Interval { milliseconds, .. } => Some(milliseconds),
                _ => None,
            },
            flush_count: row.flush_count,
            rule,
            held_count: row.held_count,
            oldest_held_age_ms: Some(at_ms.saturating_sub(oldest)),
            outcome,
        };
        if let Err(error) = store.record_decision(&decision) {
            self.fail(id, &error);
        }
    }
    fn fail(&mut self, id: &str, error: &Error) {
        let message = format!("{id}: {} ({})", error.message, error.code);
        if !self.errors.contains(&message) {
            self.errors.push(message);
        }
    }
    /// Offer Core one delivery opportunity. `when_busy` is only for an `auto` member's
    /// cap: Core then skips its live-idle evidence and keeps every other guard.
    fn flush(&mut self, row: &TickObservation, rule: Rule, when_busy: bool) -> Result<(), Error> {
        let id = row.identity_id.as_str();
        if self.failed.contains(id) || !self.within_minute()? {
            return Ok(());
        }
        let mut input = json!({"identityId": id});
        if when_busy {
            input["whenBusy"] = json!(true);
        }
        // Core owns readiness, claims and settlement for this exact UUID.
        let outcome = self
            .core
            .api("digest.checklist.flush", input)
            .and_then(|value| {
                if value["identityId"] != id {
                    return Err(invalid());
                }
                match value["state"].as_str() {
                    Some("delivered") => Ok("delivered"),
                    Some("not_idle") => Ok("not_idle"),
                    Some("nothing_due") => Ok("nothing_due"),
                    Some("unavailable") => Ok("unavailable"),
                    Some("uncertain") => Err(Error::new(
                        "DIGEST_DELIVERY_UNCERTAIN",
                        "Core retained an uncertain checklist; inspect it before any retry",
                    )),
                    _ => Err(invalid()),
                }
            });
        match outcome {
            Ok(state) => self.record(row, rule, Some(state)),
            Err(error) => self.refuse(row, rule, error),
        }
        self.collect_history(id);
        Ok(())
    }
    /// A failed or uncertain opportunity is recorded and reported after the minute, and
    /// that member is not offered another within this invocation.
    fn refuse(&mut self, row: &TickObservation, rule: Rule, error: Error) {
        let id = row.identity_id.as_str();
        self.record(
            row,
            rule,
            Some(if error.code == "DIGEST_DELIVERY_UNCERTAIN" {
                "uncertain"
            } else {
                "error"
            }),
        );
        self.failed.insert(id.into());
        self.errors
            .push(format!("{id}: {} ({})", error.message, error.code));
    }
    /// An `auto` member holding messages is offered its backlog when something new is
    /// held and again after `REOFFER`: Core delivers it when the agent is idle, and
    /// the provider's turn end delivers it otherwise. At the cap the offer does not wait
    /// for idle, so a member that never reports idle is not held for ever.
    fn offer(&mut self, row: &TickObservation) -> Result<(), Error> {
        let id = row.identity_id.as_str();
        let Some(rule) = row.rule(super::now_ms()?) else {
            self.offered.remove(id);
            return Ok(());
        };
        let held = (row.held_count, row.oldest_held_at_ms);
        if self.failed.contains(id)
            || self
                .offered
                .get(id)
                .is_some_and(|(last, at)| *last == held && at.elapsed() < REOFFER)
            || !self.within_minute()?
        {
            return Ok(());
        }
        self.offered.insert(id.into(), (held, Instant::now()));
        // Idle delivery claims only the range marked due, so mark what is held now.
        if let Err(error) = self
            .core
            .api("digest.checklist.dueNow", json!({"identityId": id}))
        {
            self.refuse(row, rule, error);
            return Ok(());
        }
        self.flush(row, rule, rule == Rule::AutoCap)
    }
    pub fn finish(self) -> Result<(), Error> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(Error::new("DIGEST_TICK_PARTIAL", self.errors.join("; ")))
        }
    }
}
impl TickPort for TickProcess {
    fn not_due(&mut self, row: &TickObservation) {
        self.record(row, Rule::NotDue, None);
    }
    fn observe(&mut self) -> Result<Vec<TickObservation>, Error> {
        if !self.within_minute()? {
            return Ok(vec![]);
        }
        let settings = DigestConfig::load(&self.settings_path)?;
        if !self.within_minute()? {
            return Ok(vec![]);
        }
        let inventory = self.core.json(&["identity", "ls"])?;
        let mut ids = inventory["identities"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|row| {
                row["id"]
                    .as_str()
                    .filter(|id| canonical_uuid(id))
                    .map(str::to_owned)
                    .ok_or_else(invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        ids.sort();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid());
        }
        for id in &ids {
            if !self.within_minute()? {
                break;
            }
            if !self.synchronized.contains_key(id) && !self.failed.contains(id) {
                self.collect_history(id);
            }
        }
        let mut observations = vec![];
        for batch in ids.chunks(256) {
            if !self.within_minute()? {
                return Ok(observations);
            }
            let policies = self
                .core
                .api("digest.policy.show", json!({"identities": batch}))?;
            let policies = ordered(&policies, "policies", batch)?;
            for (id, policy) in batch.iter().zip(policies) {
                if !self.failed.contains(id)
                    && let Err(error) = self.synchronize(id, &settings, policy)
                {
                    self.failed.insert(id.clone());
                    self.errors
                        .push(format!("{id}: {} ({})", error.message, error.code));
                }
            }
            if !self.within_minute()? {
                return Ok(observations);
            }
            let stats = self
                .core
                .api("digest.stats.show", json!({"identities": batch}))?;
            for (id, stats) in batch.iter().zip(ordered(&stats, "stats", batch)?) {
                if self.failed.contains(id) {
                    continue;
                }
                let row = observation(id, &settings, stats)?;
                match row.mode {
                    Mode::Auto => self.offer(&row)?,
                    Mode::Off => {
                        if let Some(rule) = row.rule(super::now_ms()?) {
                            self.flush(&row, rule, false)?;
                        }
                    }
                    Mode::Interval { .. } => {}
                }
                observations.push(row);
            }
        }
        Ok(observations)
    }
    fn deliver_if_current(&mut self, id: &str) -> Result<(), Error> {
        if self.failed.contains(id) || !self.within_minute()? {
            return Ok(());
        }
        let settings = DigestConfig::load(&self.settings_path)?;
        let mode = settings.effective(id)?;
        if !self.within_minute()? {
            return Ok(());
        }
        // Re-observe policy for this specific opportunity; never reuse a revision for a write.
        let policies = self
            .core
            .api("digest.policy.show", json!({"identities": [id]}))?;
        let ids = [id.to_owned()];
        self.synchronize(id, &settings, &ordered(&policies, "policies", &ids)?[0])?;
        if !self.within_minute()? {
            return Ok(());
        }
        let stats = self
            .core
            .api("digest.stats.show", json!({"identities": [id]}))?;
        let row = observation(id, &settings, &ordered(&stats, "stats", &ids)?[0])?;
        let now = super::now_ms()?;
        let rule = match mode {
            Mode::Interval { .. } => {
                let Some(rule) = row.rule(now) else {
                    return Ok(());
                };
                if rule == Rule::NotDue {
                    // A later reading postponed what the earlier one found due.
                    self.record(&row, rule, None);
                    return Ok(());
                }
                if !self.within_minute()? {
                    return Ok(());
                }
                self.core
                    .api("digest.checklist.dueNow", json!({"identityId": id}))?;
                rule
            }
            Mode::Auto => return self.offer(&row),
            Mode::Off => Rule::Off,
        };
        self.flush(&row, rule, false)?;
        Ok(())
    }
}
fn observation(id: &str, settings: &DigestConfig, stats: &Value) -> Result<TickObservation, Error> {
    let observed = number(stats, "observedAtMs")?;
    let age = if stats.get("oldestHeldAgeMs").ok_or_else(invalid)?.is_null() {
        None
    } else {
        Some(number(stats, "oldestHeldAgeMs")?)
    };
    Ok(TickObservation {
        identity_id: id.into(),
        mode: settings.effective(id)?,
        flush_count: settings.flush_count as u64,
        held_count: number(stats, "heldCount")?,
        oldest_held_at_ms: age.map(|age| observed.saturating_sub(age)),
    })
}
fn invalid() -> Error {
    Error::new(
        "CORE_RESPONSE_INVALID",
        "Core returned an incomplete or mismatched digest observation",
    )
}
fn number(document: &Value, key: &str) -> Result<u64, Error> {
    document[key]
        .as_u64()
        .filter(|n| *n <= MAX_MS)
        .ok_or_else(invalid)
}
fn ordered<'a>(document: &'a Value, key: &str, ids: &[String]) -> Result<&'a [Value], Error> {
    let rows = document[key].as_array().ok_or_else(invalid)?;
    if rows.len() != ids.len()
        || rows
            .iter()
            .zip(ids)
            .any(|(row, id)| row["identityId"].as_str() != Some(id.as_str()))
    {
        return Err(invalid());
    }
    Ok(rows)
}

#[cfg(test)]
#[path = "process/tests.rs"]
mod tests;
