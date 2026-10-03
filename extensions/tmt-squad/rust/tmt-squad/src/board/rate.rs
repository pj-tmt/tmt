//! Public cumulative counters become receipt-time completed-request buckets.
//! No provider files, core crate, context-window subtraction or in-flight estimate.
use crate::config::TokenWindow;
use serde_json::Value;
use std::collections::BTreeMap;

const SAFE: u64 = 9_007_199_254_740_991;
const SLOT_MS: u64 = 5_000;
const SLOTS: usize = 720;

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
        let rows: BTreeMap<_, _> = listed["identities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| Some((row["id"].as_str()?, row["resume"].clone())))
            .collect();
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
}

/// The shared ring describes sampled batches, not generation-time intervals.
#[derive(Debug, Clone, Copy, Default)]
struct Bucket {
    slot: Option<u64>,
    tokens: u128,
    evidence: bool,
    gap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub rate: f64,
    pub partial: bool,
    pub span: u64,
}

#[derive(Debug)]
pub struct Rate {
    members: BTreeMap<String, Member>,
    buckets: Box<[Bucket; SLOTS]>,
    began: Option<u64>,
    sampled: Option<u64>,
    last_success: Option<u64>,
}

impl Default for Rate {
    fn default() -> Self {
        Self {
            members: BTreeMap::new(),
            buckets: Box::new([Bucket::default(); SLOTS]),
            began: None,
            sampled: None,
            last_success: None,
        }
    }
}

impl Rate {
    fn bucket(&mut self, slot: u64) -> &mut Bucket {
        let bucket = &mut self.buckets[(slot % SLOTS as u64) as usize];
        if bucket.slot != Some(slot) {
            *bucket = Bucket {
                slot: Some(slot),
                ..Default::default()
            };
        }
        bucket
    }

    fn interval(&mut self, now: u64, evidence: bool, gap: bool, tokens: u128) {
        let end = now / SLOT_MS;
        let start = self
            .sampled
            .map_or(end, |at| at / SLOT_MS + 1)
            .max(end.saturating_sub(SLOTS as u64 - 1));
        for slot in start..=end {
            let bucket = self.bucket(slot);
            bucket.evidence |= evidence;
            bucket.gap |= gap;
        }
        // Several accepted observations in one slot coalesce without extra storage.
        if evidence || gap || tokens > 0 {
            let bucket = self.bucket(end);
            bucket.evidence |= evidence;
            bucket.gap |= gap;
            bucket.tokens += tokens;
        }
        self.sampled = Some(now);
    }

    /// Time is monotonic receipt time. Never-reporting drivers are excluded.
    pub fn sample(&mut self, input: &Input, now: u64) {
        self.last_success = Some(now);
        self.members.retain(|id, _| input.resumes.contains_key(id));
        let mut delta = 0_u128;
        let mut evidence = false;
        let mut gap = false;
        for (id, resume) in &input.resumes {
            let state = self.members.entry(id.clone()).or_default();
            state.reporter |= resume["consumption"].is_object();
            let Some(next) = Counter::read(resume) else {
                gap |= state.reporter;
                state.blocked = true;
                continue;
            };
            // Presence of a consumption object opts this member into coverage.
            state.reporter = true;
            if let Some(previous) = &state.previous
                && previous.key == next.key
                && (next.sequence < previous.sequence
                    || next.observed < previous.observed
                    || (next.sequence == previous.sequence && next != *previous))
            {
                state.blocked = true;
                gap = true;
                continue;
            }
            if !next.usable {
                state.previous = Some(next);
                state.blocked = true;
                gap = true;
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
            if let Some(previous) = continuous {
                evidence = true;
                delta += u128::from(next.input - previous.input)
                    + u128::from(next.output - previous.output);
            } else {
                gap |= state.previous.is_some() || self.began.is_some_and(|at| at < now);
            }
            self.began.get_or_insert(now);
            state.previous = Some(next);
            state.blocked = false;
        }
        self.interval(now, evidence, gap, delta);
    }

    /// A failed query cannot bridge recovery; its unknown intervals age out.
    pub fn failed(&mut self, now: u64, every_ms: u64) {
        let gap = self.members.values().any(|member| member.reporter)
            && self
                .last_success
                .is_some_and(|at| now.saturating_sub(at) >= every_ms * 2);
        for member in self.members.values_mut() {
            member.blocked = true;
        }
        self.interval(now, false, gap, 0);
    }

    pub fn reading(&self, now: u64, window: TokenWindow) -> Option<Reading> {
        if !self.members.values().any(|member| member.reporter) {
            return None;
        }
        let span = now.saturating_sub(self.began?).min(window.milliseconds());
        if span
            < if window == TokenWindow::Five {
                SLOT_MS
            } else {
                10_000
            }
        {
            return None;
        }
        let end = now / SLOT_MS;
        let count = window.milliseconds() / SLOT_MS;
        let buckets = || {
            self.buckets.iter().filter(|bucket| {
                bucket
                    .slot
                    .is_some_and(|slot| slot <= end && end - slot < count)
            })
        };
        if !buckets().any(|bucket| bucket.evidence) {
            return None;
        }
        Some(Reading {
            rate: buckets().map(|bucket| bucket.tokens).sum::<u128>() as f64 * 1000.0 / span as f64,
            partial: buckets().any(|bucket| bucket.gap),
            span,
        })
    }

    pub fn reporter(&self, id: &str) -> bool {
        self.members.get(id).is_some_and(|member| member.reporter)
    }

    /// Eight bucket-aligned trend slices. None differs from measured zero.
    pub fn trend(&self, now: u64, window: TokenWindow) -> [Option<f64>; 8] {
        let bar_slots = (window.milliseconds() / SLOT_MS).div_ceil(8);
        let end = now / SLOT_MS;
        std::array::from_fn(|index| {
            let behind = (7 - index) as u64 * bar_slots;
            let last = end.checked_sub(behind)?;
            let first = last.saturating_sub(bar_slots - 1);
            let buckets = || {
                self.buckets.iter().filter(|bucket| {
                    bucket
                        .slot
                        .is_some_and(|slot| (first..=last).contains(&slot))
                })
            };
            buckets().any(|bucket| bucket.evidence).then(|| {
                buckets().map(|bucket| bucket.tokens).sum::<u128>() as f64 / (bar_slots * 5) as f64
            })
        })
    }
}

#[cfg(test)]
pub(super) mod tests;
