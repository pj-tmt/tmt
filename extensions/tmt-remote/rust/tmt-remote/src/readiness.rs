//! Layered Firestore readiness: which Firebase item is not enabled for each layer, and why
//! (contract: Backends and deploy, `status --layers --json`). Pure: no I/O, clock or
//! network. The projection is derived from recorded evidence behind an injected source and
//! never asks Google; before a Firestore deployment exists there is no evidence, so the list
//! is empty. One table owns every item, reason, sentence and next command; the validator
//! accepts only what the table derives, and no provider text, secret or path can appear.
use crate::deploy_run::{DeployRecord, RunState, StepState};
use serde_json::{Value, json};

/// A prerequisite of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirestoreItem {
    Project,
    SignIn,
    Rules,
    PlanTier,
    Quota,
    /// Whether this release implements the layer at all.
    Support,
}
impl FirestoreItem {
    fn name(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::SignIn => "sign-in",
            Self::Rules => "rules",
            Self::PlanTier => "plan-tier",
            Self::Quota => "quota",
            Self::Support => "support",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirestoreReason {
    NotConfigured,
    AccessLost,
    ProviderDisabled,
    PermissionMissing,
    NotDeployed,
    OutOfDate,
    Partial,
    PaidPlanRequired,
    HeadroomLow,
    Exhausted,
    NotImplemented,
    /// No evidence: the only reason of an `unknown` prerequisite.
    NotChecked,
}
/// The planned command a prerequisite can point to; registration and the real provider
/// remain separate from the library deployment owner and its recorded evidence.
const DEPLOY: &str = "tmt remote deploy firestore";

struct Row {
    item: FirestoreItem,
    reason: FirestoreReason,
    code: &'static str,
    sentence: &'static str,
    next: Option<&'static str>,
}
use FirestoreItem as I;
use FirestoreReason as R;
const fn row(
    item: FirestoreItem,
    reason: FirestoreReason,
    code: &'static str,
    sentence: &'static str,
    next: Option<&'static str>,
) -> Row {
    Row {
        item,
        reason,
        code,
        sentence,
        next,
    }
}
/// Every reason an item can report, with its fixed sentence and next command.
const ROWS: [Row; 16] = [
    row(
        I::Project,
        R::NotConfigured,
        "not-configured",
        "No Firebase project is set up.",
        Some(DEPLOY),
    ),
    row(
        I::Project,
        R::AccessLost,
        "access-lost",
        "tmt can no longer reach the Firebase project.",
        Some(DEPLOY),
    ),
    row(
        I::Project,
        R::NotChecked,
        "not-checked",
        "The Firebase project has not been checked.",
        None,
    ),
    row(
        I::SignIn,
        R::ProviderDisabled,
        "provider-disabled",
        "Sign-in is not enabled for the project.",
        Some(DEPLOY),
    ),
    row(
        I::SignIn,
        R::PermissionMissing,
        "permission-missing",
        "This account isn't allowed to turn on sign-in for the project.",
        None,
    ),
    row(
        I::SignIn,
        R::NotChecked,
        "not-checked",
        "Sign-in has not been checked.",
        None,
    ),
    row(
        I::Rules,
        R::NotDeployed,
        "not-deployed",
        "Firestore rules are not deployed.",
        Some(DEPLOY),
    ),
    row(
        I::Rules,
        R::OutOfDate,
        "out-of-date",
        "Firestore rules are out of date.",
        Some(DEPLOY),
    ),
    row(
        I::Rules,
        R::Partial,
        "partial",
        "Firestore rules are only partly deployed.",
        Some(DEPLOY),
    ),
    row(
        I::Rules,
        R::NotChecked,
        "not-checked",
        "Firestore rules have not been checked.",
        None,
    ),
    row(
        I::PlanTier,
        R::PaidPlanRequired,
        "paid-plan-required",
        "This needs a paid Firebase plan.",
        None,
    ),
    row(
        I::PlanTier,
        R::NotChecked,
        "not-checked",
        "The Firebase plan has not been checked.",
        None,
    ),
    row(
        I::Quota,
        R::HeadroomLow,
        "headroom-low",
        "Today's free Firebase quota is almost used up.",
        None,
    ),
    row(
        I::Quota,
        R::Exhausted,
        "exhausted",
        "Today's free Firebase quota is used up.",
        None,
    ),
    row(
        I::Quota,
        R::NotChecked,
        "not-checked",
        "The free Firebase quota has not been checked.",
        None,
    ),
    row(
        I::Support,
        R::NotImplemented,
        "not-implemented",
        "This isn't available in this release yet.",
        None,
    ),
];
fn lookup(item: FirestoreItem, reason: FirestoreReason) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.item == item && r.reason == reason)
}
fn lookup_code(item: &str, code: &str) -> Option<&'static Row> {
    ROWS.iter()
        .find(|r| r.item.name() == item && r.code == code)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirestoreLayer {
    Sharing,
    Operations,
    Attachments,
}
impl FirestoreLayer {
    fn name(self) -> &'static str {
        match self {
            Self::Sharing => "sharing",
            Self::Operations => "operations",
            Self::Attachments => "attachments",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Sharing => "Page sharing",
            Self::Operations => "Operation sync",
            Self::Attachments => "Attachment storage",
        }
    }
}
/// A layer and its own prerequisites, in the order they are reported.
pub struct LayerSpec {
    pub layer: FirestoreLayer,
    /// The free Firebase plan cannot run this layer. Owner direction (Ben, 2026-10-09):
    /// device operations are not offered on the free plan, so this is one line to change.
    pub requires_paid_plan: bool,
    pub items: &'static [FirestoreItem],
}
pub const LAYERS: [LayerSpec; 3] = [
    LayerSpec {
        layer: FirestoreLayer::Sharing,
        requires_paid_plan: false,
        items: &[I::Project, I::SignIn, I::Rules, I::PlanTier, I::Quota],
    },
    LayerSpec {
        layer: FirestoreLayer::Operations,
        requires_paid_plan: true,
        items: &[I::PlanTier, I::Support],
    },
    LayerSpec {
        layer: FirestoreLayer::Attachments,
        requires_paid_plan: false,
        items: &[I::Support, I::Project, I::Rules, I::Quota],
    },
];

