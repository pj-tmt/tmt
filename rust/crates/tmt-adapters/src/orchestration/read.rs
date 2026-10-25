use super::{
    OrchZone, orch_error_code, orch_error_message,
    wire::{self, OrchReadQuery},
};
use crate::storage::{ORCH_JOURNAL_RETENTION_MS, OrchStoredState, Storage, StorageError};
use serde_json::{Value, json};
use tmt_core::orchestration::*;

#[derive(Debug)]
pub enum OrchReadFailure {
    Policy(OrchError),
    Storage(StorageError),
}
impl From<OrchError> for OrchReadFailure {
    fn from(e: OrchError) -> Self {
        Self::Policy(e)
    }
}
impl From<StorageError> for OrchReadFailure {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}
impl OrchReadFailure {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Policy(e) => orch_error_code(*e),
            Self::Storage(_) => "ORCH_STORAGE_UNAVAILABLE",
        }
    }
    pub fn message(&self) -> &'static str {
        match self {
            Self::Policy(e) => orch_error_message(*e),
            Self::Storage(_) => "Could not read orchestration storage.",
        }
    }
    pub fn exit(&self) -> u8 {
        if matches!(self, Self::Policy(OrchError::NotFound)) {
            3
        } else {
            1
        }
    }
}
impl std::fmt::Display for OrchReadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for OrchReadFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            _ => None,
        }
    }
}

