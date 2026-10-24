//! Core owns the versioned message schema (#4831); Remote admits the local command.
use crate::{canonical, error::RemoteError, limits, wire};
use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerIdentity {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
}
/// Local reply capability supplied by Core. Never include this value in a cloud envelope.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reply {
    pub request_id: String,
    pub receipt: String,
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
    pub target: Target,
    pub message: String,
    #[serde(default)]
    pub reply: Option<Reply>,
}
#[derive(Deserialize, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Sent,
    Held,
    Refused,
    Unconfirmed,
    Unavailable,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub version: u64,
    pub state: State,
    pub remote: String,
    pub target: Target,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
impl Request {
    /// Core resolves its default before preparing the request; Remote never guesses a peer.
    pub fn selected<'a>(&'a self, explicit: Option<&'a str>) -> Result<&'a str, RemoteError> {
        if explicit.is_some()
            && self.remote.as_deref().is_some()
            && explicit != self.remote.as_deref()
        {
            return Err(RemoteError::new(
                "REMOTE_SELECTOR_INVALID",
                "The command and prepared request name different remotes.",
            ));
        }
        let selected = explicit.or(self.remote.as_deref()).ok_or_else(|| {
            RemoteError::new("REMOTE_CLOUD_UNAVAILABLE", "No cloud peer was selected.")
        })?;
        if selected == "local" {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_UNAVAILABLE",
                "The selected target does not use the cloud message driver.",
            ));
        }
        if !crate::remote_catalog::name_ok(selected) {
            return Err(RemoteError::new(
                "REMOTE_SELECTOR_INVALID",
                "Invalid remote name.",
            ));
        }
        Ok(selected)
    }
    pub(crate) fn unconfirmed(&self, selected: &str) -> Answer {
        Answer {
            version: 1,
            state: State::Unconfirmed,
            remote: selected.into(),
            target: self.target.clone(),
            reason: Some("REMOTE_CLOUD_UNCONFIRMED".into()),
        }
    }
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
        if value["target"].get("id").is_some_and(|id| !id.is_string()) {
            return Err(invalid());
        }
        let mut request: Self = serde_json::from_value(value).map_err(|_| invalid())?;
        if request.version != 1 || request.created_at_ms > crate::cloud_hpke::INTEGER_MAX {
            return Err(invalid());
        }
        canonical::uuid(&request.request_id).map_err(|_| invalid())?;
        if let Some(reply) = &request.reply
            && (reply.request_id != format!("req_{}", request.request_id)
                || reply.receipt.is_empty()
                || reply.receipt.chars().any(char::is_control))
        {
            return Err(invalid());
        }
        canonical::uuid(&request.from.id).map_err(|_| invalid())?;
        if let Some(id) = &request.target.id {
            canonical::uuid(id).map_err(|_| invalid())?;
        }
        for name in [&request.from.name, &request.target.name] {
            if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
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
        serde_json::json!({"version":1,"operation":"dispatch.create","originator":"anonymous","requestDigest":crate::canonical::base64url(&self.fingerprint),"createdAtMs":self.created_at_ms,"input":{"operationId":self.request_id,"recipientIds":self.target.id.iter().collect::<Vec<_>>(),"expectedName":self.target.name,"message":self.message,"kind":"request"}})
    }
}
/// Transport freshness and the prepared request lifetime are separate clocks. Recovery controls
/// are observations, not a fresh effect, and do not extend an original request's lifetime.
pub(crate) fn dispatch_created_at(
    message: &crate::wire::SignedMessage,
) -> Result<Option<u64>, RemoteError> {
    if message.envelope().operation != "dispatch.create" {
        return Ok(None);
    }
    message
        .input
        .get("createdAtMs")
        .and_then(serde_json::Value::as_u64)
        .map(Some)
        .ok_or_else(upgrade)
}
pub(crate) fn fixed_code(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
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
