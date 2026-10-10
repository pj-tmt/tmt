//! Typed consumption of owner-issued grants; pairing remains the grant producer.
use crate::{
    canonical,
    error::RemoteError,
    store::{Grant, SUPPORTED_SCOPES},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermittedAgents {
    All,
    Selected(Vec<String>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchMode {
    Direct,
    Hold,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantAuthority {
    pub agents: PermittedAgents,
    pub mode: DispatchMode,
}
impl GrantAuthority {
    pub fn permits(&self, agent: &str) -> bool {
        canonical::is_core_id(agent)
            && match &self.agents {
                PermittedAgents::All => true,
                PermittedAgents::Selected(ids) => {
                    ids.binary_search_by(|id| id.as_str().cmp(agent)).is_ok()
                }
            }
    }
}
impl Grant {
    /// Admit persisted policy without widening malformed or future authority.
    /// `agents` retains its existing database/public-producer representation:
    /// `all`, or a JSON array of sorted canonical core UUID references.
    pub fn authority(&self) -> Result<GrantAuthority, RemoteError> {
        let invalid = || {
            RemoteError::new(
                "REMOTE_STATE_UNAVAILABLE",
                "Stored grant authority is invalid.",
            )
        };
        let agents = if self.agents == "all" {
            PermittedAgents::All
        } else {
            let ids: Vec<String> = serde_json::from_str(&self.agents).map_err(|_| invalid())?;
            if ids.len() > 256
                || ids.iter().any(|id| !canonical::is_core_id(id))
                || ids.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid());
            }
            PermittedAgents::Selected(ids)
        };
        let mode = match self.mode.as_str() {
            "direct" => DispatchMode::Direct,
            "hold" => DispatchMode::Hold,
            _ => return Err(invalid()),
        };
        if self
            .scopes
            .iter()
            .any(|scope| !SUPPORTED_SCOPES.contains(&scope.as_str()))
            || self.scopes.windows(2).any(|pair| pair[0] >= pair[1])
            || self.revision == 0
            || self.revision > 9_007_199_254_740_991
            || self.issued_at_ms > 9_007_199_254_740_991
            || self
                .expires_at_ms
                .is_some_and(|time| time > 9_007_199_254_740_991)
        {
            return Err(invalid());
        }
        Ok(GrantAuthority { agents, mode })
    }
    pub fn live_at(&self, now_ms: u64) -> bool {
        !self.disabled
            && self.expires_at_ms.is_none_or(|expiry| now_ms < expiry)
            && self.authority().is_ok()
    }
    pub fn permits_scope(&self, scope: &str) -> bool {
        self.scopes
            .binary_search_by(|value| value.as_str().cmp(scope))
            .is_ok()
    }
}
