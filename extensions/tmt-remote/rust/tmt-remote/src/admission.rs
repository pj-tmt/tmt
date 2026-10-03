//! One normal message in flight per session, with durable replay fencing.
use crate::{
    authority::GrantAuthority, error::RemoteError, pairing::now_ms, session::DoorSessions,
    store::Grant, wire::SignedMessage,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Copy)]
pub enum BindingAction {
    Append,
    Subscribe,
    Ack,
}
impl BindingAction {
    pub(crate) fn matches(self, message: &SignedMessage) -> bool {
        let envelope = message.envelope();
        match self {
            Self::Append => envelope.kind == "request",
            Self::Subscribe => envelope.kind == "control" && envelope.operation == "subscribe",
            Self::Ack => envelope.kind == "control" && envelope.operation == "ack",
        }
    }
}
#[derive(Debug)]
pub enum MessageRefusal {
    Unauthenticated,
    Signed(Vec<u8>),
}
/// Keeps the session busy until the message owner has settled its response.
/// Dropping a permit never refunds a consumed sequence or retries an effect.
pub struct MessagePermit {
    pub message: SignedMessage,
    pub grant: Grant,
    pub authority: GrantAuthority,
    pub(crate) sessions: Arc<DoorSessions>,
    pub(crate) busy: Arc<AtomicBool>,
}
impl MessagePermit {
    pub fn response(&self, payload: &Value) -> Result<Vec<u8>, RemoteError> {
        self.sessions.response(&self.message, payload)
    }
    /// Admission is not an effect lease. Every effect rechecks the persisted
    /// authority under the store owner's lock at its own durable fence.
    pub fn revalidate(&self) -> Result<(), RemoteError> {
        self.sessions
            .revalidate(&self.grant, &self.message, now_ms()?)
    }
}
impl Drop for MessagePermit {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}
pub(crate) fn scope(operation: &str) -> Option<Option<&'static str>> {
    Some(match operation {
        "capabilities" | "subscribe" | "ack" => None,
        "agents.list" => Some("agents.read"),
        "identities.status" => Some("status.read"),
        "check" => Some("check.read"),
        "dispatch.create" | "dispatch.show" | "operation.show" => Some("talk"),
        "requests.show" | "result" => Some("results.read"),
        _ => return None,
    })
}
pub(crate) fn refusal(
    sessions: &DoorSessions,
    message: &SignedMessage,
    code: &str,
    text: &str,
) -> MessageRefusal {
    sessions
        .response(message, &json!({"error":{"code":code,"message":text}}))
        .map(MessageRefusal::Signed)
        .unwrap_or(MessageRefusal::Unauthenticated)
}
