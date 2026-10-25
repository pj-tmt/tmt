use super::*;
use crate::test_support::TestDirectory;
use serde_json::{Value, json};
use std::fs;

// Independent Python datetime values, matching the cache's observed ISO shape.
const FETCHED: u64 = 1_791_604_109_763;
const FIVE_RESET: u64 = 1_791_620_000_679;
const WEEK_RESET: u64 = 1_791_948_558_000;
const NOW: u64 = FETCHED + 3_600_001;

fn config() -> Value {
    // Selected structure observed in Claude Code 2.1.308; all values synthetic.
    json!({
        "oauthAccount": {"accountUuid": "account-a", "emailAddress": "private@example.invalid"},
        "cachedUsageUtilization": {
            "fetchedAtMs": FETCHED, "accountUuid": "account-a",
            "utilization": {
                "five_hour": {"utilization": 12.5, "resets_at": "2026-10-10T08:13:20.679456+00:00"},
                "seven_day": {"utilization": 78, "resets_at": "2026-10-14T03:29:18+00:00"},
                "extra_usage": {"unrelated": ["private"]}
            }
        },
        "mcpServers": {"private": {"env": {"SECRET": "do-not-retain"}}}
    })
}

fn parsed(value: &Value, now: u64) -> Option<RateLimits> {
    parse(&serde_json::to_vec(value).unwrap(), now)
}

#[test]
fn real_cache_shape_keeps_percent_units_fetch_age_and_only_selected_fields() {
    let limits = parsed(&config(), NOW).unwrap();
    assert_eq!(limits.provider, "claude");
    assert_eq!(limits.observed_at_ms, FETCHED);
    assert_eq!(
        limits.windows,
        vec![
            RateWindow {
                window_minutes: 300,
                used_percent: 12.5,
                resets_at_ms: FIVE_RESET
            },
            RateWindow {
                window_minutes: 10_080,
                used_percent: 78.0,
                resets_at_ms: WEEK_RESET
            },
        ]
    );
    let document = serde_json::to_value(limits).unwrap();
    assert_eq!(document.as_object().unwrap().len(), 3);
    assert!(!document.to_string().contains("private"));
    // Values under one are percentages too, not evidence of a fractional unit.
    for used in [0.0, 0.5, 1.0, 100.0] {
        let mut value = config();
        value["cachedUsageUtilization"]["utilization"]["five_hour"]["utilization"] = json!(used);
        assert_eq!(parsed(&value, NOW).unwrap().windows[0].used_percent, used);
    }
}

#[test]
fn only_same_file_nonempty_account_equality_admits_a_cache() {
    for account in [
        json!(null),
        json!(""),
        json!(" "),
        json!("account-b"),
        json!(12),
    ] {
        let mut value = config();
        value["cachedUsageUtilization"]["accountUuid"] = account;
        assert_eq!(parsed(&value, NOW), None);
    }
    for field in ["oauthAccount", "cachedUsageUtilization"] {
        let mut value = config();
        value[field].as_object_mut().unwrap().remove("accountUuid");
        assert_eq!(parsed(&value, NOW), None);
    }
    let mut value = config();
    value["oauthAccount"]["accountUuid"] = json!("");
    value["cachedUsageUtilization"]["accountUuid"] = json!("");
    assert_eq!(parsed(&value, NOW), None);
}

#[test]
fn future_or_invalid_fetch_times_are_not_observations() {
    for fetched in [
        json!(NOW + 1),
        json!(0),
        json!(-1),
        json!(1.5),
        json!("1000"),
        json!(null),
        json!(9_007_199_254_740_992u64),
    ] {
        let mut value = config();
        value["cachedUsageUtilization"]["fetchedAtMs"] = fetched;
        assert_eq!(parsed(&value, NOW), None);
    }
    assert!(parsed(&config(), FETCHED).is_some());
}

#[test]
fn old_current_windows_survive_but_reset_windows_are_removed_individually() {
    let limits = parsed(&config(), FIVE_RESET).unwrap();
    assert_eq!(limits.observed_at_ms, FETCHED);
    assert_eq!(limits.windows.len(), 1);
    assert_eq!(limits.windows[0].window_minutes, 10_080);
    assert!(parsed(&config(), FIVE_RESET - 1).is_some());
    assert_eq!(parsed(&config(), WEEK_RESET), None);
}

