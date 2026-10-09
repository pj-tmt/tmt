//! The free-plan (Spark) Firestore limits table behind `tmt remote status --budget` (#2180).
//! Pure: no I/O, clock or network. It is a dated snapshot of the official limits that layer 1
//! depends on, never the project's real usage: quotas are project-wide and only the Firebase
//! console shows what is used. The numbers were read from the official quota pages on
//! `READ_ON`; two of them drive the arithmetic in `firestore_budget`, the single-field index
//! limit is enforced by `rules`, and the Remote reference guide's dated table owns them all.
//! Limits change: recheck before relying on them. The validator accepts only what `member`
//! produces for this table's keys, so no provider text, secret or path can appear.
use crate::firestore_budget::{READS_PER_DAY, REFUSE_PERCENT, WARN_PERCENT, WRITES_PER_DAY};
use crate::rules::MAX_INDEX_CONFIGS;
use FirestoreLimitUnit::{Bytes, Count};
use serde_json::{Map, Value, json};

/// The day the official pages were read (pages last updated 2026-10-07 UTC).
pub const READ_ON: &str = "2026-10-09";
/// The plan the limits belong to: Firebase's no-cost plan, no billing.
pub const PLAN: &str = "no-cost";
/// When the daily quotas reset, as the official page words it.
pub const RESETS_AT: &str = "around midnight Pacific";

const KIB: u64 = 1024;
const MIB: u64 = KIB * KIB;
const GIB: u64 = MIB * KIB;

/// How a limit's value is written for people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirestoreLimitUnit {
    Count,
    Bytes,
}
/// One row: the fixed JSON key, the sentence label, the official value and its period.
#[derive(Clone, Copy, Debug)]
pub struct FirestoreLimit {
    pub key: &'static str,
    pub label: &'static str,
    pub value: u64,
    pub unit: FirestoreLimitUnit,
    pub per: Option<&'static str>,
}
const fn row(
    key: &'static str,
    label: &'static str,
    value: u64,
    unit: FirestoreLimitUnit,
    per: Option<&'static str>,
) -> FirestoreLimit {
    FirestoreLimit {
        key,
        label,
        value,
        unit,
        per,
    }
}
/// Every Firestore limit layer 1 depends on, in display order.
pub const LIMITS: [FirestoreLimit; 10] = [
    row(
        "readsPerDay",
        "Document reads",
        READS_PER_DAY,
        Count,
        Some("day"),
    ),
    row(
        "writesPerDay",
        "Document writes",
        WRITES_PER_DAY,
        Count,
        Some("day"),
    ),
    row(
        "deletesPerDay",
        "Document deletes",
        20_000,
        Count,
        Some("day"),
    ),
    row("storedBytes", "Stored data", GIB, Bytes, None),
    row(
        "egressBytesPerMonth",
        "Data sent out",
        10 * GIB,
        Bytes,
        Some("month"),
    ),
    row("databases", "Free databases per project", 1, Count, None),
    row("compositeIndexes", "Composite indexes", 200, Count, None),
    row(
        "singleFieldIndexConfigs",
        "Single-field index configs",
        MAX_INDEX_CONFIGS as u64,
        Count,
        None,
    ),
    row("documentBytes", "Document size", MIB, Bytes, None),
    row(
        "rulesLookupsPerRequest",
        "Rules document lookups per request",
        10,
        Count,
        None,
    ),
];

/// The `firestoreBudget` member: fixed keys, integers and fixed words only.
pub fn member() -> Value {
    let limits: Map<String, Value> = LIMITS
        .iter()
        .map(|limit| (limit.key.to_owned(), json!(limit.value)))
        .collect();
    json!({
        "plan": PLAN,
        "readOn": READ_ON,
        "resetsAt": RESETS_AT,
        "limits": limits,
        "guard": {"warnPercent": WARN_PERCENT, "refusePercent": REFUSE_PERCENT},
    })
}

fn keys_are(fields: &Map<String, Value>, expected: &[&str]) -> bool {
    fields.len() == expected.len() && expected.iter().all(|key| fields.contains_key(*key))
}
fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}
/// Accept only the shape `member` produces: this table's keys with positive integers, a date,
/// the fixed plan and reset words, and a guard with a warning threshold below the refusal.
/// The values themselves may differ from this build's, so a door of another build still reads.
pub fn validate(value: &Value) -> bool {
    let Some(fields) = value.as_object() else {
        return false;
    };
    let (Some(limits), Some(guard)) = (value["limits"].as_object(), value["guard"].as_object())
    else {
        return false;
    };
    let table: Vec<&str> = LIMITS.iter().map(|limit| limit.key).collect();
    keys_are(fields, &["plan", "readOn", "resetsAt", "limits", "guard"])
        && value["plan"] == PLAN
        && value["resetsAt"] == RESETS_AT
        && value["readOn"].as_str().is_some_and(is_date)
        && keys_are(limits, &table)
        && limits
            .values()
            .all(|number| number.as_u64().is_some_and(|n| n > 0))
        && keys_are(guard, &["warnPercent", "refusePercent"])
        && matches!(
            (value["guard"]["warnPercent"].as_u64(), value["guard"]["refusePercent"].as_u64()),
            (Some(warn), Some(refuse)) if 0 < warn && warn < refuse && refuse <= 100
        )
}

fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
fn written(limit: &FirestoreLimit, value: u64) -> String {
    let amount = match limit.unit {
        Count => grouped(value),
        Bytes => [(GIB, "GiB"), (MIB, "MiB"), (KIB, "KiB")]
            .iter()
            .find(|(size, _)| value.is_multiple_of(*size))
            .map(|(size, name)| format!("{} {name}", grouped(value / size)))
            .unwrap_or_else(|| format!("{} bytes", grouped(value))),
    };
    match limit.per {
        Some(per) => format!("{amount} per {per}"),
        None => amount,
    }
}
/// The human rendering of a validated member: a heading, one aligned line per limit, and what
/// the numbers are and are not.
pub fn human_lines(value: &Value) -> Vec<String> {
    let width = LIMITS
        .iter()
        .map(|limit| limit.label.len())
        .max()
        .unwrap_or(0);
    let mut lines = vec![format!(
        "Firestore free-plan limits (read {}; Google changes them, so recheck before relying on them):",
        value["readOn"].as_str().unwrap_or_default()
    )];
    for limit in &LIMITS {
        let shown = value["limits"][limit.key].as_u64().unwrap_or_default();
        lines.push(format!(
            "  {:<width$}  {}",
            limit.label,
            written(limit, shown)
        ));
    }
    lines.push(format!(
        "The whole Firebase project shares these limits and the daily ones reset {}. Remote cannot see the project's real usage; only the Firebase console shows it.",
        value["resetsAt"].as_str().unwrap_or_default()
    ));
    lines.push(format!(
        "A client warns at {}% and refuses at {}% of its own share before it reaches a limit.",
        value["guard"]["warnPercent"], value["guard"]["refusePercent"]
    ));
    lines
}
