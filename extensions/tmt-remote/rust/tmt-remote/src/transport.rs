//! Bindings move envelopes to one message owner; they never invoke CoreClient.
use crate::{
    admission::{BindingAction, MessageRefusal},
    session::DoorSessions,
};
use serde_json::json;
use std::sync::Arc;
#[derive(Debug, PartialEq, Eq)]
pub struct Closed;
pub trait Transport {
    fn append(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
    fn subscribe(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
    fn ack(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed>;
}
// Application append stays closed until the public-operation owner is wired.
// Subscribe/ack use durable metadata streams; bindings never call core.
struct MessageService {
    sessions: Option<Arc<DoorSessions>>,
    input_limit: usize,
}
impl MessageService {
    fn handle(
        &self,
        action: BindingAction,
        origin: Option<&str>,
        envelope: &[u8],
    ) -> Result<Vec<u8>, Closed> {
        let sessions = self.sessions.as_ref().ok_or(Closed)?;
        match sessions.admit(action, origin, envelope, self.input_limit) {
            Ok(permit) => {
                let result = match action {
                    BindingAction::Subscribe => permit.subscribe(),
                    BindingAction::Ack => permit.ack(),
                    BindingAction::Append => {
                        let now = crate::pairing::now_ms().map_err(|_| Closed)?;
                        permit.with_store(|store|store.record_refusal(&permit.grant,&permit.message,"REMOTE_CLOSED",now))
                            .and_then(|_|permit.response(&json!({"error":{"code":"REMOTE_CLOSED","message":"Remote application operations are not enabled."}})))
                    }
                };
                result.or_else(|error|permit.response(&json!({"error":{"code":error.code,"message":"Remote request refused."}}))).map_err(|_|Closed)
            }
            Err(MessageRefusal::Signed(response)) => Ok(response),
            Err(MessageRefusal::Unauthenticated) => Err(Closed),
        }
    }
}
pub struct LoopbackTransport {
    message: MessageService,
}
impl Default for LoopbackTransport {
    fn default() -> Self {
        Self {
            message: MessageService {
                sessions: None,
                input_limit: 0,
            },
        }
    }
}
impl LoopbackTransport {
    pub fn new(sessions: Arc<DoorSessions>, input_limit: usize) -> Self {
        Self {
            message: MessageService {
                sessions: Some(sessions),
                input_limit,
            },
        }
    }
}
impl Transport for LoopbackTransport {
    fn append(&self, origin: Option<&str>, bytes: &[u8]) -> Result<Vec<u8>, Closed> {
        self.message.handle(BindingAction::Append, origin, bytes)
    }
    fn subscribe(&self, origin: Option<&str>, bytes: &[u8]) -> Result<Vec<u8>, Closed> {
        self.message.handle(BindingAction::Subscribe, origin, bytes)
    }
    fn ack(&self, origin: Option<&str>, bytes: &[u8]) -> Result<Vec<u8>, Closed> {
        self.message.handle(BindingAction::Ack, origin, bytes)
    }
}
