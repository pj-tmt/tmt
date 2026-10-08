//! Core-owned focus policy: one bounded read, then snapshot-only projections.
use crate::core::{Core, SquadError};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const LIMIT: usize = 256;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub identity_id: String,
    pub revision: u64,
    pub active: bool,
    pub focus_until_ms: u64,
    pub remaining_ms: u64,
    pub held_count: u64,
}
impl Policy {
    pub fn row(&self) -> Value {
        json!({"active": self.active, "focusUntilMs": self.focus_until_ms,
            "remainingMs": self.remaining_ms, "heldCount": self.held_count})
    }
}
pub fn read_policies(core: &Core, ids: &[String]) -> Result<BTreeMap<String, Policy>, SquadError> {
    let document = core.api("focus.policy.show", json!({"identities": ids}))?;
    let invalid = || {
        SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "Core returned an incomplete focus policy.",
        )
    };
    let policies = document["policies"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|value| {
            let number = |key: &str| {
                value[key]
                    .as_u64()
                    .filter(|n| *n <= 9_007_199_254_740_991)
                    .ok_or_else(invalid)
            };
            Ok(Policy {
                identity_id: value["identityId"].as_str().ok_or_else(invalid)?.into(),
                revision: number("revision")?,
                active: value["active"].as_bool().ok_or_else(invalid)?,
                focus_until_ms: number("focusUntilMs")?,
                remaining_ms: number("remainingMs")?,
                held_count: number("heldCount")?,
            })
        })
        .collect::<Result<Vec<_>, SquadError>>()?;
    if policies.len() != ids.len() || policies.iter().zip(ids).any(|(p, id)| &p.identity_id != id) {
        return Err(SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "Core returned a mismatched focus policy.",
        ));
    }
    Ok(policies
        .into_iter()
        .map(|p| (p.identity_id.clone(), p))
        .collect())
}
/// Collect every row occurrence before reading. Duplicate rows and squad membership
/// share one policy; only identities admitted by the acquired active room rosters
/// are eligible. Overflow and optional-Core failures omit focus without noise.
pub fn enrich(core: &Core, documents: &mut [&mut Value], active_ids: &BTreeSet<String>) {
    let mut ids = BTreeSet::new();
    for document in documents.iter() {
        collect(document, active_ids, &mut ids);
    }
    let ids: Vec<_> = ids.into_iter().take(LIMIT).collect();
    if ids.is_empty() {
        return;
    }
    if let Ok(policies) = read_policies(core, &ids) {
        for document in documents {
            apply(document, &policies);
        }
    }
}
fn collect(document: &Value, active_ids: &BTreeSet<String>, ids: &mut BTreeSet<String>) {
    match document {
        Value::Object(object) => {
            if object.get("name").is_some_and(Value::is_string)
                && object.contains_key("fields")
                && let Some(id) = object.get("id").and_then(Value::as_str)
                && active_ids.contains(id)
            {
                ids.insert(id.to_owned());
            }
            for value in object.values() {
                collect(value, active_ids, ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect(value, active_ids, ids);
            }
        }
        _ => {}
    }
}
fn apply(document: &mut Value, policies: &BTreeMap<String, Policy>) {
    match document {
        Value::Object(object) => {
            if object.contains_key("fields")
                && let Some(policy) = object
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| policies.get(id))
            {
                object.insert("focus".into(), policy.row());
            }
            for value in object.values_mut() {
                apply(value, policies);
            }
        }
        Value::Array(values) => {
            for value in values {
                apply(value, policies);
            }
        }
        _ => {}
    }
}
/// Board time advances independently of snapshot acquisition. Expiry hides the
/// label even while a slow/failed refresh retains the previous snapshot.
pub fn remaining(row: &Value, now: u64) -> Option<(u64, u64)> {
    let focus = &row["focus"];
    let until = focus["focusUntilMs"].as_u64()?;
    (focus["active"] == true && until > now).then(|| {
        (
            until.saturating_sub(now),
            focus["heldCount"].as_u64().unwrap_or(0),
        )
    })
}
fn minutes(ms: u64) -> String {
    let minutes = ms.div_ceil(60_000);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h{m}m"),
    }
}
pub fn label(row: &Value, now: u64) -> Option<String> {
    remaining(row, now).map(|(ms, held)| {
        let suffix = if held == 0 {
            String::new()
        } else {
            format!(" · {held} held")
        };
        format!("focus {}{suffix}", minutes(ms))
    })
}
pub fn detail(row: &Value, now: u64) -> Option<String> {
    remaining(row, now).map(|(ms, held)| {
        let time = format!("{} left", minutes(ms));
        if held == 0 {
            time
        } else {
            format!("{time} · {held} held")
        }
    })
}

