//! HPKE alone constructs verified ingress; terminal carrier refusals grant no operation access.
use crate::{
    admission::BindingAction, cloud_hpke, cloud_keys::CloudKeys, error::RemoteError,
    message::MessageService, session::DoorSessions, wire::SignedMessage,
};
use serde_json::json;
use std::sync::Arc;

pub(crate) struct Verified {
    message: SignedMessage,
    grant: crate::store::Grant,
}
impl Verified {
    pub(crate) fn message(&self) -> &SignedMessage {
        &self.message
    }
    pub(crate) fn into_parts(self) -> (SignedMessage, crate::store::Grant) {
        (self.message, self.grant)
    }
}
/// Ordinary responses retain their grant fence; an authenticated refusal carries no authority.
pub(crate) struct Reply {
    pub(crate) bytes: Vec<u8>,
    grant: Option<crate::store::Grant>,
    id: String,
    peer: String,
    sessions: Arc<DoorSessions>,
}
impl Reply {
    pub(crate) fn claim_publication(&self) -> Result<bool, RemoteError> {
        if let Some(grant) = &self.grant {
            self.sessions
                .with_cloud_store(grant, crate::pairing::now_ms()?, |store| {
                    store.cloud_claim_publication(&self.peer, &self.id)
                })
                .map(|(claimed, _)| claimed)
        } else {
            self.sessions
                .with_operation_store(|store| store.cloud_claim_publication(&self.peer, &self.id))
        }
    }
}
/// Firebase grants access to ciphertext, never to an application operation.
pub struct Receiver {
    sessions: Arc<DoorSessions>,
    keys: Arc<CloudKeys>,
    service: Arc<MessageService>,
}
impl Receiver {
    pub fn new(
        sessions: Arc<DoorSessions>,
        keys: Arc<CloudKeys>,
        service: Arc<MessageService>,
    ) -> Self {
        Self {
            sessions,
            keys,
            service,
        }
    }
    fn reply(
        &self,
        document: &super::document::Document,
        bytes: Vec<u8>,
        grant: Option<crate::store::Grant>,
    ) -> Reply {
        Reply {
            bytes,
            grant,
            id: document.id().into(),
            peer: document.peer().into(),
            sessions: self.sessions.clone(),
        }
    }
    fn refuse(
        &self,
        document: &super::document::Document,
        code: &str,
        response: Option<Vec<u8>>,
        now: u64,
    ) -> Result<Option<Reply>, RemoteError> {
        self.sessions.with_operation_store(|store| {
            store.cloud_refuse(
                document.peer(),
                document.id(),
                code,
                response.as_deref(),
                now,
            )
        })?;
        Ok(response.map(|bytes| self.reply(document, bytes, None)))
    }
    pub(crate) fn receive(
        &self,
        document: &super::document::Document,
        now: u64,
    ) -> Result<Option<Reply>, RemoteError> {
        // This is transport evidence only, not adoption. Refusals and frozen replies deduplicate
        // before key access, so a restart cannot reprocess a poison carrier.
        let replay = self.sessions.with_operation_store(|store| {
            store.cloud_begin(
                document.peer(),
                document.id(),
                document.bytes(),
                document.created_ms(),
            )
        })?;
        match replay {
            super::Replay::Refused(response) => {
                return Ok(response.map(|bytes| self.reply(document, bytes, None)));
            }
            super::Replay::Frozen(bytes) => {
                return match self.sessions.cloud_peer(document.peer(), now) {
                    Ok(grant) => Ok(Some(self.reply(document, bytes, Some(grant)))),
                    Err(error) if permanent(&error.code) => {
                        self.refuse(document, &error.code, None, now)
                    }
                    Err(error) => Err(error),
                };
            }
            _ => {}
        }
        let grant = match self.sessions.cloud_peer_key(document.peer()) {
            Ok(grant) => grant,
            Err(error) if permanent(&error.code) => {
                return self.refuse(document, &error.code, None, now);
            }
            Err(error) => return Err(error),
        };
        let Some(value) = crate::wire::strict_json(document.bytes()) else {
            return self.refuse(document, "REMOTE_CLOUD_CARRIER_INVALID", None, now);
        };
        let Some(key_id) = value["keyId"].as_str() else {
            return self.refuse(document, "REMOTE_CLOUD_CARRIER_INVALID", None, now);
        };
        let (key, descriptor) =
            match self
                .keys
                .recipient_for_document(key_id, document.created_ms(), now)
            {
                Ok(pair) => pair,
                // A missing/retired recipient or invalid creation-time validity is per document.
                // Local keyring I/O/version failures remain retryable, never blame the carrier.
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "REMOTE_CLOUD_KEY_RETIRED"
                            | "REMOTE_CLOUD_DESCRIPTOR_EXPIRED"
                            | "REMOTE_CLOUD_DESCRIPTOR_NOT_YET_VALID"
                    ) =>
                {
                    return self.refuse(document, &error.code, None, now);
                }
                Err(error) => return Err(error),
            };
        let (exact, sealer) = match cloud_hpke::open(
            document.bytes(),
            crate::limits::CLOUD_ENVELOPE_BYTES,
            &key,
            &descriptor,
            document.created_ms(),
            cloud_hpke::Sender {
                client_id: &grant.client_id,
                authentication_key: &grant.public_key,
            },
        ) {
            Ok(opened) => opened,
            Err(error) => return self.refuse(document, error.code(), None, now),
        };
        let Some(message) = SignedMessage::decode(&exact, crate::limits::CLOUD_ENVELOPE_BYTES)
        else {
            return self.refuse(document, "REMOTE_CLOUD_CARRIER_INVALID", None, now);
        };
        let envelope = message.envelope();
        let rejected = if envelope.id != document.id() {
            Some("REMOTE_CLOUD_CARRIER_INVALID")
        } else if let Err(error) =
            super::check_freshness(envelope.timestamp_ms, document.created_ms(), now)
        {
            return self.signed_refusal(document, &message, sealer, &error.code, now);
        } else {
            None
        };
        if let Some(code) = rejected {
            return self.signed_refusal(document, &message, sealer, code, now);
        }
        match crate::cloud_command::dispatch_created_at(&message) {
            Ok(Some(created)) => {
                if let Err(error) = super::check_request_age(created, now) {
                    return self.signed_refusal(document, &message, sealer, &error.code, now);
                }
            }
            Ok(None) => {}
            Err(error) => return self.signed_refusal(document, &message, sealer, &error.code, now),
        }
        let action = match envelope.kind {
            "request" => BindingAction::Append,
            "control" if envelope.operation == "subscribe" => BindingAction::Subscribe,
            "control" if envelope.operation == "ack" => BindingAction::Ack,
            _ => {
                return self.signed_refusal(
                    document,
                    &message,
                    sealer,
                    "REMOTE_INPUT_INVALID",
                    now,
                );
            }
        };
        let answer = self.service.cloud(
            action,
            Verified {
                message,
                grant: grant.clone(),
            },
        )?;
        let decoded = SignedMessage::decode_any(&answer, crate::limits::CLOUD_ENVELOPE_BYTES)
            .ok_or_else(|| {
                RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Cloud response unavailable.")
            })?;
        let refusal = decoded.input["error"]["code"].as_str();
        if let Some(code) = refusal
            && !permanent(code)
        {
            return Err(RemoteError::new(
                code,
                "Cloud request will be observed on the next poll.",
            ));
        }
        let encrypted = sealer
            .seal(&answer, crate::limits::CLOUD_ENVELOPE_BYTES, |bytes| {
                self.sessions.sign_cloud(bytes)
            })
            .map_err(|error| RemoteError::new(error.code(), &error.to_string()))?;
        if let Some(code) = refusal {
            return self.refuse(document, code, Some(encrypted), now);
        }
        self.sessions.with_cloud_store(&grant, now, |store| {
            store.cloud_freeze(document.peer(), document.id(), &encrypted)
        })?;
        Ok(Some(self.reply(document, encrypted, Some(grant))))
    }
    fn signed_refusal(
        &self,
        document: &super::document::Document,
        message: &SignedMessage,
        sealer: cloud_hpke::ResponseSealer,
        code: &str,
        now: u64,
    ) -> Result<Option<Reply>, RemoteError> {
        let answer = self.sessions.cloud_response(
            message,
            &json!({"error":{"code":code,"message":"Remote request refused."}}),
        )?;
        let encrypted = sealer
            .seal(&answer, crate::limits::CLOUD_ENVELOPE_BYTES, |bytes| {
                self.sessions.sign_cloud(bytes)
            })
            .map_err(|error| RemoteError::new(error.code(), &error.to_string()))?;
        self.refuse(document, code, Some(encrypted), now)
    }
}
/// These fixed per-document refusals cannot be repaired by refreshing Auth or retrying a poll.
/// Every other failure is retryable and leaves the checkpoint unchanged.
pub(crate) fn permanent(code: &str) -> bool {
    matches!(
        code,
        "REMOTE_CLOSED"
            | "REMOTE_GRANT_CHANGED"
            | "REMOTE_SCOPE_DENIED"
            | "REMOTE_INPUT_INVALID"
            | "REMOTE_CLOUD_CONFLICT"
            | "REMOTE_CLOUD_CARRIER_INVALID"
            | "REMOTE_CLOUD_SIGNATURE_INVALID"
            | "REMOTE_CLOUD_ENVELOPE_INVALID"
            | "REMOTE_CLOUD_BOUNDS"
            | "REMOTE_CLOUD_KEY_RETIRED"
            | "REMOTE_CLOUD_DESCRIPTOR_EXPIRED"
            | "REMOTE_CLOUD_DESCRIPTOR_NOT_YET_VALID"
            | "REMOTE_CLOUD_DESCRIPTOR_REVISION"
            | "REMOTE_CLOUD_UPGRADE_REQUIRED"
            | "REMOTE_CLOUD_TIMESTAMP_INVALID"
            | "REMOTE_CLOUD_MESSAGE_EXPIRED"
    )
}
