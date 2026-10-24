//! Bounded native enrollment metadata. Invite secrets exist only in the terminal
//! token; a mailbox request contains public identity, MAC and possession proof.
use super::{Candidate, Challenge, Reroute};
use crate::{canonical, crypto, error::RemoteError, limits, state::MachineKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_PAIR_INVALID",
        "The invite could not be verified. Check the code with the owner.",
    )
}
pub(super) fn expired() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_PAIR_EXPIRED",
        "The invite expired. Ask the owner for a new code.",
    )
}
pub(super) fn upgrade() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_UPGRADE_REQUIRED",
        "This connection needs a newer Remote. Run tmt upgrade; your pairing is kept.",
    )
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, RemoteError> {
    if bytes.len() > limits::CLOUD_ENROLLMENT_BYTES {
        return Err(invalid());
    }
    let value = crate::wire::strict_json(bytes).ok_or_else(invalid)?;
    if value["version"].as_u64().is_some_and(|v| v != 1) {
        return Err(upgrade());
    }
    serde_json::from_value(value).map_err(|_| invalid())
}
fn bytes<const N: usize>(value: &str) -> Result<[u8; N], RemoteError> {
    canonical::base64url_bytes(value, N)
        .map_err(|_| invalid())?
        .try_into()
        .map_err(|_| invalid())
}

