//! Best-effort allowance from Claude Code's own usage cache. Never refresh the
//! cache, read credentials, or retain unrelated configuration fields.
use crate::{
    runtime::consumption::{RateLimits, RateWindow},
    skill_installation::ProviderEnvironment,
};
use serde::Deserialize;

const MAX_CONFIG_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config<'a> {
    #[serde(borrow)]
    oauth_account: Account<'a>,
    #[serde(borrow)]
    cached_usage_utilization: Cache<'a>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Account<'a> {
    account_uuid: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache<'a> {
    fetched_at_ms: u64,
    account_uuid: &'a str,
    #[serde(borrow)]
    utilization: Windows<'a>,
}

#[derive(Deserialize)]
struct Windows<'a> {
    #[serde(borrow)]
    five_hour: Option<Window<'a>>,
    #[serde(borrow)]
    seven_day: Option<Window<'a>>,
}

#[derive(Deserialize)]
struct Window<'a> {
    // The persisted /usage response uses percent (0–100), unlike the SDK
    // rate_limit_event's fraction. Never guess the unit from its magnitude.
    utilization: f64,
    resets_at: &'a str,
}

pub(super) fn read(environment: &ProviderEnvironment, now_ms: u64) -> Option<RateLimits> {
    let path = environment.var("CLAUDE_CONFIG_DIR").map_or_else(
        || environment.home().join(".claude.json"),
        |_| super::config_root(environment).join(".claude.json"),
    );
    let bytes = crate::bounded_file::read_no_follow(&path, MAX_CONFIG_BYTES).ok()?;
    parse(&bytes, now_ms)
}

fn parse(bytes: &[u8], now_ms: u64) -> Option<RateLimits> {
    // Unknown fields are skipped by serde, rather than building a Value tree
    // containing provider settings, MCP configuration or other private data.
    let config: Config<'_> = serde_json::from_slice(bytes).ok()?;
    let cache = config.cached_usage_utilization;
    if cache.account_uuid.trim().is_empty()
        || cache.account_uuid != config.oauth_account.account_uuid
        || cache.fetched_at_ms > now_ms
        || !tmt_core::limits::is_valid_js_safe_integer(now_ms)
    {
        return None;
    }
    let mut limits = RateLimits {
        provider: super::NAME.into(),
        observed_at_ms: cache.fetched_at_ms,
        windows: Vec::new(),
    };
    for (minutes, window) in [
        (300, cache.utilization.five_hour),
        (10_080, cache.utilization.seven_day),
    ] {
        if let Some(window) = window {
            limits.windows.push(RateWindow {
                window_minutes: minutes,
                used_percent: window.utilization,
                resets_at_ms: reset_ms(window.resets_at)?,
            });
        }
    }
    if !limits.valid() {
        return None;
    }
    limits.windows.retain(|window| window.resets_at_ms > now_ms);
    (!limits.windows.is_empty()).then_some(limits)
}

/// Claude caches RFC3339 resets with fractional seconds and a numeric offset.
/// Reuse the strict UTC timestamp parser without broadening Codex's wire format.
fn reset_ms(text: &str) -> Option<u64> {
    use crate::runtime::consumption::timestamp_ms;
    if text.ends_with('Z') {
        return timestamp_ms(text);
    }
    let split = text.len().checked_sub(6)?;
    let offset = text.get(split..)?.as_bytes();
    if !matches!(offset[0], b'+' | b'-')
        || offset[3] != b':'
        || ![offset[1], offset[2], offset[4], offset[5]]
            .iter()
            .all(u8::is_ascii_digit)
    {
        return None;
    }
    let hour = u64::from((offset[1] - b'0') * 10 + offset[2] - b'0');
    let minute = u64::from((offset[4] - b'0') * 10 + offset[5] - b'0');
    if hour > 23 || minute > 59 {
        return None;
    }
    let local = timestamp_ms(&format!("{}Z", text.get(..split)?))?;
    let difference = (hour * 60 + minute) * 60_000;
    if offset[0] == b'+' {
        local.checked_sub(difference)
    } else {
        local.checked_add(difference)
    }
}

#[cfg(test)]
mod tests;
