//! The usage report document as the board holds it: validated once, then shown
//! as the supplier decided it. Nothing here reads Core or the clock.

use serde_json::Value;

const MAX: u64 = 9_007_199_254_740_991;

/// The windows the report offers, as its `--since` takes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Since {
    #[default]
    Day,
    Week,
    Month,
}

impl Since {
    pub const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Month];

    pub fn text(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|since| since.text() == text)
    }
}

/// One line's totals (`claude`, `gpt`, `other`, `unattributed`).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub name: String,
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    pub seats: u64,
    pub partial_seats: u64,
}

/// A name the report could not resolve is shown by the start of its UUID.
#[derive(Clone, Debug, PartialEq)]
pub struct Who {
    pub id: Option<String>,
    pub name: Option<String>,
    pub retired: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    pub who: Who,
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    /// The lines the seat's tokens belong to, as the supplier counted them.
    pub lines: Vec<String>,
    /// Input plus output per bucket, oldest first.
    pub series: Vec<u64>,
    pub covered_ms: u64,
    pub window_ms: u64,
    /// Where the seat's evidence begins inside the window; None from a supplier
    /// that does not report it.
    pub starts_at_ms: Option<u64>,
    pub complete: bool,
}

impl Seat {
    pub fn total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unavailable {
    pub who: Who,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub window_minutes: u64,
    pub used_percent: f64,
    pub elapsed_percent: Option<f64>,
    pub pace_delta_points: Option<f64>,
    pub stale: bool,
    pub seats: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LimitUnavailable {
    pub line: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tally {
    pub who: Who,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Traffic {
    pub senders: Vec<Tally>,
    pub recipients: Vec<Tally>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub section: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub since: Since,
    pub observed_at_ms: u64,
    /// The start of the window, None when no identity could be read.
    pub window_from_ms: Option<u64>,
    pub lines: Vec<Line>,
    pub seats: Vec<Seat>,
    pub unavailable: Vec<Unavailable>,
    pub limits: Vec<Limit>,
    pub limits_unavailable: Vec<LimitUnavailable>,
    /// None when the supplier could not read the flow.
    pub exchange: Option<Traffic>,
    pub warnings: Vec<Warning>,
}

fn malformed() -> String {
    "The usage report is malformed.".into()
}

fn number(value: &Value) -> Result<u64, String> {
    value.as_u64().filter(|n| *n <= MAX).ok_or_else(malformed)
}

fn optional_number(value: &Value) -> Result<Option<u64>, String> {
    if value.is_null() {
        Ok(None)
    } else {
        number(value).map(Some)
    }
}

fn text(value: &Value) -> Result<String, String> {
    value.as_str().map(clean).ok_or_else(malformed)
}

fn clean(text: &str) -> String {
    crate::board::notes::sanitize(text)
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value[key].as_array().ok_or_else(malformed)
}

fn who(value: &Value) -> Result<Who, String> {
    let id = match &value["id"] {
        Value::Null => None,
        Value::String(id) => Some(clean(id)),
        _ => return Err(malformed()),
    };
    let name = match &value["name"] {
        Value::Null => None,
        Value::String(name) => Some(clean(name)),
        _ => return Err(malformed()),
    };
    Ok(Who {
        id,
        name,
        retired: value["retired"].as_bool().ok_or_else(malformed)?,
    })
}

fn percent(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|p| p.is_finite() && (-1000.0..=1000.0).contains(p))
        .ok_or_else(malformed)
}

fn optional_percent(value: &Value) -> Result<Option<f64>, String> {
    if value.is_null() {
        Ok(None)
    } else {
        percent(value).map(Some)
    }
}

fn seat(value: &Value) -> Result<Seat, String> {
    let totals = &value["totals"];
    let coverage = &value["coverage"];
    Ok(Seat {
        who: who(value)?,
        input: number(&totals["input"])?,
        output: number(&totals["output"])?,
        cached: number(&totals["cached"])?,
        lines: array(value, "lines")?
            .iter()
            .map(text)
            .collect::<Result<_, _>>()?,
        series: array(value, "series")?
            .iter()
            .map(|point| Ok(number(&point["input"])?.saturating_add(number(&point["output"])?)))
            .collect::<Result<_, String>>()?,
        covered_ms: number(&coverage["coveredMs"])?,
        window_ms: number(&coverage["windowMs"])?,
        starts_at_ms: optional_number(&coverage["startsAtMs"])?,
        complete: coverage["complete"].as_bool().ok_or_else(malformed)?,
    })
}

fn limit(value: &Value) -> Result<Limit, String> {
    Ok(Limit {
        window_minutes: number(&value["windowMinutes"]).and_then(|minutes| {
            if minutes > 0 {
                Ok(minutes)
            } else {
                Err(malformed())
            }
        })?,
        used_percent: percent(&value["usedPercent"])?,
        elapsed_percent: optional_percent(&value["elapsedPercent"])?,
        pace_delta_points: optional_percent(&value["paceDeltaPoints"])?,
        stale: value["stale"].as_bool().ok_or_else(malformed)?,
        seats: array(value, "seats")?
            .iter()
            .map(text)
            .collect::<Result<_, _>>()?,
    })
}

fn tallies(value: &Value, key: &str) -> Result<Vec<Tally>, String> {
    array(value, key)?
        .iter()
        .map(|row| {
            Ok(Tally {
                who: who(&row["participant"])?,
                count: number(&row["count"])?,
            })
        })
        .collect()
}

impl Report {
    /// Version 1 documents only. Unknown additive fields are ignored; a field
    /// this view shows that is missing or mistyped refuses the whole document.
    pub fn parse(document: &Value) -> Result<Self, String> {
        if document["version"] != 1 {
            return Err("The usage report is not version 1.".into());
        }
        let usage = &document["usage"];
        Ok(Self {
            since: document["since"]
                .as_str()
                .and_then(Since::parse)
                .ok_or_else(malformed)?,
            observed_at_ms: number(&document["observedAtMs"])?,
            window_from_ms: optional_number(&document["window"]["fromMs"])?,
            lines: array(usage, "lines")?
                .iter()
                .map(|row| {
                    Ok(Line {
                        name: text(&row["line"])?,
                        input: number(&row["totals"]["input"])?,
                        output: number(&row["totals"]["output"])?,
                        cached: number(&row["totals"]["cached"])?,
                        seats: number(&row["seats"])?,
                        partial_seats: number(&row["partialSeats"])?,
                    })
                })
                .collect::<Result<_, String>>()?,
            seats: array(usage, "seats")?
                .iter()
                .map(seat)
                .collect::<Result<_, _>>()?,
            unavailable: array(usage, "unavailable")?
                .iter()
                .map(|row| {
                    Ok(Unavailable {
                        who: who(row)?,
                        reason: text(&row["reason"])?,
                    })
                })
                .collect::<Result<_, String>>()?,
            limits: array(document, "limits")?
                .iter()
                .map(limit)
                .collect::<Result<_, _>>()?,
            limits_unavailable: array(document, "limitsUnavailable")?
                .iter()
                .map(|row| {
                    Ok(LimitUnavailable {
                        line: text(&row["line"])?,
                        reason: text(&row["reason"])?,
                    })
                })
                .collect::<Result<_, String>>()?,
            exchange: match &document["exchange"] {
                Value::Null => None,
                exchange => Some(Traffic {
                    senders: tallies(exchange, "bySender")?,
                    recipients: tallies(exchange, "byRecipient")?,
                }),
            },
            warnings: array(document, "warnings")?
                .iter()
                .map(|row| {
                    Ok(Warning {
                        section: text(&row["section"])?,
                        message: text(&row["message"])?,
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }
}
