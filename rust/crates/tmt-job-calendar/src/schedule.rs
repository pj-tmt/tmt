use super::Error;
use jiff::{
    Timestamp, ToSpan,
    civil::Date,
    tz::{AmbiguousOffset, TimeZone},
};
use serde_json::{Value, json};

pub enum ScheduleInput<'a> {
    Every {
        duration: &'a str,
        from: Option<&'a str>,
    },
    At {
        time: &'a str,
        on: Option<&'a str>,
    },
    Cron(&'a str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Rule {
    Every {
        seconds: i64,
        anchor_ms: i64,
        from: Option<String>,
    },
    Calendar {
        pattern: Pattern,
        at: Option<String>,
        on: Option<String>,
    },
}

/// The zone is captured by name; later clock hosts do not reinterpret a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    zone: String,
    rule: Rule,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("SQUAD_CRON_SCHEDULE_INVALID", message)
}

fn timestamp(ms: i64) -> Result<Timestamp, Error> {
    Timestamp::from_millisecond(ms).map_err(|e| invalid(e.to_string()))
}

fn zone(name: &str) -> Result<TimeZone, Error> {
    TimeZone::get(name).map_err(|e| invalid(format!("Time zone {name}: {e}")))
}

/// No fixed-time catch-up through a gap, and no second occurrence in a fold.
fn fixed(zone: &TimeZone, date: Date, hour: u8, minute: u8) -> Result<Option<i64>, Error> {
    let ambiguous = zone.to_ambiguous_timestamp(date.at(hour as i8, minute as i8, 0, 0));
    if matches!(ambiguous.offset(), AmbiguousOffset::Gap { .. }) {
        return Ok(None);
    }
    ambiguous
        .earlier()
        .map(|t| Some(t.as_millisecond()))
        .map_err(|e| invalid(e.to_string()))
}

fn time(text: &str) -> Result<(u8, u8), Error> {
    let bytes = text.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
    {
        return Err(invalid("Use HH:MM (00:00 through 23:59)."));
    }
    let hour = (bytes[0] - b'0') * 10 + bytes[1] - b'0';
    let minute = (bytes[3] - b'0') * 10 + bytes[4] - b'0';
    if hour > 23 || minute > 59 {
        return Err(invalid("Use HH:MM (00:00 through 23:59)."));
    }
    Ok((hour, minute))
}

fn duration(text: &str) -> Result<i64, Error> {
    if !text.is_ascii() {
        return Err(invalid("Use a positive whole duration with s, m, h or d."));
    }
    let (digits, unit) = text.split_at(text.len().saturating_sub(1));
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => 0,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("Use a positive whole duration with s, m, h or d."));
    }
    digits
        .parse::<i64>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .filter(|n| *n > 0 && n.checked_mul(1000).is_some())
        .ok_or_else(|| invalid("Duration is zero or out of range."))
}

impl Schedule {
    pub fn local_zone() -> Result<String, Error> {
        TimeZone::try_system()
            .map_err(|e| invalid(e.to_string()))?
            .iana_name()
            .map(str::to_owned)
            .ok_or_else(|| invalid("The clock needs a named local time zone."))
    }

    pub fn parse(input: ScheduleInput<'_>, zone_name: &str, now_ms: i64) -> Result<Self, Error> {
        let timezone = zone(zone_name)?;
        let now = timestamp(now_ms)?;
        let rule = match input {
            ScheduleInput::Every {
                duration: text,
                from,
            } => {
                let seconds = duration(text)?;
                let mut anchor_ms = now_ms;
                if let Some(from) = from {
                    let (hour, minute) = time(from)?;
                    let mut day = now.to_zoned(timezone.clone()).date();
                    loop {
                        if let Some(anchor) = fixed(&timezone, day, hour, minute)? {
                            anchor_ms = anchor;
                            break;
                        }
                        day = day
                            .checked_add(1.days())
                            .map_err(|e| invalid(e.to_string()))?;
                    }
                }
                Rule::Every {
                    seconds,
                    anchor_ms,
                    from: from.map(str::to_owned),
                }
            }
            ScheduleInput::At { time: text, on } => {
                let (hour, minute) = time(text)?;
                let days = match on {
                    None => "*".to_owned(),
                    Some("weekdays") => "mon-fri".to_owned(),
                    Some(days)
                        if !days.is_empty()
                            && days
                                .split(',')
                                .all(|d| DAYS.contains(&d.to_ascii_lowercase().as_str())) =>
                    {
                        days.to_owned()
                    }
                    _ => {
                        return Err(invalid(
                            "--on accepts weekdays or comma-separated day names (mon,thu).",
                        ));
                    }
                };
                Rule::Calendar {
                    pattern: Pattern::parse(&format!("{minute} {hour} * * {days}"))?,
                    at: Some(text.into()),
                    on: on.map(str::to_owned),
                }
            }
            ScheduleInput::Cron(text) => Rule::Calendar {
                pattern: Pattern::parse(text)?,
                at: None,
                on: None,
            },
        };
        Ok(Self {
            zone: zone_name.into(),
            rule,
        })
    }

