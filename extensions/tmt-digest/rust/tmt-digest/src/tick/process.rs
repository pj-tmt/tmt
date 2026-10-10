//! Public Core calls and current settings; no terminal or Core storage access.

use super::{TickObservation, TickPort};
use crate::{
    core::{Core, Error},
    settings::{DigestConfig, Mode, canonical_uuid},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Instant,
};

const MAX_MS: u64 = 9_007_199_254_740_991;

pub struct TickProcess {
    core: Core,
    settings_path: PathBuf,
    synchronized: BTreeMap<String, Mode>,
    failed: BTreeSet<String>,
    errors: Vec<String>,
    boundary_ms: u64,
    boundary: Instant,
}
impl TickProcess {
    pub fn new(core: Core, settings_path: PathBuf, boundary_ms: u64, boundary: Instant) -> Self {
        Self {
            core,
            settings_path,
            synchronized: BTreeMap::new(),
            failed: BTreeSet::new(),
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
        let identity = |key: &str| -> Result<String, Error> {
            if revision == 0 {
                Ok(settings.setter(id).unwrap_or(id).into())
            } else {
                policy[key]
                    .as_str()
                    .filter(|id| canonical_uuid(id))
                    .map(str::to_owned)
                    .ok_or_else(invalid)
            }
        };
        let operation = match mode {
            Mode::Interval { .. } if changed || !active => Some("digest.policy.set"),
            Mode::Auto | Mode::Off if active => Some("digest.policy.clear"),
            _ => None,
        };
        if let Some(operation) = operation {
            if !self.within_minute()? {
                return Ok(());
            }
            let mut input = json!({"identityId": id, "expectedRevision": revision,
                "ownerIdentityId": identity("ownerIdentityId")?,
                "setterIdentityId": identity("setterIdentityId")?});
            if let Mode::Interval { milliseconds, .. } = mode {
                // A missed next minute does not immediately release a configured long interval.
                input["untilMs"] = json!(
                    super::now_ms()?
                        .saturating_add(milliseconds.max(120_000))
                        .min(MAX_MS)
                );
            }
            self.core.api(operation, input)?;
        }
        self.synchronized.insert(id.into(), mode);
        Ok(())
    }
    fn flush(&mut self, id: &str) -> Result<(), Error> {
        if self.failed.contains(id) || !self.within_minute()? {
            return Ok(());
        }
        // Core owns readiness, claims and settlement for this exact UUID.
        let outcome = self
            .core
            .api("digest.checklist.flush", json!({"identityId": id}))
            .and_then(|value| {
                if value["identityId"] != id {
                    return Err(invalid());
                }
                match value["state"].as_str() {
                    Some("delivered" | "not_idle" | "nothing_due" | "unavailable") => Ok(()),
                    Some("uncertain") => Err(Error::new(
                        "DIGEST_DELIVERY_UNCERTAIN",
                        "Core retained an uncertain checklist; inspect it before any retry",
                    )),
                    _ => Err(invalid()),
                }
            });
        if let Err(error) = outcome {
            self.failed.insert(id.into());
            self.errors
                .push(format!("{id}: {} ({})", error.message, error.code));
        }
        Ok(())
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
                if !matches!(row.mode, Mode::Interval { .. }) && row.held_count > 0 {
                    self.flush(id)?;
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
        if matches!(mode, Mode::Interval { .. }) {
            if !self.within_minute()? {
                return Ok(());
            }
            let stats = self
                .core
                .api("digest.stats.show", json!({"identities": [id]}))?;
            let row = observation(id, &settings, &ordered(&stats, "stats", &ids)?[0])?;
            let now = super::now_ms()?;
            if !row.deadline(now).is_some_and(|deadline| deadline <= now) {
                return Ok(());
            }
            if !self.within_minute()? {
                return Ok(());
            }
            self.core
                .api("digest.checklist.dueNow", json!({"identityId": id}))?;
        }
        self.flush(id)?;
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
