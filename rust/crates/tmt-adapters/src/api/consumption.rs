//! Bounded normalized history reads; no provider discovery or access.
use super::{Fault, Request, invalid};
use crate::storage::{BUCKET_MS, Storage};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    identity_ids: Vec<String>,
    windows_ms: Vec<u64>,
    #[serde(default = "default_max_buckets")]
    max_buckets: u64,
}
fn default_max_buckets() -> u64 {
    120
}

pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    let value: Input = serde_json::from_slice(input).map_err(|_| invalid())?;
    if !(1..=32).contains(&value.identity_ids.len())
        || value
            .identity_ids
            .iter()
            .any(|id| !tmt_core::dispatch::canonical_id(id))
        || value.identity_ids.iter().collect::<HashSet<_>>().len() != value.identity_ids.len()
        || !(1..=3).contains(&value.windows_ms.len())
        || value
            .windows_ms
            .iter()
            .any(|window| !(BUCKET_MS..=3_600_000).contains(window) || window % BUCKET_MS != 0)
        || value.windows_ms.iter().collect::<HashSet<_>>().len() != value.windows_ms.len()
        || !(1..=120).contains(&value.max_buckets)
    {
        return Err(invalid());
    }
    Ok(Request::ConsumptionHistory {
        identities: value.identity_ids,
        windows: value.windows_ms,
        max_buckets: value.max_buckets,
    })
}
pub(super) fn history(
    storage: &mut Storage,
    identities: Vec<String>,
    windows: Vec<u64>,
    max_buckets: u64,
) -> Result<Vec<u8>, Fault> {
    let now = crate::request_runtime::wall_time_ms();
    let value = storage
        .consumption_history(&identities, &windows, max_buckets, now)
        .map_err(|_| Fault::unavailable())?;
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strict_bounds_and_shared_request() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../contracts/consumption-history-v1.json"
        ))
        .unwrap();
        assert!(decode(fixture["request"]["input"].to_string().as_bytes()).is_ok());
        let good = fixture["request"]["input"].clone();
        for (key, value) in [
            ("identityIds", json!([])),
            ("identityIds", json!(["bad"])),
            (
                "identityIds",
                json!([good["identityIds"][0], good["identityIds"][0]]),
            ),
            ("windowsMs", json!([])),
            ("windowsMs", json!([60000, 60000])),
            ("windowsMs", json!([3605000])),
            ("windowsMs", json!([4999])),
            ("maxBuckets", json!(0)),
            ("maxBuckets", json!(121)),
            ("extra", json!(true)),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(decode(bad.to_string().as_bytes()).is_err(), "{bad}");
        }
        let mut default = good;
        default.as_object_mut().unwrap().remove("maxBuckets");
        assert!(matches!(
            decode(default.to_string().as_bytes()),
            Ok(Request::ConsumptionHistory {
                max_buckets: 120,
                ..
            })
        ));
    }
}
