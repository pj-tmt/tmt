//! Configuration changes over one captured snapshot. Consent, wire decoding and
//! durable publication belong to the invoking adapter; policy stays in effective().
use super::*;
use crate::{dispatch::canonical_id, limits::MAX_JS_SAFE_INTEGER};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchMutation {
    AddPlan(OrchPlan),
    SetPlan {
        expected_revision: u64,
        plan: OrchPlan,
    },
    RemovePlan {
        id: String,
        expected_revision: u64,
    },
    SetFocus {
        identity_id: String,
        start_ms: u64,
        end_ms: u64,
    },
    ClearFocus {
        identity_id: String,
    },
}

fn next(revision: u64) -> Result<u64, OrchError> {
    revision
        .checked_add(1)
        .filter(|r| *r <= MAX_JS_SAFE_INTEGER)
        .ok_or(OrchError::Limit)
}
pub fn valid_focus_window(start_ms: u64, end_ms: u64) -> bool {
    start_ms > 0
        && start_ms < end_ms
        && end_ms <= MAX_JS_SAFE_INTEGER
        && end_ms - start_ms <= 30 * 86_400_000
}

/// The returned snapshot is a candidate only: this function has no publication
/// or journal effect. The writer must still compare its captured revision.
pub fn changed_snapshot(
    snapshot: &OrchSnapshot,
    mutation: &OrchMutation,
) -> Result<OrchSnapshot, OrchError> {
    let mut candidate = snapshot.clone();
    match mutation {
        OrchMutation::AddPlan(plan) => {
            if plan.revision != 1 || !canonical_id(&plan.id) {
                return Err(OrchError::Invalid);
            }
            if candidate.plans.iter().any(|p| p.id == plan.id) {
                return Err(OrchError::Conflict);
            }
            candidate.plans.push(plan.clone());
        }
        OrchMutation::SetPlan {
            expected_revision,
            plan,
        } => {
            if plan.revision != *expected_revision {
                return Err(OrchError::Invalid);
            }
            let current = candidate
                .plans
                .iter_mut()
                .find(|p| p.id == plan.id)
                .ok_or(OrchError::NotFound)?;
            if current.revision != *expected_revision {
                return Err(OrchError::Conflict);
            }
            let mut proposed = plan.clone();
            proposed.revision = current.revision;
            if *current != proposed {
                proposed.revision = next(current.revision)?;
                *current = proposed;
            }
        }
        OrchMutation::RemovePlan {
            id,
            expected_revision,
        } => {
            let current = candidate
                .plans
                .iter()
                .position(|p| &p.id == id)
                .ok_or(OrchError::NotFound)?;
            if candidate.plans[current].revision != *expected_revision {
                return Err(OrchError::Conflict);
            }
            candidate.plans.remove(current);
        }
        OrchMutation::SetFocus {
            identity_id,
            start_ms,
            end_ms,
        } => {
            if !candidate.identities.iter().any(|i| &i.id == identity_id) {
                return Err(OrchError::MemberNotFound);
            }
            if !valid_focus_window(*start_ms, *end_ms) {
                return Err(OrchError::Invalid);
            }
            if let Some(current) = candidate
                .focus
                .iter_mut()
                .find(|f| &f.identity_id == identity_id)
            {
                if current.start_ms != *start_ms || current.end_ms != *end_ms {
                    *current = OrchFocus {
                        identity_id: identity_id.clone(),
                        revision: next(current.revision)?,
                        start_ms: *start_ms,
                        end_ms: *end_ms,
                    };
                }
            } else {
                candidate.focus.push(OrchFocus {
                    identity_id: identity_id.clone(),
                    revision: 1,
                    start_ms: *start_ms,
                    end_ms: *end_ms,
                });
            }
        }
        OrchMutation::ClearFocus { identity_id } => {
            if !candidate.identities.iter().any(|i| &i.id == identity_id) {
                return Err(OrchError::MemberNotFound);
            }
            candidate.focus.retain(|f| &f.identity_id != identity_id);
        }
    }
    if candidate.plans.len() > ORCH_MAX_PLANS || candidate.focus.len() > ORCH_MAX_MEMBERS {
        return Err(OrchError::Limit);
    }
    let mut names = BTreeSet::new();
    let mut priorities = BTreeSet::new();
    if candidate
        .plans
        .iter()
        .any(|p| !names.insert(&p.name) || !priorities.insert(p.priority))
    {
        return Err(OrchError::Conflict);
    }
    candidate.plans.sort_by_key(|p| p.priority);
    if candidate != *snapshot {
        candidate.revision = next(snapshot.revision)?;
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests;
