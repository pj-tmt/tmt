//! The only constructor of verified cloud ingress is the HPKE receiver below.
use crate::{
    admission::BindingAction, cloud_hpke, cloud_keys::CloudKeys, error::RemoteError,
    message::MessageService, session::DoorSessions, wire::SignedMessage,
};
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
/// Verified response publication retains the grant revision through the last pre-HTTP fence.
pub(crate) struct Reply {
    pub(crate) bytes: Vec<u8>,
    grant: crate::store::Grant,
    id: String,
    sessions: Arc<DoorSessions>,
}
impl Reply {
    pub(crate) fn claim_publication(&self) -> Result<bool, RemoteError> {
        self.sessions
            .with_cloud_store(&self.grant, crate::pairing::now_ms()?, |store| {
                store.cloud_claim_publication(&self.grant.client_id, &self.id)
            })
            .map(|(claimed, _)| claimed)
    }
}
/// Provider document admission supplies immutable server createTime and the exact bytes.
/// Firebase authentication is not an operation grant: the common receiver rechecks the grant.
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
    pub(crate) fn receive(
        &self,
        document: &super::document::Document,
        now: u64,
    ) -> Result<Reply, RemoteError> {
        let grant = self.sessions.cloud_peer(document.peer(), now)?;
        let value = crate::wire::strict_json(document.bytes()).ok_or_else(invalid)?;
        let key_id = value["keyId"].as_str().ok_or_else(invalid)?;
        let (key, descriptor) =
            self.keys
                .recipient_for_document(key_id, document.created_ms(), now)?;
        let (exact, sealer) = cloud_hpke::open(
            document.bytes(),
            crate::limits::CLOUD_ENVELOPE_BYTES,
            &key,
            &descriptor,
            document.created_ms(),
            cloud_hpke::Sender {
                client_id: &grant.client_id,
                authentication_key: &grant.public_key,
            },
        )
        .map_err(|e| RemoteError::new(e.code(), &e.to_string()))?;
        let message = SignedMessage::decode(&exact, crate::limits::CLOUD_ENVELOPE_BYTES)
            .ok_or_else(invalid)?;
        let envelope = message.envelope();
        if envelope.id != document.id() {
            return Err(invalid());
        }
        super::check_freshness(envelope.timestamp_ms, document.created_ms(), now)?;
        let (replay, _) = self.sessions.with_cloud_store(&grant, now, |store| {
            store.cloud_begin(
                document.peer(),
                document.id(),
                document.bytes(),
                document.created_ms(),
            )
        })?;
        if let super::Replay::Frozen(bytes) = replay {
            return Ok(Reply {
                bytes,
                grant,
                id: document.id().into(),
                sessions: self.sessions.clone(),
            });
        }
        let action = match envelope.kind {
            "request" => BindingAction::Append,
            "control" if envelope.operation == "subscribe" => BindingAction::Subscribe,
            "control" if envelope.operation == "ack" => BindingAction::Ack,
            _ => return Err(invalid()),
        };
        let answer = self.service.cloud(
            action,
            Verified {
                message,
                grant: grant.clone(),
            },
        )?;
        let encrypted = sealer
            .seal(&answer, crate::limits::CLOUD_ENVELOPE_BYTES, |bytes| {
                self.sessions.sign_cloud(bytes)
            })
            .map_err(|e| RemoteError::new(e.code(), &e.to_string()))?;
        self.sessions.with_cloud_store(&grant, now, |store| {
            store.cloud_freeze(document.peer(), document.id(), &encrypted)
        })?;
        Ok(Reply {
            bytes: encrypted,
            grant,
            id: document.id().into(),
            sessions: self.sessions.clone(),
        })
    }
}
fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_CLOUD_CARRIER_INVALID", "Invalid cloud carrier.")
}
