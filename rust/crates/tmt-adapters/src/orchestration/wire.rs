use super::calendar::{date_text, parse_date};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use tmt_core::{dispatch::canonical_id, limits::MAX_JS_SAFE_INTEGER, orchestration::*};

pub const ORCH_DOCUMENT_BYTES: usize = 32_768;
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
fn talk_limit<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<u32>>, D::Error> {
    Option::<u32>::deserialize(d).map(Some)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OrchFieldsDocument {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    state: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    receiving: Option<OrchReceivingDocument>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    sending: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    routine: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "talk_limit"
    )]
    talk_limit: Option<Option<u32>>,
}
// Tagged unit variants discard extra content in serde; empty struct variants
// keep field admission strict for all fieldless wire shapes in this module.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
enum OrchReceivingDocument {
    Direct {},
    Auto {},
    Urgent {},
    Hold {},
    Batch {
        #[serde(rename = "intervalMs")]
        interval_ms: u64,
    },
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum OrchTargetDocument {
    All {},
    Identity { id: String },
    Pattern { pattern: String },
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum OrchScheduleDocument {
    Always {},
    Once {
        #[serde(rename = "startMs")]
        start_ms: u64,
        #[serde(rename = "endMs")]
        end_ms: u64,
    },
    Daily {
        start: String,
        end: String,
        #[serde(default, rename = "allDay")]
        all_day: bool,
    },
    Weekly {
        #[serde(rename = "anchorDate")]
        anchor_date: String,
        #[serde(rename = "intervalWeeks")]
        interval_weeks: u8,
        weekdays: Vec<u8>,
        start: String,
        end: String,
        #[serde(default, rename = "allDay")]
        all_day: bool,
    },
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OrchMemberDocument {
    pub identity_id: String,
    pub fields: OrchFieldsDocument,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchPlanDocument {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) revision: u64,
    pub(crate) priority: i32,
    pub(crate) enabled: bool,
    pub(crate) targets: Vec<OrchTargetDocument>,
    pub(crate) schedule: OrchScheduleDocument,
    pub(crate) fields: OrchFieldsDocument,
    #[serde(default)]
    pub(crate) members: Vec<OrchMemberDocument>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchSubjectDocument {
    pub kind: String,
    pub id: String,
}
impl OrchSubjectDocument {
    pub fn valid(&self) -> bool {
        match self.kind.as_str() {
            "request" => self.id.strip_prefix("req_").is_some_and(canonical_id),
            "identity" => canonical_id(&self.id),
            "job" => {
                !self.id.is_empty()
                    && self.id.len() <= 128
                    && !self.id.chars().any(char::is_control)
            }
            _ => false,
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "view", rename_all = "camelCase", deny_unknown_fields)]
pub enum OrchReadQuery {
    Overview {
        #[serde(rename = "rangeDays")]
        range_days: Option<u8>,
        #[serde(rename = "atMs")]
        at_ms: Option<u64>,
        plan: Option<String>,
        #[serde(rename = "memberQuery")]
        member_query: Option<String>,
    },
    Plans {
        plan: Option<String>,
    },
    Preview {
        candidate: Box<OrchPlanDocument>,
        #[serde(rename = "rangeDays")]
        range_days: Option<u8>,
        #[serde(rename = "atMs")]
        at_ms: Option<u64>,
        plan: Option<String>,
        #[serde(rename = "memberQuery")]
        member_query: Option<String>,
    },
    Why {
        subject: OrchSubjectDocument,
        before: Option<u64>,
        limit: Option<u16>,
    },
}
pub fn decode_orch_query(bytes: &[u8]) -> Result<OrchReadQuery, OrchError> {
    if bytes.len() > ORCH_DOCUMENT_BYTES + 4096 {
        return Err(OrchError::Limit);
    }
    let query: OrchReadQuery = serde_json::from_slice(bytes).map_err(|_| OrchError::Invalid)?;
    validate_query(&query)?;
    Ok(query)
}
pub(crate) fn validate_query(query: &OrchReadQuery) -> Result<(), OrchError> {
    match &query {
        OrchReadQuery::Overview {
            range_days,
            at_ms,
            plan,
            member_query,
        }
        | OrchReadQuery::Preview {
            range_days,
            at_ms,
            plan,
            member_query,
            ..
        } => {
            if range_days.is_some_and(|d| !matches!(d, 1 | 7 | 30))
                || at_ms.is_some_and(|v| !valid_ms(v))
                || plan.as_ref().is_some_and(|s| !label(s, 128))
                || member_query.as_ref().is_some_and(|s| !label(s, 80))
            {
                return Err(OrchError::Invalid);
            }
        }
        OrchReadQuery::Plans { plan } => {
            if plan.as_ref().is_some_and(|s| !label(s, 128)) {
                return Err(OrchError::Invalid);
            }
        }
        OrchReadQuery::Why {
            subject,
            before,
            limit,
        } => {
            if !subject.valid()
                || before.is_some_and(|v| !valid_ms(v))
                || limit.is_some_and(|v| !(1..=100).contains(&v))
            {
                return Err(OrchError::Invalid);
            }
        }
    }
    if let OrchReadQuery::Preview { candidate, .. } = &query {
        candidate.to_core()?;
        if serde_json::to_vec(candidate)
            .map_err(|_| OrchError::Invalid)?
            .len()
            > ORCH_DOCUMENT_BYTES
        {
            return Err(OrchError::Limit);
        }
    }
    Ok(())
}
pub fn orch_candidate_file(path: &std::path::Path) -> Result<OrchPlanDocument, OrchError> {
    let bytes =
        crate::bounded_file::read_no_follow(path, ORCH_DOCUMENT_BYTES).map_err(|e| match e {
            crate::bounded_file::FileReadError::TooLarge => OrchError::Limit,
            _ => OrchError::Invalid,
        })?;
    let plan: OrchPlanDocument = serde_json::from_slice(&bytes).map_err(|_| OrchError::Invalid)?;
    plan.to_core()?;
    Ok(plan)
}
pub(crate) fn valid_ms(v: u64) -> bool {
    v > 0 && v <= MAX_JS_SAFE_INTEGER
}
fn label(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn minute(s: &str) -> Result<u16, OrchError> {
    let b = s.as_bytes();
    if b.len() != 5
        || b[2] != b':'
        || !b
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
    {
        return Err(OrchError::Invalid);
    }
    let hour = u16::from(b[0] - b'0') * 10 + u16::from(b[1] - b'0');
    let min = u16::from(b[3] - b'0') * 10 + u16::from(b[4] - b'0');
    if hour > 23 || min > 59 {
        Err(OrchError::Invalid)
    } else {
        Ok(hour * 60 + min)
    }
}
impl OrchFieldsDocument {
    pub(crate) fn to_core(&self) -> Result<OrchFields, OrchError> {
        let state = self
            .state
            .as_deref()
            .map(|v| match v {
                "awake" => Ok(OrchState::Awake),
                "low" => Ok(OrchState::Low),
                "focus" => Ok(OrchState::Focus),
                "sleep" => Ok(OrchState::Sleep),
                _ => Err(OrchError::Invalid),
            })
            .transpose()?;
        let receiving = self
            .receiving
            .as_ref()
            .map(|v| match v {
                OrchReceivingDocument::Direct {} => Ok(OrchReceiving::Direct),
                OrchReceivingDocument::Auto {} => Ok(OrchReceiving::Auto),
                OrchReceivingDocument::Urgent {} => Ok(OrchReceiving::Urgent),
                OrchReceivingDocument::Hold {} => Ok(OrchReceiving::Hold),
                OrchReceivingDocument::Batch { interval_ms }
                    if *interval_ms > 0
                        && *interval_ms <= 86_400_000
                        && interval_ms % 60_000 == 0 =>
                {
                    Ok(OrchReceiving::Batch(*interval_ms))
                }
                _ => Err(OrchError::Invalid),
            })
            .transpose()?;
        let sending = self
            .sending
            .as_deref()
            .map(|v| match v {
                "anyone" => Ok(OrchSending::Anyone),
                "leads" => Ok(OrchSending::Leads),
                "paused" => Ok(OrchSending::Paused),
                _ => Err(OrchError::Invalid),
            })
            .transpose()?;
        let routine = self
            .routine
            .as_deref()
            .map(|v| match v {
                "run" => Ok(OrchRoutine::Run),
                "hold" => Ok(OrchRoutine::Hold),
                _ => Err(OrchError::Invalid),
            })
            .transpose()?;
        if self.talk_limit == Some(Some(0)) {
            return Err(OrchError::Invalid);
        }
        Ok(OrchFields {
            state,
            receiving,
            sending,
            routine,
            talk_limit: self.talk_limit,
        })
    }
}
impl OrchScheduleDocument {
    fn to_core(&self) -> Result<OrchSchedule, OrchError> {
        let (anchor_day, interval_weeks, weekdays, start, end, all_day) = match self {
            Self::Always {} => return Ok(OrchSchedule::Always),
            Self::Once { start_ms, end_ms } => {
                return if valid_ms(*start_ms) && valid_ms(*end_ms) && start_ms < end_ms {
                    Ok(OrchSchedule::Once {
                        start_ms: *start_ms,
                        end_ms: *end_ms,
                    })
                } else {
                    Err(OrchError::Invalid)
                };
            }
            Self::Daily {
                start,
                end,
                all_day,
            } => (
                2_440_585,
                1,
                vec![1, 2, 3, 4, 5, 6, 7],
                start,
                end,
                *all_day,
            ),
            Self::Weekly {
                anchor_date,
                interval_weeks,
                weekdays,
                start,
                end,
                all_day,
            } => {
                let day = parse_date(anchor_date)?;
                (
                    day - day.rem_euclid(7),
                    *interval_weeks,
                    weekdays.clone(),
                    start,
                    end,
                    *all_day,
                )
            }
        };
        let start_minute = minute(start)?;
        let end_minute = minute(end)?;
        if !matches!(interval_weeks, 1 | 2)
            || weekdays.is_empty()
            || weekdays.len() > 7
            || weekdays.iter().any(|d| !(1..=7).contains(d))
            || weekdays.iter().collect::<BTreeSet<_>>().len() != weekdays.len()
            || (!all_day && start_minute == end_minute)
        {
            return Err(OrchError::Invalid);
        }
        Ok(OrchSchedule::Weekly {
            anchor_day,
            interval_weeks,
            weekdays,
            start_minute,
            end_minute,
            all_day,
        })
    }
}
impl OrchPlanDocument {
    pub(crate) fn to_core(&self) -> Result<OrchPlan, OrchError> {
        if !canonical_id(&self.id)
            || !label(&self.name, 80)
            || !valid_ms(self.revision)
            || self.targets.is_empty()
            || self.targets.len() > ORCH_MAX_MEMBERS
            || self.members.len() > ORCH_MAX_MEMBERS
        {
            return Err(OrchError::Invalid);
        }
        let mut targets = Vec::new();
        for t in &self.targets {
            targets.push(match t {
                OrchTargetDocument::All {} => OrchTarget::All,
                OrchTargetDocument::Identity { id } if canonical_id(id) => {
                    OrchTarget::Identity(id.clone())
                }
                OrchTargetDocument::Pattern { pattern } if label(pattern, 80) => {
                    OrchTarget::Pattern(pattern.clone())
                }
                _ => return Err(OrchError::Invalid),
            });
        }
        let mut members = BTreeMap::new();
        for m in &self.members {
            if !canonical_id(&m.identity_id)
                || members
                    .insert(m.identity_id.clone(), m.fields.to_core()?)
                    .is_some()
            {
                return Err(OrchError::Invalid);
            }
        }
        Ok(OrchPlan {
            id: self.id.clone(),
            name: self.name.clone(),
            revision: self.revision,
            priority: self.priority,
            enabled: self.enabled,
            targets,
            schedule: self.schedule.to_core()?,
            fields: self.fields.to_core()?,
            members,
        })
    }
}
pub(crate) fn plan_value(p: &OrchPlan) -> Result<Value, OrchError> {
    let clock = |m: u16| format!("{:02}:{:02}", m / 60, m % 60);
    let schedule = match &p.schedule {
        OrchSchedule::Always => json!({"kind":"always"}),
        OrchSchedule::Once { start_ms, end_ms } => {
            json!({"kind":"once","startMs":start_ms,"endMs":end_ms})
        }
        OrchSchedule::Weekly {
            anchor_day,
            interval_weeks,
            weekdays,
            start_minute,
            end_minute,
            all_day,
        } => {
            json!({"kind":"weekly","anchorDate":date_text(*anchor_day)?,"intervalWeeks":interval_weeks,"weekdays":weekdays,"start":clock(*start_minute),"end":clock(*end_minute),"allDay":all_day})
        }
    };
    let targets: Vec<_> = p
        .targets
        .iter()
        .map(|t| match t {
            OrchTarget::All => json!({"kind":"all"}),
            OrchTarget::Identity(id) => json!({"kind":"identity","id":id}),
            OrchTarget::Pattern(pattern) => json!({"kind":"pattern","pattern":pattern}),
        })
        .collect();
    let members: Vec<_> = p
        .members
        .iter()
        .map(|(id, f)| json!({"identityId":id,"fields":fields_value(f)}))
        .collect();
    Ok(
        json!({"id":p.id,"name":p.name,"revision":p.revision,"priority":p.priority,"enabled":p.enabled,"targets":targets,"schedule":schedule,"fields":fields_value(&p.fields),"members":members}),
    )
}
fn state_name(v: OrchState) -> &'static str {
    match v {
        OrchState::Awake => "awake",
        OrchState::Low => "low",
        OrchState::Focus => "focus",
        OrchState::Sleep => "sleep",
    }
}
fn sending_name(v: OrchSending) -> &'static str {
    match v {
        OrchSending::Anyone => "anyone",
        OrchSending::Leads => "leads",
        OrchSending::Paused => "paused",
    }
}
fn routine_name(v: OrchRoutine) -> &'static str {
    match v {
        OrchRoutine::Run => "run",
        OrchRoutine::Hold => "hold",
    }
}
fn receiving_value(v: &OrchReceiving) -> Value {
    match v {
        OrchReceiving::Direct => json!({"mode":"direct"}),
        OrchReceiving::Auto => json!({"mode":"auto"}),
        OrchReceiving::Urgent => json!({"mode":"urgent"}),
        OrchReceiving::Hold => json!({"mode":"hold"}),
        OrchReceiving::Batch(ms) => json!({"mode":"batch","intervalMs":ms}),
    }
}
fn fields_value(f: &OrchFields) -> Value {
    let mut v = json!({});
    if let Some(s) = f.state {
        v["state"] = json!(state_name(s));
    }
    if let Some(r) = &f.receiving {
        v["receiving"] = receiving_value(r);
    }
    if let Some(s) = f.sending {
        v["sending"] = json!(sending_name(s));
    }
    if let Some(r) = f.routine {
        v["routine"] = json!(routine_name(r));
    }
    if let Some(l) = f.talk_limit {
        v["talkLimit"] = json!(l);
    }
    v
}
pub(crate) fn source_value(s: &OrchSource) -> Value {
    json!({"kind":match s.kind {OrchSourceKind::Base=>"base",OrchSourceKind::Plan=>"plan",OrchSourceKind::Member=>"member",OrchSourceKind::Focus=>"focus"},"planId":s.plan_id,"identityId":s.identity_id,"revision":s.revision,"slot":s.slot.map(|s|json!({"startMs":s.start_ms,"endMs":s.end_ms})),"defaulted":s.defaulted})
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OrchSourceDocument {
    kind: String,
    plan_id: Option<String>,
    identity_id: Option<String>,
    revision: u64,
    slot: Option<OrchSlotDocument>,
    defaulted: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrchDecisionSourceDocument {
    field: String,
    source: OrchSourceDocument,
}
pub(crate) fn decision_source_value(s: &OrchDecisionSource) -> Value {
    let field = match s.field {
        OrchPolicyField::State => "state",
        OrchPolicyField::Receiving => "receiving",
        OrchPolicyField::Sending => "sending",
        OrchPolicyField::Routine => "routine",
        OrchPolicyField::TalkLimit => "talkLimit",
    };
    json!({"field":field,"source":source_value(&s.source)})
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OrchSlotDocument {
    start_ms: u64,
    end_ms: u64,
}
pub(crate) fn source_valid(s: &OrchSource) -> bool {
    s.revision <= MAX_JS_SAFE_INTEGER
        && s.plan_id.as_deref().is_none_or(canonical_id)
        && s.identity_id.as_deref().is_none_or(canonical_id)
        && s.slot
            .is_none_or(|v| valid_ms(v.start_ms) && valid_ms(v.end_ms) && v.start_ms < v.end_ms)
        && match s.kind {
            OrchSourceKind::Base => {
                s.plan_id.is_none() && s.identity_id.is_none() && s.revision == 0
            }
            OrchSourceKind::Plan => {
                s.plan_id.is_some() && s.identity_id.is_none() && s.revision > 0
            }
            OrchSourceKind::Member => {
                s.plan_id.is_some() && s.identity_id.is_some() && s.revision > 0
            }
            OrchSourceKind::Focus => {
                s.plan_id.is_none() && s.identity_id.is_some() && s.revision > 0
            }
        }
}
pub(crate) fn decode_sources(text: &str) -> Result<Vec<Value>, OrchError> {
    if text.len() > 8192 {
        return Err(OrchError::Limit);
    }
    let docs: Vec<OrchDecisionSourceDocument> =
        serde_json::from_str(text).map_err(|_| OrchError::Invalid)?;
    if docs.len() > 5 {
        return Err(OrchError::Limit);
    }
    let mut fields = BTreeSet::new();
    docs.into_iter()
        .map(|entry| {
            let field = match entry.field.as_str() {
                "state" => OrchPolicyField::State,
                "receiving" => OrchPolicyField::Receiving,
                "sending" => OrchPolicyField::Sending,
                "routine" => OrchPolicyField::Routine,
                "talkLimit" => OrchPolicyField::TalkLimit,
                _ => return Err(OrchError::Invalid),
            };
            if !fields.insert(field) {
                return Err(OrchError::Invalid);
            }
            let s = entry.source;
            let kind = match s.kind.as_str() {
                "base" => OrchSourceKind::Base,
                "plan" => OrchSourceKind::Plan,
                "member" => OrchSourceKind::Member,
                "focus" => OrchSourceKind::Focus,
                _ => return Err(OrchError::Invalid),
            };
            let source = OrchSource {
                kind,
                plan_id: s.plan_id,
                identity_id: s.identity_id,
                revision: s.revision,
                slot: s.slot.map(|v| OrchSlot {
                    start_ms: v.start_ms,
                    end_ms: v.end_ms,
                }),
                defaulted: s.defaulted,
            };
            if !source_valid(&source) {
                return Err(OrchError::Invalid);
            }
            Ok(decision_source_value(&OrchDecisionSource { field, source }))
        })
        .collect()
}
pub(crate) fn effective_value(p: &OrchEffective) -> Value {
    json!({"state":{"value":state_name(p.state.value),"source":source_value(&p.state.source)},"receiving":{"value":receiving_value(&p.receiving.value),"source":source_value(&p.receiving.source)},"sending":{"value":sending_name(p.sending.value),"source":source_value(&p.sending.source)},"routine":{"value":routine_name(p.routine.value),"source":source_value(&p.routine.source)},"talkLimit":{"value":p.talk_limit.value,"source":source_value(&p.talk_limit.source)}})
}

#[cfg(test)]
mod tests;
