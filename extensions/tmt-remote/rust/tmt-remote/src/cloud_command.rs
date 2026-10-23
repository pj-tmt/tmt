//! One provisional v1 public-command schema (#4536); Core alignment changes this owner.
use crate::{canonical, error::RemoteError, limits, wire};
use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerIdentity {
    pub id: String,
    pub name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    #[serde(skip)]
    fingerprint: [u8; 32],
    pub version: u64,
    pub remote: Option<String>,
    pub request_id: String,
    pub created_at_ms: u64,
    pub from: PeerIdentity,
    pub target: PeerIdentity,
    pub message: String,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Sent,
    Held,
    Refused,
    Unconfirmed,
    Unavailable,
}
#[derive(Serialize)]
pub struct Answer {
    pub version: u64,
    pub state: State,
    pub remote: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
impl Request {
    pub fn read(bytes: &[u8]) -> Result<Self, RemoteError> {
        if bytes.len() > limits::CLOUD_COMMAND_BYTES {
            return Err(too_large());
        }
        let value = wire::strict_json(bytes).ok_or_else(invalid)?;
        if value["version"].as_u64().is_some_and(|v| v != 1) {
            return Err(upgrade());
        }
        if !value
            .get("remote")
            .is_some_and(|v| v.is_null() || v.is_string())
        {
            return Err(invalid());
        }
        let mut request: Self = serde_json::from_value(value).map_err(|_| invalid())?;
        if request.version != 1 || request.created_at_ms > crate::cloud_hpke::INTEGER_MAX {
            return Err(invalid());
        }
        canonical::uuid(&request.request_id).map_err(|_| invalid())?;
        for identity in [&request.from, &request.target] {
            canonical::uuid(&identity.id).map_err(|_| invalid())?;
            if identity.name.is_empty()
                || identity.name.len() > 128
                || identity.name.chars().any(char::is_control)
            {
                return Err(invalid());
            }
        }
        if request
            .remote
            .as_deref()
            .is_some_and(|r| r != "local" && !crate::remote_catalog::name_ok(r))
        {
            return Err(invalid());
        }
        if request.message.len() > limits::CLOUD_MESSAGE_BYTES {
            return Err(too_large());
        }
        use sha2::Digest;
        request.fingerprint = sha2::Sha256::digest(bytes).into();
        Ok(request)
    }
    /// Core's prepared identity describes the remote sender, never a local owner identity.
    pub fn dispatch(&self) -> serde_json::Value {
        serde_json::json!({"version":1,"operation":"dispatch.create","originator":"anonymous","requestDigest":crate::canonical::base64url(&self.fingerprint),"input":{"operationId":self.request_id,"recipientIds":[self.target.id],"expectedName":self.target.name,"message":self.message,"kind":"request"}})
    }
}
pub fn upgrade() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_UPGRADE_REQUIRED",
        "Upgrade Remote and Core to use cloud messaging; pairing is unchanged.",
    )
}
fn too_large() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_MESSAGE_TOO_LARGE",
        "The message exceeds the cloud transport limit.",
    )
}
fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_INPUT_INVALID", "Invalid prepared cloud message.")
}
