//! Shared actor/member read watermarks in the existing Ops preference document.
use super::{Config, SquadError, invalid};
use std::collections::BTreeMap;

const MAX_TIMESTAMP: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReadMarkers(BTreeMap<String, BTreeMap<String, u64>>);

impl ReadMarkers {
    pub fn get(&self, actor: &str, member: &str) -> u64 {
        self.0
            .get(actor)
            .and_then(|members| members.get(member))
            .copied()
            .unwrap_or(0)
    }

    pub fn advance(&mut self, actor: &str, member: &str, at: u64) -> Result<bool, SquadError> {
        if !identity(actor) || !identity(member) || !(1..=MAX_TIMESTAMP).contains(&at) {
            return Err(invalid(
                "Read markers require canonical actor/member UUIDs and a positive safe timestamp.",
            ));
        }
        if at <= self.get(actor, member) {
            return Ok(false);
        }
        self.0
            .entry(actor.into())
            .or_default()
            .insert(member.into(), at);
        Ok(true)
    }

    pub fn merge(&mut self, other: &Self) {
        for (actor, members) in &other.0 {
            for (member, at) in members {
                let current = self
                    .0
                    .entry(actor.clone())
                    .or_default()
                    .entry(member.clone())
                    .or_default();
                *current = (*current).max(*at);
            }
        }
    }

    /// Drop only updates whose exact actor/member watermark is durable.
    pub fn remove_confirmed(&mut self, saved: &Self) {
        self.0.retain(|actor, members| {
            members.retain(|member, at| *at > saved.get(actor, member));
            !members.is_empty()
        });
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn identity(text: &str) -> bool {
    super::uuid_like(text) && !text.bytes().any(|byte| byte.is_ascii_uppercase())
}

impl Config {
    pub(crate) fn read_markers(&self) -> Result<ReadMarkers, SquadError> {
        let Some(workspace) = self.document.get("workspace") else {
            return Ok(ReadMarkers::default());
        };
        let workspace = workspace
            .as_table_like()
            .ok_or_else(|| invalid("`workspace` must be a table."))?;
        let Some(item) = workspace.get("read_markers") else {
            return Ok(ReadMarkers::default());
        };
        let actors = item
            .as_table_like()
            .ok_or_else(|| invalid("`workspace.read_markers` must be a table."))?;
        let mut result = ReadMarkers::default();
        for (actor, item) in actors.iter() {
            let members = item
                .as_table_like()
                .ok_or_else(|| invalid("Each read-marker actor must contain a member table."))?;
            if !identity(actor) {
                return Err(invalid(
                    "Read-marker actor keys must be canonical identity UUIDs.",
                ));
            }
            for (member, at) in members.iter() {
                let at = at
                    .as_integer()
                    .and_then(|at| u64::try_from(at).ok())
                    .ok_or_else(|| {
                        invalid("Read-marker timestamps must be positive safe integers.")
                    })?;
                result.advance(actor, member, at)?;
            }
        }
        Ok(result)
    }

    /// Fresh read and max-merge under the same document lock as every Config writer.
    /// A stale view cannot erase another layout's mark or an authored setting.
    pub(crate) fn save_read_markers(
        &mut self,
        updates: &ReadMarkers,
    ) -> Result<ReadMarkers, SquadError> {
        let _migration = crate::migration::config_write_guard(&self.path)?;
        let (current, markers, changed) = Self::merged(&self.path, updates)?;
        if !changed {
            *self = current;
            return Ok(markers);
        }
        let mut saved = None;
        let outcome = self.state_document()?.update(|_| {
            // The file may have moved on since the unlocked look above.
            let (mut current, markers, _) = Self::merged(&self.path, updates)?;
            let bytes = current.document.to_string().into_bytes();
            if bytes.len() as u64 > super::FILE_LIMIT {
                return Err(super::too_large());
            }
            current.original = Some(bytes.clone());
            saved = Some((current, markers));
            Ok(bytes)
        });
        self.settled(outcome)?;
        let (current, markers) = saved.expect("a successful update ran its change");
        *self = current;
        Ok(markers)
    }

    /// The file read fresh with every update that advances a watermark applied to its
    /// document, the resulting markers, and whether anything advanced.
    fn merged(
        path: &std::path::Path,
        updates: &ReadMarkers,
    ) -> Result<(Self, ReadMarkers, bool), SquadError> {
        let mut current = Self::read(path.to_path_buf())?;
        let mut markers = current.read_markers()?;
        let mut changed = false;
        for (actor, members) in &updates.0 {
            for (member, at) in members {
                if markers.advance(actor, member, *at)? {
                    current.insert_setting(
                        &["workspace", "read_markers", actor, member],
                        super::value(*at as i64),
                    )?;
                    changed = true;
                }
            }
        }
        Ok((current, markers, changed))
    }
}

#[cfg(test)]
mod tests;
