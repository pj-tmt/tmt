//! One protected binding record; lifecycle hooks hold only its opaque scope key.

use super::{
    AgentCredential, Approval, Claim, OfficeError, Proof, vault::RECORD_LIMIT, wire::valid_uuid,
};
use crate::office_deployment::{DeploymentMode, OfficeDeployment, WorldTarget};
use serde::{Deserialize, Serialize};

const APPROVAL_MS: u64 = 300_000;
const CLAIM_INTERVAL_MS: u64 = 5000;
const RENEWAL_WINDOW_MS: u64 = 300_000;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairingRecord {
    version: u8,
    origin: String,
    mode: DeploymentMode,
    descriptor: String,
    approval: Approval,
    created_at: u64,
    expires_at: u64,
    phase: Phase,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Phase {
    Revoked {},
    Pending {
        secret: String,
        next_claim_at: u64,
    },
    Paired {
        principal_uid: String,
        block_id: String,
        grant_expires_at: u64,
        credential: AgentCredential,
    },
}

impl PairingRecord {
    pub(super) fn is_pending(&self) -> bool {
        matches!(self.phase, Phase::Pending { .. })
    }

    pub(super) fn is_revoked(&self) -> bool {
        matches!(self.phase, Phase::Revoked {})
    }

    pub(super) fn has_credentials(&self) -> bool {
        matches!(self.phase, Phase::Paired { .. })
    }
    /// A hook can locate a retired identity's record without resolving its old
    /// display name. The consumer verifies the derived scope key before use.
    pub(super) fn hook_target(bytes: &[u8]) -> Result<WorldTarget, OfficeError> {
        if bytes.len() > RECORD_LIMIT {
            return Err(OfficeError::CredentialsInvalid);
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
        WorldTarget::parse(
            &format!("{}/worlds/{}", value.origin, value.approval.world_id()),
            value.mode,
        )
        .map_err(|_| OfficeError::CredentialsInvalid)
    }

    pub(super) fn revoke(
        &mut self,
        target: &WorldTarget,
        deadline: std::time::Instant,
    ) -> Result<(), OfficeError> {
        let deployment = self.deployment(target)?;
        match &self.phase {
            Phase::Pending { secret, .. } => super::remote::cancel_pairing(
                &deployment,
                &Proof::decode(secret)?,
                &self.approval,
                deadline,
            )?,
            Phase::Paired { credential, .. } => {
                credential.revoke(&deployment, &self.approval, deadline)?;
            }
            Phase::Revoked {} => return Ok(()),
        }
        // Confirmed cancellation removes live secrets, but preserves a durable
        // receipt for retry if the SQLite acknowledgment fails afterwards.
        self.phase = Phase::Revoked {};
        Ok(())
    }

    pub fn refresh_if_needed(
        &mut self,
        target: &WorldTarget,
        now_ms: u64,
        deadline: std::time::Instant,
    ) -> Result<bool, OfficeError> {
        let deployment = self.deployment(target)?;
        match &mut self.phase {
            Phase::Paired {
                principal_uid,
                credential,
                ..
            } => {
                if credential.token_expires_at() > now_ms.saturating_add(30_000) {
                    return Ok(false);
                }
                *credential = credential.refresh(
                    &deployment,
                    &self.approval,
                    principal_uid,
                    now_ms,
                    deadline,
                )?;
                Ok(true)
            }
            Phase::Pending { .. } | Phase::Revoked {} => Err(OfficeError::NotPaired),
        }
    }

    pub fn renew_if_needed(
        &mut self,
        target: &WorldTarget,
        now_ms: u64,
        deadline: std::time::Instant,
    ) -> Result<bool, OfficeError> {
        let deployment = self.deployment(target)?;
        match &mut self.phase {
            Phase::Paired {
                principal_uid,
                block_id,
                grant_expires_at,
                credential,
            } => {
                if *grant_expires_at > now_ms.saturating_add(RENEWAL_WINDOW_MS) {
                    return Ok(false);
                }
                let bytes = credential.renew_grant(
                    &deployment,
                    &self.approval,
                    deadline,
                    *grant_expires_at,
                )?;
                let expiry = super::wire::Renewal::decode(
                    &bytes,
                    &self.approval,
                    principal_uid,
                    block_id,
                    *grant_expires_at,
                )?;
                let changed = expiry != *grant_expires_at;
                *grant_expires_at = expiry;
                Ok(changed)
            }
            Phase::Pending { .. } | Phase::Revoked {} => Err(OfficeError::NotPaired),
        }
    }

    pub fn inspect(
        &self,
        target: &WorldTarget,
        now_ms: u64,
        deadline: std::time::Instant,
    ) -> Result<bool, OfficeError> {
        match &self.phase {
            Phase::Paired {
                grant_expires_at,
                block_id,
                credential,
                ..
            } => {
                if now_ms >= *grant_expires_at {
                    return Err(OfficeError::PairingExpired);
                }
                credential.inspect_block(&self.deployment(target)?, block_id, deadline)
            }
            Phase::Pending { .. } | Phase::Revoked {} => Err(OfficeError::NotPaired),
        }
    }

    pub(super) fn block(
        &self,
        target: &WorldTarget,
        now_ms: u64,
        requested_id: Option<&str>,
        edit: Option<(&tmt_office_model::office_block::BlockLayout, u64)>,
        deadline: std::time::Instant,
    ) -> Result<tmt_office_model::codec::office_block::BlockSnapshot, OfficeError> {
        match &self.phase {
            Phase::Paired {
                grant_expires_at,
                block_id,
                credential,
                ..
            } => {
                if now_ms >= *grant_expires_at {
                    return Err(OfficeError::PairingExpired);
                }
                if requested_id.is_some_and(|id| id != block_id)
                    || (edit.is_some() && self.approval.read_only())
                {
                    return Err(OfficeError::RemoteDenied);
                }
                credential.block(&self.deployment(target)?, block_id, edit, deadline)
            }
            Phase::Pending { .. } | Phase::Revoked {} => Err(OfficeError::NotPaired),
        }
    }

    pub fn pending(
        deployment: &OfficeDeployment,
        approval: Approval,
        proof: Proof,
        now_ms: u64,
    ) -> Result<Self, OfficeError> {
        if approval.pairing_id() != proof.challenge() {
            return Err(OfficeError::CredentialsInvalid);
        }
        // This also validates the approval world before any protected publication.
        approval.approval_url(deployment.target())?;
        Ok(Self {
            version: 1,
            origin: deployment.target().origin().into(),
            mode: deployment.target().mode(),
            descriptor: String::from_utf8(deployment.encode_descriptor())
                .map_err(|_| OfficeError::CredentialsInvalid)?,
            approval,
            created_at: now_ms,
            expires_at: now_ms
                .checked_add(APPROVAL_MS)
                .ok_or(OfficeError::CredentialsInvalid)?,
            phase: Phase::Pending {
                secret: proof.secret(),
                next_claim_at: now_ms,
            },
        })
    }

    pub fn decode(
        bytes: &[u8],
        target: &WorldTarget,
        installation_id: &str,
        identity_id: &str,
    ) -> Result<Self, OfficeError> {
        if bytes.len() > RECORD_LIMIT {
            return Err(OfficeError::CredentialsInvalid);
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
        if value.version != 1
            || value.origin != target.origin()
            || value.mode != target.mode()
            || value.approval.installation_id() != installation_id
            || value.approval.identity_id() != identity_id
            || value.expires_at > 8_640_000_000_000_000
            || value.created_at.checked_add(APPROVAL_MS) != Some(value.expires_at)
        {
            return Err(OfficeError::CredentialsInvalid);
        }
        value.approval.approval_url(target)?;
        let deployment = value.deployment(target)?;
        match &value.phase {
            Phase::Revoked {} => {}
            Phase::Pending {
                secret,
                next_claim_at,
            } => {
                if Proof::decode(secret)?.challenge() != value.approval.pairing_id()
                    || *next_claim_at < value.created_at
                    || *next_claim_at > value.expires_at + CLAIM_INTERVAL_MS
                {
                    return Err(OfficeError::CredentialsInvalid);
                }
            }
            Phase::Paired {
                principal_uid,
                block_id,
                grant_expires_at,
                credential,
            } => {
                if !principal_uid
                    .strip_prefix("office-agent:")
                    .is_some_and(valid_uuid)
                    || !valid_uuid(block_id)
                    || *grant_expires_at <= value.created_at
                    || *grant_expires_at > 8_640_000_000_000_000
                {
                    return Err(OfficeError::CredentialsInvalid);
                }
                credential.validate(&deployment, &value.approval, principal_uid)?;
            }
        }
        Ok(value)
    }

    pub fn encode(&self) -> Result<Vec<u8>, OfficeError> {
        let bytes = serde_json::to_vec(self).map_err(|_| OfficeError::CredentialsInvalid)?;
        if bytes.len() > RECORD_LIMIT {
            return Err(OfficeError::CredentialsInvalid);
        }
        Ok(bytes)
    }

    pub fn deployment(&self, target: &WorldTarget) -> Result<OfficeDeployment, OfficeError> {
        OfficeDeployment::decode(target.clone(), self.descriptor.as_bytes())
            .map_err(|_| OfficeError::CredentialsInvalid)
    }

    pub fn approval(&self) -> &Approval {
        &self.approval
    }

    /// Persist the returned record before submitting the returned proof. This
    /// reserves cadence across crashes and concurrent invocations, not just sleeps.
    pub fn reserve_claim(&mut self, now_ms: u64) -> Result<Proof, OfficeError> {
        if now_ms >= self.expires_at {
            return Err(OfficeError::PairingExpired);
        }
        match &mut self.phase {
            Phase::Pending {
                secret,
                next_claim_at,
            } => {
                if now_ms < *next_claim_at {
                    return Err(OfficeError::PairingPending);
                }
                *next_claim_at = now_ms
                    .checked_add(CLAIM_INTERVAL_MS)
                    .ok_or(OfficeError::CredentialsInvalid)?;
                Proof::decode(secret)
            }
            Phase::Paired { .. } | Phase::Revoked {} => Err(OfficeError::CredentialsInvalid),
        }
    }

    /// Replacing this complete record is the only transition. The old protected
    /// proof remains recoverable until the replacement write is confirmed.
    pub fn complete(
        &mut self,
        claim: Claim,
        credential: AgentCredential,
        target: &WorldTarget,
    ) -> Result<(), OfficeError> {
        if !matches!(self.phase, Phase::Pending { .. }) {
            return Err(OfficeError::CredentialsInvalid);
        }
        credential.validate(
            &self.deployment(target)?,
            &self.approval,
            &claim.principal_uid,
        )?;
        self.phase = Phase::Paired {
            principal_uid: claim.principal_uid,
            block_id: claim.block_id,
            grant_expires_at: claim.grant_expires_at,
            credential,
        };
        Ok(())
    }

    pub fn local_state(&self, now_ms: u64) -> &'static str {
        match &self.phase {
            Phase::Revoked {} => "revoked",
            Phase::Pending { .. } if now_ms < self.expires_at => "pending",
            Phase::Paired {
                grant_expires_at, ..
            } if now_ms < *grant_expires_at => "credential",
            _ => "expired",
        }
    }
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
