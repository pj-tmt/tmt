//! Trusted same-user process seam. Squad admits owner/lead writes; provider
//! adapters admit their own launch's turn boundary before requesting a claim.
use super::{Fault, Request, invalid};
use crate::{
    digest,
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tmt_core::{
    limits::MAX_JS_SAFE_INTEGER,
    request::{
        RequestError, RequestService,
        digest::{DigestOpportunity, DigestPolicyView, DigestPolicyWrite, DigestState},
    },
};

pub enum Operation {
    Write(DigestPolicyWrite),
    Show(Vec<String>),
    Stats(Vec<String>),
    Due(String),
    Flush {
        identity: String,
        /// Skip only the live-idle evidence; every other send guard stays.
        when_busy: bool,
    },
    Read {
        identity: String,
        checklist: Option<String>,
        after: u64,
        limit: u64,
    },
    History {
        identity: String,
        checklist: Option<String>,
        after: u64,
        limit: u64,
    },
    Claim {
        identity: String,
        opportunity: DigestOpportunity,
    },
    Settle {
        identity: String,
        checklist: String,
        token: String,
        state: DigestState,
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
struct Due {
    identity_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FlushInput {
    identity_id: String,
    #[serde(default)]
    when_busy: bool,
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
        "digest.policy.set" | "digest.policy.clear" => {
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
            Operation::Write(DigestPolicyWrite {
                identity_id: v.identity_id,
                owner_identity_id: v.owner_identity_id,
                setter_identity_id: v.setter_identity_id,
                expected_revision: v.expected_revision,
                until_ms: until,
            })
        }
        "digest.policy.show" | "digest.stats.show" => {
            let v: Show = serde_json::from_slice(input).map_err(|_| invalid())?;
            if v.identities.is_empty() || v.identities.len() > 256 {
                return Err(invalid());
            }
            for id in &v.identities {
                uuid(id)?;
            }
            if operation == "digest.stats.show" {
                Operation::Stats(v.identities)
            } else {
                Operation::Show(v.identities)
            }
        }
        "digest.checklist.dueNow" => {
            let v: Due = serde_json::from_slice(input).map_err(|_| invalid())?;
            uuid(&v.identity_id)?;
            Operation::Due(v.identity_id)
        }
        "digest.checklist.flush" => {
            let v: FlushInput = serde_json::from_slice(input).map_err(|_| invalid())?;
            uuid(&v.identity_id)?;
            Operation::Flush {
                identity: v.identity_id,
                when_busy: v.when_busy,
            }
        }
        "digest.checklist.read" | "digest.checklist.history" => {
            let v: Read = serde_json::from_slice(input).map_err(|_| invalid())?;
            uuid(&v.identity_id)?;
            if let Some(id) = &v.checklist_id {
                uuid(id)?;
            }
            if v.after > MAX_JS_SAFE_INTEGER || !(1..=128).contains(&v.limit) {
                return Err(invalid());
            }
            if operation.ends_with(".history") {
                Operation::History {
                    identity: v.identity_id,
                    checklist: v.checklist_id,
                    after: v.after,
                    limit: v.limit,
                }
            } else {
                Operation::Read {
                    identity: v.identity_id,
                    checklist: v.checklist_id,
                    after: v.after,
                    limit: v.limit,
                }
            }
        }
        "digest.checklist.claim" => {
            let v: Claim = serde_json::from_slice(input).map_err(|_| invalid())?;
            uuid(&v.identity_id)?;
            let opportunity = match v.opportunity.as_str() {
                "turn_boundary" => DigestOpportunity::TurnBoundary,
                "idle" => DigestOpportunity::Idle,
                _ => return Err(invalid()),
            };
            Operation::Claim {
                identity: v.identity_id,
                opportunity,
            }
        }
        "digest.checklist.settle" => {
            let v: Settle = serde_json::from_slice(input).map_err(|_| invalid())?;
            for id in [&v.identity_id, &v.checklist_id, &v.attempt_token] {
                uuid(id)?;
            }
            let state = DigestState::parse(&v.outcome)
                .filter(|s| *s != DigestState::Claimed)
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
    Ok(Request::Digest(Box::new(op)))
}
fn view(value: &DigestPolicyView) -> Value {
    let policy = value.policy.as_ref();
    json!({"identityId":policy.map(|p|p.identity_id.as_str()),"revision":policy.map_or(0,|p|p.revision),"active":policy.is_some_and(|p|p.active(value.observed_at_ms)),"digestUntilMs":policy.map_or(0,|p|p.until_ms),"remainingMs":policy.map_or(0,|p|p.remaining_ms(value.observed_at_ms)),"heldCount":value.held_count,"activeChecklist":value.active_checklist.as_ref().map(digest::checklist_value),"ownerIdentityId":policy.map(|p|p.owner_identity_id.as_str()),"setterIdentityId":policy.map(|p|p.setter_identity_id.as_str())})
}
pub(super) fn execute(
    storage: &mut Storage,
    operation: Operation,
    delay: Duration,
) -> Result<Vec<u8>, Fault> {
    let document = match operation {
        Operation::Write(input) => view(
            &RequestService::new(&mut *storage, wall_time_ms)
                .write_digest(input)
                .map_err(error)?,
        ),
        Operation::Show(ids) => {
            let views = RequestService::new(&mut *storage, wall_time_ms)
                .digest_policies(&ids)
                .map_err(error)?;
            json!({"policies":views.iter().zip(ids).map(|(v,id)|{let mut v=view(v);v["identityId"]=json!(id);v}).collect::<Vec<_>>()})
        }
        Operation::Due(identity) => {
            let due = RequestService::new(storage, wall_time_ms)
                .make_digest_due(&identity)
                .map_err(error)?;
            json!({"identityId":identity,"heldCount":due.held_count,"throughSequence":due.through_sequence})
        }
        Operation::Flush {
            identity,
            when_busy,
        } => {
            let stats = RequestService::new(&mut *storage, wall_time_ms)
                .digest_stats(std::slice::from_ref(&identity))
                .map_err(error)?;
            let state = if stats[0].due_count == 0 {
                "nothing_due"
            } else {
                let readiness = if when_busy {
                    digest::Readiness::Live
                } else {
                    digest::Readiness::Idle
                };
                match digest::flush(storage, &identity, delay, readiness).map_err(error)? {
                    None => "not_idle",
                    Some(DigestState::Delivered) => "delivered",
                    Some(DigestState::Uncertain | DigestState::Claimed) => "uncertain",
                    Some(DigestState::Unsent) => "unavailable",
                }
            };
            json!({"identityId":identity,"state":state})
        }
        Operation::Stats(ids) => {
            let stats = RequestService::new(storage, wall_time_ms)
                .digest_stats(&ids)
                .map_err(error)?;
            json!({"stats":stats.iter().map(|s| json!({"identityId":s.identity_id,"heldCount":s.held_count,"oldestHeldAgeMs":s.oldest_held_age_ms,"deliveredDigests":s.delivered_digests,"dueCount":s.due_count,"nextEligibleAtMs":s.next_eligible_at_ms,"observedAtMs":s.observed_at_ms})).collect::<Vec<_>>()})
        }
        Operation::Read {
            identity,
            checklist,
            after,
            limit,
        } => digest::read(storage, &identity, checklist.as_deref(), after, limit).map_err(error)?,
        Operation::History {
            identity,
            checklist,
            after,
            limit,
        } => {
            let page = RequestService::new(storage, wall_time_ms)
                .digest_checklist_history(&identity, checklist.as_deref(), after, limit)
                .map_err(error)?;
            history_value(&identity, &page)
        }
        Operation::Claim {
            identity,
            opportunity,
        } => {
            if opportunity == DigestOpportunity::Idle
                && digest::idle_entry(storage, &identity)
                    .map_err(|_| Fault::unavailable())?
                    .is_none()
            {
                return encode(json!({"claimed":false}));
            }
            match digest::claim(storage, &identity, opportunity).map_err(error)? {
                None => json!({"claimed":false}),
                Some((batch, text)) => {
                    let page = match digest::read(storage, &identity, Some(&batch.id), 0, 128) {
                        Ok(p) => p,
                        Err(e) => {
                            digest::release_unsent(storage, &batch).map_err(error)?;
                            return Err(error(e));
                        }
                    };
                    json!({"claimed":true,"checklist":digest::checklist_value(&batch),"page":page,"text":text})
                }
            }
        }
        Operation::Settle {
            identity,
            checklist,
            token,
            state,
        } => {
            json!({"changed":RequestService::new(storage,wall_time_ms).settle_digest_checklist(&identity,&checklist,&token,state).map_err(error)?,"state":state.as_str()})
        }
    };
    encode(document)
}
fn encode(value: Value) -> Result<Vec<u8>, Fault> {
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
}
fn history_value(identity: &str, page: &tmt_core::request::digest::DigestHistoryRead) -> Value {
    json!({"identityId":identity,"observedAtMs":page.observed_at_ms,
        "throughSequence":page.through_sequence,"retainedFromSequence":page.retained_from_sequence,
        "legacySettledCount":page.legacy_settled_count,"deliveredDigests":page.delivered_digests,
        "nextAfter":page.next_after,"checklists":page.checklists.iter().map(|row| {
            let evidence = &row.evidence;
            json!({"checklistId":evidence.checklist.id,"settlementSequence":evidence.settlement_sequence,
                "claimedAtMs":evidence.checklist.created_at_ms,"settledAtMs":evidence.settled_at_ms,
                "outcome":evidence.checklist.state.as_str(),"opportunity":evidence.opportunity.map(|o| match o {
                    DigestOpportunity::Idle => "idle", DigestOpportunity::TurnBoundary => "turn_boundary"}),
                "sealedCount":evidence.sealed_count,"retainedCount":row.retained_count,
                "membershipComplete":evidence.sealed_count.is_some_and(|count| count == row.retained_count),
                "nextItemAfter":row.next_item_after,"items":row.items.iter().map(|item| json!({
                    "sequence":item.sequence,"createdAtMs":item.created_at_ms,
                    "contextTokensAtArrival":item.context_tokens_at_arrival,
                    "contextObservedAtMs":item.context_observed_at_ms})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>()})
}

fn error(error: RequestError<StorageError>) -> Fault {
    match error {
        RequestError::Digest(reason) => Fault::new(
            reason.code(),
            "Digest operation refused; inspect the current policy or checklist before retrying.",
        ),
        RequestError::Invalid(_) => invalid(),
        _ => Fault::unavailable(),
    }
}

#[cfg(test)]
mod tests;
