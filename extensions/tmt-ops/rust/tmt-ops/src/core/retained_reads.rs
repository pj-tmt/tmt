//! Bounded automatic-view projections. The refresh owner supplies cursor/stamp
//! admission; selected reads, clocks and actions never receive this handle.

use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Default)]
pub(crate) struct RetainedReads(Mutex<Option<BTreeMap<String, Value>>>);

impl RetainedReads {
    pub(crate) fn reset(&self, enabled: bool) {
        *self.0.lock().unwrap() = enabled.then(BTreeMap::new);
    }

    pub(crate) fn clear(&self) {
        if let Some(values) = self.0.lock().unwrap().as_mut() {
            values.clear();
        }
    }

    pub(super) fn get(&self, request: &str) -> Option<Value> {
        self.0.lock().unwrap().as_ref()?.get(request).cloned()
    }

    pub(super) fn keep(&self, request: &Value, resource: &Value) {
        let reusable = match request["operation"].as_str() {
            Some("references.resolve") => {
                resource["identities"].is_array() || resource["rooms"].is_array()
            }
            // Status.stale is derived from wall time, outside the durable cursor.
            Some("rooms.roster") => resource["members"]
                .as_array()
                .is_some_and(|members| members.iter().all(|member| member["status"].is_null())),
            // Nonempty history contains time-derived delivery and body expiry.
            Some("requests.list") => resource["items"].as_array().is_some_and(Vec::is_empty),
            _ => false,
        };
        if !reusable {
            return;
        }
        let key = request.to_string();
        // Fixed entry and byte ceilings; a new population replaces the old one.
        if key.len() > 64 * 1024 || resource.to_string().len() > 64 * 1024 {
            return;
        }
        if let Some(values) = self.0.lock().unwrap().as_mut() {
            if values.len() >= 64 {
                values.clear();
            }
            values.insert(key, resource.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_cursor_covered_resources_without_time_or_error_state_are_reused() {
        let held = RetainedReads::default();
        held.reset(true);
        for (operation, resource, reusable) in [
            ("references.resolve", json!({"identities":[]}), true),
            ("rooms.roster", json!({"members":[{"status":null}]}), true),
            (
                "rooms.roster",
                json!({"members":[{"status":{"stale":false}}]}),
                false,
            ),
            ("requests.list", json!({"items":[],"nextBefore":null}), true),
            (
                "requests.list",
                json!({"items":[{"final":{"status":"retained"}}]}),
                false,
            ),
            ("digest.policy.show", json!({"policies":[]}), false),
            ("user.identity", json!({"identityId":"person"}), false),
            ("identityHooks.pending", json!({"items":[]}), false),
            ("rooms.roster", json!({"error":{"code":"FAILED"}}), false),
        ] {
            held.clear();
            let request = json!({"operation":operation,"input":{}});
            held.keep(&request, &resource);
            assert_eq!(
                held.get(&request.to_string()).is_some(),
                reusable,
                "{operation}: {resource}"
            );
        }
        held.reset(false);
        let request = json!({"operation":"requests.list","input":{}});
        held.keep(&request, &json!({"items":[]}));
        assert!(held.get(&request.to_string()).is_none());
    }
}
