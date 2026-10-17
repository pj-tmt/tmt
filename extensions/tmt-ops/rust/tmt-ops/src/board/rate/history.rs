//! Transient, bounded public API seeds. Rate consumes these into its existing rings.
use super::{SAFE, SLOT_MS};
use crate::{config::TokenWindow, core::Core};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::board) mod rollup;

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
        let buckets = read_slices(reading, from, through, bucket_ms, 120)?;
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

/// Fine and retained windows validate the same normalized deltas and coverage.
fn read_slices(
    reading: &Value,
    from: u64,
    through: u64,
    bucket_ms: u64,
    maximum: usize,
) -> Option<Vec<Slice>> {
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
    if buckets.len() > maximum {
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
    Some(buckets)
}

/// Reuse only normalized history with unchanged durable evidence and closed range.
/// One worker owns this short-lived cache; failed reads are never retained.
#[derive(Default)]
pub(in crate::board) struct Cache {
    key: Option<(u64, u64, u64)>,
    seeds: Seeds,
    retained_key: Option<(u64, u64, rollup::Request)>,
    retained: rollup::Seeds,
}

impl Cache {
    pub fn retained(
        &mut self,
        core: &Core,
        ids: BTreeSet<String>,
        request: rollup::Request,
        cursor: Option<u64>,
        now: u64,
    ) -> rollup::Seeds {
        let through = now / rollup::RESOLUTION_MS * rollup::RESOLUTION_MS;
        let key = cursor.map(|cursor| (cursor, through, request));
        if key.is_none() || self.retained_key != key {
            self.retained.clear();
            self.retained_key = key;
        }
        let missing = ids
            .iter()
            .filter(|id| !self.retained.contains_key(*id))
            .cloned()
            .collect();
        let loaded = read_window(
            core,
            missing,
            request.range_ms(),
            request.buckets(),
            |response, row| rollup::Seed::read(response, row, request),
        );
        for (id, seed) in &loaded {
            if key.is_some() && seed.as_ref().is_some_and(|seed| seed.through == through) {
                self.retained.insert(id.clone(), seed.clone());
            }
        }
        ids.into_iter()
            .map(|id| {
                let seed = self
                    .retained
                    .get(&id)
                    .or_else(|| loaded.get(&id))
                    .cloned()
                    .flatten();
                (id, seed)
            })
            .collect()
    }

    pub fn load(
        &mut self,
        core: &Core,
        ids: BTreeSet<String>,
        longest: TokenWindow,
        cursor: Option<u64>,
        now: u64,
    ) -> Seeds {
        let through = now / SLOT_MS * SLOT_MS;
        let key = cursor.map(|cursor| (cursor, through, longest.milliseconds()));
        if key.is_none() || self.key != key {
            self.seeds.clear();
            self.key = key;
        }
        let missing = ids
            .iter()
            .filter(|id| !self.seeds.contains_key(*id))
            .cloned()
            .collect();
        let loaded = load(core, missing, longest);
        for (id, seed) in &loaded {
            if key.is_some() && seed.as_ref().is_some_and(|seed| seed.through == through) {
                self.seeds.insert(id.clone(), seed.clone());
            }
        }
        ids.into_iter()
            .map(|id| {
                let seed = self
                    .seeds
                    .get(&id)
                    .or_else(|| loaded.get(&id))
                    .cloned()
                    .flatten();
                (id, seed)
            })
            .collect()
    }
}

/// Acquire only one longest window; overlapping shorter windows are never added.
pub fn load(core: &Core, ids: BTreeSet<String>, longest: TokenWindow) -> Seeds {
    let window = longest.milliseconds().min(3_600_000);
    read_window(core, ids, window, 120, |response, row| {
        Seed::read(response, row, window)
    })
}

/// Both resolutions use this one bounded public-API reader. Each window keeps
/// its own snapshot and count; callers never add overlapping seed totals.
fn read_window<T>(
    core: &Core,
    ids: BTreeSet<String>,
    window: u64,
    maximum: usize,
    decode: impl Fn(&Value, &Value) -> Option<T>,
) -> BTreeMap<String, Option<T>> {
    let ids: Vec<_> = ids.into_iter().collect();
    let mut seeds = BTreeMap::new();
    for batch in ids.chunks(32) {
        let response = core.api(
            "consumption.history",
            json!({"identityIds": batch, "windowsMs": [window], "maxBuckets": maximum}),
        );
        for id in batch {
            let seed = response.as_ref().ok().and_then(|response| {
                let rows = response["identities"].as_array()?;
                let mut matching = rows.iter().filter(|row| row["id"] == *id);
                let row = matching.next()?;
                if matching.next().is_some() {
                    return None;
                }
                decode(response, row)
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
    fn retained_cache_requires_cursor_closed_five_minute_range_and_exact_selection() {
        let request = rollup::Request::new(900_000, 21_600_000).unwrap();
        let other_each = rollup::Request::new(1_800_000, 21_600_000).unwrap();
        let other_range = rollup::Request::new(3_600_000, 86_400_000).unwrap();
        let seed = rollup::Seed {
            request,
            through: 30_000_000,
            fine_through: 30_020_000,
            sampled: None,
            latest: Value::Null,
            buckets: Vec::new(),
        };
        let core = Core::at("/nonexistent-squad-retained-cache-test".into());
        for (cursor, now, selected, reused) in [
            (Some(7), 30_299_999, request, true),
            (Some(8), 30_299_999, request, false),
            (None, 30_299_999, request, false),
            (Some(7), 30_300_000, request, false),
            (Some(7), 29_999_999, request, false),
            (Some(7), 30_299_999, other_each, false),
            (Some(7), 30_299_999, other_range, false),
        ] {
            let mut cache = Cache {
                retained_key: Some((7, seed.through, request)),
                retained: [("known".into(), Some(seed.clone()))].into(),
                ..Default::default()
            };
            let result = cache.retained(
                &core,
                ["known".into(), "new".into()].into(),
                selected,
                cursor,
                now,
            );
            assert_eq!(result["known"].is_some(), reused);
            assert!(result["new"].is_none());
            assert!(
                !cache.retained.contains_key("new"),
                "failed reads are retryable"
            );
            assert!(
                cache.seeds.is_empty(),
                "coarse reuse cannot populate fine evidence"
            );
        }
    }

    #[test]
    fn cached_history_requires_known_cursor_same_closed_range_and_window() {
        let window = TokenWindow::parse("1m").unwrap();
        let seed = Seed {
            from: 0,
            through: 60000,
            available: None,
            sampled: None,
            latest: Value::Null,
            reporting: false,
            buckets: Vec::new(),
        };
        // A cache miss cannot accidentally pass: this executable does not exist.
        let core = Core::at(std::path::PathBuf::from(
            "/nonexistent-squad-history-cache-test",
        ));
        for (cursor, now, selected, reused) in [
            (Some(7), 64999, window, true),
            (Some(8), 64999, window, false),
            (None, 64999, window, false),
            (Some(7), 65000, window, false),
            (Some(7), 59999, window, false),
            (Some(7), 64999, TokenWindow::parse("5m").unwrap(), false),
        ] {
            let mut cache = Cache {
                key: Some((7, 60000, 60000)),
                seeds: [("known".into(), Some(seed.clone()))].into(),
                ..Default::default()
            };
            let result = cache.load(
                &core,
                ["known".into(), "new".into()].into(),
                selected,
                cursor,
                now,
            );
            assert_eq!(result["known"].is_some(), reused);
            assert!(
                result["new"].is_none(),
                "new membership never borrows another identity's history"
            );
            assert!(
                !cache.seeds.contains_key("new"),
                "failed reads remain retryable"
            );
        }
    }

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
            &Core::at(fake.clone()),
            ids.clone(),
            TokenWindow::parse("24h").unwrap(),
        );
        assert_eq!(seeds.keys().cloned().collect::<BTreeSet<_>>(), ids);
        assert!(
            seeds.values().all(Option::is_none),
            "missing identities are unavailable, not zero"
        );
        let requests: Vec<Value> = std::fs::read_to_string(&calls)
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
        std::fs::write(&calls, "").unwrap();
        let request = rollup::Request::new(86_400_000, rollup::RETENTION_MS).unwrap();
        let mut cache = Cache::default();
        let retained = cache.retained(
            &Core::at(fake),
            ids.clone(),
            request,
            Some(7),
            3_000_000_000,
        );
        assert_eq!(retained.keys().cloned().collect::<BTreeSet<_>>(), ids);
        assert!(retained.values().all(Option::is_none));
        assert!(cache.retained.is_empty());
        let requests: Vec<Value> = std::fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let sizes: Vec<_> = requests
            .iter()
            .map(|value| {
                assert_eq!(value["operation"], "consumption.history");
                assert_eq!(value["input"]["windowsMs"], json!([rollup::RETENTION_MS]));
                assert_eq!(value["input"]["maxBuckets"], 30);
                value["input"]["identityIds"].as_array().unwrap().len()
            })
            .collect();
        assert_eq!(sizes, [32, 32, 1]);
        std::fs::remove_dir_all(dir).expect("remove isolated history fixture directory");
    }
}
