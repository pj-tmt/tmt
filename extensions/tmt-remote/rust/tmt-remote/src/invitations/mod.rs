//! Cross-person invitation proofs and lifecycle. Provider evidence carries no
//! owner consent: a verified candidate still requires an admitted confirmation.
use crate::{canonical, crypto, error::RemoteError, limits};

pub(crate) mod history;
pub mod port;
mod proof;
pub mod service;
pub mod store;
pub mod submission;
pub(crate) use proof::possession_digest;
pub use proof::{VerifiedCandidate, enrollment, verify_enrollment};

#[cfg(test)]
mod backend_tests;
#[cfg(test)]
mod legacy_tests;
#[cfg(test)]
mod tests;

/// Public routing context. These exact bytes are signed/MAC-bound; neither a
/// display alias nor a provider principal establishes native authority.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendBinding {
    pub backend: String,
    pub reference: Vec<u8>,
    pub owner_principal: Vec<u8>,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub binding: BackendBinding,
    pub owner: String,
    pub owner_key: [u8; 32],
    pub offer: String,
    pub nonce: [u8; 16],
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub machine: String,
    pub public_key: [u8; 32],
    pub principal: Vec<u8>,
    pub name: String,
    pub nonce: [u8; 16],
}

pub(super) fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_INVITATION_INVALID",
        "The invitation could not be verified. Compare the code with its owner.",
    )
}

impl Context {
    pub fn validate(&self) -> Result<(), RemoteError> {
        let valid = || -> canonical::Result<()> {
            canonical::require(canonical::extension_name(&self.binding.backend))?;
            for bytes in [&self.binding.reference, &self.binding.owner_principal] {
                canonical::require(
                    !bytes.is_empty() && bytes.len() <= limits::CLOUD_ENROLLMENT_BYTES,
                )?;
            }
            canonical::uuid(&self.owner)?;
            canonical::uuid(&self.offer)?;
            crypto::public_key(&self.owner_key)?;
            canonical::require(self.issued_at_ms < self.expires_at_ms)?;
            canonical::require(self.expires_at_ms < 1 << 53)?;
            canonical::require(
                self.expires_at_ms - self.issued_at_ms
                    <= crate::pairing::Timing::CONTRACT.lifetime.as_millis() as u64,
            )
        };
        valid().map_err(|_| invalid())
    }
}
