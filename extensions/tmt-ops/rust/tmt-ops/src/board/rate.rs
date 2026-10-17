//! Public closed history and live counters share one per-identity bucket ring.
//! No provider files, core crate, context-window subtraction or in-flight estimate.
use crate::config::TokenWindow;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) mod history;
mod quota;
pub(super) mod series;

const SAFE: u64 = 9_007_199_254_740_991;
pub(in crate::board) const SLOT_MS: u64 = 5_000;

/// Captured from the ordinary observed roster before sections duplicate/filter rows.
#[derive(Debug, Clone)]
pub struct Input {
    pub room: String,
    pub resumes: BTreeMap<String, Value>,
    /// The observed roster includes the lead and members omitted by row filters.
    pub names: BTreeMap<String, String>,
}

impl Input {
    pub fn observed(room: &str, members: &[crate::squad::Member]) -> Self {
        Self {
            room: room.into(),
            resumes: members
                .iter()
                .map(|member| (member.id.clone(), member.seen["resume"].clone()))
                .collect(),
            names: members
                .iter()
                .map(|member| (member.id.clone(), member.name.clone()))
                .collect(),
        }
    }

    /// A meter-only read uses exactly the already observed membership.
    pub fn listed(&self, listed: &Value) -> Self {
        self.joined(&Self::resumes(listed))
    }

