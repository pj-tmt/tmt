//! Bounded normalized history reads; no provider discovery or access.
use super::{Fault, Request, invalid};
use crate::storage::{MAX_BUCKETS, MAX_RESPONSE_BUCKETS, Storage, window_resolution_ms};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    identity_ids: Vec<String>,
    windows_ms: Vec<u64>,
    #[serde(default = "default_max_buckets")]
    max_buckets: u64,
    #[serde(default)]
    include_retired: bool,
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
        || value.windows_ms.iter().collect::<HashSet<_>>().len() != value.windows_ms.len()
        || !(1..=MAX_BUCKETS).contains(&value.max_buckets)
    {
        return Err(invalid());
    }
    // One response stays bounded: every identity returns at most `maxBuckets`
    // buckets per window, fewer when the window has fewer slots.
    let mut per_identity = 0u64;
    for window in &value.windows_ms {
        let resolution = window_resolution_ms(*window).ok_or_else(invalid)?;
        per_identity += value.max_buckets.min(window / resolution);
    }
    if per_identity * value.identity_ids.len() as u64 > MAX_RESPONSE_BUCKETS {
        return Err(invalid());
    }
    Ok(Request::ConsumptionHistory {
        identities: value.identity_ids,
        windows: value.windows_ms,
        max_buckets: value.max_buckets,
        include_retired: value.include_retired,
    })
}
pub(super) fn history(
    storage: &mut Storage,
    identities: Vec<String>,
    windows: Vec<u64>,
    max_buckets: u64,
    include_retired: bool,
) -> Result<Vec<u8>, Fault> {
    let now = crate::request_runtime::wall_time_ms();
    let value = storage
        .consumption_history(&identities, &windows, max_buckets, now, include_retired)
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
        assert!(decode(fixture["rollup"]["request"]["input"].to_string().as_bytes()).is_ok());
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
            ("maxBuckets", json!(721)),
            ("windowsMs", json!([3_650_000])),
            ("windowsMs", json!([DAYS_30 + 300_000])),
            ("includeRetired", json!("yes")),
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
                include_retired: false,
                ..
            })
        ));
    }

    const DAYS_30: u64 = 2_592_000_000;

    fn ids(count: usize) -> Vec<String> {
        (0..count)
            .map(|n| format!("11111111-1111-4111-8111-{n:012}"))
            .collect()
    }

    #[test]
    fn longer_windows_read_the_rollup_and_one_response_stays_bounded() {
        let ask = |count: usize, windows: serde_json::Value, max: u64| {
            decode(
                json!({"identityIds":ids(count),"windowsMs":windows,"maxBuckets":max,
                       "includeRetired":true})
                .to_string()
                .as_bytes(),
            )
        };
        // Five-minute multiples above an hour, up to 30 days.
        for window in [3_900_000u64, 86_400_000, 604_800_000, DAYS_30] {
            assert!(ask(1, json!([window]), 120).is_ok(), "{window}");
        }
        assert!(matches!(
            ask(2, json!([604_800_000]), 720),
            Ok(Request::ConsumptionHistory {
                include_retired: true,
                max_buckets: 720,
                ..
            })
        ));
        // 32 identities x 720 buckets is exactly the response bound; one more
        // window or one more identity-bucket is refused.
        assert!(ask(32, json!([DAYS_30]), 720).is_ok());
        assert!(ask(32, json!([DAYS_30, 604_800_000]), 720).is_err());
        assert!(ask(32, json!([86_400_000, 604_800_000, DAYS_30]), 720).is_err());
        // Today's largest valid request still passes: 32 x 3 x 120.
        assert!(ask(32, json!([60_000, 300_000, 3_600_000]), 120).is_ok());
        // A short window has fewer slots than buckets and counts only its slots.
        assert!(ask(32, json!([5_000, 10_000, 15_000]), 720).is_ok());
    }
}
