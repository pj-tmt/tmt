//! The Codex credit balance read from the same `token_count` line as the
//! counters. Like the rate-limit windows it describes the account, not the
//! session, and ages with the line: `observed_at_ms` is the line's own
//! timestamp, never the read time.
use super::rate_limits::timestamp_ms;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tmt_core::limits::is_valid_js_safe_integer;

const MAX_WHOLE_DIGITS: usize = 15;
const MAX_FRACTION_DIGITS: usize = 12;

/// The public shape. `balance` stays the provider's decimal text: it can exceed
/// what a JavaScript number holds exactly, and Core does no arithmetic on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Credits {
    pub observed_at_ms: u64,
    pub balance: String,
    pub has_credits: bool,
    pub unlimited: bool,
}

impl Credits {
    pub(crate) fn valid(&self) -> bool {
        self.observed_at_ms > 0
            && is_valid_js_safe_integer(self.observed_at_ms)
            && decimal(&self.balance)
    }
}

/// `[0-9]{1,15}` with an optional `.[0-9]{1,12}`.
fn decimal(text: &str) -> bool {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let digits = |part: &str, most: usize| {
        (1..=most).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit())
    };
    digits(whole, MAX_WHOLE_DIGITS)
        && fraction.is_none_or(|fraction| digits(fraction, MAX_FRACTION_DIGITS))
}

/// The credits on one `token_count` line, or None when the line has none or any
/// field is invalid: a partial object would show a guess as a measurement.
pub(super) fn codex(entry: &Value) -> Option<Credits> {
    let credits = entry["payload"]["rate_limits"]
        .get("credits")?
        .as_object()?;
    let parsed = Credits {
        observed_at_ms: timestamp_ms(entry["timestamp"].as_str()?)?,
        balance: credits.get("balance")?.as_str()?.to_owned(),
        has_credits: credits.get("has_credits")?.as_bool()?,
        unlimited: credits.get("unlimited")?.as_bool()?,
    };
    parsed.valid().then_some(parsed)
}

#[cfg(test)]
mod tests;
