//! Bounded recovery history. Counts select recovery input, never live authority.
use std::collections::BTreeSet;

pub const RECENT: usize = 10;
pub const HOURLY: usize = 24;
pub const MAX_GENERATIONS: usize = RECENT + HOURLY + 1;
const HOUR_MS: u64 = 3_600_000;

/// Separate from manually named snapshots: `:` is not a valid saved name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GenerationId(String);

impl GenerationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn digest(&self) -> &str {
        &self.0[2..]
    }
}

impl std::str::FromStr for GenerationId {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value
            .strip_prefix("g:")
            .is_some_and(crate::content_digest::is_sha256)
        {
            Ok(Self(value.to_owned()))
        } else {
            Err("Expected a workspace generation selector g:<sha256>.")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGeneration {
    pub id: GenerationId,
    pub captured_at_ms: u64,
    pub panes: usize,
    pub agents: usize,
    pub shrink: bool,
}

impl WorkspaceGeneration {
    pub fn smaller_than(&self, previous: &Self) -> bool {
        self.panes < previous.panes || self.agents < previous.agents
    }
    fn loses_half(&self, previous: &Self) -> bool {
        (previous.panes > 0 && self.panes <= previous.panes / 2)
            || (previous.agents > 0 && self.agents <= previous.agents / 2)
    }
}

/// Publication order is retained even when the wall clock moves backwards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalogue {
    pub entries: Vec<WorkspaceGeneration>,
    pub protected: Option<GenerationId>,
}

impl Catalogue {
    pub fn get(&self, id: &GenerationId) -> Option<&WorkspaceGeneration> {
        self.entries.iter().find(|entry| &entry.id == id)
    }
    pub fn latest(&self) -> Option<&WorkspaceGeneration> {
        self.entries.last()
    }
    pub fn protected(&self) -> Option<&WorkspaceGeneration> {
        self.protected.as_ref().and_then(|id| self.get(id))
    }
    pub fn preferred(&self) -> Option<&WorkspaceGeneration> {
        let newest = self.entries.iter().rev().find(|entry| entry.agents > 0)?;
        if newest.shrink {
            self.protected()
                .filter(|entry| entry.agents > 0)
                .or(Some(newest))
        } else {
            Some(newest)
        }
    }
    pub fn is_valid(&self) -> bool {
        self.entries.len() <= MAX_GENERATIONS
            && self.entries.iter().all(|entry| {
                entry.panes <= super::MAX_PANES
                    && entry.agents <= entry.panes
                    && entry.captured_at_ms <= crate::limits::MAX_JS_SAFE_INTEGER
            })
            && self
                .entries
                .iter()
                .map(|entry| &entry.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.entries.len()
            && self
                .protected
                .as_ref()
                .is_none_or(|id| self.get(id).is_some())
    }

    /// The caller publishes returned files' replacement index before removing them.
    pub fn append(&mut self, mut next: WorkspaceGeneration) -> Vec<GenerationId> {
        if self.get(&next.id).is_some() {
            return Vec::new();
        }
        let baseline = self.protected().or_else(|| self.latest());
        next.shrink = baseline.is_some_and(|previous| next.loses_half(previous));
        if baseline.is_none_or(|previous| !next.smaller_than(previous)) {
            self.protected = Some(next.id.clone());
        }
        let hour = next.captured_at_ms / HOUR_MS;
        self.entries.push(next);
        let mut keep = self
            .entries
            .iter()
            .rev()
            .take(RECENT)
            .map(|entry| entry.id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(id) = &self.protected {
            keep.insert(id.clone());
        }
        let mut hours = BTreeSet::new();
        for entry in self.entries.iter().rev() {
            let bucket = entry.captured_at_ms / HOUR_MS;
            if bucket <= hour && hour - bucket < HOURLY as u64 && hours.insert(bucket) {
                keep.insert(entry.id.clone());
            }
        }
        let mut removed = Vec::new();
        self.entries.retain(|entry| {
            if keep.contains(&entry.id) {
                true
            } else {
                removed.push(entry.id.clone());
                false
            }
        });
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(number: u64, panes: usize, agents: usize) -> WorkspaceGeneration {
        WorkspaceGeneration {
            id: format!("g:{number:064x}").parse().unwrap(),
            captured_at_ms: number * HOUR_MS,
            panes,
            agents,
            shrink: false,
        }
    }
    #[test]
    fn repeated_shrink_cannot_age_out_the_protected_baseline() {
        let mut history = Catalogue::default();
        let good = entry(1, 20, 10);
        history.append(good.clone());
        for number in 2..=100 {
            history.append(entry(number, 1, 1));
            assert!(history.is_valid());
            assert_eq!(history.protected(), Some(&good));
            assert_eq!(history.preferred(), Some(&good));
            assert!(history.latest().unwrap().shrink);
        }
        history.append(entry(101, 20, 10));
        assert_eq!(history.protected(), history.latest());
        assert!(!history.latest().unwrap().shrink);
    }
    #[test]
    fn retention_combines_distinct_recent_hours_and_clock_rollback() {
        let mut history = Catalogue::default();
        for number in 1..=200 {
            let mut next = entry(number, 20, 10);
            next.captured_at_ms = number * 300_000;
            history.append(next);
            assert!(history.is_valid());
        }
        for number in 201..=250 {
            let mut next = entry(number, 1, 1);
            next.captured_at_ms = 0;
            history.append(next);
            assert!(history.is_valid());
        }
        assert_eq!(history.latest().unwrap().id, entry(250, 1, 1).id);
        assert_eq!(history.protected().unwrap().id, entry(200, 20, 10).id);
    }
}