#[test]
fn missing_or_null_windows_are_optional_but_malformed_windows_reject_the_snapshot() {
    for missing in [false, true] {
        let mut value = config();
        let windows = value["cachedUsageUtilization"]["utilization"]
            .as_object_mut()
            .unwrap();
        if missing {
            windows.remove("five_hour");
        } else {
            windows.insert("five_hour".into(), Value::Null);
        }
        assert_eq!(parsed(&value, NOW).unwrap().windows.len(), 1);
        value["cachedUsageUtilization"]["utilization"]
            .as_object_mut()
            .unwrap()
            .remove("seven_day");
        assert_eq!(parsed(&value, NOW), None);
    }
    for window in [
        json!({}),
        json!({"utilization":12.5}),
        json!({"resets_at":"2026-10-14T03:29:18Z"}),
        json!({"utilization":null,"resets_at":"2026-10-14T03:29:18Z"}),
        json!(false),
    ] {
        let mut value = config();
        value["cachedUsageUtilization"]["utilization"]["five_hour"] = window;
        assert_eq!(parsed(&value, NOW), None);
    }
    for used in [json!(-0.1), json!(100.1), json!("12.5")] {
        let mut value = config();
        value["cachedUsageUtilization"]["utilization"]["five_hour"]["utilization"] = used;
        assert_eq!(parsed(&value, NOW), None);
    }
}

#[test]
fn iso_resets_support_real_fractional_offsets_without_weakening_utc_validation() {
    for text in [
        "2026-10-10T08:13:20.679456+00:00",
        "2026-10-10T08:13:20.679Z",
        "2026-10-10T17:13:20.679+09:00",
        "2026-10-10T01:13:20.679-07:00",
    ] {
        assert_eq!(reset_ms(text), Some(FIVE_RESET), "{text}");
    }
    for text in [
        "",
        "2026-10-10T08:13:20",
        "2026-10-10T08:13:20+24:00",
        "2026-10-10T08:13:20+00:60",
        "2026-02-30T08:13:20Z",
        "2026-10-10T08:13:20+0x:00",
        "2026-10-10T08:13:20+00：00",
    ] {
        assert_eq!(reset_ms(text), None, "{text}");
    }
}

#[test]
fn a_seats_selected_root_never_falls_back_to_another_roots_cache() {
    let home = TestDirectory::new();
    let default = home.path.join(".claude.json");
    let custom = home.path.join("custom");
    fs::create_dir(&custom).unwrap();
    let bytes = serde_json::to_vec(&config()).unwrap();
    fs::write(&default, &bytes).unwrap();
    let env = |root| ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), root);
    let plain = env(vec![]);
    let selected = env(vec![(
        "CLAUDE_CONFIG_DIR",
        std::path::PathBuf::from("custom"),
    )]);
    assert!(read(&plain, NOW).is_some());
    assert_eq!(read(&selected, NOW), None);
    fs::write(custom.join(".claude.json"), &bytes).unwrap();
    fs::remove_file(&default).unwrap();
    assert!(read(&selected, NOW).is_some());
    assert_eq!(read(&plain, NOW), None);
    assert_eq!(fs::read(custom.join(".claude.json")).unwrap(), bytes);
}