    pub fn resumes(listed: &Value) -> BTreeMap<String, Value> {
        listed["identities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| Some((row["id"].as_str()?.to_owned(), row["resume"].clone())))
            .collect()
    }

    pub fn joined(&self, rows: &BTreeMap<String, Value>) -> Self {
        Self {
            room: self.room.clone(),
            names: self.names.clone(),
            resumes: self
                .resumes
                .keys()
                .map(|id| {
                    (
                        id.clone(),
                        rows.get(id.as_str()).cloned().unwrap_or(Value::Null),
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Counter {
    key: (String, String, String),
    input: u64,
    output: u64,
    cached: u64,
    sequence: u64,
    observed: u64,
    usable: bool,
}

fn nonempty(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(str::to_owned)
}

/// The model a listed resume record reports. `consumption.modelId` is the
/// model of the last accepted completed turn, so it follows a mid-session
/// `/model` switch; the starting-hook `model` only changes when a session
/// starts. A missing or unusable value is unknown, never a guess.
pub(in crate::board) fn resume_model(resume: &Value) -> Option<&str> {
    [&resume["consumption"]["modelId"], &resume["model"]]
        .into_iter()
        .find_map(|value| value.as_str().filter(|s| !s.is_empty() && s.len() <= 256))
}

impl Counter {
    fn read(resume: &Value) -> Option<Self> {
        let value = &resume["consumption"];
        let number = |key: &str| value[key].as_u64().filter(|n| *n <= SAFE);
        let counter = Self {
            key: (
                nonempty(&resume["driver"])?,
                nonempty(&resume["session"])?,
                nonempty(&value["epoch"]).filter(|epoch| {
                    epoch.len() == 36
                        && epoch.bytes().enumerate().all(|(i, byte)| {
                            if [8, 13, 18, 23].contains(&i) {
                                byte == b'-'
                            } else {
                                byte.is_ascii_hexdigit()
                            }
                        })
                })?,
            ),
            input: number("inputTokens")?,
            output: number("outputTokens")?,
            cached: number("cachedInputTokens")?,
            sequence: number("sequence").filter(|n| *n > 0)?,
            observed: number("observedAtMs").filter(|n| *n > 0)?,
            usable: value["complete"].as_bool()? && !value["gap"].as_bool()?,
        };
        // Both flags are required, even when complete is false (no short circuit).
        let gap = value["gap"].as_bool()?;
        let complete = value["complete"].as_bool()?;
        (counter.cached <= counter.input
            && counter.input + counter.output <= SAFE
            && !(complete && gap))
            .then_some(counter)
    }
}

#[derive(Debug, Default)]
struct Member {
    previous: Option<Counter>,
    blocked: bool,
    reporter: bool,
    model: Option<String>,
    began: Option<u64>,
    sampled: Option<u64>,
    buckets: Vec<Bucket>,
    /// A bounded coarse resolution of the same source history. A projection
    /// chooses this or the recent ring; overlapping tiers are never added.
    retained: Option<Retained>,
    quota: Option<quota::History>,
    /// Only mixed-provenance cells need a split. Keys are the owning ring slot,
    /// removed on replacement; this never carries independent history or totals.
    provider_splits: BTreeMap<u64, BTreeMap<u16, u128>>,
    drivers: Vec<String>,
    current_driver: Option<u16>,
}

#[derive(Debug)]
struct Retained {
    request: history::rollup::Request,
    through: u64,
    buckets: Vec<Bucket>,
}

/// Verified counted provenance belongs to each ring cell. Mixed cells retain
/// only their proven provider portions; current metadata never relabels history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::board) enum Provider {
    #[default]
    Unknown,
    Driver(u16),
    Mixed,
}

/// Receipt-time batches and coverage; each reporting UUID owns one bounded ring.
#[derive(Debug, Clone, Copy, Default)]
struct Bucket {
    slot: Option<u64>,
    tokens: u128,
    provider: Provider,
    evidence: bool,
    gap: bool,
    /// Whole core rollup tokens cannot be split at a shorter window boundary.
    source_from: Option<u64>,
    /// Keep source-backed duration distinct from live receipt evidence. History
    /// padding slots carry no independent token or duration observation.
    origin: BucketOrigin,
}

#[derive(Debug, Clone, Copy, Default)]
enum BucketOrigin {
    #[default]
    Receipt,
    // Compact duration covers every admitted interval (within thirty days)
    // without expanding the recent ring's cell budget.
    History {
        covered_ms: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub tokens: u128,
    pub partial: bool,
    pub span: u64,
}

#[derive(Debug)]
pub struct Rate {
    members: BTreeMap<String, Member>,
    slots: usize,
    last_success: Option<u64>,
}

impl Default for Rate {
    fn default() -> Self {
        Self::new(TokenWindow::HOUR)
    }
}

impl Member {
    fn observe_driver(&mut self, resume: &serde_json::Value) {
        self.current_driver = resume["driver"]
            .as_str()
            .filter(|driver| crate::board::limits::Provider::from_driver(driver).is_some())
            .and_then(|driver| match self.driver(driver) {
                Provider::Driver(id) => Some(id),
                _ => None,
            });
    }
    fn driver(&mut self, driver: &str) -> Provider {
        if crate::board::limits::Provider::from_driver(driver).is_none() {
            return Provider::Unknown;
        }
        if let Some(index) = self.drivers.iter().position(|name| name == driver) {
            return Provider::Driver(index as u16);
        }
        // Keep the current display name as well as every counted ring reference.
        let reusable = (0..self.drivers.len()).find(|index| {
            let id = *index as u16;
            self.current_driver != Some(id)
                && !self
                    .buckets
                    .iter()
                    .chain(self.retained.iter().flat_map(|view| &view.buckets))
                    .any(|bucket| bucket.provider == Provider::Driver(id))
                && !self
                    .provider_splits
                    .values()
                    .any(|parts| parts.contains_key(&id))
        });
        if let Some(index) = reusable {
            self.drivers[index] = driver.to_owned();
            Provider::Driver(index as u16)
        } else if let Ok(index) = u16::try_from(self.drivers.len()) {
            self.drivers.push(driver.to_owned());
            Provider::Driver(index)
        } else {
            Provider::Unknown
        }
    }
    fn history_providers(
        &mut self,
        slices: &[history::Slice],
        latest: Option<&Counter>,
        mut linked: bool,
    ) -> Vec<Provider> {
        let mut providers = vec![Provider::Unknown; slices.len()];
        for (index, slice) in slices.iter().enumerate().rev() {
            linked &= slice.complete && !slice.gap && !slice.discontinuous;
            if linked && let Some(latest) = latest {
                providers[index] = self.driver(&latest.key.0);
            }
        }
        providers
    }

    /// A complete fine interval may witness continuity across a boundary;
    /// this proof allocates no token fraction to either side of that boundary.
    fn source_bridge(&self, from: u64, through: u64) -> bool {
        if through < from {
            return false;
        }
        let mut intervals: Vec<_> = self
            .buckets
            .iter()
            .filter_map(|bucket| {
                let start = bucket.source_from?;
                let end = bucket.slot?.checked_add(1)?.checked_mul(SLOT_MS)?;
                let BucketOrigin::History {
                    covered_ms: Some(covered),
                } = bucket.origin
                else {
                    return None;
                };
                (!bucket.gap
                    && u64::from(covered) == end.checked_sub(start)?
                    && end > from
                    && start < through)
                    .then_some((start, end))
            })
            .collect();
        intervals.sort_unstable();
        let mut cursor = from;
        for (start, end) in intervals {
            if start > cursor {
                break;
            }
            cursor = cursor.max(end);
        }
        cursor >= through
    }
    fn provider_tokens<'a>(&'a self, bucket: &Bucket) -> impl Iterator<Item = (u16, u128)> + 'a {
        let single = match bucket.provider {
            Provider::Driver(index) => Some((index, bucket.tokens)),
            _ => None,
        };
        let mixed = if bucket.provider == Provider::Mixed {
            bucket.slot.and_then(|slot| self.provider_splits.get(&slot))
        } else {
            None
        };
        single.into_iter().chain(
            mixed
                .into_iter()
                .flat_map(|parts| parts.iter().map(|(id, tokens)| (*id, *tokens))),
        )
    }

    fn interval(
        &mut self,
        sampled: Option<u64>,
        now: u64,
        evidence: bool,
        gap: bool,
        counted: (u128, Provider),
        slots: usize,
    ) {
        let (tokens, provider) = counted;
        self.sampled = Some(now);
        if !self.reporter {
            return;
        }
        if self.buckets.is_empty() {
            self.buckets.resize(slots, Bucket::default());
        }
        let end = now / SLOT_MS;
        let start = sampled
            .map_or(end, |at| at / SLOT_MS + 1)
            .max(end.saturating_sub(slots as u64 - 1));
        for slot in start..=end {
            let bucket = &mut self.buckets[(slot % slots as u64) as usize];
            if bucket.slot != Some(slot) {
                if let Some(old) = bucket.slot {
                    self.provider_splits.remove(&old);
                }
                *bucket = Bucket {
                    slot: Some(slot),
                    ..Default::default()
                };
            }
            bucket.evidence |= evidence;
            bucket.gap |= gap;
        }
        // Repeated observations within one slot coalesce. Keep proven portions
        // even when a driver switch or seeded unknown shares that same slot.
        let index = (end % slots as u64) as usize;
        if self.buckets[index].slot != Some(end) {
            if let Some(old) = self.buckets[index].slot {
                self.provider_splits.remove(&old);
            }
            self.buckets[index] = Bucket {
                slot: Some(end),
                ..Default::default()
            };
        }
        let prior = self.buckets[index];
        let mut portions: BTreeMap<_, _> = self.provider_tokens(&prior).collect();
        let bucket = &mut self.buckets[index];
        bucket.evidence |= evidence;
        bucket.gap |= gap;
        if tokens > 0 {
            if bucket.tokens == 0 || bucket.provider == provider {
                bucket.provider = provider;
            } else {
                match provider {
                    Provider::Driver(id) => *portions.entry(id).or_default() += tokens,
                    Provider::Unknown | Provider::Mixed => (),
                }
                self.provider_splits.insert(end, portions);
                bucket.provider = Provider::Mixed;
            }
        }
        bucket.tokens += tokens;
    }

    fn reading(&self, now: u64, window: TokenWindow) -> Option<(Reading, u64)> {
        let span = now.saturating_sub(self.began?).min(window.milliseconds());
        let end = now / SLOT_MS;
        let count = window.milliseconds() / SLOT_MS;
        let mut reading = Reading {
            tokens: 0,
            partial: span < window.milliseconds(),
            span,
        };
        let mut evidence = false;
        let mut observed_slots = 0;
        let mut covered_slots = 0;
        for bucket in &self.buckets {
            if bucket
                .slot
                .is_some_and(|slot| slot <= end && end - slot < count)
            {
                let cut = bucket
                    .source_from
                    .is_some_and(|from| from < now.saturating_sub(window.milliseconds()));
                if !cut {
                    reading.tokens += bucket.tokens;
                    // Known lower-bound deltas remain data without continuous coverage.
                    evidence |= bucket.tokens > 0;
                }
                reading.partial |= cut;
                reading.partial |= bucket.gap;
                evidence |= bucket.evidence;
                covered_slots += u64::from(bucket.evidence);
                observed_slots += 1;
            }
        }
        reading.partial |= observed_slots < count;
        // A baseline or uncovered zero alone is not a measured zero.
        evidence.then_some((reading, covered_slots))
    }
}

impl Rate {
    pub fn new(longest: TokenWindow) -> Self {
        Self {
            members: BTreeMap::new(),
            slots: (longest.milliseconds() / SLOT_MS) as usize,
            last_success: None,
        }
    }

    /// Replace the authoritative recent range, preserving older observations.
    pub fn seed(&mut self, input: &Input, seeds: &history::Seeds, now: u64) {
        self.retain(input);
        for id in input.resumes.keys() {
            let state = self.members.entry(id.clone()).or_default();
            state.observe_driver(&input.resumes[id]);
            let Some(seed) = seeds.get(id).and_then(Option::as_ref) else {
                state.previous = None;
                state.blocked = true;
                state.interval(
                    state.sampled,
                    now,
                    false,
                    state.reporter,
                    (0, Provider::Unknown),
                    self.slots,
                );
                continue;
            };
            if seed.through > now {
                state.previous = None;
                state.blocked = true;
                continue;
            }
            state.reporter |= seed.reporting;
            if let Some(available) = seed.available {
                state.began = Some(state.began.map_or(available, |old| old.min(available)));
            }
            if state.buckets.is_empty() {
                state.buckets.resize(self.slots, Bucket::default());
            }
            // Includes open live receipts: latest, not through, is the seed watermark.
            for bucket in &mut state.buckets {
                if bucket.slot.is_some_and(|slot| slot >= seed.from / SLOT_MS) {
                    if let Some(slot) = bucket.slot {
                        state.provider_splits.remove(&slot);
                    }
                    *bucket = Bucket::default();
                }
            }
            let latest = Counter::read(&seed.latest);
            let linked =
                latest.as_ref().is_some_and(|counter| counter.usable) && seed.sampled.is_some();
            let providers = state.history_providers(&seed.buckets, latest.as_ref(), linked);
            for (slice, provider) in seed.buckets.iter().zip(providers) {
                let first = slice.from_ms / SLOT_MS;
                let last = (slice.to_ms - 1) / SLOT_MS;
                for slot in first..=last {
                    let bucket = &mut state.buckets[(slot % self.slots as u64) as usize];
                    if let Some(old) = bucket.slot {
                        state.provider_splits.remove(&old);
                    }
                    *bucket = Bucket {
                        slot: Some(slot),
                        evidence: slice.complete,
                        gap: !slice.complete || slice.gap || slice.discontinuous,
                        origin: BucketOrigin::History {
                            covered_ms: Some(0),
                        },
                        ..Default::default()
                    };
                }
                let bucket = &mut state.buckets[(last % self.slots as u64) as usize];
                bucket.tokens = u128::from(slice.input_tokens) + u128::from(slice.output_tokens);
                bucket.provider = provider;
                bucket.source_from = Some(slice.from_ms);
                bucket.origin = BucketOrigin::History {
                    covered_ms: u32::try_from(slice.covered_ms).ok(),
                };
                bucket.evidence |= slice.covered_ms > 0;
            }
            state.previous = Counter::read(&seed.latest);
            state.blocked = state
                .previous
                .as_ref()
                .is_none_or(|counter| !counter.usable);
            state.sampled = seed.sampled;
        }
    }

    /// Retained evidence stays in this Rate owner, with at most 48 whole source
    /// intervals per UUID. It never changes the recent ring or live baseline.
    pub fn seed_retained(&mut self, input: &Input, seeds: &history::rollup::Seeds, now: u64) {
        self.retain(input);
        for id in input.resumes.keys() {
            let state = self.members.entry(id.clone()).or_default();
            state.retained = None;
            let Some(seed) = seeds
                .get(id)
                .and_then(Option::as_ref)
                .filter(|seed| seed.through <= now)
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
            let buckets = seed
                .buckets
                .iter()
                .zip(providers)
                .map(|(slice, provider)| Bucket {
                    slot: Some((slice.to_ms - 1) / SLOT_MS),
                    tokens: u128::from(slice.input_tokens) + u128::from(slice.output_tokens),
                    provider,
                    evidence: slice.complete || slice.covered_ms > 0,
                    gap: !slice.complete || slice.gap || slice.discontinuous,
                    source_from: Some(slice.from_ms),
                    origin: BucketOrigin::History {
                        covered_ms: u32::try_from(slice.covered_ms).ok(),
                    },
                })
                .collect();
            state.retained = Some(Retained {
                request: seed.request,
                through: seed.through,
                buckets,
            });
        }
    }

    /// Monotonic receipt time; model attribution is deliberately best effort.
    pub fn sample(&mut self, input: &Input, now: u64) {
        self.last_success = Some(now);
        self.retain(input);
        for (id, resume) in &input.resumes {
            let state = self.members.entry(id.clone()).or_default();
            state.observe_driver(&input.resumes[id]);
            state.model = resume_model(resume).map(str::to_owned);
            state.reporter |= resume["consumption"].is_object();
            let Some(next) = Counter::read(resume) else {
                state.blocked = true;
                state.interval(
                    state.sampled,
                    now,
                    false,
                    state.reporter,
                    (0, Provider::Unknown),
                    self.slots,
                );
                continue;
            };
            state.reporter = true;
            if let Some(previous) = &state.previous
                && previous.key == next.key
                && (next.sequence < previous.sequence
                    || next.observed < previous.observed
                    || (next.sequence == previous.sequence && next != *previous))
            {
                state.blocked = true;
                state.interval(
                    state.sampled,
                    now,
                    false,
                    true,
                    (0, Provider::Unknown),
                    self.slots,
                );
                continue;
            }
            quota::observe(state, resume, &next, now);
            if !next.usable {
                state.previous = Some(next);
                state.blocked = true;
                state.interval(
                    state.sampled,
                    now,
                    false,
                    true,
                    (0, Provider::Unknown),
                    self.slots,
                );
                continue;
            }
            let continuous = state.previous.as_ref().filter(|previous| {
                !state.blocked
                    && previous.usable
                    && previous.key == next.key
                    && next.input >= previous.input
                    && next.output >= previous.output
                    && next.cached >= previous.cached
            });
            let (evidence, gap, tokens) = match continuous {
                Some(previous) => (
                    true,
                    false,
                    u128::from(next.input - previous.input)
                        + u128::from(next.output - previous.output),
                ),
                None => (
                    false,
                    state.previous.is_some() || state.sampled.is_some_and(|at| at < now),
                    0,
                ),
            };
            state.began.get_or_insert(now);
            let provider = state.driver(&next.key.0);
            state.previous = Some(next);
            state.blocked = false;
            state.interval(
                state.sampled,
                now,
                evidence,
                gap,
                (tokens, provider),
                self.slots,
            );
        }
    }

    pub fn retain(&mut self, input: &Input) {
        self.members.retain(|id, _| input.resumes.contains_key(id));
    }

    /// Failed reads and tab suspension cannot bridge unobserved intervals.
    pub fn failed(&mut self, now: u64, every_ms: u64) {
        let gap = self
            .last_success
            .is_some_and(|at| now.saturating_sub(at) >= every_ms * 2);
        for member in self.members.values_mut() {
            member.blocked = true;
            member.interval(
                member.sampled,
                now,
                false,
                gap,
                (0, Provider::Unknown),
                self.slots,
            );
        }
    }

    pub fn member(&self, id: &str, now: u64, window: TokenWindow) -> Option<Reading> {
        self.observation(id, now, window)
            .map(|(reading, _)| reading)
    }

    /// Retained verified bucket evidence ranks duplicate UUID observations.
    /// This is an evidence count, not a reconstructed coverage duration.
    pub fn observation(&self, id: &str, now: u64, window: TokenWindow) -> Option<(Reading, u64)> {
        self.members.get(id)?.reading(now, window)
    }

    pub fn model(&self, id: &str) -> Option<&str> {
        self.members.get(id)?.model.as_deref()
    }

    pub fn reading(&self, now: u64, window: TokenWindow) -> Option<Reading> {
        let mut sum = Reading {
            tokens: 0,
            partial: false,
            span: window.milliseconds(),
        };
        let mut evidence = false;
        for member in self.members.values() {
            if let Some((reading, _)) = member.reading(now, window) {
                sum.tokens += reading.tokens;
                sum.partial |= reading.partial;
                sum.span = sum.span.min(reading.span);
                evidence = true;
            } else {
                sum.partial = true;
            }
        }
        evidence.then_some(sum)
    }

    pub fn reporter(&self, id: &str) -> bool {
        self.members.get(id).is_some_and(|member| member.reporter)
    }

    /// Eight bucket-aligned totals, from the same UUID rings as member readings.
    /// Inclusive bucket groups share these bounds with their hover readout.
    pub fn bar_end(now: u64, window: TokenWindow, index: usize) -> u64 {
        let slots = (window.milliseconds() / SLOT_MS).div_ceil(8);
        (now / SLOT_MS)
            .saturating_sub((7 - index.min(7)) as u64 * slots)
            .saturating_add(1)
            .saturating_mul(SLOT_MS)
    }
    pub fn trend(&self, now: u64, window: TokenWindow) -> [Option<f64>; 8] {
        let bar_slots = (window.milliseconds() / SLOT_MS).div_ceil(8);
        let end = now / SLOT_MS;
        std::array::from_fn(|index| {
            let last = end.checked_sub((7 - index) as u64 * bar_slots)?;
            let first = last.saturating_sub(bar_slots - 1);
            let mut tokens = 0_u128;
            let mut evidence = false;
            for bucket in self.members.values().flat_map(|member| &member.buckets) {
                if bucket
                    .slot
                    .is_some_and(|slot| (first..=last).contains(&slot))
                {
                    if bucket
                        .source_from
                        .is_none_or(|from| from >= now.saturating_sub(window.milliseconds()))
                    {
                        tokens += bucket.tokens;
                        evidence |= bucket.tokens > 0;
                    }
                    evidence |= bucket.evidence;
                }
            }
            evidence.then_some(tokens as f64)
        })
    }
}

#[cfg(test)]
pub(super) mod tests;
