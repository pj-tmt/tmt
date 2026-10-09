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
// This registry admits names; exhaustive typed matches require every admitted
// operation to declare its scope and journal class. Wire input supplies no class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationClass {
    Read,
    RecoveryObservation,
    Effect,
    Control,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum RemoteOperation {
    Capabilities,
    Subscribe,
    Ack,
    SettingsShow,
    SettingsSet,
    DevicesList,
    DevicesRename,
    DevicesRevoke,
    DevicesTalk,
    ManagementOperation,
    AgentsList,
    IdentitiesStatus,
    Check,
    DispatchCreate,
    DispatchShow,
    OperationShow,
    RequestsShow,
    Result,
}
impl RemoteOperation {
    const NAMES: &'static [(&'static str, Self)] = &[
        ("capabilities", Self::Capabilities),
        ("subscribe", Self::Subscribe),
        ("ack", Self::Ack),
        ("remote.settings.show", Self::SettingsShow),
        ("remote.settings.set", Self::SettingsSet),
        ("remote.devices.list", Self::DevicesList),
        ("remote.devices.rename", Self::DevicesRename),
        ("remote.devices.revoke", Self::DevicesRevoke),
        ("remote.devices.talk", Self::DevicesTalk),
        ("remote.management.operation", Self::ManagementOperation),
        ("agents.list", Self::AgentsList),
        ("identities.status", Self::IdentitiesStatus),
        ("check", Self::Check),
        ("dispatch.create", Self::DispatchCreate),
        ("dispatch.show", Self::DispatchShow),
        ("operation.show", Self::OperationShow),
        ("requests.show", Self::RequestsShow),
        ("result", Self::Result),
    ];
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, operation)| *operation)
    }
    pub(crate) fn class(self) -> OperationClass {
        match self {
            Self::Capabilities => OperationClass::Read,
            Self::Subscribe => OperationClass::Control,
            Self::Ack => OperationClass::Control,
            Self::SettingsShow => OperationClass::Read,
            Self::SettingsSet => OperationClass::Effect,
            Self::DevicesList => OperationClass::Read,
            Self::DevicesRename => OperationClass::Effect,
            Self::DevicesRevoke => OperationClass::Effect,
            Self::DevicesTalk => OperationClass::Effect,
            Self::ManagementOperation => OperationClass::Read,
            Self::AgentsList => OperationClass::Read,
            Self::IdentitiesStatus => OperationClass::Read,
            Self::Check => OperationClass::Read,
            Self::DispatchCreate => OperationClass::Effect,
            Self::DispatchShow => OperationClass::Read,
            Self::OperationShow => OperationClass::RecoveryObservation,
            Self::RequestsShow => OperationClass::Read,
            Self::Result => OperationClass::Read,
        }
    }
    fn scope(self) -> Option<&'static str> {
        match self {
            Self::Capabilities => None,
            Self::Subscribe => None,
            Self::Ack => None,
            Self::SettingsShow => None,
            Self::SettingsSet => None,
            Self::DevicesList => None,
            Self::DevicesRename => None,
            Self::DevicesRevoke => None,
            Self::DevicesTalk => None,
            Self::ManagementOperation => None,
            Self::AgentsList => Some("agents.read"),
            Self::IdentitiesStatus => Some("status.read"),
            Self::Check => Some("check.read"),
            Self::DispatchCreate => Some("talk"),
            Self::DispatchShow => Some("talk"),
            Self::OperationShow => Some("talk"),
            Self::RequestsShow => Some("results.read"),
            Self::Result => Some("results.read"),
        }
    }
}
pub(crate) fn scope(operation: &str) -> Option<Option<&'static str>> {
    RemoteOperation::parse(operation).map(RemoteOperation::scope)
}
pub(crate) fn refusal(
    sessions: &DoorSessions,
    message: &SignedMessage,
    code: &str,
    text: &str,
) -> MessageRefusal {
    refusal_with_limit(sessions, message, code, text, None)
}
pub(crate) fn refusal_with_limit(
    sessions: &DoorSessions,
    message: &SignedMessage,
    code: &str,
    text: &str,
    limit: Option<usize>,
) -> MessageRefusal {
    signed_refusal(
        sessions,
        message,
        json!({"code":code,"message":text}),
        limit,
    )
}

pub(crate) fn missing_talk(sessions: &DoorSessions, message: &SignedMessage) -> MessageRefusal {
    signed_refusal(
        sessions,
        message,
        json!({"code":"REMOTE_SCOPE_DENIED",
        "message":"Device scope does not admit this operation.","scope":"talk",
        "settingsUrl":format!("{}/settings", sessions.door_origin())}),
        None,
    )
}

fn signed_refusal(
    sessions: &DoorSessions,
    message: &SignedMessage,
    error: Value,
    limit: Option<usize>,
) -> MessageRefusal {
    let mut payload = json!({"error":error});
    if let Some(limit) = limit {
        payload["error"]["limit"] = json!(limit);
    }
    sessions
        .response(message, &payload)
        .map(MessageRefusal::Signed)
        .unwrap_or(MessageRefusal::Unauthenticated)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_scoped_operation_has_an_explicit_class() {
        for (name, operation) in RemoteOperation::NAMES {
            assert_eq!(scope(name), Some(operation.scope()));
            assert_eq!(
                RemoteOperation::parse(name).unwrap().class(),
                operation.class()
            );
        }
        assert!(scope("future.operation").is_none());
    }
}