/// What one prerequisite was last recorded as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observed {
    Enabled,
    Off(FirestoreReason),
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirestoreTier {
    Free,
    Paid,
    Unknown,
}
/// Recorded evidence about the owner's Firebase project. Quota exhaustion is learned only
/// from a refusal, so `Exhausted` appears here only if one was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirestoreEvidence {
    pub project: Observed,
    pub sign_in: Observed,
    pub rules: Observed,
    pub quota: Observed,
    pub tier: FirestoreTier,
}
impl FirestoreEvidence {
    /// A damaged or unreadable record is not evidence of an unconfigured deployment.
    pub fn unknown() -> Self {
        Self {
            project: Observed::Unknown,
            sign_in: Observed::Unknown,
            rules: Observed::Unknown,
            quota: Observed::Unknown,
            tier: FirestoreTier::Unknown,
        }
    }
}

/// Recorded outcomes only, never a live provider check. A draft has no evidence. Tier and
/// quota are absent from the record: browser-to-Firestore traffic cannot be observed here.
pub fn from_record(record: &DeployRecord) -> Option<FirestoreEvidence> {
    let run = record.run.as_ref()?;
    let finished = |state: &StepState| matches!(state, StepState::Done | StepState::Adopted);
    let mut evidence = FirestoreEvidence::unknown();
    let sign_in: Vec<_> = run
        .steps
        .iter()
        .filter(|s| s.id.starts_with("sign-in:"))
        .collect();
    // The private reader validates the record schema; prerequisites still need their
    // recognized identities. Missing/unknown rows must not turn a binding into an allow.
    if ["database", "rules", "verify"]
        .iter()
        .any(|id| run.steps.iter().filter(|s| s.id == *id).count() != 1)
        || sign_in.is_empty()
        || sign_in
            .iter()
            .any(|s| !matches!(s.id.as_str(), "sign-in:anonymous" | "sign-in:google.com"))
    {
        return Some(evidence);
    }
    if record.usable_binding().is_some()
        || run
            .steps
            .iter()
            .any(|s| s.id == "database" && finished(&s.state))
    {
        evidence.project = Observed::Enabled;
    }
    if sign_in.iter().all(|s| finished(&s.state)) {
        evidence.sign_in = Observed::Enabled;
    }
    evidence.rules = if run.rules_attempted && run.state != RunState::Complete {
        Observed::Off(FirestoreReason::Partial)
    } else if record.usable_binding().is_some() {
        Observed::Enabled
    } else {
        Observed::Unknown
    };
    Some(evidence)
}

