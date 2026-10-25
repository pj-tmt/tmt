//! Explicit Firestore binding of the shared invitation profile. Version 1 keeps
//! its own decoder and proof; neither profile is reinterpreted as the other.
use crate::{
    canonical, crypto,
    error::RemoteError,
    invitations::{self, Context, port, service::Delivery},
    limits,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
#[path = "v2_tests.rs"]
mod tests;

/// Deliberately no Debug: the private token carries the one-time code.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Invite {
    version: u64,
    pub context: Context,
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
    pub fn from_delivery(
        delivery: &Delivery,
        public_config: Value,
        machine_name: String,
        now: u64,
    ) -> Result<Self, RemoteError> {
        let invite = delivery.with_code(|code| Self {
            version: 2,
            context: delivery.context.clone(),
            public_config,
            machine_name,
            code: *code,
        });
        invite.validate(now)?;
        Ok(invite)
    }
    fn validate(&self, now: u64) -> Result<(), RemoteError> {
        if self.version != 2 {
            return Err(super::upgrade());
        }
        validate_context(&self.context)?;
        let project = crate::firebase_auth::Project::from_public_config(&self.public_config)
            .map_err(|_| super::invalid())?;
        if project.project_id.as_bytes() != self.context.binding.reference
            || !canonical::device_name(&self.machine_name)
            || self.context.issued_at_ms.saturating_sub(now)
                > crate::session::CLOCK_SKEW.as_millis() as u64
        {
            return Err(super::invalid());
        }
        if now >= self.context.expires_at_ms {
            return Err(super::expired());
        }
        Ok(())
    }
    pub fn token(&self) -> Result<String, RemoteError> {
        let raw = serde_json::to_vec(self).map_err(|_| super::invalid())?;
        if raw.len() > limits::CLOUD_INVITE_BYTES {
            return Err(super::invalid());
        }
        Ok(canonical::base64url(&raw))
    }
    pub fn read(text: &str, now: u64) -> Result<Self, RemoteError> {
        if text.len() > limits::CLOUD_INVITE_BYTES * 4 / 3 + 4 {
            return Err(super::invalid());
        }
        let raw = canonical::base64url_decode(text).map_err(|_| super::invalid())?;
        if raw.len() > limits::CLOUD_INVITE_BYTES {
            return Err(super::invalid());
        }
        let value = crate::wire::strict_json(&raw).ok_or_else(super::invalid)?;
        if value["version"]
            .as_u64()
            .is_some_and(|version| version != 2)
        {
            return Err(super::upgrade());
        }
        let invite: Self = serde_json::from_value(value).map_err(|_| super::invalid())?;
        invite.validate(now)?;
        Ok(invite)
    }
    pub fn with_code<T>(&self, use_code: impl FnOnce(&[u8; 16]) -> T) -> T {
        use_code(&self.code)
    }
}

pub fn validate_context(context: &Context) -> Result<(), RemoteError> {
    context.validate()?;
    let project = std::str::from_utf8(&context.binding.reference).map_err(|_| super::invalid())?;
    let uid =
        std::str::from_utf8(&context.binding.owner_principal).map_err(|_| super::invalid())?;
    if context.binding.backend != "firestore"
        || !crate::deploy_run::project_ok(project)
        || !super::super::uid_ok(uid)
    {
        return Err(super::invalid());
    }
    Ok(())
}

