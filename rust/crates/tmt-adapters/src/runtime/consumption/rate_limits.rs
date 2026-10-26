//! Provider-neutral account windows. Codex reads its `token_count` line here;
//! Claude's driver owns its separate cache reader. Both keep the source's own
//! timestamp, never the read time, so old evidence remains visibly old.
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
    /// The driver of the source line these windows came from. Consumers use it and never infer
    /// a provider from a seat's model lines. Every stored value predates the field and came from
    /// Codex, the only source, so a missing one reads as `codex`.
    #[serde(default = "codex_provider")]
    pub provider: String,
    pub observed_at_ms: u64,
    pub windows: Vec<Window>,
    /// Which of the provider's accounts these windows belong to, when the source can tell. Absent
    /// for a provider with one implicit account (Codex, agy) and for every value stored before
    /// the field: a consumer treats absence as that one account (#5274).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<RateAccount>,
}

/// A provider account as a consumer may show and group it: an opaque key and a short label,
/// never the raw account id, an email or an organization name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateAccount {
    /// Stable per provider account: the first 12 hex digits of SHA-256 over the provider and
    /// the account id. It is not reversible to the account id.
    pub key: String,
    /// A display name taken from the account's local config root, for example `claude-work`.
    pub label: String,
}

const ACCOUNT_KEY_LENGTH: usize = 12;
const ACCOUNT_LABEL_MAX: usize = 32;

impl RateAccount {
    /// `account_id` is the provider's own stable id for the account and `root_name` the file
    /// name of its config root. Only `[A-Za-z0-9._-]` survives into the label, so an address or
    /// a name with spaces never appears; an empty result falls back to the provider.
    pub fn derive(provider: &str, account_id: &str, root_name: &str) -> Self {
        use sha2::{Digest, Sha256};
        let digest = Sha256::new()
            .chain_update(provider.as_bytes())
            .chain_update([0])
            .chain_update(account_id.as_bytes())
            .finalize();
        let key = digest
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0xf])
            .take(ACCOUNT_KEY_LENGTH)
            .map(|nibble| char::from_digit(u32::from(nibble), 16).unwrap_or('0'))
            .collect();
        // A name that looks like an address is never shown, even cleaned.
        let root_name = if root_name.contains('@') {
            ""
        } else {
            root_name
        };
        let label: String = root_name
            .trim_start_matches('.')
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '-'
                }
            })
            .take(ACCOUNT_LABEL_MAX)
            .collect();
        let label = label.trim_matches(|c| matches!(c, '.' | '-' | '_'));
        Self {
            key,
            label: if label.is_empty() { provider } else { label }.into(),
        }
    }

    fn valid(&self) -> bool {
        self.key.len() == ACCOUNT_KEY_LENGTH
            && self
                .key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && (1..=ACCOUNT_LABEL_MAX).contains(&self.label.len())
            && self
                .label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    }
}

fn codex_provider() -> String {
    tmt_core::driver::descriptor::CODEX.name.into()
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
        tmt_core::binding::session::HarnessId::new(&self.provider).is_ok()
            && self.observed_at_ms > 0
            && is_valid_js_safe_integer(self.observed_at_ms)
            && (1..=MAX_WINDOWS).contains(&self.windows.len())
            && self.windows.iter().all(Window::valid)
            && self
                .windows
                .windows(2)
                .all(|pair| pair[0].window_minutes < pair[1].window_minutes)
            && self.account.as_ref().is_none_or(RateAccount::valid)
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
        provider: codex_provider(),
        observed_at_ms: timestamp_ms(entry["timestamp"].as_str()?)?,
        windows,
        account: None,
    };
    parsed.valid().then_some(parsed)
}

/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, the only form the rollout writes.
pub(crate) fn timestamp_ms(text: &str) -> Option<u64> {
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
