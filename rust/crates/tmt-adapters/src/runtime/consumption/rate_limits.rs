//! Provider rate-limit windows read from the same Codex `token_count` line as
//! the counters. They describe the account, not the session, and age with the
//! line: `observed_at_ms` is the line's own timestamp, never the read time, so
//! an idle session reports old limits as old.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{Date, Month};
use tmt_core::limits::is_valid_js_safe_integer;

/// Codex reports a primary and a secondary window.
const MAX_WINDOWS: usize = 2;
/// Days from the Julian day number epoch to 1970-01-01.
const UNIX_EPOCH_JULIAN_DAY: i64 = 2_440_588;

/// The public shape: provider-neutral windows, which consumers pick by length.
/// Windows are strictly ascending by `window_minutes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateLimits {
    pub observed_at_ms: u64,
    pub windows: Vec<Window>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Window {
    pub window_minutes: u64,
    pub used_percent: f64,
    pub resets_at_ms: u64,
}

// `valid` admits only finite percentages, so equality is reflexive.
impl Eq for Window {}

impl Window {
    fn valid(&self) -> bool {
        self.window_minutes > 0
            && is_valid_js_safe_integer(self.window_minutes)
            && self.used_percent.is_finite()
            && (0.0..=100.0).contains(&self.used_percent)
            && self.resets_at_ms > 0
            && is_valid_js_safe_integer(self.resets_at_ms)
    }
}

impl RateLimits {
    pub(crate) fn valid(&self) -> bool {
        self.observed_at_ms > 0
            && is_valid_js_safe_integer(self.observed_at_ms)
            && (1..=MAX_WINDOWS).contains(&self.windows.len())
            && self.windows.iter().all(Window::valid)
            && self
                .windows
                .windows(2)
                .all(|pair| pair[0].window_minutes < pair[1].window_minutes)
    }
}

/// The limits on one `token_count` line, or None when the line has none or any
/// field is invalid: a partial object would show a guess as a measurement.
pub(super) fn codex(entry: &Value) -> Option<RateLimits> {
    let limits = entry["payload"].get("rate_limits")?.as_object()?;
    let mut windows = Vec::new();
    for key in ["primary", "secondary"] {
        match limits.get(key) {
            None | Some(Value::Null) => {}
            Some(window) => windows.push(Window {
                window_minutes: window["window_minutes"].as_u64()?,
                used_percent: window["used_percent"].as_f64()?,
                // The provider reports epoch seconds.
                resets_at_ms: window["resets_at"].as_u64()?.checked_mul(1000)?,
            }),
        }
    }
    windows.sort_by_key(|window| window.window_minutes);
    let parsed = RateLimits {
        observed_at_ms: timestamp_ms(entry["timestamp"].as_str()?)?,
        windows,
    };
    parsed.valid().then_some(parsed)
}

/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, the only form the rollout writes.
fn timestamp_ms(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || (bytes[4], bytes[7], bytes[10], bytes[13], bytes[16]) != (b'-', b'-', b'T', b':', b':')
        || bytes.last() != Some(&b'Z')
    {
        return None;
    }
    let field = |from: usize, to: usize| -> Option<u64> {
        let digits = text.get(from..to)?;
        digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse().ok())?
    };
    let milliseconds = match bytes[19] {
        b'Z' if bytes.len() == 20 => 0,
        b'.' => {
            let fraction = text.get(20..bytes.len() - 1)?;
            if fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            format!("{fraction:0<3}")[..3].parse().ok()?
        }
        _ => return None,
    };
    let date = Date::from_calendar_date(
        i32::try_from(field(0, 4)?).ok()?,
        Month::try_from(u8::try_from(field(5, 7)?).ok()?).ok()?,
        u8::try_from(field(8, 10)?).ok()?,
    )
    .ok()?;
    let (hour, minute, second) = (field(11, 13)?, field(14, 16)?, field(17, 19)?);
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = u64::try_from(i64::from(date.to_julian_day()) - UNIX_EPOCH_JULIAN_DAY).ok()?;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    seconds.checked_mul(1000)?.checked_add(milliseconds)
}

#[cfg(test)]
mod tests;
