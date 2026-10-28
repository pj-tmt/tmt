//! Best-effort allowance from Claude Code's own usage cache and from the observations the
//! first-party mod pushes (#5187). Never refresh the cache, read credentials, or retain unrelated
//! configuration fields. The newest observation per account wins, whole.
use crate::{
    runtime::consumption::{AccountLimits, NoLimits, RateAccount, RateLimits, RateWindow},
    skill_installation::ProviderEnvironment,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

const MAX_CONFIG_BYTES: usize = 1024 * 1024;
/// Pushed observations, under Core's data directory.
/// The first-party mod setup keeps in Claude's skills root once usage is consented. Its sources
/// are the plugin directory in the repository, locked to this build (#5187).
pub(super) static MOD: crate::setup::provider_mod::ProviderMod =
    crate::setup::provider_mod::ProviderMod {
        directory: "skills/tmt-allowance",
        files: &[
            (
                ".claude-plugin/plugin.json",
                include_bytes!(
                    "../../../../../../drivers/claude/plugins/tmt-allowance/.claude-plugin/plugin.json"
                ),
            ),
            (
                "hooks/hooks.json",
                include_bytes!(
                    "../../../../../../drivers/claude/plugins/tmt-allowance/hooks/hooks.json"
                ),
            ),
            (
                "hooks/register.mjs",
                include_bytes!(
                    "../../../../../../drivers/claude/plugins/tmt-allowance/hooks/register.mjs"
                ),
            ),
        ],
        purpose: "Claude Code mod that hands its rate-limit windows to tmt, so allowance figures stay fresh",
        activation: "Usage appears once a Claude session starts with the mod: run /reload-plugins in any Claude session, or start a new one. One session per account is enough.",
    };

const STORE: &str = "claude/allowance.json";
const STORE_LIMIT: usize = 8 * 1024;
const MAX_ACCOUNTS: usize = 8;
/// The mod sends the windows it measured; only these two are kept.
const MAX_PUSHED: usize = 3;

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

fn config_path(environment: &ProviderEnvironment) -> PathBuf {
    environment.var("CLAUDE_CONFIG_DIR").map_or_else(
        || environment.home().join(".claude.json"),
        |_| super::config_root(environment).join(".claude.json"),
    )
}

/// The allowance as both sources show it now. `None` means the root's config could not be read,
/// so nothing was observed and a caller keeps what it already holds (#5418); `Some(None)` means
/// the sources were read and neither has a current window, which is an observation.
pub(super) fn read(environment: &ProviderEnvironment, now_ms: u64) -> Option<Option<RateLimits>> {
    let bytes =
        match crate::bounded_file::read_no_follow(&config_path(environment), MAX_CONFIG_BYTES) {
            Ok(bytes) => bytes,
            // No config file means no account, no cache and no push: there is nothing to read.
            Err(crate::bounded_file::FileReadError::Io(error))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                return Some(None);
            }
            Err(_) => return None,
        };
    let cached = parse(&bytes, now_ms);
    let pushed = environment
        .data_root()
        .zip(account(&bytes))
        .and_then(|(root, account)| pushed(root, account, now_ms));
    // The newest observation wins whole; a tie prefers the cache. Never merged per window.
    Some(match (cached, pushed) {
        (Some(cached), Some(pushed)) if pushed.observed_at_ms > cached.observed_at_ms => {
            Some(pushed)
        }
        (Some(cached), _) => Some(cached),
        (None, pushed) => pushed,
    })
}

/// The account of `environment`'s Claude root as consumers group it (#5274): an opaque key over
/// the root's `oauthAccount.accountUuid` and the root's name. Both allowance sources are this
/// account's (the cache is admitted only when its account is the file's own). It is attached to
/// sampled readings and never stored in the 1024-byte remembered state.
pub(super) fn rate_account(environment: &ProviderEnvironment) -> Option<RateAccount> {
    let bytes =
        crate::bounded_file::read_no_follow(&config_path(environment), MAX_CONFIG_BYTES).ok()?;
    Some(root_account(environment, account(&bytes)?))
}

/// The account `uuid` as the environment's Claude root names it.
fn root_account(environment: &ProviderEnvironment, uuid: &str) -> RateAccount {
    let root = super::config_root(environment);
    let name = root.file_name().and_then(|name| name.to_str());
    RateAccount::derive(super::NAME, uuid, name.unwrap_or_default())
}

