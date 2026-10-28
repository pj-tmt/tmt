//! Bounded normalized history reads, and the account-level limits read.
use super::{Fault, Request, invalid};
use crate::storage::{BUCKET_MS, MAX_BUCKETS, MAX_RESPONSE_BUCKETS, Storage, window_resolution_ms};
use serde::{Deserialize, Serialize};
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
    /// The cut of an earlier batch of the same refresh (#4176). Absent means the
    /// read time; an explicit null is not a cut.
    #[serde(default, deserialize_with = "cut")]
    through_ms: Option<u64>,
}
fn cut<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(deserializer).map(Some)
}
/// A pin carries one refresh's cut to its remaining batches; it is not a
/// historical query, so one older than this before the read is refused.
const PIN_MAX_AGE_MS: u64 = 60_000;
/// Whether a pinned cut is refused at read time `now`: it must sit on the 5-second
/// resolution and be neither in the future nor more than a minute old.
fn pin_refused(cut: u64, now: u64) -> bool {
    !cut.is_multiple_of(BUCKET_MS) || cut > now || now - cut > PIN_MAX_AGE_MS
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
        through_ms: value.through_ms,
    })
}
/// The limits of every provider account Core can read without an agent (#5418). Storage-free:
/// each driver reads its own account sources (the pushed store and the ambient root), so the
/// call opens no database and creates nothing.
pub(super) fn limits(data_root: &std::path::Path) -> Result<Vec<u8>, Fault> {
    let environment = crate::skill_installation::ProviderEnvironment::capture()
        .map_err(|_| Fault::unavailable())?
        .with_data_root(data_root);
    limits_in(&environment, crate::request_runtime::wall_time_ms())
}

fn limits_in(
    environment: &crate::skill_installation::ProviderEnvironment,
    now: u64,
) -> Result<Vec<u8>, Fault> {
    #[derive(Serialize)]
    struct Entry<'a> {
        driver: &'a str,
        #[serde(flatten)]
        limits: crate::runtime::consumption::AccountLimits,
    }
    let registry = crate::runtime::RuntimeRegistry::first_party();
    let mut entries = Vec::new();
    for driver in registry.harnesses() {
        let Some(lifecycle) = tmt_core::binding::session::HarnessId::new(driver)
            .ok()
            .and_then(|harness| registry.lifecycle(&harness))
        else {
            continue;
        };
        entries.extend(
            lifecycle
                .account_limits(environment, now)
                .into_iter()
                .map(|limits| Entry { driver, limits }),
        );
    }
    entries.sort_by(|left, right| {
        (left.driver, &left.limits.account.key).cmp(&(right.driver, &right.limits.account.key))
    });
    serde_json::to_vec(&serde_json::json!({"observedAtMs": now, "accounts": entries}))
        .map_err(|_| Fault::unavailable())
}

