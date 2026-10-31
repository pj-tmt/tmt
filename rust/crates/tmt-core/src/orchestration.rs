//! One pure policy resolver. Calendar conversion and durable state belong to ports.

use std::collections::{BTreeMap, BTreeSet};

pub const ORCH_MAX_PLANS: usize = 128;
pub const ORCH_MAX_MEMBERS: usize = 256;
pub const ORCH_MAX_TRANSITIONS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchError {
    Invalid,
    Conflict,
    Limit,
    TimeZone,
    NotFound,
    MemberNotFound,
    ReceivingUnavailable,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchState {
    Awake,
    Low,
    Focus,
    Sleep,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchReceiving {
    Direct,
    Auto,
    Batch(u64),
    Urgent,
    Hold,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchSending {
    Anyone,
    Leads,
    Paused,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchRoutine {
    Run,
    Hold,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrchFields {
    pub state: Option<OrchState>,
    pub receiving: Option<OrchReceiving>,
    pub sending: Option<OrchSending>,
    pub routine: Option<OrchRoutine>,
    /// None inherits; Some(None) explicitly removes a lower hourly limit.
    pub talk_limit: Option<Option<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchTarget {
    All,
    Identity(String),
    Pattern(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchSchedule {
    Always,
    Once {
        start_ms: u64,
        end_ms: u64,
    },
    Weekly {
        /// Local Julian day, normalized to the Monday of the anchor week.
        anchor_day: i32,
        interval_weeks: u8,
        weekdays: Vec<u8>,
        start_minute: u16,
        end_minute: u16,
        all_day: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchPlan {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub priority: i32,
    pub enabled: bool,
    pub targets: Vec<OrchTarget>,
    pub schedule: OrchSchedule,
    pub fields: OrchFields,
    pub members: BTreeMap<String, OrchFields>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchIdentity {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchFocus {
    pub identity_id: String,
    pub revision: u64,
    pub start_ms: u64,
    pub end_ms: u64,
}
/// The configured Digest mode is the bottom receiving input, not a second
/// precedence pass after plans. Its revision identifies that published input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchBaseReceiving {
    pub revision: u64,
    pub receiving: OrchReceiving,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchSnapshot {
    pub revision: u64,
    pub identities: Vec<OrchIdentity>,
    pub plans: Vec<OrchPlan>,
    pub focus: Vec<OrchFocus>,
    pub base_receiving: BTreeMap<String, OrchBaseReceiving>,
    /// Digest's explicit master switch is above all scheduled receiving.
    pub digest_enabled: bool,
    pub digest_revision: u64,
}
impl Default for OrchSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            identities: Vec::new(),
            plans: Vec::new(),
            focus: Vec::new(),
            base_receiving: BTreeMap::new(),
            digest_enabled: true,
            digest_revision: 0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrchSlot {
    pub start_ms: u64,
    pub end_ms: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchSourceKind {
    Base,
    Plan,
    Member,
    Focus,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchSource {
    pub kind: OrchSourceKind,
    pub plan_id: Option<String>,
    pub identity_id: Option<String>,
    pub revision: u64,
    pub slot: Option<OrchSlot>,
    pub defaulted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchValue<T> {
    pub value: T,
    pub source: OrchSource,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchEffective {
    pub state: OrchValue<OrchState>,
    pub receiving: OrchValue<OrchReceiving>,
    pub sending: OrchValue<OrchSending>,
    pub routine: OrchValue<OrchRoutine>,
    pub talk_limit: OrchValue<Option<u32>>,
    pub active_plans: Vec<String>,
    pub next_change_at_ms: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchTimelineInterval {
    pub start_ms: u64,
    pub end_ms: u64,
    pub plan_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchSubjectKind {
    Request,
    Identity,
    Job,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchDecisionAction {
    Block,
    Delay,
    Send,
    JobRun,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchDecisionOutcome {
    Allowed,
    Held,
    Sent,
    Uncertain,
    Failed,
    Completed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrchPolicyField {
    State,
    Receiving,
    Sending,
    Routine,
    TalkLimit,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchDecisionSource {
    pub field: OrchPolicyField,
    pub source: OrchSource,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchDecision {
    pub recorded_at_ms: u64,
    pub subject_kind: OrchSubjectKind,
    pub subject_id: String,
    pub actor_id: Option<String>,
    pub recipient_id: Option<String>,
    pub action: OrchDecisionAction,
    pub outcome: OrchDecisionOutcome,
    pub reason: String,
    pub configuration_revision: u64,
    pub time_zone: String,
    pub sources: Vec<OrchDecisionSource>,
}

/// Calendar arithmetic only. Implementations do not choose recurrence or policy.
pub trait OrchCalendar {
    fn day_at(&self, at_ms: u64) -> Result<i32, OrchError>;
    fn boundary(&self, day: i32, minute: u16, end: bool) -> Result<u64, OrchError>;
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    let p: Vec<_> = pattern.chars().collect();
    let n: Vec<_> = name.chars().collect();
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            retry += 1;
            j = retry;
            i = s + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

pub fn orch_matches(plan: &OrchPlan, identity: &OrchIdentity) -> bool {
    plan.targets.iter().any(|target| match target {
        OrchTarget::All => true,
        OrchTarget::Identity(id) => id == &identity.id,
        OrchTarget::Pattern(pattern) => pattern_matches(pattern, &identity.name),
    })
}

fn slot_on(
    schedule: &OrchSchedule,
    day: i32,
    calendar: &impl OrchCalendar,
) -> Result<Option<OrchSlot>, OrchError> {
    match schedule {
        OrchSchedule::Always => Ok(None),
        OrchSchedule::Once { start_ms, end_ms } => Ok(Some(OrchSlot {
            start_ms: *start_ms,
            end_ms: *end_ms,
        })),
        OrchSchedule::Weekly {
            anchor_day,
            interval_weeks,
            weekdays,
            start_minute,
            end_minute,
            all_day,
        } => {
            if !matches!(interval_weeks, 1 | 2)
                || *start_minute >= 1440
                || *end_minute >= 1440
                || (!*all_day && start_minute == end_minute)
            {
                return Err(OrchError::Invalid);
            }
            let weekday = day.rem_euclid(7) as u8 + 1;
            let monday = day - i32::from(weekday - 1);
            if !weekdays.contains(&weekday)
                || (monday - anchor_day)
                    .div_euclid(7)
                    .rem_euclid(i32::from(*interval_weeks))
                    != 0
            {
                return Ok(None);
            }
            let start = if *all_day { 0 } else { *start_minute };
            let end = if *all_day { 0 } else { *end_minute };
            let end_day = day + i32::from(*all_day || end < start);
            let start_ms = calendar.boundary(day, start, false)?;
            let end_ms = calendar.boundary(end_day, end, true)?;
            Ok((start_ms < end_ms).then_some(OrchSlot { start_ms, end_ms }))
        }
    }
}

pub fn orch_slots(
    schedule: &OrchSchedule,
    range: OrchSlot,
    calendar: &impl OrchCalendar,
) -> Result<Vec<OrchSlot>, OrchError> {
    match schedule {
        OrchSchedule::Always => Ok(vec![range]),
        OrchSchedule::Once { start_ms, end_ms } => Ok(((*start_ms < range.end_ms)
            && (*end_ms > range.start_ms))
            .then_some(OrchSlot {
                start_ms: *start_ms,
                end_ms: *end_ms,
            })
            .into_iter()
            .collect()),
        OrchSchedule::Weekly { .. } => {
            let first = calendar.day_at(range.start_ms)? - 1;
            let last = calendar.day_at(range.end_ms)?;
            if last - first > 33 {
                return Err(OrchError::Limit);
            }
            let mut slots = Vec::new();
            for day in first..=last {
                if let Some(slot) = slot_on(schedule, day, calendar)?
                    && slot.start_ms < range.end_ms
                    && slot.end_ms > range.start_ms
                {
                    slots.push(slot);
                }
            }
            Ok(slots)
        }
    }
}

fn apply(fields: &OrchFields, mut source: OrchSource, policy: &mut OrchEffective) {
    if let Some(state) = fields.state {
        policy.state = OrchValue {
            value: state,
            source: source.clone(),
        };
        let (receiving, routine) = match state {
            OrchState::Awake => (OrchReceiving::Direct, OrchRoutine::Run),
            OrchState::Low => (OrchReceiving::Batch(1_800_000), OrchRoutine::Run),
            OrchState::Focus => (OrchReceiving::Urgent, OrchRoutine::Hold),
            OrchState::Sleep => (OrchReceiving::Hold, OrchRoutine::Hold),
        };
        source.defaulted = true;
        policy.receiving = OrchValue {
            value: receiving,
            source: source.clone(),
        };
        policy.routine = OrchValue {
            value: routine,
            source: source.clone(),
        };
        source.defaulted = false;
    }
    if let Some(value) = &fields.receiving {
        policy.receiving = OrchValue {
            value: value.clone(),
            source: source.clone(),
        };
    }
    if let Some(value) = fields.sending {
        policy.sending = OrchValue {
            value,
            source: source.clone(),
        };
    }
    if let Some(value) = fields.routine {
        policy.routine = OrchValue {
            value,
            source: source.clone(),
        };
    }
    if let Some(value) = fields.talk_limit {
        policy.talk_limit = OrchValue { value, source };
    }
}

pub fn effective(
    snapshot: &OrchSnapshot,
    identity_id: &str,
    at_ms: u64,
    calendar: &impl OrchCalendar,
) -> Result<OrchEffective, OrchError> {
    if snapshot.plans.len() > ORCH_MAX_PLANS || snapshot.identities.len() > ORCH_MAX_MEMBERS {
        return Err(OrchError::Limit);
    }
    let identity = snapshot
        .identities
        .iter()
        .find(|i| i.id == identity_id)
        .ok_or(OrchError::NotFound)?;
    let base = OrchSource {
        kind: OrchSourceKind::Base,
        plan_id: None,
        identity_id: None,
        revision: 0,
        slot: None,
        defaulted: false,
    };
    let mut result = OrchEffective {
        state: OrchValue {
            value: OrchState::Awake,
            source: base.clone(),
        },
        receiving: OrchValue {
            value: OrchReceiving::Direct,
            source: base.clone(),
        },
        sending: OrchValue {
            value: OrchSending::Anyone,
            source: base.clone(),
        },
        routine: OrchValue {
            value: OrchRoutine::Run,
            source: base.clone(),
        },
        talk_limit: OrchValue {
            value: None,
            source: base,
        },
        active_plans: Vec::new(),
        next_change_at_ms: None,
    };
    if let Some(configured) = snapshot.base_receiving.get(identity_id) {
        result.receiving = OrchValue {
            value: configured.receiving.clone(),
            source: OrchSource {
                kind: OrchSourceKind::Base,
                plan_id: None,
                identity_id: Some(identity_id.into()),
                revision: configured.revision,
                slot: None,
                defaulted: false,
            },
        };
    }
    let mut plans: Vec<_> = snapshot
        .plans
        .iter()
        .filter(|p| p.enabled && orch_matches(p, identity))
        .collect();
    plans.sort_by_key(|p| p.priority);
    let mut priorities = BTreeSet::new();
    let day = calendar.day_at(at_ms)?;
    let horizon = calendar.boundary(day + 16, 0, false)?;
    let future = OrchSlot {
        start_ms: at_ms,
        end_ms: horizon,
    };
    let mut changes = BTreeSet::new();
    for plan in plans {
        if !priorities.insert(plan.priority) {
            return Err(OrchError::Invalid);
        }
        let slots = orch_slots(&plan.schedule, future, calendar)?;
        for slot in slots
            .iter()
            .filter(|_| !matches!(plan.schedule, OrchSchedule::Always))
        {
            for boundary in [slot.start_ms, slot.end_ms] {
                if boundary > at_ms {
                    changes.insert(boundary);
                }
            }
        }
        if let OrchSchedule::Once { start_ms, .. } = plan.schedule
            && start_ms > at_ms
        {
            changes.insert(start_ms);
        }
        let active = if matches!(plan.schedule, OrchSchedule::Always) {
            Some(None)
        } else {
            slots
                .into_iter()
                .find(|s| s.start_ms <= at_ms && at_ms < s.end_ms)
                .map(Some)
        };
        if let Some(slot) = active {
            result.active_plans.push(plan.id.clone());
            let source = OrchSource {
                kind: OrchSourceKind::Plan,
                plan_id: Some(plan.id.clone()),
                identity_id: None,
                revision: plan.revision,
                slot,
                defaulted: false,
            };
            apply(&plan.fields, source.clone(), &mut result);
            if let Some(fields) = plan.members.get(identity_id) {
                apply(
                    fields,
                    OrchSource {
                        kind: OrchSourceKind::Member,
                        identity_id: Some(identity_id.into()),
                        ..source
                    },
                    &mut result,
                );
            }
        }
    }
    for focus in snapshot
        .focus
        .iter()
        .filter(|f| f.identity_id == identity_id)
    {
        for boundary in [focus.start_ms, focus.end_ms] {
            if boundary > at_ms {
                changes.insert(boundary);
            }
        }
        if focus.start_ms <= at_ms && at_ms < focus.end_ms {
            apply(
                &OrchFields {
                    state: Some(OrchState::Focus),
                    ..Default::default()
                },
                OrchSource {
                    kind: OrchSourceKind::Focus,
                    identity_id: Some(identity_id.into()),
                    plan_id: None,
                    revision: focus.revision,
                    slot: Some(OrchSlot {
                        start_ms: focus.start_ms,
                        end_ms: focus.end_ms,
                    }),
                    defaulted: false,
                },
                &mut result,
            );
        }
    }
    if !snapshot.digest_enabled {
        result.receiving = OrchValue {
            value: OrchReceiving::Direct,
            source: OrchSource {
                kind: OrchSourceKind::Base,
                plan_id: None,
                identity_id: None,
                revision: snapshot.digest_revision,
                slot: None,
                defaulted: false,
            },
        };
    }
    result.next_change_at_ms = changes.into_iter().next();
    Ok(result)
}

pub fn orch_timeline(
    snapshot: &OrchSnapshot,
    range: OrchSlot,
    calendar: &impl OrchCalendar,
) -> Result<Vec<OrchTimelineInterval>, OrchError> {
    let mut boundaries = BTreeSet::from([range.start_ms, range.end_ms]);
    let mut slots = Vec::new();
    for plan in snapshot
        .plans
        .iter()
        .filter(|p| p.enabled && snapshot.identities.iter().any(|i| orch_matches(p, i)))
    {
        for slot in orch_slots(&plan.schedule, range, calendar)? {
            boundaries.insert(slot.start_ms.max(range.start_ms));
            boundaries.insert(slot.end_ms.min(range.end_ms));
            slots.push((&plan.id, slot));
            if boundaries.len() > ORCH_MAX_TRANSITIONS {
                return Err(OrchError::Limit);
            }
        }
    }
    let boundaries: Vec<_> = boundaries.into_iter().collect();
    Ok(boundaries
        .windows(2)
        .map(|pair| {
            let mut plan_ids: Vec<_> = slots
                .iter()
                .filter(|(_, slot)| slot.start_ms <= pair[0] && pair[0] < slot.end_ms)
                .map(|(id, _)| (*id).clone())
                .collect();
            plan_ids.sort();
            plan_ids.dedup();
            OrchTimelineInterval {
                start_ms: pair[0],
                end_ms: pair[1],
                plan_ids,
            }
        })
        .collect())
}

pub mod mutation;
pub mod receiving;
pub mod sending;
#[cfg(test)]
mod tests;