/// What each Claude account Core can see has now (#5418): the accounts in the pushed store and
/// the ambient root's own. Per account the newest observation with a current window wins whole
/// and a tie prefers the cache, as in `read`. An account with none is still returned, with
/// `expired` when an observation existed and has elapsed and `none_observed` when the account
/// is known and nothing was ever seen. A source that cannot be read adds nothing.
pub(super) fn accounts(environment: &ProviderEnvironment, now_ms: u64) -> Vec<AccountLimits> {
    #[derive(Default)]
    struct Seen {
        account: Option<RateAccount>,
        pushed: Option<RateLimits>,
        cached: Option<RateLimits>,
    }
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    if let Some(store) = environment
        .data_root()
        .and_then(|root| load(&root.join(STORE)))
    {
        for (uuid, mut stored) in store.accounts {
            if stored.provider != super::NAME || stored.observed_at_ms > now_ms || !stored.valid() {
                continue;
            }
            seen.insert(
                uuid,
                Seen {
                    account: stored.account.take(),
                    pushed: Some(stored),
                    cached: None,
                },
            );
        }
    }
    if let Ok(bytes) =
        crate::bounded_file::read_no_follow(&config_path(environment), MAX_CONFIG_BYTES)
        && let Some(uuid) = account(&bytes)
    {
        let entry = seen.entry(uuid.to_owned()).or_default();
        // The root's own name labels its account, whatever an older push recorded.
        entry.account = Some(root_account(environment, uuid));
        entry.cached = cache(&bytes, now_ms);
    }
    let mut listed: Vec<AccountLimits> = seen
        .into_iter()
        .map(|(uuid, seen)| {
            let account = seen
                .account
                .unwrap_or_else(|| RateAccount::derive(super::NAME, &uuid, ""));
            let observed = seen.pushed.is_some() || seen.cached.is_some();
            let (cached, pushed) = (
                seen.cached.and_then(|limits| current(limits, now_ms)),
                seen.pushed.and_then(|limits| current(limits, now_ms)),
            );
            let newest = match (cached, pushed) {
                (Some(cached), Some(pushed)) if pushed.observed_at_ms > cached.observed_at_ms => {
                    Some(pushed)
                }
                (Some(cached), _) => Some(cached),
                (None, pushed) => pushed,
            };
            AccountLimits {
                reason: newest.is_none().then_some(if observed {
                    NoLimits::Expired
                } else {
                    NoLimits::NoneObserved
                }),
                rate_limits: newest.map(|mut limits| {
                    limits.account = Some(account.clone());
                    limits
                }),
                account,
            }
        })
        .collect();
    listed.sort_by(|left, right| left.account.key.cmp(&right.account.key));
    listed
}

/// The windows of `limits` that have not elapsed, or nothing when none remain.
fn current(mut limits: RateLimits, now_ms: u64) -> Option<RateLimits> {
    limits.windows.retain(|window| window.resets_at_ms > now_ms);
    (!limits.windows.is_empty()).then_some(limits)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OauthAccountShape<'a> {
    #[serde(borrow)]
    oauth_account: Account<'a>,
}

