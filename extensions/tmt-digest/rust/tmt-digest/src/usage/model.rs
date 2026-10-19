//! The usage and exchange report: pure joins over Core's public documents.
//! Nothing here reads Core, a store or the clock, and missing evidence is
//! carried as unavailable, never as zero.

use crate::core::Error;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const MAX: u64 = 9_007_199_254_740_991;

fn invalid(what: &str) -> Error {
    Error::new(
        "CORE_RESPONSE_INVALID",
        format!("Core returned an invalid {what}"),
    )
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().filter(|n| *n <= MAX)
}

/// An identity as the report names it. A name is absent when Core could not
/// resolve it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub id: String,
    pub name: Option<String>,
    pub retired: bool,
}

impl Label {
    fn json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "retired": self.retired})
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    input: u64,
    output: u64,
    cached: u64,
}

impl Tokens {
    fn read(value: &Value) -> Option<Self> {
        Some(Self {
            input: number(&value["inputTokens"])?,
            output: number(&value["outputTokens"])?,
            cached: number(&value["cachedInputTokens"])?,
        })
    }
    fn add(self, other: Self) -> Option<Self> {
        let sum = Self {
            input: self.input.checked_add(other.input)?,
            output: self.output.checked_add(other.output)?,
            cached: self.cached.checked_add(other.cached)?,
        };
        (sum.input <= MAX && sum.output <= MAX && sum.cached <= MAX).then_some(sum)
    }
    fn sub(self, other: Self) -> Option<Self> {
        Some(Self {
            input: self.input.checked_sub(other.input)?,
            output: self.output.checked_sub(other.output)?,
            cached: self.cached.checked_sub(other.cached)?,
        })
    }
    fn json(self) -> Value {
        json!({"input": self.input, "output": self.output, "cached": self.cached})
    }
    fn is_zero(self) -> bool {
        self == Self::default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelTotals {
    model_id: String,
    tokens: Tokens,
}

#[derive(Clone, Debug, PartialEq)]
struct Point {
    from_ms: u64,
    to_ms: u64,
    tokens: Tokens,
    complete: bool,
    /// The newest credits Core saw in the bucket, and the service tier of its
    /// newest token-moving read; each None when Core observed none.
    credits: Option<Credit>,
    service_tier: Option<String>,
}

/// An account's credit balance as one bucket's newest provider line reported it.
/// `balance` stays the provider's decimal text: it is never summed or rounded.
#[derive(Clone, Debug, PartialEq)]
struct Credit {
    observed_at_ms: u64,
    balance: String,
    has_credits: bool,
    unlimited: bool,
}

/// Rate limits as one seat's newest rollup bucket reported them.
#[derive(Clone, Debug, PartialEq)]
struct Observed {
    observed_at_ms: u64,
    windows: Vec<(u64, f64, u64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    label: Label,
    totals: Tokens,
    /// None when Core attributed none of the total to models; otherwise rows that
    /// sum to at most the total, the rest being unattributed.
    by_model: Option<Vec<ModelTotals>>,
    series: Vec<Point>,
    covered_ms: u64,
    window_ms: u64,
    /// Where the seat's evidence begins inside the window: Core's
    /// `availableFromMs`, but never before the window itself.
    starts_at_ms: u64,
    complete: bool,
    gap: bool,
    limits: Option<Observed>,
}

impl Seat {
    /// The tokens no model row covers: the whole total when there are no rows.
    fn unattributed(&self) -> Result<Tokens, Error> {
        let mut held = Tokens::default();
        for model in self.by_model.iter().flatten() {
            held = held
                .add(model.tokens)
                .ok_or_else(|| invalid("consumption attribution"))?;
        }
        self.totals
            .sub(held)
            .ok_or_else(|| invalid("consumption attribution"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unavailable {
    label: Label,
    reason: &'static str,
    /// Core's reason for a `no_evidence` identity, such as `no_agent`. A value
    /// this build does not know is passed through; None when Core gave none.
    cause: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub from_ms: u64,
    pub to_ms: u64,
}

#[derive(Debug, Default, PartialEq)]
pub struct Usage {
    pub window: Option<Window>,
    pub seats: Vec<Seat>,
    pub unavailable: Vec<Unavailable>,
}

impl Usage {
    pub fn merge(&mut self, other: Usage) {
        self.window = self.window.or(other.window);
        self.seats.extend(other.seats);
        self.unavailable.extend(other.unavailable);
    }

    /// Identities whose read failed: unavailable, never an empty seat.
    pub fn unreadable(ids: &[String], labels: &BTreeMap<String, Label>) -> Self {
        Self {
            window: None,
            seats: Vec::new(),
            unavailable: ids
                .iter()
                .map(|id| Unavailable {
                    label: label_of(id, labels),
                    reason: "read_failed",
                    cause: None,
                })
                .collect(),
        }
    }
}

fn label_of(id: &str, labels: &BTreeMap<String, Label>) -> Label {
    labels.get(id).cloned().unwrap_or_else(|| Label {
        id: id.to_owned(),
        name: None,
        retired: false,
    })
}

fn identity<'a>(document: &'a Value, ids: &[String]) -> Result<&'a Vec<Value>, Error> {
    let rows = document["identities"]
        .as_array()
        .filter(|rows| rows.len() == ids.len())
        .ok_or_else(|| invalid("consumption history"))?;
    rows.iter()
        .zip(ids)
        .all(|(row, id)| row["id"] == *id)
        .then_some(rows)
        .ok_or_else(|| invalid("consumption history"))
}

fn models(bucket: &Value, totals: Tokens) -> Result<Option<Vec<ModelTotals>>, Error> {
    let Some(rows) = bucket.get("byModel") else {
        return Ok(None);
    };
    let rows = rows
        .as_array()
        .ok_or_else(|| invalid("consumption attribution"))?;
    let mut sum = Tokens::default();
    let mut parsed = Vec::with_capacity(rows.len());
    for row in rows {
        let tokens = Tokens::read(row).ok_or_else(|| invalid("consumption attribution"))?;
        let model_id = row["modelId"]
            .as_str()
            .ok_or_else(|| invalid("consumption attribution"))?;
        sum = sum
            .add(tokens)
            .ok_or_else(|| invalid("consumption attribution"))?;
        parsed.push(ModelTotals {
            model_id: model_id.to_owned(),
            tokens,
        });
    }
    // Rows may cover only part of the bucket, never more than it.
    if totals.sub(sum).is_none() {
        return Err(invalid("consumption attribution"));
    }
    Ok((!parsed.is_empty()).then_some(parsed))
}

fn credit(value: &Value) -> Result<Option<Credit>, Error> {
    if value.is_null() {
        return Ok(None);
    }
    let read = || {
        let balance = value["balance"].as_str().filter(|text| {
            (1..=28).contains(&text.len())
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
                && text.bytes().filter(|byte| *byte == b'.').count() <= 1
                && !text.starts_with('.')
                && !text.ends_with('.')
        })?;
        Some(Credit {
            observed_at_ms: number(&value["observedAtMs"]).filter(|n| *n > 0)?,
            balance: balance.to_owned(),
            has_credits: value["hasCredits"].as_bool()?,
            unlimited: value["unlimited"].as_bool()?,
        })
    };
    read()
        .map(Some)
        .ok_or_else(|| invalid("consumption series"))
}

/// Core's `cause` for an identity that reports nothing: a plain lowercase id, else
/// none. Unknown ids pass through, because later hosts add causes additively.
fn cause(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| {
            (1..=24).contains(&text.len())
                && text.starts_with(|first: char| first.is_ascii_lowercase())
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        })
        .map(str::to_owned)
}

fn tier(value: &Value) -> Result<Option<String>, Error> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .filter(|text| {
            (1..=24).contains(&text.len())
                && text.starts_with(|first: char| first.is_ascii_lowercase())
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        })
        .map(|text| Some(text.to_owned()))
        .ok_or_else(|| invalid("consumption series"))
}

fn observed(value: &Value) -> Option<Observed> {
    let windows = value["windows"]
        .as_array()?
        .iter()
        .map(|window| {
            Some((
                number(&window["windowMinutes"]).filter(|n| *n > 0)?,
                window["usedPercent"]
                    .as_f64()
                    .filter(|p| (0.0..=100.0).contains(p))?,
                number(&window["resetsAtMs"])?,
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Observed {
        observed_at_ms: number(&value["observedAtMs"])?,
        windows,
    })
}

/// Joins a one-bucket history (totals) with a day-bucket history (series) for
/// `ids`, in request order. An identity with no successful read in the rollup is
/// unavailable, not an empty seat.
pub fn usage(
    totals: &Value,
    series: &Value,
    ids: &[String],
    labels: &BTreeMap<String, Label>,
) -> Result<Usage, Error> {
    let (totals, series) = (identity(totals, ids)?, identity(series, ids)?);
    let mut result = Usage::default();
    for ((total, line), id) in totals.iter().zip(series).zip(ids) {
        let label = label_of(id, labels);
        if total["found"] != true {
            result.unavailable.push(Unavailable {
                label,
                reason: "not_found",
                cause: None,
            });
            continue;
        }
        let window = total["windows"]
            .get(0)
            .ok_or_else(|| invalid("consumption history"))?;
        let span = Window {
            from_ms: number(&window["fromMs"]).ok_or_else(|| invalid("consumption window"))?,
            to_ms: number(&window["toMs"]).ok_or_else(|| invalid("consumption window"))?,
        };
        result.window = result.window.or(Some(span));
        if window["availableFromMs"].is_null() {
            result.unavailable.push(Unavailable {
                label,
                reason: "no_evidence",
                cause: cause(&total["cause"]),
            });
            continue;
        }
        let available =
            number(&window["availableFromMs"]).ok_or_else(|| invalid("consumption window"))?;
        let bucket = match window["buckets"].as_array().map(Vec::as_slice) {
            Some([bucket]) => bucket,
            _ => return Err(invalid("consumption history")),
        };
        let tokens = Tokens::read(bucket).ok_or_else(|| invalid("consumption bucket"))?;
        let mut points = Vec::new();
        let mut limits: Option<Observed> = None;
        for point in line["windows"]
            .get(0)
            .and_then(|window| window["buckets"].as_array())
            .ok_or_else(|| invalid("consumption series"))?
        {
            points.push(Point {
                from_ms: number(&point["fromMs"]).ok_or_else(|| invalid("consumption series"))?,
                to_ms: number(&point["toMs"]).ok_or_else(|| invalid("consumption series"))?,
                tokens: Tokens::read(point).ok_or_else(|| invalid("consumption series"))?,
                complete: point["complete"] == true,
                credits: credit(&point["credits"])?,
                service_tier: tier(&point["serviceTier"])?,
            });
            if let Some(found) = point.get("rateLimits").and_then(observed)
                && limits
                    .as_ref()
                    .is_none_or(|held| found.observed_at_ms >= held.observed_at_ms)
            {
                limits = Some(found);
            }
        }
        result.seats.push(Seat {
            label,
            totals: tokens,
            by_model: models(bucket, tokens)?,
            series: points,
            covered_ms: number(&bucket["coveredMs"])
                .ok_or_else(|| invalid("consumption bucket"))?,
            window_ms: span.to_ms.saturating_sub(span.from_ms),
            starts_at_ms: available.max(span.from_ms),
            complete: bucket["complete"] == true,
            gap: bucket["gap"] == true,
            limits,
        });
    }
    Ok(result)
}

/// Model families by id prefix, for presentation only. The flag says whether
/// Core observes the family's account limits; the first has no source yet (#1724).
const FAMILIES: [(&str, bool); 2] = [("claude-", false), ("gpt-", true)];
const OTHER: &str = "other";

/// The line of a model id: its family (the prefix without the dash), else `other`.
pub fn line_of(model_id: &str) -> &'static str {
    FAMILIES
        .iter()
        .find(|(prefix, _)| model_id.starts_with(prefix))
        .map_or(OTHER, |(prefix, _)| prefix.trim_end_matches('-'))
}

fn has_limit_source(line: &str) -> bool {
    FAMILIES
        .iter()
        .find(|(prefix, _)| prefix.trim_end_matches('-') == line)
        .is_none_or(|(_, observed)| *observed)
}

const UNATTRIBUTED: &str = "unattributed";

#[derive(Default)]
struct Line {
    tokens: Tokens,
    seats: BTreeSet<String>,
    partial: BTreeSet<String>,
}

fn lines(seats: &[Seat]) -> Result<BTreeMap<&'static str, Line>, Error> {
    let mut lines = BTreeMap::<&'static str, Line>::new();
    for seat in seats {
        let mut add = |name: &'static str, tokens: Tokens| -> Result<(), Error> {
            let line = lines.entry(name).or_default();
            line.tokens = line
                .tokens
                .add(tokens)
                .ok_or_else(|| invalid("usage total"))?;
            line.seats.insert(seat.label.id.clone());
            if !seat.complete {
                line.partial.insert(seat.label.id.clone());
            }
            Ok(())
        };
        if let Some(models) = &seat.by_model {
            for model in models {
                add(line_of(&model.model_id), model.tokens)?;
            }
        }
        let rest = seat.unattributed()?;
        if !rest.is_zero() {
            add(UNATTRIBUTED, rest)?;
        }
    }
    Ok(lines)
}

/// One account window as seats reported it, deduplicated by length and reset.
struct Account {
    observed_at_ms: u64,
    used_percent: f64,
    seats: BTreeSet<String>,
}

fn rounded(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn limits(seats: &[Seat], as_of_ms: u64) -> Vec<Value> {
    let mut accounts = BTreeMap::<(u64, u64), Account>::new();
    for seat in seats {
        let Some(observed) = &seat.limits else {
            continue;
        };
        for &(minutes, used, resets) in &observed.windows {
            let account = accounts.entry((minutes, resets)).or_insert(Account {
                observed_at_ms: observed.observed_at_ms,
                used_percent: used,
                seats: BTreeSet::new(),
            });
            if observed.observed_at_ms > account.observed_at_ms {
                account.observed_at_ms = observed.observed_at_ms;
                account.used_percent = used;
            }
            account.seats.insert(seat.label.id.clone());
        }
    }
    accounts
        .into_iter()
        .map(|((minutes, resets), account)| {
            let length = minutes.saturating_mul(60_000);
            let start = resets.saturating_sub(length);
            // A window that already reset says nothing about the current pace.
            let live = resets > as_of_ms && length > 0;
            let elapsed = live.then(|| {
                rounded(as_of_ms.saturating_sub(start).min(length) as f64 / length as f64 * 100.0)
            });
            json!({
                "windowMinutes": minutes, "resetsAtMs": resets,
                "usedPercent": account.used_percent, "observedAtMs": account.observed_at_ms,
                "elapsedPercent": elapsed,
                "paceDeltaPoints": elapsed.map(|elapsed| rounded(account.used_percent - elapsed)),
                "stale": !live,
                "seats": account.seats,
            })
        })
        .collect()
}

fn limits_unavailable(seats: &[Seat], lines: &BTreeMap<&'static str, Line>) -> Vec<Value> {
    let with_limits: BTreeSet<&str> = seats
        .iter()
        .filter(|seat| seat.limits.is_some())
        .map(|seat| seat.label.id.as_str())
        .collect();
    lines
        .iter()
        .filter(|(name, line)| {
            **name != UNATTRIBUTED
                && !line
                    .seats
                    .iter()
                    .any(|seat| with_limits.contains(seat.as_str()))
        })
        .map(|(name, _)| {
            // A family Core has no limit source for is unavailable by construction;
            // the others simply had none observed.
            let reason = if has_limit_source(name) {
                "not_observed"
            } else {
                "no_source"
            };
            json!({"line": name, "reason": reason})
        })
        .collect()
}

/// The lines a seat's tokens belong to, as `lines[]` counts them.
fn seat_lines(seat: &Seat) -> Vec<&'static str> {
    let mut lines = seat
        .by_model
        .iter()
        .flatten()
        .map(|model| line_of(&model.model_id))
        .collect::<BTreeSet<_>>();
    if seat.unattributed().is_ok_and(|rest| !rest.is_zero()) {
        lines.insert(UNATTRIBUTED);
    }
    lines.into_iter().collect()
}

fn seat_json(seat: &Seat) -> Value {
    let mut value = json!({
        "id": seat.label.id, "name": seat.label.name, "retired": seat.label.retired,
        "totals": seat.totals.json(),
        "lines": seat_lines(seat),
        "byModel": seat.by_model.as_ref().map(|models| models.iter().map(|model| {
            let mut row = model.tokens.json();
            row["modelId"] = json!(model.model_id);
            row
        }).collect::<Vec<_>>()),
        "series": seat.series.iter().map(|point| {
            let mut row = point.tokens.json();
            row["fromMs"] = json!(point.from_ms);
            row["toMs"] = json!(point.to_ms);
            row["complete"] = json!(point.complete);
            if let Some(tier) = &point.service_tier {
                row["serviceTier"] = json!(tier);
            }
            row
        }).collect::<Vec<_>>(),
        "coverage": {"coveredMs": seat.covered_ms, "windowMs": seat.window_ms,
                     "startsAtMs": seat.starts_at_ms,
                     "complete": seat.complete, "gap": seat.gap},
    });
    // Only a seat that is partly attributed names its remainder; a seat with no
    // `byModel` is unattributed as a whole.
    if seat.by_model.is_some()
        && let Ok(rest) = seat.unattributed()
        && !rest.is_zero()
    {
        value["unattributed"] = rest.json();
    }
    value
}

/// Sessions that belong to no seat (an agent run outside any seat's session)
/// have no Core source: the row is always present and never a zero.
pub const OTHER_SESSIONS: &str = "other sessions";

fn other_sessions() -> Value {
    json!({"name": OTHER_SESSIONS, "reason": "no_source"})
}

fn unavailable_json(row: &Unavailable) -> Value {
    let mut value = json!({"id": row.label.id, "name": row.label.name,
                           "retired": row.label.retired, "reason": row.reason});
    if let Some(cause) = &row.cause {
        value["cause"] = json!(cause);
    }
    value
}

/// The `usage`, `limits` and `limitsUnavailable` sections.
pub fn usage_sections(
    usage: &Usage,
    as_of_ms: u64,
) -> Result<(Value, Vec<Value>, Vec<Value>), Error> {
    let grouped = lines(&usage.seats)?;
    let section = json!({
        "seats": usage.seats.iter().map(seat_json).collect::<Vec<_>>(),
        "unavailable": usage.unavailable.iter().map(unavailable_json).collect::<Vec<_>>(),
        "otherSessions": other_sessions(),
        "lines": grouped.iter().map(|(name, line)| json!({
            "line": name, "totals": line.tokens.json(),
            "seats": line.seats.len(), "partialSeats": line.partial.len(),
        })).collect::<Vec<_>>(),
    });
    Ok((
        section,
        limits(&usage.seats, as_of_ms),
        limits_unavailable(&usage.seats, &grouped),
    ))
}

#[derive(Clone, Copy, Default)]
struct Volume {
    count: u64,
    bytes: u64,
}

impl Volume {
    fn add(&mut self, count: u64, bytes: u64) -> Option<()> {
        self.count = self.count.checked_add(count).filter(|n| *n <= MAX)?;
        self.bytes = self.bytes.checked_add(bytes).filter(|n| *n <= MAX)?;
        Some(())
    }
    fn json(self) -> Value {
        json!({"count": self.count, "bytes": self.bytes})
    }
}

fn participant(id: &Option<String>, labels: &BTreeMap<String, Label>) -> Value {
    match id {
        None => json!({"id": null, "name": null, "retired": false}),
        Some(id) => labels.get(id).map_or_else(
            || json!({"id": id, "name": null, "retired": false}),
            Label::json,
        ),
    }
}

/// Sums `tmt x flow` rows into pairs and per-participant totals.
pub fn exchange(flow: &Value, labels: &BTreeMap<String, Label>) -> Result<Value, Error> {
    type Key = (Option<String>, Option<String>, String);
    let rows = flow["rows"]
        .as_array()
        .ok_or_else(|| invalid("exchange flow"))?;
    let mut pairs = BTreeMap::<Key, Volume>::new();
    let mut senders = BTreeMap::<Option<String>, Volume>::new();
    let mut recipients = BTreeMap::<Option<String>, Volume>::new();
    for row in rows {
        let side = |key: &str| -> Result<Option<String>, Error> {
            match &row[key] {
                Value::Null => Ok(None),
                Value::String(id) => Ok(Some(id.clone())),
                _ => Err(invalid("exchange flow")),
            }
        };
        let (sender, recipient) = (side("senderIdentityId")?, side("recipientIdentityId")?);
        let kind = row["kind"]
            .as_str()
            .filter(|kind| ["talk", "reply", "result", "ack"].contains(kind))
            .ok_or_else(|| invalid("exchange flow"))?;
        let (count, bytes) = (
            number(&row["count"]).ok_or_else(|| invalid("exchange flow"))?,
            number(&row["bytes"]).ok_or_else(|| invalid("exchange flow"))?,
        );
        let overflow = || invalid("exchange flow total");
        pairs
            .entry((sender.clone(), recipient.clone(), kind.to_owned()))
            .or_default()
            .add(count, bytes)
            .ok_or_else(overflow)?;
        senders
            .entry(sender)
            .or_default()
            .add(count, bytes)
            .ok_or_else(overflow)?;
        recipients
            .entry(recipient)
            .or_default()
            .add(count, bytes)
            .ok_or_else(overflow)?;
    }
    let ranked = |side: BTreeMap<Option<String>, Volume>| -> Vec<Value> {
        let mut rows: Vec<_> = side.into_iter().collect();
        rows.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(&b.0)));
        rows.into_iter()
            .map(|(id, volume)| {
                let mut row = volume.json();
                row["participant"] = participant(&id, labels);
                row
            })
            .collect()
    };
    Ok(json!({
        "sinceMs": flow["sinceMs"], "observedAtMs": flow["observedAtMs"],
        "pairs": pairs.into_iter().map(|((sender, recipient, kind), volume)| {
            let mut row = volume.json();
            row["sender"] = participant(&sender, labels);
            row["recipient"] = participant(&recipient, labels);
            row["kind"] = json!(kind);
            row
        }).collect::<Vec<_>>(),
        "bySender": ranked(senders),
        "byRecipient": ranked(recipients),
    }))
}

/// The identities a flow document names, for label resolution.
pub fn flow_participants(flow: &Value) -> BTreeSet<String> {
    flow["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| [&row["senderIdentityId"], &row["recipientIdentityId"]])
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

pub struct Report {
    pub as_of_ms: u64,
    pub since: &'static str,
    pub usage: Usage,
    pub exchange: Option<Value>,
    pub warnings: Vec<Value>,
}

/// The account's credit balance over the window: one point per series bucket in
/// which a seat observed one, taken from the seat with the newest observation so
/// seats on one account give one series and a balance is never summed. `serviceTiers`
/// is the sorted set of tiers the seats reported for that bucket. A bucket nobody
/// observed has no point, and nothing is carried into it. Null when no seat has any.
fn credits(seats: &[Seat]) -> Value {
    #[derive(Default)]
    struct Bucket {
        newest: Option<Credit>,
        tiers: BTreeSet<String>,
    }
    let mut buckets = BTreeMap::<(u64, u64), Bucket>::new();
    for point in seats.iter().flat_map(|seat| &seat.series) {
        let bucket = buckets.entry((point.from_ms, point.to_ms)).or_default();
        if let Some(tier) = &point.service_tier {
            bucket.tiers.insert(tier.clone());
        }
        if let Some(credit) = &point.credits
            && bucket
                .newest
                .as_ref()
                .is_none_or(|held| credit.observed_at_ms > held.observed_at_ms)
        {
            bucket.newest = Some(credit.clone());
        }
    }
    let points: Vec<Value> = buckets
        .into_iter()
        .filter_map(|((from_ms, to_ms), bucket)| {
            bucket.newest.map(|credit| {
                json!({
                    "fromMs": from_ms, "toMs": to_ms, "balance": credit.balance,
                    "observedAtMs": credit.observed_at_ms, "hasCredits": credit.has_credits,
                    "unlimited": credit.unlimited, "serviceTiers": bucket.tiers,
                })
            })
        })
        .collect();
    if points.is_empty() {
        Value::Null
    } else {
        json!({"points": points})
    }
}

pub fn document(report: &Report) -> Result<Value, Error> {
    let (usage, limits, unavailable) = usage_sections(&report.usage, report.as_of_ms)?;
    Ok(json!({
        "version": 1, "observedAtMs": report.as_of_ms, "since": report.since,
        "window": report.usage.window
            .map(|window| json!({"fromMs": window.from_ms, "toMs": window.to_ms})),
        "usage": usage,
        "limits": limits,
        "limitsUnavailable": unavailable,
        "credits": credits(&report.usage.seats),
        "exchange": report.exchange,
        "warnings": report.warnings,
    }))
}