/// Do not derive Debug: the token contains the one-time secret.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Invite {
    version: u64,
    pub challenge: Challenge,
    pub public_config: Value,
    pub machine_name: String,
    code: [u8; 16],
}
impl Drop for Invite {
    fn drop(&mut self) {
        self.code.fill(0);
    }
}
impl Invite {
    pub fn new(
        project: &str,
        owner: &str,
        key: &[u8; 32],
        config: Value,
        name: &str,
        now: u64,
    ) -> Result<Self, RemoteError> {
        let mut code = [0; 16];
        let mut nonce = [0; 16];
        getrandom::fill(&mut code)
            .and_then(|()| getrandom::fill(&mut nonce))
            .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain invite entropy."))?;
        let expires_at_ms = now
            .checked_add(crate::pairing::Timing::CONTRACT.lifetime.as_millis() as u64)
            .filter(|v| *v < 1 << 53)
            .ok_or_else(invalid)?;
        let invite = Self {
            version: 1,
            challenge: Challenge {
                project: project.into(),
                owner: owner.into(),
                owner_key: *key,
                offer: crate::store::uuid_v4()?,
                nonce,
                generation: 1,
                expires_at_ms,
            },
            public_config: config,
            machine_name: name.into(),
            code,
        };
        invite.validate(now)?;
        Ok(invite)
    }
    fn validate(&self, now: u64) -> Result<(), RemoteError> {
        if self.version != 1
            || self.challenge.generation != 1
            || !canonical::device_name(&self.machine_name)
        {
            return Err(invalid());
        }
        canonical::uuid(&self.challenge.owner)
            .and_then(|()| canonical::uuid(&self.challenge.offer))
            .map_err(|_| invalid())?;
        crypto::public_key(&self.challenge.owner_key).map_err(|_| invalid())?;
        let project = crate::firebase_auth::Project::from_public_config(&self.public_config)
            .map_err(|_| invalid())?;
        if project.project_id != self.challenge.project || self.challenge.expires_at_ms >= 1 << 53 {
            return Err(invalid());
        }
        if now >= self.challenge.expires_at_ms {
            return Err(expired());
        }
        if self.challenge.expires_at_ms - now
            > crate::pairing::Timing::CONTRACT.lifetime.as_millis() as u64
                + crate::session::CLOCK_SKEW.as_millis() as u64
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn token(&self) -> Result<String, RemoteError> {
        let raw = serde_json::to_vec(self).map_err(|_| invalid())?;
        if raw.len() > limits::CLOUD_INVITE_BYTES {
            return Err(invalid());
        }
        Ok(canonical::base64url(&raw))
    }
    pub fn read(text: &str, now: u64) -> Result<Self, RemoteError> {
        if text.len() > limits::CLOUD_INVITE_BYTES * 4 / 3 + 4 {
            return Err(invalid());
        }
        let raw = canonical::base64url_decode(text).map_err(|_| invalid())?;
        let invite: Self = decode(&raw)?;
        invite.validate(now)?;
        Ok(invite)
    }
    pub fn submit(
        &self,
        candidate: Candidate,
        key: &MachineKey,
    ) -> Result<Submission, RemoteError> {
        if candidate.public_key != key.public() {
            return Err(invalid());
        }
        let transcript = super::enrollment(&self.challenge, &candidate).map_err(|_| invalid())?;
        let mac = crypto::enrollment_mac(&self.code, &transcript);
        let signature = key.sign(&canonical::possession(&transcript, &mac).map_err(|_| invalid())?);
        Ok(Submission {
            candidate,
            mac,
            signature,
        })
    }
    pub fn verify(
        &self,
        submission: &Submission,
        uid: &str,
        now: u64,
    ) -> Result<super::VerifiedCandidate, RemoteError> {
        self.validate(now)?;
        super::verify_enrollment(
            &self.challenge,
            &submission.candidate,
            uid,
            &self.code,
            &submission.mac,
            &submission.signature,
            now,
        )
        .map_err(|_| invalid())
    }
    pub fn response_key(&self, candidate: &Candidate) -> Result<[u8; 32], RemoteError> {
        super::response_key(&self.code, &self.challenge, candidate).map_err(|_| invalid())
    }
}

pub struct Submission {
    pub candidate: Candidate,
    pub mac: [u8; 32],
    pub signature: [u8; 64],
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Join {
    version: u64,
    kind: String,
    offer: String,
    peer: String,
    public_key: String,
    uid: String,
    name: String,
    nonce: String,
    mac: String,
    signature: String,
}
impl Submission {
    pub fn encode(&self, offer: &str) -> Result<Vec<u8>, RemoteError> {
        let c = &self.candidate;
        serde_json::to_vec(&json!({"version":1,"kind":"join","offer":offer,"peer":c.peer,"publicKey":canonical::base64url(&c.public_key),"uid":c.uid,"name":c.name,"nonce":canonical::base64url(&c.nonce),"mac":canonical::base64url(&self.mac),"signature":canonical::base64url(&self.signature)})).map_err(|_|invalid())
    }
    pub fn read(raw: &[u8], offer: &str) -> Result<Self, RemoteError> {
        let row: Join = decode(raw)?;
        if row.version != 1 || row.kind != "join" || row.offer != offer {
            return Err(invalid());
        }
        Ok(Self {
            candidate: Candidate {
                peer: row.peer,
                public_key: bytes(&row.public_key)?,
                uid: row.uid,
                name: row.name,
                nonce: bytes(&row.nonce)?,
            },
            mac: bytes(&row.mac)?,
            signature: bytes(&row.signature)?,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Route {
    version: u64,
    kind: String,
    machine_id: String,
    owner_key: String,
    peer: String,
    uid: String,
    generation: u64,
    timestamp_ms: u64,
    signature: String,
}
pub fn route_request(request: &Reroute, key: &MachineKey) -> Result<Vec<u8>, RemoteError> {
    let signature = key.sign(&super::reroute(request).map_err(|_| invalid())?);
    serde_json::to_vec(&Route {
        version: 1,
        kind: "reroute".into(),
        machine_id: request.machine_id.clone(),
        owner_key: canonical::base64url(&request.owner_key),
        peer: request.peer.clone(),
        uid: request.uid.clone(),
        generation: request.generation,
        timestamp_ms: request.timestamp_ms,
        signature: canonical::base64url(&signature),
    })
    .map_err(|_| invalid())
}
pub fn read_route(
    raw: &[u8],
    uid: &str,
    key: &[u8; 32],
    created: u64,
) -> Result<super::VerifiedReroute, RemoteError> {
    let row: Route = decode(raw)?;
    if row.version != 1 || row.kind != "reroute" {
        return Err(invalid());
    }
    let request = Reroute {
        machine_id: row.machine_id,
        owner_key: bytes(&row.owner_key)?,
        peer: row.peer,
        uid: row.uid,
        generation: row.generation,
        timestamp_ms: row.timestamp_ms,
    };
    super::verify_reroute(&request, uid, key, &bytes(&row.signature)?, created)
        .map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_and_submission_refuse_duplicate_fields_versions_and_uid_substitution() {
        let (challenge, candidate, _) = super::super::tests::fixture();
        let invite = Invite {
            version: 1,
            challenge,
            public_config: json!({"projectId":"demo-pairing-test","appId":"1:123:web:test","apiKey":"public-test"}),
            machine_name: "Owner".into(),
            code: [4; 16],
        };
        let token = invite.token().unwrap();
        let decoded = Invite::read(&token, 1).unwrap();
        assert_eq!(decoded.challenge, invite.challenge);
        assert!(Invite::read(&token, invite.challenge.expires_at_ms).is_err());
        let raw = br#"{"version":1,"version":1}"#;
        assert!(Submission::read(raw, &invite.challenge.offer).is_err());
        let mut value = serde_json::to_value(&invite).unwrap();
        value["version"] = json!(2);
        assert!(
            Invite::read(
                &canonical::base64url(&serde_json::to_vec(&value).unwrap()),
                1
            )
            .is_err()
        );
        assert!(
            Submission::read(
                &vec![b' '; limits::CLOUD_ENROLLMENT_BYTES + 1],
                &invite.challenge.offer
            )
            .is_err()
        );
        assert!(canonical::fingerprint_words(&candidate.public_key).is_ok());
    }
}
