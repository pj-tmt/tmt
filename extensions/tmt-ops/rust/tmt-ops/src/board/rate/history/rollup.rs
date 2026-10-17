//! Bounded closed windows from Core's retained tier, admitted by the one history reader.
use super::{SAFE, SLOT_MS, Slice, read_slices};
use serde_json::Value;
use std::collections::BTreeMap;

pub(in crate::board) type Seeds = BTreeMap<String, Option<Seed>>;

pub(in crate::board) const RESOLUTION_MS: u64 = 300_000;
pub(in crate::board) const RETENTION_MS: u64 = 30 * 86_400_000;
const MAX_BARS: u64 = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::board) struct Request {
    each_ms: u64,
    range_ms: u64,
}

impl Request {
    pub fn new(each_ms: u64, range_ms: u64) -> Option<Self> {
        (each_ms >= RESOLUTION_MS
            && each_ms.is_multiple_of(RESOLUTION_MS)
            && (3_600_001..=RETENTION_MS).contains(&range_ms)
            && range_ms.is_multiple_of(each_ms)
            && (1..=MAX_BARS).contains(&(range_ms / each_ms)))
        .then_some(Self { each_ms, range_ms })
    }

    /// Asking Core for exactly this count returns whole Each intervals; a fixed
    /// 120-bucket request over seven days cannot produce six-hour bars exactly.
    pub fn buckets(self) -> usize {
        (self.range_ms / self.each_ms) as usize
    }

    pub fn each_ms(self) -> u64 {
        self.each_ms
    }
    pub fn range_ms(self) -> u64 {
        self.range_ms
    }
}

#[derive(Clone, Debug)]
pub(in crate::board) struct Seed {
    pub request: Request,
    pub through: u64,
    pub fine_through: u64,
    pub sampled: Option<u64>,
    pub latest: Value,
    pub buckets: Vec<Slice>,
    pub limits: Vec<(usize, crate::board::limits::observation::Report)>,
}

