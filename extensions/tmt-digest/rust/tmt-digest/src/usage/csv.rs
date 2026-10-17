//! CSV views of the report document: each row is derived from the JSON rows, so
//! the two cannot disagree.

use super::{Table, model::line_of};
use crate::core::Error;
use serde_json::Value;

const USAGE: &[&str] = &[
    "id",
    "name",
    "retired",
    "line",
    "model",
    "input",
    "output",
    "cached",
    "coveredMs",
    "windowMs",
    "startsAtMs",
    "complete",
    "unavailable",
];
const CREDITS: &[&str] = &[
    "fromMs",
    "toMs",
    "balance",
    "observedAtMs",
    "hasCredits",
    "unlimited",
    "serviceTiers",
];
const EXCHANGE: &[&str] = &[
    "senderId",
    "sender",
    "recipientId",
    "recipient",
    "kind",
    "count",
    "bytes",
];

/// RFC 4180 quoting with LF row ends. A text cell a spreadsheet could read as a
/// formula (leading `=`, `+`, `-`, `@`, tab or carriage return) gets a leading `'`.
pub fn cell(text: &str) -> String {
    let guarded = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => cell(text),
        other => other.to_string(),
    }
}

fn row(cells: &[Value]) -> String {
    cells.iter().map(text).collect::<Vec<_>>().join(",") + "\n"
}

pub fn table(document: &Value, table: Table) -> Result<String, Error> {
    let (header, rows) = match table {
        Table::Usage => (USAGE, usage(document)),
        Table::Credits => (CREDITS, credits(document)),
        Table::Exchange => (EXCHANGE, exchange(document)?),
    };
    Ok(header.join(",") + "\n" + &rows.concat())
}

fn usage(document: &Value) -> Vec<String> {
    let empty = Vec::new();
    let usage = &document["usage"];
    let mut rows = Vec::new();
    for seat in usage["seats"].as_array().unwrap_or(&empty) {
        let coverage = &seat["coverage"];
        let tail = |totals: &Value| {
            [
                totals["input"].clone(),
                totals["output"].clone(),
                totals["cached"].clone(),
                coverage["coveredMs"].clone(),
                coverage["windowMs"].clone(),
                coverage["startsAtMs"].clone(),
                coverage["complete"].clone(),
                Value::Null,
            ]
        };
        let head = |line: &str, model: &Value| {
            [
                seat["id"].clone(),
                seat["name"].clone(),
                seat["retired"].clone(),
                Value::String(line.to_owned()),
                model.clone(),
            ]
        };
        match seat["byModel"]
            .as_array()
            .filter(|models| !models.is_empty())
        {
            Some(models) => {
                for model in models {
                    let line = line_of(model["modelId"].as_str().unwrap_or_default());
                    let cells = [&head(line, &model["modelId"])[..], &tail(model)[..]].concat();
                    rows.push(row(&cells));
                }
            }
            None => {
                let line = if seat["byModel"].is_null() {
                    "unattributed"
                } else {
                    ""
                };
                let cells = [&head(line, &Value::Null)[..], &tail(&seat["totals"])[..]].concat();
                rows.push(row(&cells));
            }
        }
    }
    for seat in usage["unavailable"].as_array().unwrap_or(&empty) {
        let mut cells = vec![
            seat["id"].clone(),
            seat["name"].clone(),
            seat["retired"].clone(),
        ];
        cells.extend(std::iter::repeat_n(Value::Null, 9));
        cells.push(seat["reason"].clone());
        rows.push(row(&cells));
    }
    rows
}

/// One row per point of the balance series; empty when no seat observed credits.
/// `serviceTiers` is the point's tier set joined by spaces (empty: unknown).
fn credits(document: &Value) -> Vec<String> {
    let empty = Vec::new();
    document["credits"]["points"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .map(|point| {
            let tiers = point["serviceTiers"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            row(&[
                point["fromMs"].clone(),
                point["toMs"].clone(),
                point["balance"].clone(),
                point["observedAtMs"].clone(),
                point["hasCredits"].clone(),
                point["unlimited"].clone(),
                Value::String(tiers),
            ])
        })
        .collect()
}

fn exchange(document: &Value) -> Result<Vec<String>, Error> {
    let pairs = document["exchange"]["pairs"].as_array().ok_or_else(|| {
        let reason = document["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|warning| warning["section"] == "exchange")
            .and_then(|warning| warning["message"].as_str())
            .unwrap_or("Core returned no exchange flow");
        Error::new("EXCHANGE_UNAVAILABLE", reason)
    })?;
    Ok(pairs
        .iter()
        .map(|pair| {
            row(&[
                pair["sender"]["id"].clone(),
                pair["sender"]["name"].clone(),
                pair["recipient"]["id"].clone(),
                pair["recipient"]["name"].clone(),
                pair["kind"].clone(),
                pair["count"].clone(),
                pair["bytes"].clone(),
            ])
        })
        .collect())
}
