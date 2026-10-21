//! Public Core observations for the shared TOK window. The admitted roster is
//! passed in; neither a provider scan nor a second roster read belongs here.
use super::model::{HourlySeries, Pair, Report, Seat, Since, Traffic, Unavailable, Warning, Who};
use crate::{config::TokRange, core::Core};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub range: TokRange,
    pub identities: std::sync::Arc<[String]>,
}
impl Request {
    /// Identities come from the acquired roster's unique, ordered UUID keys.
    pub fn new(range: TokRange, identities: impl Into<std::sync::Arc<[String]>>) -> Self {
        Self {
            range,
            identities: identities.into(),
        }
    }
    pub fn since(&self) -> Since {
        self.range
    }
}
fn number(value: &Value) -> Result<u64, String> {
    value
        .as_u64()
        .filter(|n| *n <= 9_007_199_254_740_991)
        .ok_or_else(|| "Malformed consumption observation.".into())
}
fn who(id: &str) -> Who {
    Who {
        id: Some(id.into()),
        name: None,
        retired: false,
    }
}
fn window<'a>(
    document: &'a Value,
    id: &str,
    duration: u64,
) -> Result<(&'a Value, &'a Value), String> {
    let observed = number(&document["asOfMs"])?;
    let through = number(&document["throughMs"])?;
    let rows = document["identities"]
        .as_array()
        .ok_or("Malformed consumption identities.")?;
    let mut rows = rows.iter().filter(|row| row["id"] == id);
    let row = rows.next().ok_or("Missing consumption identity.")?;
    if rows.next().is_some() || row["found"] != true {
        return Err("Unavailable consumption identity.".into());
    }
    let mut windows = row["windows"]
        .as_array()
        .ok_or("Missing consumption windows.")?
        .iter()
        .filter(|w| w["windowMs"] == duration);
    let window = windows.next().ok_or("Missing consumption window.")?;
    if windows.next().is_some() {
        return Err("Duplicate consumption window.".into());
    }
    let from = number(&window["fromMs"])?;
    let to = number(&window["toMs"])?;
    let resolution = if duration <= 3_600_000 {
        5_000
    } else {
        300_000
    };
    if through > observed
        || to != through / resolution * resolution
        || to.checked_sub(from) != Some(duration)
    {
        return Err("Mismatched consumption window.".into());
    }
    Ok((row, window))
}
fn seat(
    totals: &Value,
    series: Result<&Value, &String>,
    id: &str,
    range: TokRange,
) -> Result<(Seat, Option<String>), String> {
    let (identity, total) = window(totals, id, range.milliseconds())?;
    let available = if range == TokRange::Hour {
        &identity["availableFromMs"]
    } else {
        &total["availableFromMs"]
    };
    let available = number(available).map_err(|_| "No token observations.".to_string())?;
    let buckets = total["buckets"].as_array().ok_or("Missing token totals.")?;
    let [bucket] = buckets.as_slice() else {
        return Err("Token totals must be one Core aggregate.".into());
    };
    if available > number(&total["toMs"])?
        || bucket["fromMs"] != total["fromMs"]
        || bucket["toMs"] != total["toMs"]
    {
        return Err("Mismatched token aggregate interval.".into());
    }
    let (points, warning) = match series
        .map_err(Clone::clone)
        .and_then(|series| hourly(series, id))
    {
        Ok(points) => (points, None),
        Err(message) => (HourlySeries::default(), Some(message)),
    };
    let covered_ms = number(&bucket["coveredMs"])?;
    if covered_ms > range.milliseconds()
        || (bucket["complete"] == true && covered_ms != range.milliseconds())
        || number(&bucket["cachedInputTokens"])? > number(&bucket["inputTokens"])?
    {
        return Err("Invalid token coverage.".into());
    }
    Ok((
        Seat {
            model_inputs: model_inputs(bucket),
            who: who(id),
            input: number(&bucket["inputTokens"])?,
            output: number(&bucket["outputTokens"])?,
            cached: number(&bucket["cachedInputTokens"])?,
            series: points,
            covered_ms,
            window_ms: range.milliseconds(),
            from_ms: number(&total["fromMs"])?,
            to_ms: number(&total["toMs"])?,
            observed_at_ms: number(&totals["asOfMs"])?,
            starts_at_ms: Some(available.max(number(&total["fromMs"])?)),
            complete: bucket["complete"]
                .as_bool()
                .ok_or("Missing token coverage.")?,
        },
        warning,
    ))
}

/// Optional attribution can make a price unknown without erasing valid usage.
/// A partial model split is not extrapolated to the remaining input.
fn model_inputs(bucket: &Value) -> Option<Vec<(String, u64)>> {
    let raw = bucket["byModel"].as_array()?;
    let mut seen = std::collections::BTreeSet::new();
    let mut models = Vec::with_capacity(raw.len());
    let mut input = 0u64;
    let mut output = 0u64;
    let mut cached = 0u64;
    for row in raw {
        let id = row["modelId"].as_str()?;
        if id.is_empty() || !seen.insert(id) {
            return None;
        }
        let i = number(&row["inputTokens"]).ok()?;
        let o = number(&row["outputTokens"]).ok()?;
        let c = number(&row["cachedInputTokens"]).ok()?;
        if c > i {
            return None;
        }
        input = input.checked_add(i)?;
        output = output.checked_add(o)?;
        cached = cached.checked_add(c)?;
        models.push((id.to_string(), i));
    }
    (input == number(&bucket["inputTokens"]).ok()?
        && output <= number(&bucket["outputTokens"]).ok()?
        && cached <= number(&bucket["cachedInputTokens"]).ok()?)
    .then_some(models)
}