/// Where evidence comes from. `None` means no Firestore deployment exists.
pub trait FirestoreEvidenceSource: Send + Sync {
    fn evidence(&self) -> Option<FirestoreEvidence>;
}
/// An empty source for a door with no deployment evidence.
pub struct NotConfigured;
impl FirestoreEvidenceSource for NotConfigured {
    fn evidence(&self) -> Option<FirestoreEvidence> {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Enabled,
    Off(FirestoreReason),
    Unknown,
}
fn verdict(item: FirestoreItem, spec: &LayerSpec, evidence: &FirestoreEvidence) -> Verdict {
    let observed = |o: Observed| match o {
        Observed::Enabled => Verdict::Enabled,
        // A reason the item cannot report is no evidence at all, never a guess.
        Observed::Off(reason) if lookup(item, reason).is_some() => Verdict::Off(reason),
        Observed::Off(_) | Observed::Unknown => Verdict::Unknown,
    };
    match item {
        I::Project => observed(evidence.project),
        I::SignIn => observed(evidence.sign_in),
        I::Rules => observed(evidence.rules),
        I::Quota => observed(evidence.quota),
        I::PlanTier => match evidence.tier {
            FirestoreTier::Unknown => Verdict::Unknown,
            FirestoreTier::Free if spec.requires_paid_plan => Verdict::Off(R::PaidPlanRequired),
            FirestoreTier::Free | FirestoreTier::Paid => Verdict::Enabled,
        },
        I::Support => Verdict::Off(R::NotImplemented),
    }
}
fn state_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Enabled => "enabled",
        Verdict::Off(_) => "not-enabled",
        Verdict::Unknown => "unknown",
    }
}
/// Any prerequisite off makes the layer off; otherwise any unknown makes it unknown.
fn layer_state<'a>(states: impl Iterator<Item = &'a str> + Clone) -> &'static str {
    if states.clone().any(|s| s == "not-enabled") {
        "not-enabled"
    } else if states.clone().any(|s| s == "unknown") {
        "unknown"
    } else {
        "enabled"
    }
}

/// The `firestoreLayers` value: empty without evidence, else one entry per layer.
pub fn project(evidence: Option<&FirestoreEvidence>, layers: &[LayerSpec]) -> Value {
    let Some(evidence) = evidence else {
        return json!([]);
    };
    Value::Array(
        layers
            .iter()
            .map(|spec| {
                let items: Vec<(FirestoreItem, Verdict)> = spec
                    .items
                    .iter()
                    .map(|item| (*item, verdict(*item, spec, evidence)))
                    .collect();
                let state = layer_state(items.iter().map(|(_, v)| state_name(*v)));
                let prerequisites: Vec<Value> = items
                    .iter()
                    .map(|(item, verdict)| {
                        let mut value = json!({"item": item.name(), "state": state_name(*verdict)});
                        let reason = match verdict {
                            Verdict::Enabled => return value,
                            Verdict::Off(reason) => *reason,
                            Verdict::Unknown => R::NotChecked,
                        };
                        let row = lookup(*item, reason).expect("a verdict names a table row");
                        value["reason"] = json!(row.code);
                        if let Some(next) = row.next {
                            value["next"] = json!(next);
                        }
                        value
                    })
                    .collect();
                json!({"layer": spec.layer.name(), "state": state, "prerequisites": prerequisites})
            })
            .collect(),
    )
}

