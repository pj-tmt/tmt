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
// Application adoption belongs to the later durable journal. A valid message
// receives a signed closed response now, with no journal entry or core effect.
struct MessageService {
    sessions: Option<Arc<DoorSessions>>,
    input_limit: usize,
}
impl MessageService {
    fn refuse(
        &self,
        action: BindingAction,
        origin: Option<&str>,
        envelope: &[u8],
    ) -> Result<Vec<u8>, Closed> {
        let sessions = self.sessions.as_ref().ok_or(Closed)?;
        match sessions.admit(action, origin, envelope, self.input_limit) {
            Ok(permit) => permit.response(&json!({"error":{"code":"REMOTE_CLOSED","message":"Remote application operations are not enabled."}})).map_err(|_| Closed),
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
        self.message.refuse(BindingAction::Append, origin, bytes)
    }
    fn subscribe(&self, origin: Option<&str>, bytes: &[u8]) -> Result<Vec<u8>, Closed> {
        self.message.refuse(BindingAction::Subscribe, origin, bytes)
    }
    fn ack(&self, origin: Option<&str>, bytes: &[u8]) -> Result<Vec<u8>, Closed> {
        self.message.refuse(BindingAction::Ack, origin, bytes)
    }
}