    /// Strictly future slots only. The caller chooses the current window;
    /// this function never reads an old run or reconstructs missed executions.
    pub fn next_after(&self, after_ms: i64) -> Result<Option<i64>, Error> {
        let after = timestamp(after_ms)?;
        match &self.rule {
            Rule::Every {
                seconds, anchor_ms, ..
            } => {
                let period = i128::from(*seconds) * 1000;
                let anchor = i128::from(*anchor_ms);
                let steps = ((i128::from(after_ms) - anchor).div_euclid(period) + 1).max(0);
                let next = i64::try_from(anchor + steps * period)
                    .ok()
                    .filter(|ms| timestamp(*ms).is_ok());
                Ok(next)
            }
            Rule::Calendar { pattern, .. } => {
                let timezone = zone(&self.zone)?;
                let mut date = after.to_zoned(timezone.clone()).date();
                // One Gregorian cycle covers every possible month/day/weekday
                // combination, without unbounded search for impossible dates.
                for _ in 0..=146097 {
                    if pattern.date_matches(date) {
                        for hour in pattern.hours.values(0, 23) {
                            for minute in pattern.minutes.values(0, 59) {
                                if let Some(slot) = fixed(&timezone, date, hour, minute)?
                                    && slot > after_ms
                                {
                                    return Ok(Some(slot));
                                }
                            }
                        }
                    }
                    let Ok(next) = date.checked_add(1.days()) else {
                        break;
                    };
                    date = next;
                }
                Ok(None)
            }
        }
    }

    pub fn document(&self) -> Value {
        match &self.rule {
            Rule::Every {
                seconds,
                anchor_ms,
                from,
            } => {
                json!({"kind":"every","zone":self.zone,"seconds":seconds,"anchorMs":anchor_ms,"from":from})
            }
            Rule::Calendar {
                at: Some(at), on, ..
            } => json!({"kind":"at","zone":self.zone,"at":at,"on":on}),
            Rule::Calendar { pattern, .. } => {
                json!({"kind":"cron","zone":self.zone,"expression":pattern.expression})
            }
        }
    }

    pub fn from_document(value: &Value) -> Result<Self, Error> {
        let text = |key: &str| {
            value[key]
                .as_str()
                .ok_or_else(|| invalid(format!("Missing schedule {key}.")))
        };
        let nullable = |key: &str| {
            if value[key].is_null() {
                Ok(None)
            } else {
                text(key).map(Some)
            }
        };
        let zone_name = text("zone")?;
        match text("kind")? {
            "every" => {
                let seconds = value["seconds"]
                    .as_i64()
                    .ok_or_else(|| invalid("Missing schedule seconds."))?;
                let anchor_ms = value["anchorMs"]
                    .as_i64()
                    .ok_or_else(|| invalid("Missing schedule anchorMs."))?;
                let seconds = duration(&format!("{seconds}s"))?;
                timestamp(anchor_ms)?;
                zone(zone_name)?;
                let from = nullable("from")?;
                if let Some(from) = from {
                    time(from)?;
                }
                // A stored elapsed anchor is already a UTC instant. Validate
                // its inputs without deriving it again from local wall time.
                Ok(Self {
                    zone: zone_name.into(),
                    rule: Rule::Every {
                        seconds,
                        anchor_ms,
                        from: from.map(str::to_owned),
                    },
                })
            }
            "at" => Self::parse(
                ScheduleInput::At {
                    time: text("at")?,
                    on: nullable("on")?,
                },
                zone_name,
                0,
            ),
            "cron" => Self::parse(ScheduleInput::Cron(text("expression")?), zone_name, 0),
            _ => Err(invalid("Unknown schedule kind.")),
        }
    }