/// Fitting keeps the focus word whole; expansion carries omitted details.
pub fn fitted(row: &Value, now: u64, budget: usize) -> (String, String, bool) {
    let Some(label) = label(row, now) else {
        return (String::new(), String::new(), false);
    };
    if budget < 5 {
        return (String::new(), String::new(), false);
    }
    if unicode_width::UnicodeWidthStr::width(label.as_str()) <= budget {
        ("focus".into(), label[5..].into(), true)
    } else {
        ("focus".into(), String::new(), false)
    }
}

/// Shared heading data for boxed and HOME row templates.
pub fn pieces(row: &Value, now: u64, budget: usize) -> Value {
    let (word, suffix, _) = fitted(row, now, budget);
    if word.is_empty() {
        json!([])
    } else {
        json!([{"word":word,"suffix":suffix}])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_rounds_minutes_and_hides_expired_or_inactive_policies() {
        let mut row = json!({"focus":{"active":true,"focusUntilMs":4_801_001,"heldCount":2}});
        assert_eq!(label(&row, 1001).as_deref(), Some("focus 1h20m · 2 held"));
        assert_eq!(label(&row, 4_741_001).as_deref(), Some("focus 1m · 2 held"));
        assert_eq!(label(&row, 4_801_000).as_deref(), Some("focus 1m · 2 held"));
        assert_eq!(label(&row, 4_801_001), None);
        row["focus"]["heldCount"] = json!(0);
        assert_eq!(label(&row, 1000).as_deref(), Some("focus 1h21m"));
        assert_eq!(detail(&row, 1000).as_deref(), Some("1h21m left"));
        assert_eq!(label(&row, 1).as_deref(), Some("focus 1h21m"));
        assert_eq!(minutes(7_200_000), "2h");
        row["focus"]["active"] = json!(false);
        assert_eq!(label(&row, 0), None);
        assert_eq!(label(&json!({}), 0), None);
    }
    #[test]
    fn batching_deduplicates_rows_bounds_input_and_omits_unavailable_or_malformed_core() {
        use crate::cron_service::test_support::{Fixture, WORKER};
        let f = Fixture::new();
        let script = f.directory.join("focus-core");
        crate::test_support::write_ready_executable(
            &script,
            &format!(
                r#"#!/bin/sh
read -r request
printf '%s\n' "$request" >> '{}/focus-calls'
cat '{}/focus-reply'
"#,
                f.directory.display(),
                f.directory.display()
            ),
        );
        let core = Core::at(script);
        let reply = f.directory.join("focus-reply");
        std::fs::write(&reply, json!({"policies":[{"identityId":WORKER,"revision":1,"active":true,"focusUntilMs":100,"remainingMs":99,"heldCount":3}]}).to_string()).unwrap();
        let row = json!({"id":WORKER,"name":"worker","fields":{}});
        let retired = json!({"id":"retired","name":"old member","fields":{}});
        let absent = json!({"id":"absent","name":"removed member","fields":{}});
        let mut doc = json!({"squad":{"lead":row},"sections":[{"rows":[row,row,retired,absent]}]});
        enrich(&core, &mut [&mut doc], &BTreeSet::from([WORKER.into()]));
        assert_eq!(
            doc["squad"]["lead"]["focus"],
            doc["sections"][0]["rows"][0]["focus"]
        );
        assert_eq!(doc["sections"][0]["rows"][1]["focus"]["heldCount"], 3);
        assert!(doc["sections"][0]["rows"][2].get("focus").is_none());
        assert!(doc["sections"][0]["rows"][3].get("focus").is_none());
        let request: Value = serde_json::from_str(
            std::fs::read_to_string(f.directory.join("focus-calls"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(
            request,
            json!({"version":1,"operation":"focus.policy.show","input":{"identities":[WORKER]}})
        );
        for response in [
            json!({"error":{"code":"API_INPUT_INVALID","message":"old core"}}),
            json!({"policies":[{"identityId":WORKER}]}),
        ] {
            std::fs::write(&reply, response.to_string()).unwrap();
            let mut fresh = row.clone();
            enrich(&core, &mut [&mut fresh], &BTreeSet::from([WORKER.into()]));
            assert!(fresh.get("focus").is_none());
        }
        let mut large = json!(
            (0..300)
                .map(|n| json!({"id":format!("id-{n:03}"),"name":"worker","fields":{}}))
                .collect::<Vec<_>>()
        );
        let active_ids = (0..300).map(|n| format!("id-{n:03}")).collect();
        enrich(&core, &mut [&mut large], &active_ids);
        let calls = std::fs::read_to_string(f.directory.join("focus-calls")).unwrap();
        let request: Value = serde_json::from_str(calls.lines().last().unwrap()).unwrap();
        assert_eq!(
            request["input"]["identities"].as_array().unwrap().len(),
            LIMIT
        );
        assert_eq!(calls.lines().count(), 4);
    }
}
