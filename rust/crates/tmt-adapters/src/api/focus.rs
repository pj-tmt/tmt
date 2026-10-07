//! Trusted same-user process seam. Squad admits owner/lead writes; provider
//! adapters admit their own launch's turn boundary before requesting a claim.
use super::{Fault, Request, invalid};
use crate::{
    focus,
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tmt_core::{
    limits::MAX_JS_SAFE_INTEGER,
    request::{
        RequestError, RequestService,
        focus::{FocusOpportunity, FocusPolicyView, FocusPolicyWrite, FocusState},
    },
};

pub enum Operation {
    Write(FocusPolicyWrite),
    Show(Vec<String>),
    Read {
        identity: String,
        checklist: Option<String>,
        after: u64,
        limit: u64,
    },
    Claim {
        identity: String,
        opportunity: FocusOpportunity,
    },
    Settle {
        identity: String,
        checklist: String,
        token: String,
        state: FocusState,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Write {
    identity_id: String,
    owner_identity_id: String,
    setter_identity_id: String,
    expected_revision: u64,
    #[serde(default)]
    until_ms: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Show {
    identities: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Read {
    identity_id: String,
    #[serde(default)]
    checklist_id: Option<String>,
    #[serde(default = "page_limit")]
    limit: u64,
    #[serde(default)]
    after: u64,
}
fn page_limit() -> u64 {
    32
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Claim {
    identity_id: String,
    opportunity: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settle {
    identity_id: String,
    checklist_id: String,
    attempt_token: String,
    outcome: String,
}
fn uuid(id: &str) -> Result<(), Fault> {
    if tmt_core::dispatch::canonical_id(id) {
        Ok(())
    } else {
        Err(invalid())
    }
}

pub(super) fn decode(operation: &str, input: &[u8]) -> Result<Request, Fault> {
    let op = match operation {
        "focus.policy.set" | "focus.policy.clear" => {
            let v: Write = serde_json::from_slice(input).map_err(|_| invalid())?;
            for id in [&v.identity_id, &v.owner_identity_id, &v.setter_identity_id] {
                uuid(id)?;
            }
            if v.expected_revision > MAX_JS_SAFE_INTEGER {
                return Err(invalid());
            }
            let until = if operation.ends_with(".clear") {
                if v.until_ms.is_some() {
                    return Err(invalid());
                }
                0
            } else {
                v.until_ms
                    .filter(|n| *n > 0 && *n <= MAX_JS_SAFE_INTEGER)
                    .ok_or_else(invalid)?
            };
            Operation::Write(FocusPolicyWrite {
                identity_id: v.identity_id,
                owner_identity_id: v.owner_identity_id,
                setter_identity_id: v.setter_identity_id,
                expected_revision: v.expected_revision,
                until_ms: until,
            })
        }
        "focus.policy.show" => {
            let v: Show = serde_json::from_slice(input).map_err(|_| invalid())?;
            if v.identities.is_empty() || v.identities.len() > 256 {
                return Err(invalid());
            }
            for id in &v.identities {
                uuid(id)?;
            }
            Operation::Show(v.identities)
        }
        "focus.checklist.read" => {
            let v: Read = serde_json::from_slice(input).map_err(|_| invalid())?;
            uuid(&v.identity_id)?;
            if let Some(id) = &v.checklist_id {
                uuid(id)?;
            }
            if v.after > MAX_JS_SAFE_INTEGER || !(1..=128).contains(&v.limit) {
                return Err(invalid());
            }
            Operation::Read {
                identity: v.identity_id,
                checklist: v.checklist_id,
                after: v.after,
                limit: v.limit,
            }
        }
        "focus.checklist.claim" => {
            let v: Claim = serde_json::from_slice(input).map_err(|_| invalid())?;
            uuid(&v.identity_id)?;
            let opportunity = match v.opportunity.as_str() {
                "turn_boundary" => FocusOpportunity::TurnBoundary,
                "idle" => FocusOpportunity::Idle,
                _ => return Err(invalid()),
            };
            Operation::Claim {
                identity: v.identity_id,
                opportunity,
            }
        }
        "focus.checklist.settle" => {
            let v: Settle = serde_json::from_slice(input).map_err(|_| invalid())?;
            for id in [&v.identity_id, &v.checklist_id, &v.attempt_token] {
                uuid(id)?;
            }
            let state = FocusState::parse(&v.outcome)
                .filter(|s| *s != FocusState::Claimed)
                .ok_or_else(invalid)?;
            Operation::Settle {
                identity: v.identity_id,
                checklist: v.checklist_id,
                token: v.attempt_token,
                state,
            }
        }
        _ => return Err(invalid()),
    };
    Ok(Request::Focus(Box::new(op)))
}
fn view(value: &FocusPolicyView) -> Value {
    let policy = value.policy.as_ref();
    json!({"identityId":policy.map(|p|p.identity_id.as_str()),"revision":policy.map_or(0,|p|p.revision),"active":policy.is_some_and(|p|p.active(value.observed_at_ms)),"focusUntilMs":policy.map_or(0,|p|p.until_ms),"remainingMs":policy.map_or(0,|p|p.remaining_ms(value.observed_at_ms)),"heldCount":value.held_count,"activeChecklist":value.active_checklist.as_ref().map(focus::checklist_value),"ownerIdentityId":policy.map(|p|p.owner_identity_id.as_str()),"setterIdentityId":policy.map(|p|p.setter_identity_id.as_str())})
}
pub(super) fn execute(storage: &mut Storage, operation: Operation) -> Result<Vec<u8>, Fault> {
    let document = match operation {
        Operation::Write(input) => view(
            &RequestService::new(&mut *storage, wall_time_ms)
                .write_focus(input)
                .map_err(error)?,
        ),
        Operation::Show(ids) => {
            let views = RequestService::new(&mut *storage, wall_time_ms)
                .focus_policies(&ids)
                .map_err(error)?;
            json!({"policies":views.iter().zip(ids).map(|(v,id)|{let mut v=view(v);v["identityId"]=json!(id);v}).collect::<Vec<_>>()})
        }
        Operation::Read {
            identity,
            checklist,
            after,
            limit,
        } => focus::read(storage, &identity, checklist.as_deref(), after, limit).map_err(error)?,
        Operation::Claim {
            identity,
            opportunity,
        } => {
            if opportunity == FocusOpportunity::Idle
                && focus::idle_entry(storage, &identity)
                    .map_err(|_| Fault::unavailable())?
                    .is_none()
            {
                return encode(json!({"claimed":false}));
            }
            let batch = RequestService::new(&mut *storage, wall_time_ms)
                .claim_focus_checklist(
                    &identity,
                    tmt_core::operation::new_operation_id(),
                    tmt_core::operation::new_operation_id(),
                    opportunity,
                )
                .map_err(error)?;
            match batch {
                None => json!({"claimed":false}),
                Some(batch) => {
                    let page = match focus::read(storage, &identity, Some(&batch.id), 0, 128) {
                        Ok(p) => p,
                        Err(e) => {
                            RequestService::new(&mut *storage, wall_time_ms)
                                .settle_focus_checklist(
                                    &identity,
                                    &batch.id,
                                    &batch.attempt_token,
                                    FocusState::Unsent,
                                )
                                .map_err(error)?;
                            return Err(error(e));
                        }
                    };
                    json!({"claimed":true,"checklist":focus::checklist_value(&batch),"page":page,"text":focus::digest(&page,&batch)})
                }
            }
        }
        Operation::Settle {
            identity,
            checklist,
            token,
            state,
        } => {
            json!({"changed":RequestService::new(storage,wall_time_ms).settle_focus_checklist(&identity,&checklist,&token,state).map_err(error)?,"state":state.as_str()})
        }
    };
    encode(document)
}
fn encode(value: Value) -> Result<Vec<u8>, Fault> {
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
}
fn error(error: RequestError<StorageError>) -> Fault {
    match error {
        RequestError::Focus(reason) => Fault::new(
            reason.code(),
            "Focus operation refused; inspect the current policy or checklist before retrying.",
        ),
        RequestError::Invalid(_) => invalid(),
        _ => Fault::unavailable(),
    }
}
