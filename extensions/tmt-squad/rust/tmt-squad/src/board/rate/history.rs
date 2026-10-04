//! Transient, bounded public API seeds. Rate consumes these into its existing rings.
use super::{SAFE, SLOT_MS};
use crate::{config::TokenWindow, core::Core};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub type Seeds = BTreeMap<String, Option<Seed>>;

#[derive(Debug, Clone)]
pub struct Slice {
    pub from_ms: u64,
    pub to_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub covered_ms: u64,
    pub complete: bool,
    pub gap: bool,
    pub discontinuous: bool,
}

#[derive(Debug, Clone)]
pub struct Seed {
    pub from: u64,
    pub through: u64,
    pub available: Option<u64>,
    pub sampled: Option<u64>,
    pub latest: Value,
    pub reporting: bool,
    pub buckets: Vec<Slice>,
}

impl Seed {
    pub fn read(response: &Value, row: &Value, window: u64) -> Option<Self> {
        let as_of = response["asOfMs"].as_u64().filter(|n| *n <= SAFE)?;
        let through = response["throughMs"].as_u64()?;
        if response["resolutionMs"] != SLOT_MS || through != as_of / SLOT_MS * SLOT_MS {
            return None;
        }
        let retained = response["retainedFromMs"].as_u64()?;
        if retained != through.saturating_sub(7_200_000) || row["found"] != true {
            return None;
        }
        let windows = row["windows"].as_array()?;
        let reading = windows.iter().find(|v| v["windowMs"] == window)?;
        let from = reading["fromMs"].as_u64()?;
        let bucket_ms = reading["bucketMs"].as_u64()?;
        if from != through.saturating_sub(window)
            || reading["toMs"] != through
            || bucket_ms != (window / SLOT_MS).div_ceil(120) * SLOT_MS
        {
            return None;
        }
        let buckets: Vec<Slice> = reading["buckets"]
            .as_array()?
            .iter()
            .map(|v| {
                Some(Slice {
                    from_ms: v["fromMs"].as_u64()?,
                    to_ms: v["toMs"].as_u64()?,
                    input_tokens: v["inputTokens"].as_u64()?,
                    output_tokens: v["outputTokens"].as_u64()?,
                    cached_input_tokens: v["cachedInputTokens"].as_u64()?,
                    covered_ms: v["coveredMs"].as_u64()?,
                    complete: v["complete"].as_bool()?,
                    gap: v["gap"].as_bool()?,
                    discontinuous: v["discontinuous"].as_bool()?,
                })
            })
            .collect::<Option<_>>()?;
        if buckets.len() > 120 {
            return None;
        }
        let mut cursor = from;
        for bucket in &buckets {
            if bucket.from_ms != cursor
                || bucket.to_ms != (cursor + bucket_ms).min(through)
                || bucket.to_ms <= cursor
                || bucket.covered_ms > bucket.to_ms - cursor
                || bucket.input_tokens > SAFE
                || bucket.output_tokens > SAFE - bucket.input_tokens
                || bucket.cached_input_tokens > bucket.input_tokens
                || (bucket.complete && (bucket.gap || bucket.covered_ms != bucket.to_ms - cursor))
            {
                return None;
            }
            cursor = bucket.to_ms;
        }
        if cursor != through {
            return None;
        }
        let timestamp = |key: &str| {
            if row[key].is_null() {
                Some(None)
            } else {
                row[key]
                    .as_u64()
                    .filter(|at| (retained..through).contains(at))
                    .map(Some)
            }
        };
        Some(Self {
            from,
            through,
            available: timestamp("availableFromMs")?,
            sampled: timestamp("lastSampleAtMs")?,
            latest: row["latest"].clone(),
            reporting: row["reporting"].as_bool()?,
            buckets,
        })
    }
}

/// Acquire only one longest window; overlapping shorter windows are never added.
pub fn load(core: &Core, ids: BTreeSet<String>, longest: TokenWindow) -> Seeds {
    let ids: Vec<_> = ids.into_iter().collect();
    let window = longest.milliseconds().min(3_600_000);
    let mut seeds = BTreeMap::new();
    for batch in ids.chunks(32) {
        let response = core.api(
            "consumption.history",
            json!({"identityIds": batch, "windowsMs": [window], "maxBuckets": 120}),
        );
        for id in batch {
            let seed = response.as_ref().ok().and_then(|response| {
                let rows = response["identities"].as_array()?;
                let mut matching = rows.iter().filter(|row| row["id"] == *id);
                let row = matching.next()?;
                if matching.next().is_some() {
                    return None;
                }
                Seed::read(response, row, window)
            });
            seeds.insert(id.clone(), seed);
        }
    }
    seeds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_history_batches_roster_uuids_and_caps_long_configured_windows() {
        let dir = std::env::temp_dir().join(format!(
            "squad-history-batch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).expect("create unique history fixture directory");
        let calls = dir.join("calls");
        let fake = dir.join("tmt");
        crate::test_support::write_ready_executable(
            &fake,
            &format!(
                "#!/bin/sh\n[ \"$1\" = api ] || exit 2\nrequest=$(cat)\nprintf '%s\\n' \"$request\" >> '{}'\nprintf '%s\\n' '{{\"identities\":[]}}'\n",
                calls.display()
            ),
        );
        let ids: BTreeSet<_> = (0..65)
            .map(|n| format!("{n:08x}-0000-4000-8000-000000000001"))
            .collect();
        let seeds = load(
            &Core::at(fake),
            ids.clone(),
            TokenWindow::parse("24h").unwrap(),
        );
        assert_eq!(seeds.keys().cloned().collect::<BTreeSet<_>>(), ids);
        assert!(
            seeds.values().all(Option::is_none),
            "missing identities are unavailable, not zero"
        );
        let requests: Vec<Value> = std::fs::read_to_string(calls)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(requests.len(), 3);
        let sizes: Vec<_> = requests
            .iter()
            .map(|v| {
                assert_eq!(v["operation"], "consumption.history");
                assert_eq!(v["input"]["windowsMs"], json!([3_600_000]));
                assert_eq!(v["input"]["maxBuckets"], 120);
                v["input"]["identityIds"].as_array().unwrap().len()
            })
            .collect();
        assert_eq!(sizes, [32, 32, 1]);
        std::fs::remove_dir_all(dir).expect("remove isolated history fixture directory");
    }
}