pub(super) fn history(
    storage: &mut Storage,
    identities: Vec<String>,
    windows: Vec<u64>,
    max_buckets: u64,
    include_retired: bool,
    through_ms: Option<u64>,
) -> Result<Vec<u8>, Fault> {
    let now = crate::request_runtime::wall_time_ms();
    if through_ms.is_some_and(|cut| pin_refused(cut, now)) {
        return Err(invalid());
    }
    let value = storage
        .consumption_history(
            &identities,
            &windows,
            max_buckets,
            now,
            include_retired,
            through_ms,
        )
        .map_err(|_| Fault::unavailable())?;
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use crate::{skill_installation::ProviderEnvironment, test_support::TestDirectory};
    use serde_json::{Value, json};

    const NOW: u64 = 1_791_607_000_000;
    const RESET: u64 = 1_791_620_000_679;

    fn environment(home: &TestDirectory, data: &TestDirectory) -> ProviderEnvironment {
        ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), [])
            .with_data_root(&data.path)
    }

    fn write_config(home: &TestDirectory, cache: bool) {
        let mut config = json!({"oauthAccount": {"accountUuid": "account-a"}});
        if cache {
            config["cachedUsageUtilization"] = json!({
                "fetchedAtMs": NOW - 1_000, "accountUuid": "account-a",
                "utilization": {
                    "five_hour": {"utilization": 8, "resets_at": "2026-10-10T08:13:20.679456+00:00"},
                    "seven_day": null
                }
            });
        }
        std::fs::write(
            home.path.join(".claude.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
    }

    fn read(environment: &ProviderEnvironment) -> Value {
        serde_json::from_slice(&limits_in(environment, NOW).unwrap()).unwrap()
    }

    #[test]
    fn an_account_comes_back_with_the_public_rate_limit_shape_the_driver_and_its_key() {
        let (home, data) = (TestDirectory::new(), TestDirectory::new());
        write_config(&home, true);
        let result = read(&environment(&home, &data));
        assert_eq!(result["observedAtMs"], NOW);
        let [entry] = result["accounts"].as_array().unwrap().as_slice() else {
            panic!("one account expected: {result}");
        };
        assert_eq!(entry["driver"], "claude");
        assert_eq!(entry["account"]["label"], "claude");
        assert_eq!(entry["account"]["key"].as_str().unwrap().len(), 12);
        assert_eq!(entry.get("reason"), None);
        assert_eq!(
            entry["rateLimits"],
            json!({
                "provider": "claude",
                "observedAtMs": NOW - 1_000,
                "windows": [{"windowMinutes": 300, "usedPercent": 8.0, "resetsAtMs": RESET}],
                "account": entry["account"]
            })
        );
    }

    #[test]
    fn a_known_account_with_no_current_window_is_null_with_its_reason() {
        let (home, data) = (TestDirectory::new(), TestDirectory::new());
        write_config(&home, false);
        let result = read(&environment(&home, &data));
        let entry = &result["accounts"][0];
        assert_eq!(entry["rateLimits"], Value::Null);
        assert_eq!(entry["reason"], "none_observed");
        assert_eq!(entry["driver"], "claude");
        // The cache's only window has elapsed by the time of the read.
        write_config(&home, true);
        let later = serde_json::from_slice::<Value>(
            &limits_in(&environment(&home, &data), RESET + 1).unwrap(),
        )
        .unwrap();
        assert_eq!(later["accounts"][0]["rateLimits"], Value::Null);
        assert_eq!(later["accounts"][0]["reason"], "expired");
    }

    #[test]
    fn nothing_known_is_an_empty_list_and_the_read_creates_nothing() {
        let (home, data) = (TestDirectory::new(), TestDirectory::new());
        let result = read(&environment(&home, &data));
        assert_eq!(result["accounts"], json!([]));
        write_config(&home, true);
        let before = std::fs::read(home.path.join(".claude.json")).unwrap();
        let _ = read(&environment(&home, &data));
        assert_eq!(std::fs::read_dir(&data.path).unwrap().count(), 0);
        assert_eq!(
            std::fs::read(home.path.join(".claude.json")).unwrap(),
            before
        );
    }
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
            // A cut is a non-negative integer; an explicit null is not one.
            ("throughMs", json!(null)),
            ("throughMs", json!(-5000)),
            ("throughMs", json!(1.5)),
            ("throughMs", json!("5000")),
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

    #[test]
    fn a_pinned_cut_is_decoded_and_only_a_recent_aligned_past_one_is_admitted() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../contracts/consumption-history-v1.json"
        ))
        .unwrap();
        let mut input = fixture["request"]["input"].clone();
        input["throughMs"] = json!(15_000);
        assert!(matches!(
            decode(input.to_string().as_bytes()),
            Ok(Request::ConsumptionHistory {
                through_ms: Some(15_000),
                ..
            })
        ));
        input.as_object_mut().unwrap().remove("throughMs");
        assert!(matches!(
            decode(input.to_string().as_bytes()),
            Ok(Request::ConsumptionHistory {
                through_ms: None,
                ..
            })
        ));
        let now = 1_000_000;
        for (cut, refused) in [
            (now, false),
            (now - 5_000, false),
            (995_000, false),
            (now - 60_000, false),
            // Older than a minute, in the future, or off the 5-second resolution.
            (now - 65_000, true),
            (now + 5_000, true),
            (now - 1, true),
            (now + 1, true),
            (0, true),
        ] {
            assert_eq!(pin_refused(cut, now), refused, "cut {cut} at {now}");
        }
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
