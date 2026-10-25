//! Typed configured Digest input. Policy precedence remains in Core effective().
use serde::{Deserialize, Serialize};
use tmt_core::{dispatch::canonical_id, limits::MAX_JS_SAFE_INTEGER, orchestration::*};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum DigestReceiving {
    Direct {},
    Auto {},
    Batch {
        #[serde(rename = "intervalMs")]
        interval_ms: u64,
    },
}
impl DigestReceiving {
    fn valid(&self) -> bool {
        !matches!(self, Self::Batch { interval_ms } if *interval_ms == 0 || *interval_ms > MAX_JS_SAFE_INTEGER)
    }
    pub(crate) fn to_core(&self) -> OrchReceiving {
        match self {
            Self::Direct {} => OrchReceiving::Direct,
            Self::Auto {} => OrchReceiving::Auto,
            Self::Batch { interval_ms } => OrchReceiving::Batch(*interval_ms),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DigestReceivingMember {
    pub identity_id: String,
    pub receiving: DigestReceiving,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DigestReceivingSettings {
    pub enabled: bool,
    pub default_receiving: DigestReceiving,
    pub members: Vec<DigestReceivingMember>,
}
impl DigestReceivingSettings {
    pub fn normalize(&mut self) -> Result<(), OrchError> {
        if !self.default_receiving.valid() || self.members.len() > 256 {
            return Err(OrchError::Invalid);
        }
        self.members
            .sort_by(|a, b| a.identity_id.cmp(&b.identity_id));
        if self
            .members
            .iter()
            .any(|m| !canonical_id(&m.identity_id) || !m.receiving.valid())
            || self
                .members
                .windows(2)
                .any(|pair| pair[0].identity_id == pair[1].identity_id)
        {
            return Err(OrchError::Invalid);
        }
        Ok(())
    }
    pub(crate) fn apply(&self, snapshot: &mut OrchSnapshot, revision: u64) {
        snapshot.digest_enabled = self.enabled;
        snapshot.digest_revision = revision;
        snapshot.base_receiving = snapshot
            .identities
            .iter()
            .map(|id| {
                let mode = self
                    .members
                    .iter()
                    .find(|m| m.identity_id == id.id)
                    .map_or(&self.default_receiving, |m| &m.receiving);
                (
                    id.id.clone(),
                    OrchBaseReceiving {
                        revision,
                        receiving: mode.to_core(),
                    },
                )
            })
            .collect();
    }
}
