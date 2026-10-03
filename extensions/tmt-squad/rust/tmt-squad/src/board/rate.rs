//! Public cumulative counters become receipt-time completed-request buckets.
//! No provider files, core crate, context-window subtraction or in-flight estimate.
use crate::config::TokenWindow;
use serde_json::Value;
use std::collections::BTreeMap;

const SAFE: u64 = 9_007_199_254_740_991;
const SLOT_MS: u64 = 5_000;

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
    buckets: Vec<Bucket>,
}

/// Receipt-time batches and coverage; each reporting UUID owns one bounded ring.
#[derive(Debug, Clone, Copy, Default)]
struct Bucket {
    slot: Option<u64>,
    tokens: u128,
    evidence: bool,
    gap: bool,
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
    sampled: Option<u64>,
    last_success: Option<u64>,
}

impl Default for Rate {
    fn default() -> Self {
        Self::new(TokenWindow::HOUR)
    }
}

impl Member {
    fn interval(
        &mut self,
        sampled: Option<u64>,
        now: u64,
        evidence: bool,
        gap: bool,
        tokens: u128,
        slots: usize,
    ) {
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
                *bucket = Bucket {
                    slot: Some(slot),
                    ..Default::default()
                };
            }
            bucket.evidence |= evidence;
            bucket.gap |= gap;
        }
        // Repeated observations within one slot coalesce, including gap evidence.
        let bucket = &mut self.buckets[(end % slots as u64) as usize];
        if bucket.slot != Some(end) {
            *bucket = Bucket {
                slot: Some(end),
                ..Default::default()
            };
        }
        bucket.evidence |= evidence;
        bucket.gap |= gap;
        bucket.tokens += tokens;
    }

    fn reading(&self, now: u64, window: TokenWindow) -> Option<Reading> {
        let span = now.saturating_sub(self.began?).min(window.milliseconds());
        let end = now / SLOT_MS;
        let count = window.milliseconds() / SLOT_MS;
        let mut reading = Reading {
            tokens: 0,
            partial: span < window.milliseconds(),
            span,
        };
        let mut evidence = false;
        for bucket in &self.buckets {
            if bucket
                .slot
                .is_some_and(|slot| slot <= end && end - slot < count)
            {
                reading.tokens += bucket.tokens;
                reading.partial |= bucket.gap;
                evidence |= bucket.evidence;
            }
        }
        // A baseline alone is not a measured zero.
        (span >= 10_000 && evidence).then_some(reading)
    }
}

impl Rate {
    pub fn new(longest: TokenWindow) -> Self {
        Self {
            members: BTreeMap::new(),
            slots: (longest.milliseconds() / SLOT_MS) as usize,
            sampled: None,
            last_success: None,
        }
    }

    /// Monotonic receipt time; model attribution is deliberately best effort.
    pub fn sample(&mut self, input: &Input, now: u64) {
        self.last_success = Some(now);
        self.retain(input);
        for (id, resume) in &input.resumes {
            let state = self.members.entry(id.clone()).or_default();
            state.model = nonempty(&resume["model"]);
            state.reporter |= resume["consumption"].is_object();
            let Some(next) = Counter::read(resume) else {
                state.blocked = true;
                state.interval(self.sampled, now, false, state.reporter, 0, self.slots);
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
                state.interval(self.sampled, now, false, true, 0, self.slots);
                continue;
            }
            if !next.usable {
                state.previous = Some(next);
                state.blocked = true;
                state.interval(self.sampled, now, false, true, 0, self.slots);
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
                    state.previous.is_some() || self.sampled.is_some_and(|at| at < now),
                    0,
                ),
            };
            state.began.get_or_insert(now);
            state.previous = Some(next);
            state.blocked = false;
            state.interval(self.sampled, now, evidence, gap, tokens, self.slots);
        }
        self.sampled = Some(now);
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
            member.interval(self.sampled, now, false, gap, 0, self.slots);
        }
        self.sampled = Some(now);
    }

    pub fn member(&self, id: &str, now: u64, window: TokenWindow) -> Option<Reading> {
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
            if let Some(reading) = member.reading(now, window) {
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

    pub fn reporting(&self) -> bool {
        self.members.values().any(|member| member.reporter)
    }

    /// Eight bucket-aligned totals, from the same UUID rings as member readings.
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
                    tokens += bucket.tokens;
                    evidence |= bucket.evidence;
                }
            }
            evidence.then_some(tokens as f64)
        })
    }
}

#[cfg(test)]
pub(super) mod tests;
