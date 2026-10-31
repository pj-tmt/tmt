//! Preview and publication share the same normalized intent and policy owner.
use super::{
    OrchReadFailure, OrchZone, read,
    wire::{self, OrchPlanDocument},
};
use crate::storage::{OrchStoredState, Storage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tmt_core::{
    content_digest,
    dispatch::canonical_id,
    orchestration::{
        mutation::{OrchMutation, changed_snapshot},
        *,
    },
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum OrchMutationDocument {
    #[serde(rename = "plan.add")]
    AddPlan { candidate: Box<OrchPlanDocument> },
    #[serde(rename = "plan.set")]
    SetPlan {
        plan: String,
        candidate: Box<OrchPlanDocument>,
    },
    #[serde(rename = "plan.rm")]
    RemovePlan { plan: String },
    #[serde(rename = "focus.set")]
    SetFocus {
        #[serde(rename = "identityId")]
        identity_id: String,
        #[serde(rename = "startMs")]
        start_ms: u64,
        #[serde(rename = "endMs")]
        end_ms: u64,
    },
    #[serde(rename = "focus.clear")]
    ClearFocus {
        #[serde(rename = "identityId")]
        identity_id: String,
    },
    /// Replace the leads list (#4892).
    #[serde(rename = "leads.set")]
    SetLeads {
        #[serde(rename = "identityIds")]
        identity_ids: Vec<String>,
    },
    /// Allow one ordered route past its sending rule until `untilMs`. `actorId` attributes
    /// the caller (a verified agent must be a lead); it authenticates no one.
    #[serde(rename = "route.allow")]
    AllowRoute {
        #[serde(rename = "fromId")]
        from_id: String,
        #[serde(rename = "toId")]
        to_id: String,
        #[serde(rename = "untilMs")]
        until_ms: u64,
        #[serde(rename = "actorId", default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<String>,
    },
}
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase", deny_unknown_fields)]
pub enum OrchWriteQuery {
    Preview {
        mutation: OrchMutationDocument,
    },
    Apply {
        mutation: OrchMutationDocument,
        #[serde(rename = "expectedConfigurationRevision")]
        expected_configuration_revision: u64,
        #[serde(rename = "previewAtMs")]
        preview_at_ms: u64,
        #[serde(rename = "previewDigest")]
        preview_digest: String,
        consent: bool,
    },
}
pub fn decode_orch_write(bytes: &[u8]) -> Result<OrchWriteQuery, OrchError> {
    if bytes.len() > wire::ORCH_DOCUMENT_BYTES + 8192 {
        return Err(OrchError::Limit);
    }
    let query: OrchWriteQuery = serde_json::from_slice(bytes).map_err(|_| OrchError::Invalid)?;
    let mutation = match &query {
        OrchWriteQuery::Preview { mutation } => mutation,
        OrchWriteQuery::Apply {
            mutation,
            expected_configuration_revision,
            preview_at_ms,
            preview_digest,
            consent,
        } => {
            if !consent
                || *expected_configuration_revision > tmt_core::limits::MAX_JS_SAFE_INTEGER
                || !wire::valid_ms(*preview_at_ms)
                || !content_digest::is_sha256(preview_digest)
            {
                return Err(OrchError::Invalid);
            }
            mutation
        }
    };
    validate_mutation(mutation)?;
    Ok(query)
}
fn validate_mutation(mutation: &OrchMutationDocument) -> Result<(), OrchError> {
    let plan_selector = |plan: &str| -> Result<(), OrchError> {
        if plan.trim().is_empty() || plan.len() > 80 || plan.chars().any(char::is_control) {
            Err(OrchError::Invalid)
        } else {
            Ok(())
        }
    };
    let candidate = match mutation {
        OrchMutationDocument::AddPlan { candidate } => Some(candidate),
        OrchMutationDocument::SetPlan { plan, candidate } => {
            plan_selector(plan)?;
            Some(candidate)
        }
        OrchMutationDocument::RemovePlan { plan } => {
            plan_selector(plan)?;
            None
        }
        OrchMutationDocument::SetFocus {
            identity_id,
            start_ms,
            end_ms,
        } => {
            if !canonical_id(identity_id)
                || !tmt_core::orchestration::mutation::valid_focus_window(*start_ms, *end_ms)
            {
                return Err(OrchError::Invalid);
            }
            None
        }
        OrchMutationDocument::ClearFocus { identity_id } => {
            if !canonical_id(identity_id) {
                return Err(OrchError::Invalid);
            }
            None
        }
        OrchMutationDocument::SetLeads { identity_ids } => {
            if identity_ids.len() > tmt_core::orchestration::ORCH_MAX_MEMBERS
                || identity_ids.iter().any(|id| !canonical_id(id))
            {
                return Err(OrchError::Invalid);
            }
            None
        }
        OrchMutationDocument::AllowRoute {
            from_id,
            to_id,
            until_ms,
            actor_id,
        } => {
            if !canonical_id(from_id)
                || !canonical_id(to_id)
                || !wire::valid_ms(*until_ms)
                || actor_id.as_deref().is_some_and(|id| !canonical_id(id))
            {
                return Err(OrchError::Invalid);
            }
            None
        }
    };
    if let Some(candidate) = candidate {
        candidate.to_core()?;
        if serde_json::to_vec(candidate)
            .map_err(|_| OrchError::Invalid)?
            .len()
            > wire::ORCH_DOCUMENT_BYTES
        {
            return Err(OrchError::Limit);
        }
    }
    Ok(())
}
fn normalized(
    snapshot: &OrchSnapshot,
    doc: &OrchMutationDocument,
    at: u64,
) -> Result<(OrchMutationDocument, OrchMutation), OrchError> {
    let selected = |selector: &str| -> Result<&OrchPlan, OrchError> {
        let id = read::selected_plan(snapshot, Some(selector))?.ok_or(OrchError::NotFound)?;
        snapshot
            .plans
            .iter()
            .find(|p| p.id == id)
            .ok_or(OrchError::NotFound)
    };
    Ok(match doc {
        OrchMutationDocument::AddPlan { candidate } => {
            (doc.clone(), OrchMutation::AddPlan(candidate.to_core()?))
        }
        OrchMutationDocument::SetPlan { plan, candidate } => {
            let current = selected(plan)?;
            if candidate.id != current.id {
                return Err(OrchError::Invalid);
            }
            if candidate.revision != current.revision {
                return Err(OrchError::Conflict);
            }
            (
                OrchMutationDocument::SetPlan {
                    plan: current.id.clone(),
                    candidate: candidate.clone(),
                },
                OrchMutation::SetPlan {
                    expected_revision: current.revision,
                    plan: candidate.to_core()?,
                },
            )
        }
        OrchMutationDocument::RemovePlan { plan } => {
            let current = selected(plan)?;
            (
                OrchMutationDocument::RemovePlan {
                    plan: current.id.clone(),
                },
                OrchMutation::RemovePlan {
                    id: current.id.clone(),
                    expected_revision: current.revision,
                },
            )
        }
        OrchMutationDocument::SetFocus {
            identity_id,
            start_ms,
            end_ms,
        } => {
            if !canonical_id(identity_id) {
                return Err(OrchError::Invalid);
            }
            (
                doc.clone(),
                OrchMutation::SetFocus {
                    identity_id: identity_id.clone(),
                    start_ms: *start_ms,
                    end_ms: *end_ms,
                },
            )
        }
        OrchMutationDocument::ClearFocus { identity_id } => {
            if !canonical_id(identity_id) {
                return Err(OrchError::Invalid);
            }
            (
                doc.clone(),
                OrchMutation::ClearFocus {
                    identity_id: identity_id.clone(),
                },
            )
        }
        OrchMutationDocument::SetLeads { identity_ids } => (
            doc.clone(),
            OrchMutation::SetLeads {
                identity_ids: identity_ids.clone(),
            },
        ),
        OrchMutationDocument::AllowRoute {
            from_id,
            to_id,
            until_ms,
            actor_id,
        } => {
            // A verified agent may allow a route only as a current lead; a person (no
            // verified caller) may always. Local attribution, not authentication.
            if let Some(actor) = actor_id
                && !crate::storage::orchestration_active_leads(snapshot).contains(&actor.as_str())
            {
                return Err(OrchError::NotPermitted);
            }
            (
                doc.clone(),
                OrchMutation::AllowRoute {
                    grant: tmt_core::orchestration::sending::OrchRouteGrant {
                        from_id: from_id.clone(),
                        to_id: to_id.clone(),
                        until_ms: *until_ms,
                        granted_by: actor_id.clone(),
                    },
                    now_ms: at,
                },
            )
        }
    })
}

struct Preview {
    candidate: OrchSnapshot,
    value: Value,
    digest: String,
    valid_until: Option<u64>,
}
fn preview(
    state: &OrchStoredState,
    zone: &OrchZone,
    doc: &OrchMutationDocument,
    at: u64,
) -> Result<Preview, OrchError> {
    if !wire::valid_ms(at) {
        return Err(OrchError::Invalid);
    }
    let (doc, mutation) = normalized(&state.snapshot, doc, at)?;
    let candidate = changed_snapshot(&state.snapshot, &mutation)?;
    let before = read::overview(&state.snapshot, zone, at, 1, None, at)?;
    let after = read::overview(&candidate, zone, at, 1, None, at)?;
    let changes = read::changed_members(&before, &after)?;
    let valid_until = [&before, &after]
        .into_iter()
        .flat_map(|v| v["members"].as_array().into_iter().flatten())
        .filter_map(|m| m["nextChangeAtMs"].as_u64())
        .min();
    let mut value = json!({"schemaVersion":1,"configurationRevision":state.snapshot.revision,"previewAtMs":at,"timeZone":before["timeZone"],"mutation":doc,"before":before,"after":after,"changes":changes,"validUntilMs":valid_until,"requiresConsent":true});
    // A higher plan can hide a changed Base input at the selected instant. That
    // input still belongs to the preview fence (and can matter after expiry).
    let base_inputs:Vec<_>=state.snapshot.base_receiving.iter().map(|(id,input)|json!({"identityId":id,"revision":input.revision,"receiving":wire::receiving_value(&input.receiving)})).collect();
    let guard = json!({"preview":value,"configuredTimeZone":state.time_zone,"digestEnabled":state.snapshot.digest_enabled,"digestRevision":state.snapshot.digest_revision,"baseReceiving":base_inputs});
    let digest = content_digest::framed_sha256(
        b"tmt-orchestration-preview-v1",
        &serde_json::to_vec(&guard).map_err(|_| OrchError::Invalid)?,
    );
    value["previewDigest"] = json!(digest);
    Ok(Preview {
        candidate,
        value,
        digest,
        valid_until,
    })
}
fn encode(value: &Value) -> Result<Vec<u8>, OrchReadFailure> {
    let bytes = serde_json::to_vec(value).map_err(|_| OrchError::Invalid)?;
    if bytes.len() > crate::api::OUTPUT_LIMIT {
        return Err(OrchError::Limit.into());
    }
    Ok(bytes)
}
fn receiving_configuration(snapshot: &OrchSnapshot) -> Vec<OrchPlan> {
    let mut plans = snapshot.plans.clone();
    let strip = |fields: &mut OrchFields| {
        fields.sending = None;
        fields.routine = None;
        fields.talk_limit = None;
    };
    for plan in &mut plans {
        plan.revision = 0;
        plan.name.clear();
        strip(&mut plan.fields);
        for fields in plan.members.values_mut() {
            strip(fields);
        }
        plan.members
            .retain(|_, fields| *fields != OrchFields::default());
    }
    plans.retain(|p| p.fields != OrchFields::default() || !p.members.is_empty());
    plans
}
pub fn orchestration_write(
    storage: &mut Storage,
    query: OrchWriteQuery,
    clock: impl Fn() -> u64,
) -> Result<Vec<u8>, OrchReadFailure> {
    match query {
        OrchWriteQuery::Preview { mutation } => {
            let state = storage.orchestration_snapshot(None)?;
            let zone = OrchZone::load(state.time_zone.as_deref())?;
            encode(&preview(&state, &zone, &mutation, clock())?.value)
        }
        OrchWriteQuery::Apply {
            mutation,
            expected_configuration_revision,
            preview_at_ms,
            preview_digest,
            consent,
        } => {
            if !consent
                || !content_digest::is_sha256(&preview_digest)
                || !wire::valid_ms(preview_at_ms)
            {
                return Err(OrchError::Invalid.into());
            }
            let observed = storage.orchestration_snapshot(None)?;
            let zone = OrchZone::load(observed.time_zone.as_deref())?;
            storage.update_orchestration(|state| {
                let now=clock();
                if !wire::valid_ms(now) || preview_at_ms>now {return Err(OrchError::Invalid.into());}
                if state.snapshot.revision!=expected_configuration_revision || state.time_zone!=observed.time_zone {return Err(OrchError::Conflict.into());}
                let p=preview(state,&zone,&mutation,preview_at_ms)?;
                if p.digest!=preview_digest || p.valid_until.is_some_and(|end|now>=end) || matches!(&mutation,OrchMutationDocument::SetFocus {end_ms,..} if *end_ms<=now) {return Err(OrchError::Conflict.into());}
                if state.snapshot.digest_revision==0 &&
                    (state.snapshot.focus!=p.candidate.focus || receiving_configuration(&state.snapshot)!=receiving_configuration(&p.candidate)) {
                    return Err(OrchError::ReceivingUnavailable.into());
                }
                let value=json!({"schemaVersion":1,"changed":p.candidate!=state.snapshot,"configurationRevision":p.candidate.revision,"requiresConsent":false,"preview":p.value});
                Ok((p.candidate,encode(&value)?))
            })
        }
    }
}

#[cfg(test)]
mod tests;