    pub fn readable(&self) -> String {
        let text = match &self.rule {
            Rule::Every { seconds, from, .. } => {
                let (number, unit) = [(86400, "d"), (3600, "h"), (60, "m"), (1, "s")]
                    .into_iter()
                    .find(|(n, _)| seconds % n == 0)
                    .map(|(n, unit)| (seconds / n, unit))
                    .unwrap();
                format!(
                    "every {number}{unit}{}",
                    from.as_ref()
                        .map(|t| format!(" from {t}"))
                        .unwrap_or_default()
                )
            }
            Rule::Calendar {
                at: Some(at), on, ..
            } => format!("{} {at}", on.as_deref().unwrap_or("daily")),
            Rule::Calendar { pattern, .. } => format!("cron {}", pattern.expression),
        };
        format!("{text} · {}", self.zone)
    }
}

const MONTHS: &[&str] = &[
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAYS: &[&str] = &["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    bits: u64,
    star: bool,
}

impl Field {
    fn parse(text: &str, min: u8, max: u8, names: &[&str]) -> Result<Self, Error> {
        let number = |part: &str| -> Result<u8, Error> {
            let value = names
                .iter()
                .position(|name| name.eq_ignore_ascii_case(part))
                .map(|i| i as u8 + min)
                .or_else(|| {
                    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| part.parse().ok())
                        .flatten()
                });
            value
                .filter(|n| *n >= min && *n <= max)
                .ok_or_else(|| invalid(format!("Invalid cron field {text}.")))
        };
        let mut bits = 0;
        for item in text.split(',') {
            let (range, step) = item
                .split_once('/')
                .map_or((item, None), |(r, s)| (r, Some(s)));
            let step = match step {
                None => 1,
                Some(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => s
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| invalid("Invalid cron step."))?,
                _ => return Err(invalid("Invalid cron step.")),
            };
            let (start, end) = if range == "*" {
                (min, max)
            } else if let Some((start, end)) = range.split_once('-') {
                (number(start)?, number(end)?)
            } else {
                if item.contains('/') {
                    return Err(invalid("Use */N or A-B/N for cron steps."));
                }
                let n = number(range)?;
                (n, n)
            };
            if start > end {
                return Err(invalid("Cron ranges must ascend."));
            }
            for n in (start..=end).step_by(step) {
                bits |= 1u64 << if max == 7 && n == 7 { 0 } else { n };
            }
        }
        Ok(Self {
            bits,
            star: text.starts_with('*'),
        })
    }
    fn contains(&self, value: u8) -> bool {
        self.bits & (1u64 << value) != 0
    }
    fn values(&self, min: u8, max: u8) -> impl Iterator<Item = u8> + '_ {
        (min..=max).filter(|n| self.contains(*n))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    expression: String,
    minutes: Field,
    hours: Field,
    days: Field,
    months: Field,
    weekdays: Field,
}

impl Pattern {
    fn parse(text: &str) -> Result<Self, Error> {
        if text.len() > 256 {
            return Err(invalid("Cron expressions are at most 256 bytes."));
        }
        let words: Vec<_> = text.split_whitespace().collect();
        let [minutes, hours, days, months, weekdays] = words.as_slice() else {
            return Err(invalid(
                "Cron needs five fields: minute hour day month weekday.",
            ));
        };
        Ok(Self {
            expression: words.join(" "),
            minutes: Field::parse(minutes, 0, 59, &[])?,
            hours: Field::parse(hours, 0, 23, &[])?,
            days: Field::parse(days, 1, 31, &[])?,
            months: Field::parse(months, 1, 12, MONTHS)?,
            weekdays: Field::parse(weekdays, 0, 7, DAYS)?,
        })
    }
    fn date_matches(&self, date: Date) -> bool {
        let dom = self.days.contains(date.day() as u8);
        let dow = self
            .weekdays
            .contains(date.weekday().to_sunday_zero_offset() as u8);
        // Vixie day rule: with a leading star in either field both match;
        // otherwise restricted day-of-month/day-of-week are alternatives.
        self.months.contains(date.month() as u8)
            && if self.days.star || self.weekdays.star {
                dom && dow
            } else {
                dom || dow
            }
    }
}

#[cfg(test)]
mod tests;