#[test]
fn unreadable_malformed_oversized_or_linked_cache_is_unavailable_without_writes() {
    let home = TestDirectory::new();
    let environment = ProviderEnvironment::from_parts(&home.path, &home.path, Vec::new(), []);
    let path = home.path.join(".claude.json");
    assert_eq!(read(&environment, NOW), None);
    for bytes in [b"{broken".to_vec(), vec![b' '; MAX_CONFIG_BYTES + 1]] {
        fs::write(&path, &bytes).unwrap();
        assert_eq!(read(&environment, NOW), None);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(read(&environment, NOW), None);
    fs::remove_dir(&path).unwrap();
    let target = home.path.join("target.json");
    fs::write(&target, serde_json::to_vec(&config()).unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert_eq!(read(&environment, NOW), None);
    assert!(path.is_symlink());
}

// Observations the first-party mod pushes (#5187). The reset instants are the cache's, written the
// way Claude Code reports them to a mod: UTC with milliseconds.
const PUSHED_FIVE_RESET: u64 = FIVE_RESET - 679;

fn push_body(five: f64, week: f64) -> Vec<u8> {
    serde_json::to_vec(&json!({"rateLimits": [
        {"kind": "five_hour", "percentUsed": five, "resetsAt": "2026-10-10T08:13:20.000Z"},
        {"kind": "seven_day", "percentUsed": week, "resetsAt": "2026-10-14T03:29:18.000Z"},
        {"kind": "spend_limit", "percentUsed": 3}
    ]}))
    .unwrap()
}

struct PushedFixture {
    home: TestDirectory,
    data: TestDirectory,
}

impl PushedFixture {
    fn new(config: &Value) -> Self {
        let pushed = Self {
            home: TestDirectory::new(),
            data: TestDirectory::new(),
        };
        pushed.config(config);
        pushed
    }
    fn config(&self, config: &Value) {
        fs::write(
            self.home.path.join(".claude.json"),
            serde_json::to_vec(config).unwrap(),
        )
        .unwrap();
    }
    fn environment(&self) -> ProviderEnvironment {
        ProviderEnvironment::from_parts(&self.home.path, &self.home.path, Vec::new(), [])
            .with_data_root(&self.data.path)
    }
    fn push(&self, body: &[u8], now: u64) -> io::Result<bool> {
        push(&self.environment(), &self.data.path, body, now)
    }
    fn read(&self, now: u64) -> Option<RateLimits> {
        read(&self.environment(), now)
    }
    fn store(&self) -> Option<PushedWindowStore> {
        load(&self.data.path.join(STORE))
    }
}

fn used(limits: &RateLimits) -> Vec<f64> {
    limits
        .windows
        .iter()
        .map(|window| window.used_percent)
        .collect()
}

#[test]
fn the_newest_observation_wins_whole_between_a_push_and_the_cache() {
    let pushed = PushedFixture::new(&config());
    let cache = fs::read(pushed.home.path.join(".claude.json")).unwrap();
    // Before any push the cache answers, with its own age.
    assert_eq!(pushed.read(NOW).unwrap().observed_at_ms, FETCHED);
    assert!(pushed.push(&push_body(20.0, 80.0), NOW).unwrap());
    let limits = pushed.read(NOW).unwrap();
    assert_eq!(
        (limits.provider.as_str(), limits.observed_at_ms),
        ("claude", NOW)
    );
    assert_eq!(used(&limits), [20.0, 80.0]);
    assert_eq!(limits.windows[0].resets_at_ms, PUSHED_FIVE_RESET);
    assert_eq!(limits.windows[1].resets_at_ms, WEEK_RESET);
    // The cache file itself is never written by a push.
    assert_eq!(
        fs::read(pushed.home.path.join(".claude.json")).unwrap(),
        cache
    );
    // A newer cache beats the older push, whole: the windows are never mixed.
    let mut newer = config();
    newer["cachedUsageUtilization"]["fetchedAtMs"] = json!(NOW + 1_000);
    pushed.config(&newer);
    let limits = pushed.read(NOW + 5_000).unwrap();
    assert_eq!(limits.observed_at_ms, NOW + 1_000);
    assert_eq!(used(&limits), [12.5, 78.0]);
    // A tie prefers the cache; a newer push beats it again.
    assert!(pushed.push(&push_body(30.0, 81.0), NOW + 1_000).unwrap());
    assert_eq!(used(&pushed.read(NOW + 5_000).unwrap()), [12.5, 78.0]);
    assert!(pushed.push(&push_body(31.0, 82.0), NOW + 2_000).unwrap());
    let limits = pushed.read(NOW + 5_000).unwrap();
    assert_eq!(
        (limits.observed_at_ms, used(&limits)),
        (NOW + 2_000, vec![31.0, 82.0])
    );
}

#[test]
fn a_push_answers_alone_for_an_account_that_never_opened_usage_and_only_for_that_account() {
    let mut bare = config();
    bare.as_object_mut()
        .unwrap()
        .remove("cachedUsageUtilization");
    let pushed = PushedFixture::new(&bare);
    assert_eq!(pushed.read(NOW), None);
    assert!(pushed.push(&push_body(9.0, 59.0), NOW).unwrap());
    assert_eq!(used(&pushed.read(NOW).unwrap()), [9.0, 59.0]);
    // The same data root under another account's root reads nothing.
    let mut other = bare.clone();
    other["oauthAccount"]["accountUuid"] = json!("account-b");
    pushed.config(&other);
    assert_eq!(pushed.read(NOW), None);
    // An elapsed window is dropped individually, an elapsed push entirely.
    pushed.config(&bare);
    assert_eq!(pushed.read(PUSHED_FIVE_RESET).unwrap().windows.len(), 1);
    assert_eq!(pushed.read(WEEK_RESET), None);
    // A push from the future is not an observation.
    assert_eq!(pushed.read(NOW - 1), None);
}

#[test]
fn a_malformed_push_is_refused_and_leaves_no_store() {
    let pushed = PushedFixture::new(&config());
    let window =
        json!({"kind": "five_hour", "percentUsed": 1, "resetsAt": "2026-10-10T08:13:20.000Z"});
    let refused: Vec<Vec<u8>> = vec![
        b"{broken".to_vec(),
        serde_json::to_vec(&json!({"rateLimits": "no"})).unwrap(),
        serde_json::to_vec(&json!({"rateLimits": [window, window, window, window]})).unwrap(),
        // A duplicate kind, an out-of-range percentage and a window without its reset.
        serde_json::to_vec(&json!({"rateLimits": [window, window]})).unwrap(),
        serde_json::to_vec(
            &json!({"rateLimits": [{"kind": "five_hour", "percentUsed": 100.5,
            "resetsAt": "2026-10-10T08:13:20.000Z"}]}),
        )
        .unwrap(),
        serde_json::to_vec(&json!({"rateLimits": [{"kind": "seven_day", "percentUsed": 5}]}))
            .unwrap(),
        serde_json::to_vec(
            &json!({"rateLimits": [{"kind": "seven_day", "percentUsed": 5,
            "resetsAt": "yesterday"}]}),
        )
        .unwrap(),
    ];
    for body in &refused {
        assert!(
            pushed.push(body, NOW).is_err(),
            "{}",
            String::from_utf8_lossy(body)
        );
    }
    assert!(pushed.store().is_none());
    assert!(!pushed.data.path.join(STORE).exists());
    // Nothing relevant, or only elapsed windows, is not an error and stores nothing.
    for body in [
        serde_json::to_vec(&json!({"rateLimits": []})).unwrap(),
        serde_json::to_vec(&json!({"rateLimits": [{"kind": "spend_limit", "percentUsed": 3}]}))
            .unwrap(),
    ] {
        assert!(!pushed.push(&body, NOW).unwrap());
    }
    assert!(!pushed.push(&push_body(1.0, 2.0), WEEK_RESET + 1).unwrap());
    assert!(pushed.store().is_none());
    // Without an account in the root's own file there is nowhere to keep it.
    pushed.config(&json!({"mcpServers": {}}));
    assert!(!pushed.push(&push_body(1.0, 2.0), NOW).unwrap());
    assert!(pushed.store().is_none());
}

#[test]
fn the_store_is_private_bounded_to_eight_accounts_and_drops_the_oldest() {
    use std::os::unix::fs::PermissionsExt;
    let pushed = PushedFixture::new(&config());
    for index in 0..10_u64 {
        let mut account = config();
        account["oauthAccount"]["accountUuid"] = json!(format!("account-{index}"));
        pushed.config(&account);
        assert!(pushed.push(&push_body(1.0, 2.0), NOW + index).unwrap());
    }
    let store = pushed.store().unwrap();
    assert_eq!(store.accounts.len(), MAX_ACCOUNTS);
    for index in 0..10 {
        assert_eq!(
            store.accounts.contains_key(&format!("account-{index}")),
            index >= 2,
            "account-{index}"
        );
    }
    let path = pushed.data.path.join(STORE);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    // A foreign or corrupt store is replaced by the next push, never trusted.
    fs::write(&path, b"{\"version\":2,\"accounts\":{}}").unwrap();
    assert!(pushed.store().is_none());
    assert!(pushed.push(&push_body(3.0, 4.0), NOW + 20).unwrap());
    assert_eq!(pushed.store().unwrap().accounts.len(), 1);
}

#[test]
fn without_a_data_root_the_cache_alone_answers() {
    let pushed = PushedFixture::new(&config());
    assert!(pushed.push(&push_body(20.0, 80.0), NOW).unwrap());
    let rootless =
        ProviderEnvironment::from_parts(&pushed.home.path, &pushed.home.path, Vec::new(), []);
    assert_eq!(read(&rootless, NOW).unwrap().observed_at_ms, FETCHED);
}
