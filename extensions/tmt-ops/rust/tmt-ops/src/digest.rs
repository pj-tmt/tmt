//! Core-owned digest policy: one bounded read, then snapshot-only projections.
use crate::core::{Core, SquadError};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const LIMIT: usize = 256;
/// The word that names the policy chip.
pub const WORD: &str = "digest";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub identity_id: String,
    pub revision: u64,
    pub active: bool,
    pub digest_until_ms: u64,
    pub remaining_ms: u64,
    pub held_count: u64,
}
impl Policy {
    pub fn row(&self) -> Value {
        json!({"active": self.active, "digestUntilMs": self.digest_until_ms,
            "remainingMs": self.remaining_ms, "heldCount": self.held_count})
    }
}
pub fn read_policies(core: &Core, ids: &[String]) -> Result<BTreeMap<String, Policy>, SquadError> {
    let document = core.api("digest.policy.show", json!({"identities": ids}))?;
    let invalid = || {
        SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "Core returned an incomplete digest policy.",
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
                digest_until_ms: number("digestUntilMs")?,
                remaining_ms: number("remainingMs")?,
                held_count: number("heldCount")?,
            })
        })
        .collect::<Result<Vec<_>, SquadError>>()?;
    if policies.len() != ids.len() || policies.iter().zip(ids).any(|(p, id)| &p.identity_id != id) {
        return Err(SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "Core returned a mismatched digest policy.",
        ));
    }
    Ok(policies
        .into_iter()
        .map(|p| (p.identity_id.clone(), p))
        .collect())
}
/// Collect every row occurrence before reading. Duplicate rows and squad membership
/// share one policy; only identities admitted by the acquired active room rosters
/// are eligible. Overflow and optional-Core failures omit digest without noise.
pub fn enrich(core: &Core, documents: &mut [&mut Value], active_ids: &BTreeSet<String>) {
    let ids = eligible(documents.iter().map(|document| &**document), active_ids);
    if ids.is_empty() {
        return;
    }
    if let Some(rows) = rows(core, &ids) {
        for document in documents {
            set(document, Some(&rows));
        }
    }
}

/// The identities one bounded policy read covers, in read order.
pub fn eligible<'a>(
    documents: impl IntoIterator<Item = &'a Value>,
    active_ids: &BTreeSet<String>,
) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for document in documents {
        collect(document, active_ids, &mut ids);
    }
    ids.into_iter().take(LIMIT).collect()
}

/// Each identity's projected digest row; `None` when the optional read failed.
pub fn rows(core: &Core, ids: &[String]) -> Option<BTreeMap<String, Value>> {
    if ids.is_empty() {
        return Some(BTreeMap::new());
    }
    let policies = read_policies(core, ids).ok()?;
    Some(
        policies
            .into_iter()
            .map(|(id, policy)| (id, policy.row()))
            .collect(),
    )
}

