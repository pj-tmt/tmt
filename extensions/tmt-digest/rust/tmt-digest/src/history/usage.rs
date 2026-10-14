//! Public closed usage buckets become minute snapshots, never cumulative deltas.

use super::{MAX, Store, invalid};
use crate::core::{Core, Error};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};

const STEP: u64 = 5_000;
pub(super) const MINUTE: u64 = 60_000;
const LOOKBACK: u64 = 600_000;

pub(super) struct UsageMinute {
    pub start: u64,
    pub input: Option<u64>,
}
pub(super) struct UsagePage {
    pub observed: u64,
    pub through: u64,
    pub model: Option<(String, u64)>,
    pub minutes: Vec<UsageMinute>,
}

fn number(value: &Value, key: &str) -> Result<u64, Error> {
    value[key]
        .as_u64()
        .filter(|n| *n <= MAX)
        .ok_or_else(|| invalid(format!("Invalid usage {key}")))
}
fn flag(value: &Value, key: &str) -> Result<bool, Error> {
    value[key]
        .as_bool()
        .ok_or_else(|| invalid(format!("Invalid usage {key}")))
}

impl UsagePage {
    pub fn parse(document: &Value, id: &str) -> Result<Self, Error> {
        let observed = number(document, "asOfMs")?;
        let through = number(document, "throughMs")?;
        if number(document, "resolutionMs")? != STEP
            || through % STEP != 0
            || observed < through
            || observed - through >= STEP
            || number(document, "retainedFromMs")? != through.saturating_sub(7_200_000)
        {
            return Err(invalid("Invalid closed usage watermark"));
        }
        let rows = document["identities"]
            .as_array()
            .filter(|rows| rows.len() == 1 && rows[0]["id"] == id)
            .ok_or_else(|| invalid("Mismatched usage identity"))?;
        let row = &rows[0];
        if !flag(row, "found")? {
            return Ok(Self {
                observed,
                through,
                model: None,
                minutes: vec![],
            });
        }
        let reporting = flag(row, "reporting")?;
        let consumption = &row["latest"]["consumption"];
        // Invalid/missing model provenance removes the quote, not valid usage.
        let model = consumption["modelId"]
            .as_str()
            .filter(|s| reporting && !s.is_empty() && s.len() <= 128 && s.is_ascii())
            .zip(
                number(consumption, "observedAtMs")
                    .ok()
                    .filter(|time| *time <= through),
            )
            .map(|(model, time)| (model.to_owned(), time));
        let windows = row["windows"]
            .as_array()
            .filter(|windows| windows.len() == 1)
            .ok_or_else(|| invalid("Invalid usage window"))?;
        let window = &windows[0];
        let from = through.saturating_sub(LOOKBACK);
        if number(window, "windowMs")? != LOOKBACK
            || number(window, "fromMs")? != from
            || number(window, "toMs")? != through
            || number(window, "bucketMs")? != STEP
        {
            return Err(invalid("Invalid base usage window"));
        }
        let buckets = window["buckets"]
            .as_array()
            .filter(|buckets| buckets.len() as u64 == (through - from) / STEP)
            .ok_or_else(|| invalid("Incomplete usage bucket inventory"))?;
        let mut groups: BTreeMap<u64, (u64, Option<u64>)> = BTreeMap::new();
        let mut next = from;
        for bucket in buckets {
            if number(bucket, "fromMs")? != next || number(bucket, "toMs")? != next + STEP {
                return Err(invalid("Noncontiguous closed usage buckets"));
            }
            let input = number(bucket, "inputTokens")?;
            let covered = number(bucket, "coveredMs")?;
            if covered > STEP {
                return Err(invalid("Invalid usage coverage duration"));
            }
            let complete = flag(bucket, "complete")?;
            let gap = flag(bucket, "gap")?;
            let discontinuous = flag(bucket, "discontinuous")?;
            if complete && (covered != STEP || gap) {
                return Err(invalid("Contradictory usage coverage"));
            }
            let (count, total) = groups.entry(next / MINUTE * MINUTE).or_insert((0, Some(0)));
            *count += 1;
            *total = total
                .filter(|_| reporting && complete && covered == STEP && !gap && !discontinuous)
                .and_then(|sum| sum.checked_add(input))
                .filter(|sum| *sum <= MAX);
            next += STEP;
        }
        let minutes = groups
            .into_iter()
            .filter(|(_, (count, _))| *count == MINUTE / STEP)
            .map(|(start, (_, input))| UsageMinute { start, input })
            .collect();
        Ok(Self {
            observed,
            through,
            model,
            minutes,
        })
    }
}

pub fn collect(core: &Core, store: &mut Store, id: &str, deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline || store.cursor(id)?.is_none() {
        return Ok(());
    }
    // Ten minutes at 120 buckets yields exact 5-second inputs. A larger window
    // would aggregate shifted edge buckets; it cannot be prorated into minutes.
    let document = core.api(
        "consumption.history",
        json!({"identityIds":[id],"windowsMs":[LOOKBACK],"maxBuckets":120}),
    )?;
    store.record_usage(id, &UsagePage::parse(&document, id)?)
}