/// The nonempty account of the root's own `.claude.json`: selected field only.
fn account(bytes: &[u8]) -> Option<&str> {
    let identity: OauthAccountShape<'_> = serde_json::from_slice(bytes).ok()?;
    let uuid = identity.oauth_account.account_uuid;
    (!uuid.trim().is_empty()).then_some(uuid)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PushedWindowStore {
    version: u8,
    accounts: BTreeMap<String, RateLimits>,
}

fn load(path: &Path) -> Option<PushedWindowStore> {
    let bytes = crate::bounded_file::read_no_follow(path, STORE_LIMIT).ok()?;
    let store: PushedWindowStore = serde_json::from_slice(&bytes).ok()?;
    (store.version == 1).then_some(store)
}

/// Whether the mod has pushed nothing yet for the environment's Claude account: the store has no
/// entry for it, expired or not.
pub(super) fn waiting(environment: &ProviderEnvironment, data_root: &Path) -> bool {
    let account = crate::bounded_file::read_no_follow(&config_path(environment), MAX_CONFIG_BYTES)
        .ok()
        .and_then(|bytes| account(&bytes).map(str::to_owned));
    !account.is_some_and(|account| {
        load(&data_root.join(STORE)).is_some_and(|store| store.accounts.contains_key(&account))
    })
}

fn pushed(data_root: &Path, account: &str, now_ms: u64) -> Option<RateLimits> {
    let mut limits = load(&data_root.join(STORE))?.accounts.remove(account)?;
    if limits.provider != super::NAME || limits.observed_at_ms > now_ms || !limits.valid() {
        return None;
    }
    // Callers attach the account of their own root; the stored label is for the account view.
    limits.account = None;
    current(limits, now_ms)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushDocument {
    rate_limits: Vec<PushedWindow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushedWindow {
    kind: String,
    percent_used: f64,
    #[serde(default)]
    resets_at: Option<String>,
}

/// Keep what the mod measured for the account of `environment`'s Claude root, stamped with the
/// receive time `now_ms`. `Ok(false)` means there was nothing to keep (no relevant or only
/// elapsed windows, no account); a malformed document is an error and changes nothing.
pub(super) fn push(
    environment: &ProviderEnvironment,
    data_root: &Path,
    input: &[u8],
    now_ms: u64,
) -> io::Result<bool> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "Invalid Claude allowance push.");
    let push: PushDocument = serde_json::from_slice(input).map_err(|_| invalid())?;
    if push.rate_limits.len() > MAX_PUSHED || !tmt_core::limits::is_valid_js_safe_integer(now_ms) {
        return Err(invalid());
    }
    let mut windows = Vec::new();
    for limit in push.rate_limits {
        let window_minutes = match limit.kind.as_str() {
            "five_hour" => 300,
            "seven_day" => 10_080,
            _ => continue,
        };
        let resets_at_ms = limit
            .resets_at
            .as_deref()
            .and_then(reset_ms)
            .ok_or_else(invalid)?;
        windows.push(RateWindow {
            window_minutes,
            used_percent: limit.percent_used,
            resets_at_ms,
        });
    }
    if windows.is_empty() {
        return Ok(false);
    }
    windows.sort_by_key(|window| window.window_minutes);
    let mut limits = RateLimits {
        provider: super::NAME.into(),
        observed_at_ms: now_ms,
        windows,
        account: None,
    };
    if limits
        .windows
        .windows(2)
        .any(|pair| pair[0].window_minutes == pair[1].window_minutes)
        || !limits.valid()
    {
        return Err(invalid());
    }
    limits.windows.retain(|window| window.resets_at_ms > now_ms);
    if limits.windows.is_empty() {
        return Ok(false);
    }
    let bytes = crate::bounded_file::read_no_follow(&config_path(environment), MAX_CONFIG_BYTES)
        .map_err(io::Error::other)?;
    let Some(account) = account(&bytes) else {
        return Ok(false);
    };
    // The store is keyed by the raw account id and has no root name, so the label the account
    // view shows is recorded with the entry.
    limits.account = Some(root_account(environment, account));
    let path = data_root.join(STORE);
    let mut store = load(&path).unwrap_or(PushedWindowStore {
        version: 1,
        accounts: BTreeMap::new(),
    });
    store.accounts.insert(account.to_owned(), limits);
    while store.accounts.len() > MAX_ACCOUNTS {
        let Some(oldest) = store
            .accounts
            .iter()
            .min_by_key(|(_, limits)| limits.observed_at_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        store.accounts.remove(&oldest);
    }
    let bytes = serde_json::to_vec(&store).map_err(io::Error::other)?;
    if bytes.len() > STORE_LIMIT {
        return Err(invalid());
    }
    if let Some(directory) = path.parent() {
        crate::private_file::ensure_private_directory(directory)?;
    }
    crate::private_file::replace(&path, &bytes)?;
    Ok(true)
}

fn parse(bytes: &[u8], now_ms: u64) -> Option<RateLimits> {
    cache(bytes, now_ms).and_then(|limits| current(limits, now_ms))
}

/// The root's usage cache as it was fetched, elapsed windows included.
fn cache(bytes: &[u8], now_ms: u64) -> Option<RateLimits> {
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
        account: None,
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
    limits.valid().then_some(limits)
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
