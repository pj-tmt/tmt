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

    pub(crate) fn for_actor(&self, actor: &str) -> BTreeMap<String, u64> {
        self.0.get(actor).cloned().unwrap_or_default()
    }

    #[cfg(test)]
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

    /// Consume only the cutoffs whose public answer walk has settled. Fresh read and
    /// exact comparison under the document lock preserve a concurrent legacy writer.
    pub(crate) fn consume_read_markers(
        &mut self,
        actor: &str,
        completed: &BTreeMap<String, u64>,
    ) -> Result<(), SquadError> {
        let _migration = crate::migration::config_write_guard(&self.path)?;
        let mut saved = None;
        let outcome = self.state_document()?.update(|_| {
            let mut current = Self::read(self.path.clone())?;
            let markers = current.read_markers()?;
            if let Some(members) = current
                .document
                .get_mut("workspace")
                .and_then(toml_edit::Item::as_table_like_mut)
                .and_then(|workspace| workspace.get_mut("read_markers"))
                .and_then(toml_edit::Item::as_table_like_mut)
                .and_then(|actors| actors.get_mut(actor))
                .and_then(toml_edit::Item::as_table_like_mut)
            {
                for (member, cutoff) in completed {
                    if markers.get(actor, member) == *cutoff {
                        members.remove(member);
                    }
                }
            }
            let bytes = current.document.to_string().into_bytes();
            if bytes.len() as u64 > super::FILE_LIMIT {
                return Err(super::too_large());
            }
            current.original = Some(bytes.clone());
            saved = Some(current);
            Ok(bytes)
        });
        self.settled(outcome)?;
        *self = saved.expect("a successful document update ran its change");
        Ok(())
    }
}

#[cfg(test)]
mod tests;