impl Seed {
    pub fn read(response: &Value, row: &Value, request: Request) -> Option<Self> {
        let as_of = response["asOfMs"].as_u64().filter(|n| *n <= SAFE)?;
        let fine_through = response["throughMs"].as_u64()?;
        if response["resolutionMs"] != SLOT_MS
            || fine_through != as_of / SLOT_MS * SLOT_MS
            || response["retainedFromMs"] != fine_through.saturating_sub(7_200_000)
            || row["found"] != true
        {
            return None;
        }
        let mut matching = row["windows"]
            .as_array()?
            .iter()
            .filter(|window| window["windowMs"] == request.range_ms);
        let window = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        let through = as_of / RESOLUTION_MS * RESOLUTION_MS;
        let from = through.saturating_sub(request.range_ms);
        if window["resolutionMs"] != RESOLUTION_MS
            || window["fromMs"] != from
            || window["toMs"] != through
            || window["bucketMs"] != request.each_ms
        {
            return None;
        }
        let available = if window["availableFromMs"].is_null() {
            None
        } else {
            Some(
                window["availableFromMs"]
                    .as_u64()
                    .filter(|at| *at <= through)?,
            )
        };
        let retained_from = through
            .saturating_sub(RETENTION_MS)
            .max(available.unwrap_or(0));
        if window["retainedFromMs"] != retained_from {
            return None;
        }
        let sampled =
            if row["lastSampleAtMs"].is_null() {
                None
            } else {
                Some(row["lastSampleAtMs"].as_u64().filter(|at| {
                    (fine_through.saturating_sub(7_200_000)..fine_through).contains(at)
                })?)
            };
        let buckets = read_slices(window, from, through, request.each_ms, request.buckets())?;
        let limits = window["buckets"]
            .as_array()?
            .iter()
            .enumerate()
            .filter_map(|(index, raw)| {
                let report = crate::board::limits::observation::Report::read(&raw["rateLimits"])?;
                let bucket = &buckets[index];
                (bucket.from_ms..bucket.to_ms)
                    .contains(&report.observed_at_ms)
                    .then_some((index, report))
            })
            .collect();
        Some(Self {
            request,
            through,
            fine_through,
            sampled,
            latest: row["latest"].clone(),
            buckets,
            limits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(request: Request) -> Value {
        let through = 1_791_600_000_000 / RESOLUTION_MS * RESOLUTION_MS;
        let from = through - request.range_ms;
        let buckets: Vec<_> = (0..request.buckets()).map(|index| {
            let start = from + index as u64 * request.each_ms;
            json!({"fromMs":start,"toMs":start+request.each_ms,"inputTokens":0,"outputTokens":0,"cachedInputTokens":0,"coveredMs":request.each_ms,"complete":true,"gap":false,"discontinuous":false})
        }).collect();
        let fine_through = through + 20_000;
        json!({"asOfMs":through+22_500,"throughMs":fine_through,"resolutionMs":SLOT_MS,"retainedFromMs":fine_through-7_200_000,"identities":[{"found":true,"reporting":false,"availableFromMs":null,"lastSampleAtMs":null,"latest":null,"windows":[{"windowMs":request.range_ms,"fromMs":from,"toMs":through,"bucketMs":request.each_ms,"resolutionMs":RESOLUTION_MS,"availableFromMs":from,"retainedFromMs":from.max(through-RETENTION_MS),"buckets":buckets}]}]})
    }

    #[test]
    fn requested_intervals_are_exact_and_response_size_stays_bounded() {
        for (range, intervals) in [
            (360, vec![15, 30, 60]),
            (1_440, vec![60, 120]),
            (10_080, vec![360, 720, 1_440]),
            (43_200, vec![1_440, 2_880]),
        ] {
            for each in intervals {
                let request = Request::new(each * 60_000, range * 60_000).unwrap();
                assert!(request.buckets() <= 48);
                assert!(32 * request.buckets() <= 23_040);
                let rounded = (request.range_ms / RESOLUTION_MS).div_ceil(request.buckets() as u64)
                    * RESOLUTION_MS;
                assert_eq!(rounded, request.each_ms);
            }
        }
        for (each, range) in [
            (300_000, 3_600_000),
            (300_000, RETENTION_MS),
            (86_400_000, RETENTION_MS + 86_400_000),
            (300_001, 21_600_000),
            (0, 21_600_000),
        ] {
            assert!(Request::new(each, range).is_none());
        }
    }

    #[test]
    fn retained_evidence_does_not_require_recent_fine_reporting() {
        let request = Request::new(21_600_000, 604_800_000).unwrap();
        let mut value = response(request);
        value["identities"][0]["windows"][0]["buckets"][0]["inputTokens"] = json!(17);
        let seed = Seed::read(&value, &value["identities"][0], request).unwrap();
        assert_eq!(seed.through + 20_000, seed.fine_through);
        assert_eq!(seed.buckets[0].input_tokens, 17);
        assert_eq!(seed.buckets[1].input_tokens, 0);
        assert!(seed.buckets[1].complete);
        assert_eq!(seed.buckets.len(), 28);
        assert!(seed.latest.is_null() && seed.sampled.is_none());
    }

    #[test]
    fn unavailable_and_partial_intervals_never_become_measured_zero() {
        let request = Request::new(86_400_000, RETENTION_MS).unwrap();
        let mut value = response(request);
        let bucket = &mut value["identities"][0]["windows"][0]["buckets"][0];
        bucket["coveredMs"] = json!(0);
        bucket["complete"] = json!(false);
        bucket["gap"] = json!(true);
        let next = &mut value["identities"][0]["windows"][0]["buckets"][1];
        next["inputTokens"] = json!(9);
        next["coveredMs"] = json!(300_000);
        next["complete"] = json!(false);
        next["gap"] = json!(true);
        let seed = Seed::read(&value, &value["identities"][0], request).unwrap();
        assert!(!seed.buckets[0].complete && seed.buckets[0].covered_ms == 0);
        assert_eq!(seed.buckets[1].input_tokens, 9);
        assert_eq!(seed.buckets[1].covered_ms, 300_000);
    }

    #[test]
    fn optional_quota_keeps_source_time_inside_its_bucket_and_cannot_destroy_tokens() {
        let request = crate::board::rate::history::quota_request();
        let mut value = response(request);
        let from = value["identities"][0]["windows"][0]["buckets"][0]["fromMs"]
            .as_u64()
            .unwrap();
        let raw = json!({"observedAtMs":from+60_000,"windows":[{"windowMinutes":300,"usedPercent":40,"resetsAtMs":from+86_400_000}]});
        value["identities"][0]["windows"][0]["buckets"][0]["rateLimits"] = raw;
        for (at, count) in [(from + 60_000, 1), (from - 1, 0), (from + 300_000, 0)] {
            value["identities"][0]["windows"][0]["buckets"][0]["rateLimits"]["observedAtMs"] =
                json!(at);
            let seed = Seed::read(&value, &value["identities"][0], request).unwrap();
            assert_eq!(seed.limits.len(), count);
            assert_eq!(seed.buckets.len(), 48);
        }
        value["identities"][0]["windows"][0]["buckets"][0]["rateLimits"]["windows"][0]["usedPercent"] =
            json!(101);
        let seed = Seed::read(&value, &value["identities"][0], request).unwrap();
        assert!(seed.limits.is_empty());
        assert!(seed.buckets[0].complete);
    }

    #[test]
    fn wrong_tier_bounds_bucket_width_coverage_and_duplicate_windows_are_refused() {
        let request = Request::new(3_600_000, 86_400_000).unwrap();
        let original = response(request);
        for (key, value) in [
            ("resolutionMs", json!(5_000)),
            ("toMs", original["throughMs"].clone()),
            ("bucketMs", json!(300_000)),
            ("retainedFromMs", json!(0)),
        ] {
            let mut bad = original.clone();
            bad["identities"][0]["windows"][0][key] = value;
            assert!(
                Seed::read(&bad, &bad["identities"][0], request).is_none(),
                "{key}"
            );
        }
        let mut bad = original.clone();
        bad["identities"][0]["windows"][0]["buckets"][0]["coveredMs"] = json!(request.each_ms + 1);
        assert!(Seed::read(&bad, &bad["identities"][0], request).is_none());
        let mut bad = original.clone();
        let duplicate = bad["identities"][0]["windows"][0].clone();
        bad["identities"][0]["windows"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(Seed::read(&bad, &bad["identities"][0], request).is_none());
    }
}