/// Accept only what `project` can produce for `layers`: the empty list, or every layer in
/// table order with its own prerequisites in order, each state, reason and next from the table.
pub fn validate(value: &Value, layers: &[LayerSpec]) -> bool {
    let Some(entries) = value.as_array() else {
        return false;
    };
    if entries.is_empty() {
        return true;
    }
    entries.len() == layers.len()
        && entries
            .iter()
            .zip(layers)
            .all(|(entry, spec)| valid_layer(entry, spec))
}
fn valid_layer(entry: &Value, spec: &LayerSpec) -> bool {
    let Some(fields) = entry.as_object().filter(|f| f.len() == 3) else {
        return false;
    };
    let Some(prerequisites) = fields.get("prerequisites").and_then(Value::as_array) else {
        return false;
    };
    if fields.get("layer").and_then(Value::as_str) != Some(spec.layer.name())
        || prerequisites.len() != spec.items.len()
    {
        return false;
    }
    let mut states = Vec::new();
    for (prerequisite, item) in prerequisites.iter().zip(spec.items) {
        let Some(fields) = prerequisite.as_object() else {
            return false;
        };
        if fields.get("item").and_then(Value::as_str) != Some(item.name()) {
            return false;
        }
        let state = match fields.get("state").and_then(Value::as_str) {
            Some(state @ ("enabled" | "not-enabled" | "unknown")) => state,
            _ => return false,
        };
        states.push(state);
        if state == "enabled" {
            if fields.len() != 2 {
                return false;
            }
            continue;
        }
        let Some(row) = fields
            .get("reason")
            .and_then(Value::as_str)
            .and_then(|code| lookup_code(item.name(), code))
        else {
            return false;
        };
        // Only a layer the free plan cannot run can report that it needs a paid plan.
        if row.reason == R::PaidPlanRequired && !spec.requires_paid_plan {
            return false;
        }
        // `unknown` is only ever "not checked"; `not-enabled` is never that.
        if (state == "unknown") != (row.reason == R::NotChecked) {
            return false;
        }
        let expected = 3 + usize::from(row.next.is_some());
        if fields.len() != expected || fields.get("next").and_then(Value::as_str) != row.next {
            return false;
        }
    }
    fields.get("state").and_then(Value::as_str) == Some(layer_state(states.into_iter()))
}

/// One line of the human rendering. `hint` is the next command, when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanLine {
    pub enabled: bool,
    pub what: String,
    pub hint: Option<String>,
}
/// The human rendering of a validated projection: one line per layer, naming the first
/// prerequisite whose state is the layer's state.
pub fn human_lines(value: &Value, layers: &[LayerSpec]) -> Vec<HumanLine> {
    let Some(entries) = value.as_array().filter(|e| !e.is_empty()) else {
        return vec![HumanLine {
            enabled: false,
            what: "Firestore is not configured.".into(),
            hint: None,
        }];
    };
    entries
        .iter()
        .zip(layers)
        .map(|(entry, spec)| {
            let title = spec.layer.title();
            if entry["state"] == "enabled" {
                return HumanLine {
                    enabled: true,
                    what: format!("{title} is enabled."),
                    hint: None,
                };
            }
            // The layer's own state names the culprit: a definite `not-enabled` outranks an
            // `unknown` that happens to come first.
            let first = entry["prerequisites"]
                .as_array()
                .and_then(|p| p.iter().find(|p| p["state"] == entry["state"]))
                .expect("a layer's state comes from one of its prerequisites");
            let row = lookup_code(
                first["item"].as_str().unwrap_or_default(),
                first["reason"].as_str().unwrap_or_default(),
            )
            .expect("a validated prerequisite names a table row");
            let state = if entry["state"] == "unknown" {
                "not checked"
            } else {
                "not enabled"
            };
            HumanLine {
                enabled: false,
                what: format!("{title} is {state}. {}", row.sentence),
                hint: row.next.map(str::to_owned),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_one_row_per_item_and_reason() {
        let mut seen = std::collections::BTreeSet::new();
        for row in &ROWS {
            assert!(seen.insert((row.item.name(), row.code)), "{}", row.code);
            assert!(!row.sentence.is_empty() && row.sentence.ends_with('.'));
        }
    }
}
