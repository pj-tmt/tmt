//! Read-only projection of the service's existing live slots, never an activation trigger.
use super::{ActivateError, State};
use crate::{
    mount::{Extension, ObjectDeclaration},
    objects::ExtensionId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// The control socket may retain this view, but cannot open storage or start a worker.
/// ObjectService shutdown removes and joins every Running before its lease ends.
#[derive(Clone, Default)]
pub struct ObjectReadiness {
    state: Option<Arc<Mutex<State>>>,
    failed: Vec<&'static str>,
}
impl ObjectReadiness {
    pub(super) fn live(state: Arc<Mutex<State>>) -> Self {
        Self {
            state: Some(state),
            failed: Vec::new(),
        }
    }
    /// Failed storage preparation is unavailable, not an invitation to reopen a ledger.
    pub fn unavailable(extensions: &'static [Extension]) -> Self {
        Self {
            state: None,
            failed: extensions
                .iter()
                .filter(|e| e.objects == ObjectDeclaration::Local)
                .map(|e| e.name)
                .collect(),
        }
    }
    pub fn snapshot(&self) -> Value {
        let Some(state) = &self.state else {
            return Value::Array(
                self.failed
                    .iter()
                    .map(|name| json!({"extension":name,"state":"unavailable","reason":"storage"}))
                    .collect(),
            );
        };
        let state = state.lock().unwrap_or_else(|p| p.into_inner());
        Value::Array(
            state
                .slots
                .iter()
                .map(|slot| {
                    if slot.running() {
                        return json!({"extension":slot.name,"state":"ready"});
                    }
                    if slot.setup && !state.stopped {
                        return json!({"extension":slot.name,"state":"starting"});
                    }
                    let reason = if slot.active.is_some() {
                        "channel-ended"
                    } else if slot.failure == Some(ActivateError::Capacity) {
                        "capacity"
                    } else {
                        "setup"
                    };
                    json!({"extension":slot.name,"state":"unavailable","reason":reason})
                })
                .collect(),
        )
    }
    /// Validate the optional projection without consulting declarations or touching storage.
    pub fn validate(value: &Value) -> bool {
        let Some(values) = value.as_array() else {
            return false;
        };
        let mut names = std::collections::BTreeSet::new();
        values.iter().all(|value| {
            let Ok(item) = serde_json::from_value::<ObjectChannelStatus>(value.clone()) else {
                return false;
            };
            value.as_object().is_some_and(|fields| {
                fields.len() == if item.state == "unavailable" { 3 } else { 2 }
            }) && ExtensionId::new(&item.extension).is_ok()
                && names.insert(item.extension)
                && match item.state.as_str() {
                    "ready" | "starting" => item.reason.is_none(),
                    "unavailable" => item.reason.is_some_and(|r| {
                        ["storage", "setup", "channel-ended", "capacity"].contains(&r.as_str())
                    }),
                    _ => false,
                }
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectChannelStatus {
    extension: String,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}
