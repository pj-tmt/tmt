//! Native cloud enrollment proofs. These bytes grant no operation authority;
//! only the local owner's later terminal confirmation may create a grant.
use crate::{
    canonical::{self, Result, framed, require},
    crypto,
};
use serde::{Deserialize, Serialize};

mod provider;
mod store;
pub mod wire;
pub use provider::{EnrollmentDocument, EnrollmentProof};
pub mod service;
pub use store::{Connection, PairReceipt, RouteChange};

#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod tests;

/// Fresh owner-authored challenge, scoped to one deployment and native identity.
/// The adapter must verify the owner signature/pin before using this context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub project: String,
    pub owner: String,
    pub owner_key: [u8; 32],
    pub offer: String,
    pub nonce: [u8; 16],
    pub generation: u64,
    pub expires_at_ms: u64,
}

/// The Firebase uid comes from the Rules-pinned submitting document. It is
/// transport routing data, while `peer` and `public_key` are stable native identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub peer: String,
    pub public_key: [u8; 32],
    pub uid: String,
    pub name: String,
    pub nonce: [u8; 16],
}

/// Produced only after both the code MAC and native possession proof pass.
pub struct VerifiedCandidate {
    challenge: Challenge,
    candidate: Candidate,
    digest: [u8; 32],
}
impl VerifiedCandidate {
    pub fn challenge(&self) -> &Challenge {
        &self.challenge
    }
    pub fn candidate(&self) -> &Candidate {
        &self.candidate
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    pub fn words(&self) -> Result<[&'static str; 4]> {
        canonical::fingerprint_words(&self.candidate.public_key)
    }
}

/// Domain-separated transcript: a local-browser enrollment cannot be replayed
/// into cloud pairing, nor can an invite proof be replayed as a uid reroute.
fn transcript(kind: &[u8], challenge: &Challenge, candidate: &Candidate) -> Result<Vec<u8>> {
    require(crate::deploy_run::project_ok(&challenge.project))?;
    canonical::uuid(&challenge.owner)?;
    canonical::uuid(&challenge.offer)?;
    canonical::uuid(&candidate.peer)?;
    require(candidate.peer != challenge.owner)?;
    crypto::public_key(&challenge.owner_key)?;
    crypto::public_key(&candidate.public_key)?;
    require(canonical::device_name(&candidate.name) && uid_ok(&candidate.uid))?;
    require(challenge.generation > 0 && challenge.generation < (1 << 53))?;
    require(challenge.expires_at_ms > 0 && challenge.expires_at_ms < (1 << 53))?;
    framed(&[
        b"tmt-cloud-pair-v1",
        kind,
        challenge.project.as_bytes(),
        challenge.owner.as_bytes(),
        &challenge.owner_key,
        challenge.offer.as_bytes(),
        &challenge.nonce,
        challenge.generation.to_string().as_bytes(),
        challenge.expires_at_ms.to_string().as_bytes(),
        candidate.peer.as_bytes(),
        &candidate.public_key,
        candidate.uid.as_bytes(),
        candidate.name.as_bytes(),
        &candidate.nonce,
    ])
}

/// Firebase accepts uid strings up to 128 characters. Routing uses exact values;
/// this profile also excludes path separators and controls before path building.
pub(crate) fn uid_ok(uid: &str) -> bool {
    !uid.is_empty()
        && uid.len() <= 128
        && uid != "."
        && uid != ".."
        && !uid.chars().any(|c| c.is_control() || c == '/')
}

pub fn enrollment(challenge: &Challenge, candidate: &Candidate) -> Result<Vec<u8>> {
    transcript(b"join", challenge, candidate)
}

pub fn verify_enrollment(
    challenge: &Challenge,
    candidate: &Candidate,
    authenticated_uid: &str,
    code: &[u8; 16],
    mac: &[u8; 32],
    signature: &[u8; 64],
    now: u64,
) -> Result<VerifiedCandidate> {
    require(now < challenge.expires_at_ms && authenticated_uid == candidate.uid)?;
    let bytes = enrollment(challenge, candidate)?;
    crypto::verify_mac(code, &bytes, mac)?;
    let digest = possession_digest(challenge, candidate, mac, signature)?;
    Ok(VerifiedCandidate {
        challenge: challenge.clone(),
        candidate: candidate.clone(),
        digest,
    })
}

/// Native possession alone is not enrollment. On a lost-join observation the
/// Store must also match this digest to an already-confirmed, unexpired receipt.
fn possession_digest(
    challenge: &Challenge,
    candidate: &Candidate,
    mac: &[u8; 32],
    signature: &[u8; 64],
) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let bytes = canonical::possession(&enrollment(challenge, candidate)?, mac)?;
    crypto::verify_signature(&candidate.public_key, &bytes, signature)?;
    Ok(Sha256::digest(&bytes).into())
}

/// The receipt proof reuses the manual ceremony's response-key primitive over
/// the distinct cloud transcript. It authenticates exact frozen receipt bytes.
pub fn response_key(
    code: &[u8; 16],
    challenge: &Challenge,
    candidate: &Candidate,
) -> Result<[u8; 32]> {
    crypto::response_key(code, &enrollment(challenge, candidate)?)
}

/// One-shot uid reassociation, signed by the already-confirmed native peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reroute {
    pub machine_id: String,
    pub owner_key: [u8; 32],
    pub peer: String,
    pub uid: String,
    pub generation: u64,
    pub timestamp_ms: u64,
}

pub fn reroute(request: &Reroute) -> Result<Vec<u8>> {
    canonical::uuid(&request.machine_id)?;
    canonical::uuid(&request.peer)?;
    require(request.machine_id != request.peer && uid_ok(&request.uid))?;
    crypto::public_key(&request.owner_key)?;
    require(request.generation > 0 && request.generation < 1 << 53)?;
    require(request.timestamp_ms < 1 << 53)?;
    framed(&[
        b"tmt-cloud-pair-v1",
        b"reroute",
        request.machine_id.as_bytes(),
        &request.owner_key,
        request.peer.as_bytes(),
        request.uid.as_bytes(),
        request.generation.to_string().as_bytes(),
        request.timestamp_ms.to_string().as_bytes(),
    ])
}

/// Signature and Rules-pinned uid verified; Store still rechecks current grant,
/// owner pin and strictly increasing generation inside the writer transaction.
pub struct VerifiedReroute {
    request: Reroute,
    public_key: [u8; 32],
}
impl VerifiedReroute {
    pub fn request(&self) -> &Reroute {
        &self.request
    }
}
pub fn verify_reroute(
    request: &Reroute,
    authenticated_uid: &str,
    pinned_key: &[u8; 32],
    signature: &[u8; 64],
    server_create_time_ms: u64,
) -> Result<VerifiedReroute> {
    require(authenticated_uid == request.uid)?;
    require(
        request.timestamp_ms.abs_diff(server_create_time_ms)
            <= crate::session::CLOCK_SKEW.as_millis() as u64,
    )?;
    crypto::verify_signature(pinned_key, &reroute(request)?, signature)?;
    Ok(VerifiedReroute {
        request: request.clone(),
        public_key: *pinned_key,
    })
}
