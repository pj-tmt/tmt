//! Quota evidence shares Rate's source proofs, roster lifetime and serialized worker.
use super::{Counter, Input, Member, Provider, Rate, history};
use crate::board::limits::observation::Report;

#[derive(Debug, Default)]
pub(super) struct History {
    reports: Vec<(String, Report)>,
    latest: Option<(String, Report)>,
}

impl Rate {
    /// The fixed quota tier never replaces the chart tier or recent token ring.
    pub fn seed_quota(&mut self, input: &Input, seeds: Option<&history::rollup::Seeds>, now: u64) {
        self.retain(input);
        for id in input.resumes.keys() {
            let state = self.members.entry(id.clone()).or_default();
            state.quota = seeds.map(|_| History::default());
            let Some(seed) = seeds
                .and_then(|seeds| seeds.get(id))
                .and_then(Option::as_ref)
                .filter(|seed| seed.request == history::quota_request() && seed.through <= now)
            else {
                continue;
            };
            let latest = Counter::read(&seed.latest);
            let linked = latest
                .as_ref()
                .is_some_and(|latest| latest.usable && state.previous.as_ref() == Some(latest))
                && seed.sampled.is_some()
                && state.source_bridge(seed.through, seed.fine_through);
            let providers = state.history_providers(&seed.buckets, latest.as_ref(), linked);
            let reports = seed
                .limits
                .iter()
                .filter_map(|(index, report)| {
                    let Provider::Driver(driver) = *providers.get(*index)? else {
                        return None;
                    };
                    Some((
                        state.drivers.get(usize::from(driver))?.clone(),
                        report.clone(),
                    ))
                })
                .collect();
            state.quota.as_mut().expect("active quota demand").reports = reports;
        }
    }

    pub(in crate::board) fn quota_reports(&self) -> impl Iterator<Item = (&str, &Report)> {
        self.members
            .values()
            .flat_map(|member| member.quota.iter())
            .flat_map(|quota| {
                quota
                    .reports
                    .iter()
                    .chain(quota.latest.iter())
                    .map(|(driver, report)| (driver.as_str(), report))
            })
    }
}

pub(super) fn observe(state: &mut Member, resume: &serde_json::Value, counter: &Counter, now: u64) {
    let Some(quota) = state.quota.as_mut() else {
        return;
    };
    quota.latest = Report::read(&resume["consumption"]["rateLimits"])
        .filter(|report| report.observed_at_ms <= now && report.observed_at_ms == counter.observed)
        .map(|report| (counter.key.0.clone(), report));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::limits::{QuotaWindow, pace};
    use serde_json::json;
    const NOW: u64 = 1_791_600_000_000;
    fn seeded() -> (Rate, Input, history::rollup::Seeds) {
        let mut input = super::super::tests::input(100);
        input.resumes.get_mut("a").unwrap()["consumption"]["observedAtMs"] = json!(NOW - 5_000);
        let latest = input.resumes["a"].clone();
        let fine = history::Seed {
            from: NOW - 3_600_000,
            through: NOW,
            available: Some(NOW - 3_600_000),
            sampled: Some(NOW - 5_000),
            latest: latest.clone(),
            reporting: true,
            buckets: vec![history::Slice {
                from_ms: NOW - 3_600_000,
                to_ms: NOW,
                input_tokens: 10,
                output_tokens: 0,
                cached_input_tokens: 0,
                covered_ms: 3_600_000,
                complete: true,
                gap: false,
                discontinuous: false,
            }],
        };
        let mut rate = Rate::default();
        rate.seed(&input, &[("a".into(), Some(fine))].into(), NOW);
        let request = history::quota_request();
        let buckets: Vec<_> = (0..48)
            .map(|i| {
                let from = NOW - 14_400_000 + i * 300_000;
                history::Slice {
                    from_ms: from,
                    to_ms: from + 300_000,
                    input_tokens: 1,
                    output_tokens: 0,
                    cached_input_tokens: 0,
                    covered_ms: 300_000,
                    complete: true,
                    gap: false,
                    discontinuous: false,
                }
            })
            .collect();
        let limits = buckets.iter().enumerate().map(|(i,b)| (i,Report::read(&json!({"observedAtMs":b.from_ms+60_000,
            "windows":[{"windowMinutes":10080,"usedPercent":20.0+i as f64,"resetsAtMs":NOW+86_400_000}]})).unwrap())).collect();
        let seed = history::rollup::Seed {
            request,
            through: NOW,
            fine_through: NOW,
            sampled: Some(NOW - 5_000),
            latest,
            buckets,
            limits,
        };
        (rate, input, [("a".into(), Some(seed))].into())
    }
    #[test]
    fn all_four_hours_share_the_verified_suffix_without_changing_token_history() {
        let (mut rate, input, seeds) = seeded();
        let before = rate.reading(NOW, crate::config::TokenWindow::HOUR);
        rate.seed_quota(&input, Some(&seeds), NOW);
        assert_eq!(rate.quota_reports().count(), 48);
        let observed = pace::project(
            rate.quota_reports().map(|(_, r)| r),
            QuotaWindow::Weekly,
            NOW,
            NOW,
        );
        assert_eq!(observed.per_hour, Some(12.0));
        assert_eq!(observed.trend, Some(crate::board::limits::Trend::Same));
        assert_eq!(rate.reading(NOW, crate::config::TokenWindow::HOUR), before);
        assert!(rate.members["a"].retained.is_none());
        rate.seed_quota(&input, None, NOW);
        assert_eq!(rate.quota_reports().count(), 0);
    }
    #[test]
    fn current_short_only_quota_is_available_even_when_token_continuity_is_unknown() {
        let (mut rate, mut input, _) = seeded();
        rate.seed_quota(&input, Some(&Default::default()), NOW);
        let resume = input.resumes.get_mut("a").unwrap();
        resume["consumption"]["sequence"] = json!(102);
        resume["consumption"]["observedAtMs"] = json!(NOW - 4_000);
        resume["consumption"]["complete"] = json!(false);
        resume["consumption"]["gap"] = json!(true);
        resume["consumption"]["rateLimits"] = json!({"observedAtMs":NOW-4_000,
            "windows":[{"windowMinutes":300,"usedPercent":20,"resetsAtMs":NOW+3_600_000}]});
        rate.sample(&input, NOW);
        let reports: Vec<_> = rate.quota_reports().map(|(_, report)| report).collect();
        assert_eq!(reports.len(), 1);
        let value = pace::project(reports, QuotaWindow::Short, NOW, NOW);
        assert_eq!(value.latest.unwrap().balance().left, 80.0);
        assert_eq!(value.per_hour, None);
    }

    #[test]
    fn gaps_watermark_mismatch_and_roster_removal_cannot_attribute_old_quota() {
        let (mut rate, input, mut seeds) = seeded();
        let seed = seeds.get_mut("a").unwrap().as_mut().unwrap();
        seed.buckets[30].complete = false;
        seed.buckets[30].gap = true;
        rate.seed_quota(&input, Some(&seeds), NOW);
        assert_eq!(rate.quota_reports().count(), 17);
        let seed = seeds.get_mut("a").unwrap().as_mut().unwrap();
        seed.latest["driver"] = json!("different-driver");
        rate.seed_quota(&input, Some(&seeds), NOW);
        assert_eq!(rate.quota_reports().count(), 0);
        let mut empty = input;
        empty.resumes.clear();
        rate.retain(&empty);
        assert!(rate.members.is_empty());
    }
}