fn hourly(series: &Value, id: &str) -> Result<HourlySeries, String> {
    let (_, day) = window(series, id, 86_400_000)?;
    let from = number(&day["fromMs"])?;
    if day["bucketMs"] != 3_600_000 {
        return Err("Activity is not hourly.".into());
    }
    let raw = day["buckets"]
        .as_array()
        .ok_or("Missing hourly observations.")?;
    if raw.len() != 24 {
        return Err("Activity must contain 24 hourly observations.".into());
    }
    let mut tokens = [0; 24];
    let mut complete = [false; 24];
    for (index, point) in raw.iter().enumerate() {
        let expected = from + index as u64 * 3_600_000;
        if point["fromMs"] != expected || point["toMs"] != expected + 3_600_000 {
            return Err("Mismatched hourly interval.".into());
        }
        let covered = number(&point["coveredMs"])?;
        if covered > 3_600_000
            || (point["complete"] == true && covered != 3_600_000)
            || number(&point["cachedInputTokens"])? > number(&point["inputTokens"])?
        {
            return Err("Invalid hourly coverage.".into());
        }
        tokens[index] = number(&point["inputTokens"])? + number(&point["outputTokens"])?;
        complete[index] = point["complete"]
            .as_bool()
            .ok_or("Missing hourly coverage.")?;
    }
    Ok(HourlySeries::new(from, tokens, complete))
}
fn traffic(document: &Value) -> Result<Traffic, String> {
    let since_ms = number(&document["sinceMs"])?;
    let observed_at_ms = number(&document["observedAtMs"])?;
    if observed_at_ms < since_ms {
        return Err("Invalid flow window.".into());
    }
    let mut pairs = Vec::new();
    for row in document["rows"]
        .as_array()
        .ok_or("Malformed exchange flow.")?
    {
        let participant = |key: &str| -> Result<Who, String> {
            match &row[key] {
                Value::Null => Ok(Who {
                    id: None,
                    name: None,
                    retired: false,
                }),
                Value::String(id) => Ok(who(id)),
                _ => Err("Malformed flow participant.".into()),
            }
        };
        let at = number(&row["bucketStartMs"])?;
        if !(since_ms..=observed_at_ms).contains(&at) {
            return Err("Flow bucket outside its window.".into());
        }
        pairs.push(Pair {
            sender: participant("senderIdentityId")?,
            recipient: participant("recipientIdentityId")?,
            kind: row["kind"].as_str().ok_or("Missing flow kind.")?.into(),
            count: number(&row["count"])?,
        });
    }
    Traffic::from_pairs(since_ms, observed_at_ms, pairs)
}

pub(super) fn read(core: &Core, request: &Request, budget: Duration) -> Result<Report, String> {
    let end = Instant::now() + budget;
    let remaining = || end.saturating_duration_since(Instant::now());
    let mut report = Report {
        since: request.since(),
        observed_at_ms: 0,
        window_from_ms: None,
        seats: Vec::new(),
        unavailable: Vec::new(),
        exchange: None,
        warnings: Vec::new(),
    };
    // A pair-sized bucket asks Core to perform window aggregation; no result or
    // acknowledgement count is silently folded into message traffic here.
    match core
        .json_within(
            &[
                "x",
                "flow",
                "--since",
                request.range.argument(),
                "--bucket",
                request.range.argument(),
            ],
            remaining(),
        )
        .map_err(|e| e.message)
        .and_then(|d| traffic(&d))
    {
        Ok(flow) => {
            report.observed_at_ms = flow.observed_at_ms;
            report.exchange = Some(flow);
        }
        Err(message) => report.warnings.push(Warning {
            section: "exchange".into(),
            message,
        }),
    }
    // One refresh carries the first successful history response's closed cut to
    // every subsequent aggregate and activity call; admission times may differ.
    let mut through = None;
    for batch in request.identities.chunks(32) {
        let mut read = |duration, maximum| {
            if remaining().is_zero() {
                return Err("Observation deadline exceeded.".into());
            }
            let mut input =
                json!({"identityIds":batch,"windowsMs":[duration],"maxBuckets":maximum});
            if let Some(cut) = through {
                input["throughMs"] = json!(cut);
            }
            let document = core
                .api_within("consumption.history", input, remaining())
                .map_err(|e| e.message)?;
            let cut = number(&document["throughMs"])?;
            let observed = number(&document["asOfMs"])?;
            if !cut.is_multiple_of(5_000)
                || cut > observed
                || through.is_some_and(|previous| previous != cut)
            {
                return Err("Mismatched consumption cut.".into());
            }
            through = Some(cut);
            Ok(document)
        };
        let totals = read(request.range.milliseconds(), 1);
        let series = read(86_400_000u64, 24);
        if let Ok(totals) = &totals
            && let Some(observed) = totals["asOfMs"].as_u64()
        {
            report.observed_at_ms = report.observed_at_ms.max(observed);
        }
        for id in batch {
            let found = totals
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|totals| seat(totals, series.as_ref(), id, request.range));
            match found {
                Ok((seat, warning)) => {
                    if let Some(message) = warning {
                        report.warnings.push(Warning {
                            section: format!("activity:{id}"),
                            message,
                        });
                    }
                    report.seats.push(seat);
                }
                Err(reason) => report.unavailable.push(Unavailable {
                    who: who(id),
                    reason,
                }),
            }
        }
    }
    // This clock describes the admitted seats, never an arbitrary first batch
    // identity that may be missing or malformed.
    report.window_from_ms = report.seats.first().map(|seat| seat.from_ms);
    if report.exchange.is_none() && report.seats.is_empty() {
        return Err(report
            .warnings
            .first()
            .map_or("No observations available.".into(), |w| w.message.clone()));
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
