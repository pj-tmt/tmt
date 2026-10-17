//! Interval totals over the existing UUID rings, never a second history store.
use super::{BucketOrigin, Member, Rate, SLOT_MS, history::rollup};
use crate::config::TokenWindow;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::board) struct Grid {
    each_ms: u64,
    range_ms: u64,
    through_ms: i64,
}

impl Grid {
    pub fn new(each_ms: u64, range_ms: u64, now_ms: u64) -> Result<Self, &'static str> {
        if each_ms < 300_000
            || !each_ms.is_multiple_of(60_000)
            || each_ms > range_ms
            || !range_ms.is_multiple_of(each_ms)
            || (range_ms > 3_600_000 && rollup::Request::new(each_ms, range_ms).is_none())
        {
            return Err("The interval grid must use bounded whole source intervals");
        }
        let resolution = if range_ms > 3_600_000 {
            rollup::RESOLUTION_MS
        } else {
            SLOT_MS
        };
        let through_ms = i64::try_from(now_ms / resolution * resolution)
            .map_err(|_| "The observation time is outside the supported range")?;
        Ok(Self {
            each_ms,
            range_ms,
            through_ms,
        })
    }

    fn start(self) -> i64 {
        self.through_ms - self.range_ms as i64
    }
    fn count(self) -> usize {
        (self.range_ms / self.each_ms) as usize
    }
    fn retained(self, member: &Member) -> Option<&super::Retained> {
        member.retained.as_ref().filter(|retained| {
            retained.request.each_ms() == self.each_ms
                && retained.request.range_ms() == self.range_ms
                && i64::try_from(retained.through).ok() == Some(self.through_ms)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Interval {
    pub from_ms: i64,
    pub to_ms: i64,
    pub tokens: Option<u128>,
    /// Verified provider portions; the remainder of tokens is unknown.
    pub providers: BTreeMap<String, u128>,
    pub identities: usize,
    pub observed_identities: usize,
    pub complete_identities: usize,
    /// Sum of reported source durations: identity-milliseconds, not a claim
    /// about the joint elapsed duration covered by different identities.
    pub source_identity_ms: u128,
    pub duration_identities: usize,
    pub boundary_cut: bool,
    pub overflow: bool,
    pub scope_complete: bool,
}

impl Interval {
    pub fn complete(&self) -> bool {
        self.scope_complete
            && self.identities > 0
            && self.complete_identities == self.identities
            && !self.overflow
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Series {
    pub drivers: BTreeSet<String>,
    pub grid: Grid,
    pub buckets: Vec<Interval>,
}

#[derive(Clone, Default)]
struct Evidence {
    tokens: u128,
    providers: BTreeMap<u16, u128>,
    known: bool,
    covered_ms: u64,
    source_duration: bool,
    gap: bool,
    cut: bool,
    overflow: bool,
}

fn intervals(member: &Member, grid: Grid) -> Vec<Evidence> {
    let mut values = vec![Evidence::default(); grid.count()];
    let selected = if grid.range_ms > 3_600_000 {
        let Some(retained) = grid.retained(member) else {
            return values;
        };
        &retained.buckets
    } else {
        &member.buckets
    };
    for bucket in selected {
        if matches!(bucket.origin, BucketOrigin::History { .. }) && bucket.source_from.is_none() {
            // A rollup owns its whole token count and duration at its final slot;
            // intervening ring padding is not an independent measured zero.
            continue;
        }
        let Some(slot) = bucket.slot else { continue };
        let Some(to) = slot
            .checked_add(1)
            .and_then(|slot| slot.checked_mul(SLOT_MS))
        else {
            continue;
        };
        let from = bucket.source_from.unwrap_or(to - SLOT_MS);
        let (Ok(from), Ok(to)) = (i64::try_from(from), i64::try_from(to)) else {
            continue;
        };
        if to <= from || to <= grid.start() || from >= grid.through_ms {
            continue;
        }
        let first = ((from.max(grid.start()) - grid.start()) as u64 / grid.each_ms) as usize;
        let last = (((to.min(grid.through_ms) - 1) - grid.start()) as u64 / grid.each_ms) as usize;
        if from < grid.start() || to > grid.through_ms || first != last {
            for value in &mut values[first..=last] {
                value.cut = true;
            }
            // The source provides no fractional token attribution across this
            // boundary. Neither tokens nor coverage may be prorated into it.
            continue;
        }
        let value = &mut values[first];
        value.gap |= bucket.gap;
        value.known |= bucket.evidence || bucket.tokens > 0;
        match value.tokens.checked_add(bucket.tokens) {
            Some(total) => value.tokens = total,
            None => value.overflow = true,
        }
        for (id, part) in member.provider_tokens(bucket) {
            let sum = value.providers.entry(id).or_default();
            match sum.checked_add(part) {
                Some(total) => *sum = total,
                None => value.overflow = true,
            }
        }
        if let BucketOrigin::History {
            covered_ms: Some(covered_ms),
        } = bucket.origin
        {
            value.source_duration = true;
            value.known |= covered_ms > 0;
            match value.covered_ms.checked_add(u64::from(covered_ms)) {
                Some(total) => value.covered_ms = total,
                None => value.overflow = true,
            }
        } else if matches!(bucket.origin, BucketOrigin::History { covered_ms: None }) {
            value.gap = true;
        }
    }
    values
}

fn rank(member: &Member, grid: Grid) -> (bool, u64, u64) {
    if grid.range_ms > 3_600_000 {
        let Some(retained) = grid.retained(member) else {
            return (false, 0, 0);
        };
        let complete=retained.buckets.len()==grid.count() && retained.buckets.iter().all(|bucket| {
            !bucket.gap && matches!(bucket.origin, BucketOrigin::History {covered_ms:Some(covered)} if u64::from(covered)==grid.each_ms)
        });
        let covered = retained
            .buckets
            .iter()
            .filter_map(|bucket| match bucket.origin {
                BucketOrigin::History {
                    covered_ms: Some(covered),
                } => Some(u64::from(covered)),
                _ => None,
            })
            .sum();
        let observed = retained
            .buckets
            .iter()
            .filter(|bucket| bucket.evidence || bucket.tokens > 0)
            .count() as u64;
        return (complete, covered, observed);
    }
    let window =
        TokenWindow::parse(&format!("{}m", grid.range_ms / 60_000)).expect("bounded minute grid");
    member
        .reading(grid.through_ms as u64, window)
        .map_or((false, 0, 0), |(reading, slots)| {
            (!reading.partial, slots, reading.span)
        })
}

/// Stable caller order breaks equal-evidence duplicate UUIDs, as in the existing
/// header. Pick one whole observation per UUID, never add overlapping copies.
pub(in crate::board) fn project<'a>(
    rates: impl IntoIterator<Item = &'a Rate>,
    grid: Grid,
    expected: Option<&BTreeSet<&str>>,
) -> Series {
    let mut members: BTreeMap<&str, &Member> = BTreeMap::new();
    for rate in rates {
        for (id, candidate) in &rate.members {
            if expected.is_some_and(|ids| !ids.contains(id.as_str())) {
                continue;
            }
            let replace = members
                .get(id.as_str())
                .is_none_or(|old| rank(candidate, grid) > rank(old, grid));
            if replace {
                members.insert(id, candidate);
            }
        }
    }
    let mut buckets: Vec<_> = (0..grid.count())
        .map(|index| {
            let from_ms = grid.start() + index as i64 * grid.each_ms as i64;
            Interval {
                from_ms,
                to_ms: from_ms + grid.each_ms as i64,
                tokens: None,
                providers: BTreeMap::new(),
                identities: expected.map_or(members.len(), BTreeSet::len),
                observed_identities: 0,
                complete_identities: 0,
                source_identity_ms: 0,
                duration_identities: 0,
                boundary_cut: false,
                overflow: false,
                scope_complete: true,
            }
        })
        .collect();
    for member in members.values() {
        for (bucket, evidence) in buckets.iter_mut().zip(intervals(member, grid)) {
            bucket.boundary_cut |= evidence.cut;
            bucket.overflow |= evidence.overflow;
            if evidence.source_duration {
                bucket.duration_identities += 1;
                bucket.source_identity_ms += u128::from(evidence.covered_ms);
            }
            bucket.complete_identities += usize::from(
                evidence.source_duration
                    && evidence.covered_ms == grid.each_ms
                    && !evidence.gap
                    && !evidence.cut
                    && !evidence.overflow,
            );
            if evidence.known {
                bucket.observed_identities += 1;
                for (id, part) in evidence.providers {
                    let Some(driver) = member.drivers.get(usize::from(id)) else {
                        continue;
                    };
                    let sum = bucket.providers.entry(driver.clone()).or_default();
                    match sum.checked_add(part) {
                        Some(total) => *sum = total,
                        None => bucket.overflow = true,
                    }
                }
                match bucket.tokens.unwrap_or(0).checked_add(evidence.tokens) {
                    Some(total) => bucket.tokens = Some(total),
                    None => bucket.overflow = true,
                }
            }
        }
    }
    for bucket in &mut buckets {
        if bucket.overflow {
            bucket.tokens = None;
            bucket.providers.clear();
        }
    }
    let mut drivers: BTreeSet<_> = buckets
        .iter()
        .flat_map(|bucket| bucket.providers.keys().cloned())
        .collect();
    drivers.extend(members.values().flat_map(|member| {
        member
            .current_driver
            .and_then(|id| member.drivers.get(usize::from(id)))
            .cloned()
    }));
    Series {
        drivers,
        grid,
        buckets,
    }
}

#[cfg(test)]
mod tests;