/// Scoped response-key derivation using the existing forward/reverse framing;
/// no private key bytes cross the invitation adapter port.
pub fn reroute_tokens(
    context: &Context,
    candidate: &invitations::Candidate,
    key: &port::ReceiptKey,
) -> Result<([u8; 32], [u8; 32]), RemoteError> {
    validate_context(context)?;
    invitations::enrollment(context, candidate)?;
    let project = std::str::from_utf8(&context.binding.reference).map_err(|_| super::invalid())?;
    Ok((
        key.capability(&super::reroute_context(
            project,
            &context.owner,
            &candidate.machine,
            false,
        )?),
        key.capability(&super::reroute_context(
            project,
            &context.owner,
            &candidate.machine,
            true,
        )?),
    ))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Enrollment {
    version: u64,
    kind: String,
    context: Context,
    candidate: invitations::Candidate,
    mac: String,
    signature: String,
}
pub fn submission(context: &Context, submitted: &port::Submission) -> Result<Vec<u8>, RemoteError> {
    validate_context(context)?;
    let transcript = invitations::enrollment(context, &submitted.candidate)?;
    let possession =
        canonical::possession(&transcript, &submitted.mac).map_err(|_| super::invalid())?;
    crypto::verify_signature(
        &submitted.candidate.public_key,
        &possession,
        &submitted.signature,
    )
    .map_err(|_| super::invalid())?;
    bounded(&Enrollment {
        version: 2,
        kind: "join".into(),
        context: context.clone(),
        candidate: submitted.candidate.clone(),
        mac: canonical::base64url(&submitted.mac),
        signature: canonical::base64url(&submitted.signature),
    })
}
pub fn read_submission(
    bytes: &[u8],
    context: &Context,
    authenticated_uid: &str,
    gate: [u8; 32],
) -> Result<port::Submission, RemoteError> {
    validate_context(context)?;
    let raw: Enrollment = read(bytes)?;
    if raw.version != 2
        || raw.kind != "join"
        || raw.context != *context
        || raw.candidate.principal != authenticated_uid.as_bytes()
        || !super::super::uid_ok(authenticated_uid)
    {
        return Err(super::invalid());
    }
    let submitted = port::Submission {
        candidate: raw.candidate,
        gate,
        mac: super::bytes(&raw.mac)?,
        signature: super::bytes(&raw.signature)?,
    };
    // Native possession is also required for an exact lost-publication read;
    // enrollment admission still additionally checks the memory-only code MAC.
    submission(context, &submitted)?;
    Ok(submitted)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u64,
    kind: String,
    snapshot: String,
    receipt: String,
    proof: String,
}
pub fn receipt(
    snapshot: &[u8; 32],
    bytes: &[u8],
    proof: &[u8; 32],
) -> Result<Vec<u8>, RemoteError> {
    if bytes.is_empty() || bytes.len() > limits::CLOUD_ENROLLMENT_BYTES {
        return Err(super::invalid());
    }
    bounded(&Response {
        version: 2,
        kind: "receipt".into(),
        snapshot: canonical::base64url(snapshot),
        receipt: canonical::base64url(bytes),
        proof: canonical::base64url(proof),
    })
}
pub fn read_receipt(bytes: &[u8]) -> Result<port::Payload, RemoteError> {
    let response: Response = read(bytes)?;
    if response.version != 2 || response.kind != "receipt" {
        return Err(super::invalid());
    }
    let receipt = canonical::base64url_decode(&response.receipt).map_err(|_| super::invalid())?;
    if receipt.is_empty() || receipt.len() > limits::CLOUD_ENROLLMENT_BYTES {
        return Err(super::invalid());
    }
    Ok(port::Payload::Receipt {
        snapshot: super::bytes(&response.snapshot)?,
        bytes: receipt,
        proof: super::bytes(&response.proof)?,
    })
}
fn bounded(value: &impl Serialize) -> Result<Vec<u8>, RemoteError> {
    let bytes = serde_json::to_vec(value).map_err(|_| super::invalid())?;
    if bytes.len() > limits::CLOUD_ENROLLMENT_BYTES {
        return Err(super::invalid());
    }
    Ok(bytes)
}
fn read<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, RemoteError> {
    if bytes.len() > limits::CLOUD_ENROLLMENT_BYTES {
        return Err(super::invalid());
    }
    serde_json::from_value(crate::wire::strict_json(bytes).ok_or_else(super::invalid)?)
        .map_err(|_| super::invalid())
}
