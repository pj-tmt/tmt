use super::OfficeError;
use crate::office_deployment::WorldTarget;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use tmt_core::content_digest::is_sha256;
use tmt_core::content_digest::sha256;

const APPROVAL_LIMIT: usize = 2048;
pub(super) const RESPONSE_LIMIT: usize = 16 * 1024;
const MAX_TIMESTAMP: u64 = 8_640_000_000_000_000;

/// Public consent only. Serialize this allowlisted value, never a secret record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Approval {
    version: u8,
    pairing_id: String,
    world_id: String,
    installation_id: String,
    identity_id: String,
    installation_label: String,
    identity_label: String,
    capabilities: Vec<String>,
}

impl Approval {
    pub fn new(
        target: &WorldTarget,
        installation_id: &str,
        identity_id: &str,
        identity_label: &str,
        read_only: bool,
        proof: &Proof,
    ) -> Result<Self, OfficeError> {
        let value = Self {
            version: 1,
            pairing_id: proof.challenge(),
            world_id: target.world_id().into(),
            installation_id: installation_id.into(),
            identity_id: identity_id.into(),
            installation_label: "TMT installation".into(),
            identity_label: identity_label.into(),
            capabilities: if read_only {
                vec!["layout.read".into()]
            } else {
                vec!["layout.read".into(), "layout.write".into()]
            },
        };
        value.validate()?;
        Ok(value)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, OfficeError> {
        if bytes.len() > APPROVAL_LIMIT {
            return Err(OfficeError::CredentialsInvalid);
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), OfficeError> {
        if self.version != 1
            || !is_sha256(&self.pairing_id)
            || self.world_id.len() != 20
            || !self.world_id.bytes().all(|b| b.is_ascii_alphanumeric())
            || !valid_uuid(&self.installation_id)
            || !valid_uuid(&self.identity_id)
            || !valid_label(&self.installation_label)
            || !valid_label(&self.identity_label)
            || !(self.capabilities == ["layout.read"]
                || self.capabilities == ["layout.read", "layout.write"])
        {
            return Err(OfficeError::CredentialsInvalid);
        }
        Ok(())
    }

    pub fn approval_url(&self, target: &WorldTarget) -> Result<String, OfficeError> {
        self.validate()?;
        if self.world_id != target.world_id() {
            return Err(OfficeError::CredentialsInvalid);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| OfficeError::CredentialsInvalid)?;
        if bytes.len() > APPROVAL_LIMIT {
            return Err(OfficeError::CredentialsInvalid);
        }
        Ok(format!(
            "{}/worlds/{}/pair#tmt-pair={}",
            target.origin(),
            self.world_id,
            URL_SAFE_NO_PAD.encode(bytes)
        ))
    }

    pub fn pairing_id(&self) -> &str {
        &self.pairing_id
    }
    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }
    pub fn identity_id(&self) -> &str {
        &self.identity_id
    }

    pub fn world_id(&self) -> &str {
        &self.world_id
    }

    pub fn read_only(&self) -> bool {
        self.capabilities.len() == 1
    }
}

/// Entropy comes directly from the OS, not a UUID or a noncryptographic PRNG.
pub struct Proof([u8; 32]);

impl Proof {
    pub fn generate() -> Result<Self, OfficeError> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| OfficeError::CredentialsUnavailable)?;
        Ok(Self(bytes))
    }

    pub fn decode(secret: &str) -> Result<Self, OfficeError> {
        if secret.len() != 43 {
            return Err(OfficeError::CredentialsInvalid);
        }
        let bytes: [u8; 32] = URL_SAFE_NO_PAD
            .decode(secret)
            .map_err(|_| OfficeError::CredentialsInvalid)?
            .try_into()
            .map_err(|_| OfficeError::CredentialsInvalid)?;
        if URL_SAFE_NO_PAD.encode(bytes) != secret {
            return Err(OfficeError::CredentialsInvalid);
        }
        Ok(Self(bytes))
    }

    pub fn challenge(&self) -> String {
        sha256(&self.0)
    }
    pub fn secret(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }
}

/// Deserialize the exact flat issuer wire object before comparing any authority.
/// Do not use serde(flatten): it weakens duplicate/unknown field rejection.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClaimWire {
    version: u8,
    pairing_id: String,
    world_id: String,
    installation_id: String,
    identity_id: String,
    installation_label: String,
    identity_label: String,
    capabilities: Vec<String>,
    principal_uid: String,
    block_id: String,
    expires_at: u64,
    grant_expires_at: u64,
    custom_token: String,
}

pub struct Claim {
    pub principal_uid: String,
    pub block_id: String,
    pub grant_expires_at: u64,
    pub(super) custom_token: String,
}

/// An authenticated lease readback, not a new approval or credential.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Renewal {
    version: u8,
    pairing_id: String,
    world_id: String,
    installation_id: String,
    identity_id: String,
    principal_uid: String,
    block_id: String,
    capabilities: Vec<String>,
    grant_expires_at: u64,
}

impl Renewal {
    pub fn decode(
        bytes: &[u8],
        approval: &Approval,
        principal: &str,
        block: &str,
        expected_expiry: u64,
    ) -> Result<u64, OfficeError> {
        if bytes.len() > RESPONSE_LIMIT {
            return Err(OfficeError::CredentialsInvalid);
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
        if value.version != 1
            || value.pairing_id != approval.pairing_id
            || value.world_id != approval.world_id
            || value.installation_id != approval.installation_id
            || value.identity_id != approval.identity_id
            || value.capabilities != approval.capabilities
            || value.principal_uid != principal
            || value.block_id != block
            || value.grant_expires_at < expected_expiry
            || value.grant_expires_at > MAX_TIMESTAMP
        {
            return Err(OfficeError::CredentialsInvalid);
        }
        Ok(value.grant_expires_at)
    }
}

impl Claim {
    pub fn decode(bytes: &[u8], expected: &Approval, now_ms: u64) -> Result<Self, OfficeError> {
        if bytes.len() > RESPONSE_LIMIT {
            return Err(OfficeError::CredentialsInvalid);
        }
        expected.validate()?;
        let wire: ClaimWire =
            serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
        let approval = Approval {
            version: wire.version,
            pairing_id: wire.pairing_id,
            world_id: wire.world_id,
            installation_id: wire.installation_id,
            identity_id: wire.identity_id,
            installation_label: wire.installation_label,
            identity_label: wire.identity_label,
            capabilities: wire.capabilities,
        };
        if &approval != expected
            || !wire
                .principal_uid
                .strip_prefix("office-agent:")
                .is_some_and(valid_uuid)
            || !valid_uuid(&wire.block_id)
            || !(now_ms < wire.expires_at && wire.expires_at <= MAX_TIMESTAMP)
            || !(now_ms < wire.grant_expires_at && wire.grant_expires_at <= MAX_TIMESTAMP)
            || !valid_token(&wire.custom_token)
        {
            return Err(OfficeError::CredentialsInvalid);
        }
        Ok(Self {
            principal_uid: wire.principal_uid,
            block_id: wire.block_id,
            grant_expires_at: wire.grant_expires_at,
            custom_token: wire.custom_token,
        })
    }
}

pub(super) fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 8192 && value.bytes().all(|b| b.is_ascii_graphic())
}

pub(super) fn valid_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value)
}

fn valid_label(value: &str) -> bool {
    // Rust strings cannot contain surrogate scalars. Match ECMAScript trim's
    // BOM exception as well as whitespace, rather than accepting a blank label.
    value.chars().count() <= 80
        && !value
            .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
            .is_empty()
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
