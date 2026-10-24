//! The single signed-message receiver, shared by backend bindings.
//! Session admission, operation effects and journal controls retain their owners.
use crate::{
    admission::{BindingAction, MessageRefusal},
    operations::Operations,
    session::DoorSessions,
    transport::Closed,
};
use serde_json::json;
use std::sync::Arc;

// Foreground composition injects the operation owner; pairing-only fixtures remain closed.
// A binding receives signed bytes or Closed, never a permit or Core authority.
pub struct MessageService {
    sessions: Arc<DoorSessions>,
    input_limit: usize,
    operations: Option<Arc<Operations>>,
}
/// The signed bytes are unchanged; only cloud ingress consumes internal read provenance.
pub(crate) struct MessageAnswer {
    pub(crate) bytes: Vec<u8>,
    pub(crate) owned_read: Option<crate::error::OwnedReadFailure>,
}
impl MessageService {
    pub fn new(sessions: Arc<DoorSessions>, input_limit: usize) -> Self {
        Self {
            sessions,
            input_limit,
            operations: None,
        }
    }
    pub fn with_operations(mut self, operations: Arc<Operations>) -> Self {
        self.operations = Some(operations);
        self
    }
    pub(crate) fn cloud_final_for(
        &self,
        grant: &crate::store::Grant,
        machine: &str,
        operation: &str,
    ) -> Result<Option<crate::cloud_inbox::finals::Final>, crate::error::RemoteError> {
        self.sessions
            .with_cloud_store(grant, crate::pairing::now_ms()?, |store| {
                self.operations
                    .as_ref()
                    .ok_or_else(crate::cloud_command::upgrade)?
                    .cloud_final_for(store, grant, machine, operation)
            })
            .map(|(value, _)| value)
    }
    pub fn append(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed> {
        self.handle(BindingAction::Append, origin, envelope)
    }
    pub fn subscribe(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed> {
        self.handle(BindingAction::Subscribe, origin, envelope)
    }
    pub fn ack(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed> {
        self.handle(BindingAction::Ack, origin, envelope)
    }

    pub(crate) fn cloud(
        &self,
        action: BindingAction,
        verified: crate::cloud_inbox::Verified,
    ) -> Result<MessageAnswer, crate::error::RemoteError> {
        let message = verified.message().clone();
        match self.sessions.admit_cloud(action, verified) {
            Ok(permit) => self.execute(action, permit),
            Err(error) => self
                .sessions
                .cloud_response(
                    &message,
                    &json!({"error":{"code":error.code,"message":"Remote request refused."}}),
                )
                .map(|bytes| MessageAnswer {
                    bytes,
                    owned_read: error.owned_read(),
                }),
        }
    }
    fn execute(
        &self,
        action: BindingAction,
        permit: crate::admission::MessagePermit,
    ) -> Result<MessageAnswer, crate::error::RemoteError> {
        let result = match action {
            BindingAction::Subscribe => permit.subscribe(),
            BindingAction::Ack => permit.ack(),
            BindingAction::Append if self.operations.is_some() => self
                .operations
                .as_ref()
                .expect("operation owner")
                .append(&permit),
            BindingAction::Append => {
                let now = crate::pairing::now_ms()?;
                permit
                    .with_store(|store| {
                        store.record_refusal(&permit.grant, &permit.message, "REMOTE_CLOSED", now)
                    })
                    .and_then(|_| {
                        permit.response(&json!({"error": {
                            "code": "REMOTE_CLOSED",
                            "message": "Remote application operations are not enabled."
                        }}))
                    })
            }
        };
        result
            .map(|bytes| MessageAnswer {
                bytes,
                owned_read: None,
            })
            .or_else(|error| {
                let now = crate::pairing::now_ms()?;
                let audit = permit.with_store(|store| {
                    store.record_refusal(&permit.grant, &permit.message, &error.code, now)
                });
                let error = audit.err().unwrap_or(error);
                let mut payload = json!({"error": {
                    "code": error.code,
                    "message": "Remote request refused."
                }});
                if let Some(limit) = error.limit {
                    payload["error"]["limit"] = json!(limit);
                }
                permit.response(&payload).map(|bytes| MessageAnswer {
                    bytes,
                    owned_read: error.owned_read(),
                })
            })
    }

    fn handle(
        &self,
        action: BindingAction,
        origin: Option<&str>,
        envelope: &[u8],
    ) -> Result<Vec<u8>, Closed> {
        match self
            .sessions
            .admit(action, origin, envelope, self.input_limit)
        {
            Ok(permit) => self
                .execute(action, permit)
                .map(|answer| answer.bytes)
                .map_err(|_| Closed),
            Err(MessageRefusal::Signed(response)) => Ok(response),
            Err(MessageRefusal::Unauthenticated) => Err(Closed),
        }
    }
}
