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
    pub fn append(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed> {
        self.handle(BindingAction::Append, origin, envelope)
    }
    pub fn subscribe(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed> {
        self.handle(BindingAction::Subscribe, origin, envelope)
    }
    pub fn ack(&self, origin: Option<&str>, envelope: &[u8]) -> Result<Vec<u8>, Closed> {
        self.handle(BindingAction::Ack, origin, envelope)
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
            Ok(permit) => {
                let result = match action {
                    BindingAction::Subscribe => permit.subscribe(),
                    BindingAction::Ack => permit.ack(),
                    BindingAction::Append if self.operations.is_some() => self
                        .operations
                        .as_ref()
                        .expect("operation owner")
                        .append(&permit),
                    BindingAction::Append => {
                        let now = crate::pairing::now_ms().map_err(|_| Closed)?;
                        permit
                            .with_store(|store| {
                                store.record_refusal(
                                    &permit.grant,
                                    &permit.message,
                                    "REMOTE_CLOSED",
                                    now,
                                )
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
                        permit.response(&payload)
                    })
                    .map_err(|_| Closed)
            }
            Err(MessageRefusal::Signed(response)) => Ok(response),
            Err(MessageRefusal::Unauthenticated) => Err(Closed),
        }
    }
}
