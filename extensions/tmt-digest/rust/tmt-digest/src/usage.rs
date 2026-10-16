//! `tmt digest usage report`: Core's public consumption, limit and exchange
//! documents joined once for people and spreadsheets. No store, scheduler or
//! collector: every run reads Core and prints.

mod csv;
mod model;
#[cfg(test)]
mod tests;

use crate::{
    core::{Core, Error},
    status::inventory,
};
use model::{Label, Report, Usage};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const DAY_MS: u64 = 86_400_000;
// 16 seats x 30 daily buckets with attribution and limits stay far inside the
// 1 MiB a single Core call may return.
const BATCH: usize = 16;
const RESOLVE_BATCH: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Since {
    Day,
    Week,
    Month,
}
impl Since {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "24h" => Ok(Self::Day),
            "7d" => Ok(Self::Week),
            "30d" => Ok(Self::Month),
            _ => Err("Use 24h, 7d or 30d".into()),
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
        }
    }
    fn window_ms(self) -> u64 {
        match self {
            Self::Day => DAY_MS,
            Self::Week => 7 * DAY_MS,
            Self::Month => 30 * DAY_MS,
        }
    }
    /// Hourly for a day, daily for longer windows.
    fn buckets(self) -> u64 {
        match self {
            Self::Day => 24,
            Self::Week => 7,
            Self::Month => 30,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Usage,
    Exchange,
}
impl Table {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "usage" => Ok(Self::Usage),
            "exchange" => Ok(Self::Exchange),
            _ => Err("Use usage or exchange".into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Json,
    Csv(Table),
}

/// The bytes to print: one JSON document or one CSV table.
pub fn render(since: Since, format: Format) -> Result<String, Error> {
    let core = Core::discover()?.until(Instant::now() + Duration::from_secs(60));
    let document = execute(&core, since, crate::tick::now_ms()?)?;
    match format {
        Format::Json => serde_json::to_string(&document)
            .map(|text| text + "\n")
            .map_err(|error| Error::new("DIGEST_OUTPUT_INVALID", error.to_string())),
        Format::Csv(table) => csv::table(&document, table),
    }
}

fn execute(core: &Core, since: Since, now: u64) -> Result<Value, Error> {
    model::document(&collect(core, since, now)?)
}

fn warning(section: &str, error: &Error) -> Value {
    json!({"section": section, "code": error.code, "message": error.message})
}

/// Only the roster is required; every other section degrades to unavailable
/// with a warning rather than to zero.
fn collect(core: &Core, since: Since, now: u64) -> Result<Report, Error> {
    let roster = inventory(&core.json(&["identity", "ls"])?)?;
    let mut labels: BTreeMap<String, Label> = roster
        .into_iter()
        .map(|(id, name)| {
            (
                id.clone(),
                Label {
                    id,
                    name: Some(name),
                    retired: false,
                },
            )
        })
        .collect();
    let mut warnings = Vec::new();

    let flow = match core.json(&[
        "x",
        "flow",
        "--since",
        since.text(),
        "--bucket",
        since.text(),
    ]) {
        Ok(flow) => Some(flow),
        Err(error) => {
            warnings.push(warning("exchange", &error));
            None
        }
    };
    if let Some(flow) = &flow {
        let outsiders: Vec<String> = model::flow_participants(flow)
            .into_iter()
            .filter(|id| !labels.contains_key(id))
            .collect();
        for batch in outsiders.chunks(RESOLVE_BATCH) {
            match resolve(core, batch) {
                Ok(found) => labels.extend(found),
                Err(error) => warnings.push(warning("labels", &error)),
            }
        }
    }
    let exchange = flow.and_then(|flow| match model::exchange(&flow, &labels) {
        Ok(exchange) => Some(exchange),
        Err(error) => {
            warnings.push(warning("exchange", &error));
            None
        }
    });

    // Retired participants keep their retained rollup, so they are read too.
    let mut seats: Vec<&Label> = labels
        .values()
        .filter(|label| label.name.is_some() || label.retired)
        .collect();
    seats.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    let ids: Vec<String> = seats.into_iter().map(|label| label.id.clone()).collect();
    let mut usage = Usage::default();
    for batch in ids.chunks(BATCH) {
        usage.merge(match read(core, since, batch, &labels) {
            Ok(usage) => usage,
            Err(error) => {
                warnings.push(warning("usage", &error));
                Usage::unreadable(batch, &labels)
            }
        });
    }
    Ok(Report {
        as_of_ms: now,
        since: since.text(),
        usage,
        exchange,
        warnings,
    })
}

/// Names and retirement for flow participants that are not on the roster.
fn resolve(core: &Core, ids: &[String]) -> Result<BTreeMap<String, Label>, Error> {
    let document = core.api("references.resolve", json!({"identityIds": ids}))?;
    let rows = document["identities"]
        .as_array()
        .filter(|rows| rows.len() == ids.len())
        .ok_or_else(|| {
            Error::new(
                "CORE_RESPONSE_INVALID",
                "Core returned an invalid reference lookup",
            )
        })?;
    Ok(rows
        .iter()
        .zip(ids)
        .map(|(row, id)| {
            let found = row["found"] == true;
            (
                id.clone(),
                Label {
                    id: id.clone(),
                    name: found
                        .then(|| row["name"].as_str().map(str::to_owned))
                        .flatten(),
                    retired: found && row["retired"] == true,
                },
            )
        })
        .collect())
}

/// Two reads of the same window: one bucket for totals with attribution, and
/// the series. Core aggregates both, so no total here is a sum of buckets.
fn read(
    core: &Core,
    since: Since,
    ids: &[String],
    labels: &BTreeMap<String, Label>,
) -> Result<Usage, Error> {
    let history = |max_buckets: u64| {
        core.api(
            "consumption.history",
            json!({
                "identityIds": ids,
                "windowsMs": [since.window_ms()],
                "maxBuckets": max_buckets,
                "includeRetired": true,
            }),
        )
    };
    model::usage(&history(1)?, &history(since.buckets())?, ids, labels)
}