/// Gives every member row exactly the digest `rows` holds for it, removing any
/// other; `None` (a failed read) removes all, as a document read without digest.
pub fn set(document: &mut Value, rows: Option<&BTreeMap<String, Value>>) {
    match document {
        Value::Object(object) => {
            if object.contains_key("fields")
                && let Some(id) = object.get("id").and_then(Value::as_str)
            {
                match rows.and_then(|rows| rows.get(id)) {
                    Some(row) => {
                        object.insert("digest".into(), row.clone());
                    }
                    None => {
                        object.remove("digest");
                    }
                }
            }
            for value in object.values_mut() {
                set(value, rows);
            }
        }
        Value::Array(values) => {
            for value in values {
                set(value, rows);
            }
        }
        _ => {}
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
/// Board time advances independently of snapshot acquisition. Expiry hides the
/// label even while a slow/failed refresh retains the previous snapshot.
pub fn remaining(row: &Value, now: u64) -> Option<(u64, u64)> {
    let digest = &row["digest"];
    let until = digest["digestUntilMs"].as_u64()?;
    (digest["active"] == true && until > now).then(|| {
        (
            until.saturating_sub(now),
            digest["heldCount"].as_u64().unwrap_or(0),
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
        format!("{WORD} {}{suffix}", minutes(ms))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_rounds_minutes_and_hides_expired_or_inactive_policies() {
        let mut row = json!({"digest":{"active":true,"digestUntilMs":4_801_001,"heldCount":2}});
        assert_eq!(label(&row, 1001).as_deref(), Some("digest 1h20m · 2 held"));
        assert_eq!(
            label(&row, 4_741_001).as_deref(),
            Some("digest 1m · 2 held")
        );
        assert_eq!(
            label(&row, 4_801_000).as_deref(),
            Some("digest 1m · 2 held")
        );
        assert_eq!(label(&row, 4_801_001), None);
        row["digest"]["heldCount"] = json!(0);
        assert_eq!(label(&row, 1000).as_deref(), Some("digest 1h21m"));
        assert_eq!(detail(&row, 1000).as_deref(), Some("1h21m left"));
        assert_eq!(label(&row, 1).as_deref(), Some("digest 1h21m"));
        assert_eq!(minutes(7_200_000), "2h");
        row["digest"]["active"] = json!(false);
        assert_eq!(label(&row, 0), None);
        assert_eq!(label(&json!({}), 0), None);
    }
    #[test]
    fn batching_deduplicates_rows_bounds_input_and_omits_unavailable_or_malformed_core() {
        use crate::cron_service::test_support::{Fixture, WORKER};
        let f = Fixture::new();
        let script = f.directory.join("digest-core");
        crate::test_support::write_ready_executable(
            &script,
            &format!(
                r#"#!/bin/sh
read -r request
printf '%s\n' "$request" >> '{}/digest-calls'
cat '{}/digest-reply'
"#,
                f.directory.display(),
                f.directory.display()
            ),
        );
        let core = Core::at(script);
        let reply = f.directory.join("digest-reply");
        std::fs::write(&reply, json!({"policies":[{"identityId":WORKER,"revision":1,"active":true,"digestUntilMs":100,"remainingMs":99,"heldCount":3}]}).to_string()).unwrap();
        let row = json!({"id":WORKER,"name":"worker","fields":{}});
        let retired = json!({"id":"retired","name":"old member","fields":{}});
        let absent = json!({"id":"absent","name":"removed member","fields":{}});
        let mut doc = json!({"squad":{"lead":row},"sections":[{"rows":[row,row,retired,absent]}]});
        enrich(&core, &mut [&mut doc], &BTreeSet::from([WORKER.into()]));
        assert_eq!(
            doc["squad"]["lead"]["digest"],
            doc["sections"][0]["rows"][0]["digest"]
        );
        assert_eq!(doc["sections"][0]["rows"][1]["digest"]["heldCount"], 3);
        assert!(doc["sections"][0]["rows"][2].get("digest").is_none());
        assert!(doc["sections"][0]["rows"][3].get("digest").is_none());
        let request: Value = serde_json::from_str(
            std::fs::read_to_string(f.directory.join("digest-calls"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(
            request,
            json!({"version":1,"operation":"digest.policy.show","input":{"identities":[WORKER]}})
        );
        for response in [
            json!({"error":{"code":"API_INPUT_INVALID","message":"old core"}}),
            json!({"policies":[{"identityId":WORKER}]}),
        ] {
            std::fs::write(&reply, response.to_string()).unwrap();
            let mut fresh = row.clone();
            enrich(&core, &mut [&mut fresh], &BTreeSet::from([WORKER.into()]));
            assert!(fresh.get("digest").is_none());
        }
        let mut large = json!(
            (0..300)
                .map(|n| json!({"id":format!("id-{n:03}"),"name":"worker","fields":{}}))
                .collect::<Vec<_>>()
        );
        let active_ids = (0..300).map(|n| format!("id-{n:03}")).collect();
        enrich(&core, &mut [&mut large], &active_ids);
        let calls = std::fs::read_to_string(f.directory.join("digest-calls")).unwrap();
        let request: Value = serde_json::from_str(calls.lines().last().unwrap()).unwrap();
        assert_eq!(
            request["input"]["identities"].as_array().unwrap().len(),
            LIMIT
        );
        assert_eq!(calls.lines().count(), 4);
    }
}
