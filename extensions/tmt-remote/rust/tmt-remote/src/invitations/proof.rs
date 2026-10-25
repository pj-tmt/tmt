//! One transcript owner for every invitation adapter. The existing Ed25519,
//! HMAC, possession and response primitives retain their exact algorithms.
use super::{Candidate, Context, invalid};
use crate::{canonical, crypto, error::RemoteError, limits};
use sha2::{Digest, Sha256};

/// Version 2 is disjoint from both local-browser pairing and v1 Firestore codes.
/// Length framing makes backend/reference/principal substitution unambiguous.
pub fn enrollment(context: &Context, candidate: &Candidate) -> Result<Vec<u8>, RemoteError> {
    context.validate()?;
    let encode = || -> canonical::Result<Vec<u8>> {
        canonical::uuid(&candidate.machine)?;
        canonical::require(candidate.machine != context.owner)?;
        crypto::public_key(&candidate.public_key)?;
        canonical::require(canonical::device_name(&candidate.name))?;
        canonical::require(
            !candidate.principal.is_empty()
                && candidate.principal.len() <= limits::CLOUD_ENROLLMENT_BYTES,
        )?;
        let bytes = canonical::framed(&[
            b"tmt-invitation-v2",
            b"enrollment",
            context.binding.backend.as_bytes(),
            &context.binding.reference,
            &context.binding.owner_principal,
            context.owner.as_bytes(),
            &context.owner_key,
            context.offer.as_bytes(),
            &context.nonce,
            context.issued_at_ms.to_string().as_bytes(),
            context.expires_at_ms.to_string().as_bytes(),
            candidate.machine.as_bytes(),
            &candidate.public_key,
            &candidate.principal,
            candidate.name.as_bytes(),
            &candidate.nonce,
        ])?;
        canonical::require(bytes.len() <= limits::CLOUD_ENROLLMENT_BYTES)?;
        Ok(bytes)
    };
    encode().map_err(|_| invalid())
}

/// Proof admission alone cannot construct a member, grant or receipt.
pub struct VerifiedCandidate {
    pub(super) context: Context,
    pub(super) candidate: Candidate,
    pub(super) digest: [u8; 32],
}

impl VerifiedCandidate {
    pub fn context(&self) -> &Context {
        &self.context
    }
    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    pub fn words(&self) -> Result<[&'static str; 4], RemoteError> {
        canonical::fingerprint_words(&self.candidate.public_key).map_err(|_| invalid())
    }
}

pub fn verify_enrollment(
    context: &Context,
    candidate: &Candidate,
    authenticated_principal: &[u8],
    code: &[u8; 16],
    mac: &[u8; 32],
    signature: &[u8; 64],
    now_ms: u64,
) -> Result<VerifiedCandidate, RemoteError> {
    if now_ms >= context.expires_at_ms
        || context.issued_at_ms.saturating_sub(now_ms)
            > crate::session::CLOCK_SKEW.as_millis() as u64
        || candidate.principal != authenticated_principal
    {
        return Err(invalid());
    }
    let bytes = enrollment(context, candidate)?;
    crypto::verify_mac(code, &bytes, mac).map_err(|_| invalid())?;
    let possession = canonical::possession(&bytes, mac).map_err(|_| invalid())?;
    crypto::verify_signature(&candidate.public_key, &possession, signature)
        .map_err(|_| invalid())?;
    Ok(VerifiedCandidate {
        context: context.clone(),
        candidate: candidate.clone(),
        digest: Sha256::digest(&possession).into(),
    })
}
