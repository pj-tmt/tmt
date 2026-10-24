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
