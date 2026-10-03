//! One normal message in flight per session, with durable replay fencing.
use crate::{
    authority::GrantAuthority, error::RemoteError, pairing::now_ms, session::DoorSessions,
    store::Grant, wire::SignedMessage,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Subscribe {
    cursor: Value,
    limit: usize,
    wait_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    cursor: String,
}
fn invalid() -> RemoteError {
    RemoteError::new("REMOTE_INPUT_INVALID", "Invalid signed control payload.")
}
impl MessagePermit {
    pub fn with_store<T>(
        &self,
        action: impl FnOnce(&mut crate::store::Store) -> Result<T, RemoteError>,
    ) -> Result<T, RemoteError> {
        self.sessions
            .with_store(&self.grant, &self.message, now_ms()?, action)
            .map(|value| value.0)
    }
    /// Commit sanitized metadata and frozen send ownership before the bounded core call.
    pub fn adopt(
        &self,
        frozen: Option<&[u8]>,
        resources: &[String],
    ) -> Result<crate::journal::Owned, RemoteError> {
        let input = self.message.envelope();
        let phase = if input.operation == "dispatch.create" {
            if self.grant.mode == "hold" {
                "held"
            } else {
                "dispatching"
            }
        } else {
            "observed"
        };
        let metadata = self.response(
            &json!({"requestEnvelopeId":input.id,"operation":input.operation,"state":phase}),
        )?;
        let now = now_ms()?;
        let result = self.with_store(|store| {
            store.adopt(
                &self.grant,
                &self.message,
                frozen,
                resources,
                &metadata,
                now,
            )
        })?;
        self.sessions.changed();
        Ok(result)
    }
    pub(crate) fn subscribe(&self) -> Result<Vec<u8>, RemoteError> {
        let input: Subscribe =
            serde_json::from_value(self.message.input.clone()).map_err(|_| invalid())?;
        if !(1..=50).contains(&input.limit)
            || input.wait_ms > 25000
            || !(input.cursor.is_null() || input.cursor.is_string())
        {
            return Err(invalid());
        }
        let deadline = Instant::now() + Duration::from_millis(input.wait_ms);
        loop {
            let now = now_ms()?;
            let (payload, generation) =
                self.sessions
                    .with_store(&self.grant, &self.message, now, |store| {
                        store.page(&self.grant, input.cursor.as_str(), input.limit, now)
                    })?;
            if !payload["entries"]
                .as_array()
                .expect("journal entries")
                .is_empty()
                || Instant::now() >= deadline
            {
                return self.response(&payload);
            }
            self.sessions.wait(generation, deadline)?;
        }
    }
    pub(crate) fn ack(&self) -> Result<Vec<u8>, RemoteError> {
        let input: Ack =
            serde_json::from_value(self.message.input.clone()).map_err(|_| invalid())?;
        let now = now_ms()?;
        let result = self.with_store(|store| store.ack(&self.grant, &input.cursor, now))?;
        self.response(&result)
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