fn selected_plan(
    snapshot: &OrchSnapshot,
    selector: Option<&str>,
) -> Result<Option<String>, OrchError> {
    let Some(selector) = selector else {
        return Ok(None);
    };
    if tmt_core::dispatch::canonical_id(selector) {
        return snapshot
            .plans
            .iter()
            .find(|p| p.id == selector)
            .map(|p| Some(p.id.clone()))
            .ok_or(OrchError::NotFound);
    }
    let matches: Vec<_> = snapshot
        .plans
        .iter()
        .filter(|p| p.id == selector || p.name == selector)
        .collect();
    match matches.as_slice() {
        [] => Err(OrchError::NotFound),
        [p] => Ok(Some(p.id.clone())),
        _ => Err(OrchError::Ambiguous),
    }
}
fn validate(snapshot: &OrchSnapshot) -> Result<(), OrchError> {
    if snapshot.plans.len() > ORCH_MAX_PLANS
        || snapshot.identities.len() > ORCH_MAX_MEMBERS
        || snapshot.focus.len() > ORCH_MAX_MEMBERS
    {
        return Err(OrchError::Limit);
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut priorities = std::collections::BTreeSet::new();
    if snapshot
        .plans
        .iter()
        .any(|p| !ids.insert(&p.id) || !priorities.insert(p.priority))
    {
        return Err(OrchError::Invalid);
    }
    Ok(())
}
fn overview(
    snapshot: &OrchSnapshot,
    zone: &OrchZone,
    at: u64,
    days: u8,
    plan: Option<&str>,
    observed: u64,
) -> Result<Value, OrchError> {
    validate(snapshot)?;
    let selected = selected_plan(snapshot, plan)?;
    let range = zone.range(at, days)?;
    let mut display = snapshot.clone();
    if let Some(id) = &selected {
        display.plans.retain(|p| &p.id == id);
    }
    let timeline = orch_timeline(&display, range, zone)?;
    let members=snapshot.identities.iter().map(|i|{
        let p=effective(snapshot,&i.id,at,zone)?;
        Ok(json!({"identityId":i.id,"name":i.name,"effective":wire::effective_value(&p),"activePlans":p.active_plans,"nextChangeAtMs":p.next_change_at_ms,"jobs":null}))
    }).collect::<Result<Vec<_>,OrchError>>()?;
    let plans = snapshot
        .plans
        .iter()
        .map(wire::plan_value)
        .collect::<Result<Vec<_>, _>>()?;
    let timeline: Vec<_> = timeline
        .iter()
        .map(|t| json!({"startMs":t.start_ms,"endMs":t.end_ms,"planIds":t.plan_ids}))
        .collect();
    Ok(
        json!({"schemaVersion":1,"configurationRevision":snapshot.revision,"observedAtMs":observed,"atMs":at,"timeZone":zone.name(),"range":{"startMs":range.start_ms,"endMs":range.end_ms},"selectedPlanId":selected,"plans":plans,"members":members,"timeline":timeline,"emptyState":if snapshot.plans.is_empty(){Some("Base only. Plan creation/editing and personal focus controls arrive in orchestration slice 2.")}else{None}}),
    )
}

pub fn orchestration_read(
    storage: &Storage,
    query: OrchReadQuery,
    observed: u64,
) -> Result<Vec<u8>, OrchReadFailure> {
    orchestration_read_with_time(storage, query, observed, None)
}
pub fn orchestration_read_with_time(
    storage: &Storage,
    query: OrchReadQuery,
    observed: u64,
    local_at: Option<&str>,
) -> Result<Vec<u8>, OrchReadFailure> {
    wire::validate_query(&query)?;
    if !wire::valid_ms(observed) {
        return Err(OrchError::Invalid.into());
    }
    let value = match query {
        OrchReadQuery::Why {
            subject,
            before,
            limit,
        } => {
            let page =
                storage.orchestration_journal(&subject, before, limit.unwrap_or(50), observed)?;
            json!({"schemaVersion":1,"observedAtMs":observed,"subject":subject,"items":page.items,"nextBefore":page.next_before,"coverage":{"retentionMs":ORCH_JOURNAL_RETENTION_MS,"retainedAfterMs":page.retained_after_ms,"liveProducers":false,"historyComplete":false}})
        }
        OrchReadQuery::Plans { plan } => {
            let stored = storage.orchestration_snapshot(None)?;
            if stored.snapshot.plans.len() > ORCH_MAX_PLANS {
                return Err(OrchError::Limit.into());
            }
            let selected = selected_plan(&stored.snapshot, plan.as_deref())?;
            let plans = stored
                .snapshot
                .plans
                .iter()
                .filter(|p| selected.as_ref().is_none_or(|id| &p.id == id))
                .map(wire::plan_value)
                .collect::<Result<Vec<_>, _>>()?;
            json!({"schemaVersion":1,"configurationRevision":stored.snapshot.revision,"observedAtMs":observed,"plans":plans})
        }
        OrchReadQuery::Overview {
            range_days,
            at_ms,
            plan,
            member_query,
        } => {
            let OrchStoredState {
                snapshot,
                time_zone,
            } = storage.orchestration_snapshot(member_query.as_deref())?;
            let zone = OrchZone::load(time_zone.as_deref())?;
            let at = local_at
                .map(|s| zone.parse_at(s))
                .transpose()?
                .or(at_ms)
                .unwrap_or(observed);
            overview(
                &snapshot,
                &zone,
                at,
                range_days.unwrap_or(1),
                plan.as_deref(),
                observed,
            )?
        }
        OrchReadQuery::Preview {
            candidate,
            range_days,
            at_ms,
            plan,
            member_query,
        } => {
            let OrchStoredState {
                snapshot,
                time_zone,
            } = storage.orchestration_snapshot(member_query.as_deref())?;
            let zone = OrchZone::load(time_zone.as_deref())?;
            let candidate = candidate.to_core()?;
            let mut after = snapshot.clone();
            if after.plans.iter().any(|p| {
                p.id != candidate.id
                    && (p.priority == candidate.priority || p.name == candidate.name)
            }) {
                return Err(OrchError::Invalid.into());
            }
            after.plans.retain(|p| p.id != candidate.id);
            after.plans.push(candidate);
            after.plans.sort_by_key(|p| p.priority);
            let at = local_at
                .map(|s| zone.parse_at(s))
                .transpose()?
                .or(at_ms)
                .unwrap_or(observed);
            let days = range_days.unwrap_or(1);
            let before = overview(&snapshot, &zone, at, days, plan.as_deref(), observed)?;
            let after = overview(&after, &zone, at, days, plan.as_deref(), observed)?;
            let changes: Vec<_> = before["members"]
                .as_array()
                .expect("overview members")
                .iter()
                .zip(after["members"].as_array().expect("overview members"))
                .filter_map(|(b, a)| {
                    let fields: Vec<_> = ["state", "receiving", "sending", "routine", "talkLimit"]
                        .into_iter()
                        .filter(|f| b["effective"][*f] != a["effective"][*f])
                        .collect();
                    (!fields.is_empty())
                        .then(|| json!({"identityId":b["identityId"],"fields":fields}))
                })
                .collect();
            json!({"schemaVersion":1,"observedAtMs":observed,"configurationRevision":snapshot.revision,"before":before,"after":after,"changes":changes})
        }
    };
    let bytes = serde_json::to_vec(&value).map_err(|_| OrchError::Invalid)?;
    if bytes.len() > crate::api::OUTPUT_LIMIT {
        return Err(OrchError::Limit.into());
    }
    Ok(bytes)
}
