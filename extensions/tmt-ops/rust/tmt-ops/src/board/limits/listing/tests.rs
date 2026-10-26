use super::*;
use crate::board::limits::Window;
use serde_json::json;

const OBSERVED: u64 = 1_791_604_109_763;
const RESET: u64 = 1_791_948_558_000;

fn window(minutes: u64, used: f64, resets_at_ms: u64) -> Value {
    json!({"windowMinutes": minutes, "usedPercent": used, "resetsAtMs": resets_at_ms})
}

fn limits(observed: u64, windows: Vec<Value>) -> Value {
    json!({"observedAtMs": observed, "windows": windows})
}

fn codex_row(name: &str, rate_limits: Value) -> Value {
    json!({
        "name": name,
        "presence": "active",
        "resume": {"driver": "codex", "usage": {"observedAtMs": 1},
                   "consumption": {"rateLimits": rate_limits}},
    })
}

fn claude_row(name: &str, presence: &str, observed: Option<u64>) -> Value {
    let usage = observed.map_or(Value::Null, |ms| json!({"observedAtMs": ms}));
    json!({"name": name, "presence": presence, "resume": {"driver": "claude", "usage": usage}})
}

fn listed(rows: Vec<Value>) -> Value {
    json!({"identities": rows})
}

fn codex(listed: &Value) -> Option<Reading> {
    reported(listed).remove(&Provider::named("codex"))
}

fn candidates(listed: &Value, driver: &str) -> Vec<Candidate> {
    footer_candidates(listed)
        .remove(&Provider::named(driver))
        .unwrap_or_default()
}

#[test]
fn codex_windows_become_percent_left_with_the_longest_as_the_week() {
    let row = codex_row(
        "a",
        limits(
            OBSERVED,
            vec![window(300, 12.5, RESET - 1000), window(10_080, 78.0, RESET)],
        ),
    );
    let reading = codex(&listed(vec![row])).unwrap();
    assert_eq!(reading.observed_at_ms, OBSERVED);
    assert_eq!(
        reading.weekly,
        Window {
            left: 22.0,
            resets_at_ms: RESET
        }
    );
    assert_eq!(
        reading.short,
        Some(Window {
            left: 87.5,
            resets_at_ms: RESET - 1000
        })
    );
}

#[test]
fn a_short_window_of_another_length_is_left_out_not_mislabelled() {
    let row = codex_row(
        "a",
        limits(
            OBSERVED,
            vec![window(60, 5.0, RESET - 1000), window(10_080, 78.0, RESET)],
        ),
    );
    let reading = codex(&listed(vec![row])).unwrap();
    assert_eq!(reading.short, None);
    assert_eq!(reading.weekly.left, 22.0);
}

#[test]
fn a_lone_weekly_window_has_no_short_window() {
    let row = codex_row("a", limits(OBSERVED, vec![window(10_080, 0.0, RESET)]));
    let reading = codex(&listed(vec![row])).unwrap();
    assert_eq!(reading.weekly.left, 100.0);
    assert_eq!(reading.short, None);
}

#[test]
fn the_newest_session_of_a_provider_wins_and_providers_stay_apart() {
    let older = codex_row("old", limits(OBSERVED, vec![window(10_080, 50.0, RESET)]));
    let newer = codex_row(
        "new",
        limits(OBSERVED + 5000, vec![window(10_080, 60.0, RESET)]),
    );
    let mut claude = claude_row("c", "active", Some(OBSERVED + 9000));
    claude["resume"]["consumption"] =
        json!({"rateLimits": limits(OBSERVED + 9000, vec![window(10_080, 1.0, RESET)])});
    let listing = listed(vec![older, newer, claude]);
    assert_eq!(codex(&listing).unwrap().weekly.left, 40.0);
    let other = reported(&listing)
        .remove(&Provider::named("claude"))
        .unwrap();
    assert_eq!(other.weekly.left, 99.0);
}

#[test]
fn a_listing_without_usable_limits_reports_none() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "no consumption",
            json!({"name": "a", "resume": {"driver": "codex"}}),
        ),
        ("null limits", codex_row("a", Value::Null)),
        ("no windows", codex_row("a", limits(OBSERVED, vec![]))),
        (
            "no week",
            codex_row("a", limits(OBSERVED, vec![window(300, 10.0, RESET)])),
        ),
        (
            "a 3-day window is not a week",
            codex_row("a", limits(OBSERVED, vec![window(4320, 10.0, RESET)])),
        ),
        (
            "percent above 100",
            codex_row("a", limits(OBSERVED, vec![window(10_080, 100.5, RESET)])),
        ),
        (
            "negative percent",
            codex_row("a", limits(OBSERVED, vec![window(10_080, -1.0, RESET)])),
        ),
        (
            "zero reset",
            codex_row("a", limits(OBSERVED, vec![window(10_080, 10.0, 0)])),
        ),
        (
            "zero observation",
            codex_row("a", limits(0, vec![window(10_080, 10.0, RESET)])),
        ),
        (
            "one bad window rejects the whole object",
            codex_row(
                "a",
                limits(
                    OBSERVED,
                    vec![window(300, 200.0, RESET), window(10_080, 10.0, RESET)],
                ),
            ),
        ),
    ];
    for (name, row) in cases {
        assert_eq!(codex(&listed(vec![row])), None, "{name}");
    }
    assert_eq!(codex(&json!({})), None);
    assert_eq!(codex(&json!({"identities": "x"})), None);
}

#[test]
fn footer_candidates_are_active_sessions_with_usage_newest_first_and_capped() {
    let rows = vec![
        claude_row("quiet", "active", Some(100)),
        claude_row("busy", "active", Some(900)),
        claude_row("away", "offline", Some(950)),
        claude_row("no-usage", "active", None),
        claude_row("tie-b", "active", Some(500)),
        claude_row("tie-a", "active", Some(500)),
        codex_row("not-claude", Value::Null),
    ];
    let listing = listed(rows);
    let names: Vec<_> = candidates(&listing, "claude")
        .into_iter()
        .map(|c| (c.name, c.observed_at_ms))
        .collect();
    assert_eq!(
        names,
        [
            ("busy".into(), 900),
            ("tie-a".into(), 500),
            ("tie-b".into(), 500)
        ]
    );
    assert_eq!(candidates(&listing, "codex").len(), 1);
}

#[test]
fn a_driver_that_reports_no_usage_has_no_footer_candidates() {
    let plain = json!({"name": "g", "presence": "active", "resume": {"driver": "gemini"}});
    assert!(footer_candidates(&listed(vec![plain])).is_empty());
}

#[test]
fn only_plain_driver_ids_name_a_provider() {
    for id in ["claude", "codex", "my-driver_2"] {
        assert_eq!(Provider::from_driver(id).unwrap().id(), id);
    }
    for id in ["", "two words", "a/b", "über", "x".repeat(25).as_str()] {
        assert_eq!(Provider::from_driver(id), None, "{id:?}");
    }
    assert_eq!(Provider::named("claude").label(), "Claude");
    assert_eq!(Provider::named("x").label(), "X");
}

#[test]
fn present_names_every_provider_with_a_listed_session() {
    let rows = vec![claude_row("c", "offline", None), json!({"name": "plain"})];
    assert_eq!(
        present(&listed(rows)),
        BTreeSet::from([Provider::named("claude")])
    );
    let rows = vec![
        codex_row("x", Value::Null),
        claude_row("c", "active", Some(1)),
    ];
    assert_eq!(present(&listed(rows)).len(), 2);
    assert!(present(&listed(vec![])).is_empty());
}

const WORK: &str = "0123456789ab";
const HOME: &str = "ba9876543210";

fn account_row(name: &str, key: &str, label: &str, used: f64, observed: u64) -> Value {
    let mut rate_limits = limits(observed, vec![window(10_080, used, RESET)]);
    rate_limits["account"] = json!({"key": key, "label": label});
    let mut row = codex_row(name, rate_limits);
    row["resume"]["driver"] = json!("claude");
    row
}

fn keys(providers: impl IntoIterator<Item = Provider>) -> Vec<String> {
    providers
        .into_iter()
        .map(|provider| provider.key())
        .collect()
}

#[test]
fn two_accounts_of_one_driver_are_two_providers_each_with_its_newest_reading_and_label() {
    let listing = listed(vec![
        account_row("a", WORK, "work", 20.0, OBSERVED),
        account_row("b", HOME, "home", 60.0, OBSERVED + 1000),
        account_row("c", WORK, "work-renamed", 30.0, OBSERVED + 2000),
    ]);
    let found = reported(&listing);
    assert_eq!(
        keys(found.keys().cloned()),
        [format!("claude.{WORK}"), format!("claude.{HOME}")]
    );
    let (work, reading) = found
        .iter()
        .find(|(provider, _)| provider.account_key() == Some(WORK))
        .unwrap();
    assert_eq!(work.account_label(), Some("work-renamed"));
    assert_eq!(reading.weekly.left, 70.0);
    assert!(
        found
            .iter()
            .any(|(provider, reading)| provider.account_key() == Some(HOME)
                && reading.weekly.left == 40.0)
    );
}

#[test]
fn a_session_without_an_account_stays_the_plain_driver() {
    let mut named = account_row("a", WORK, "work", 20.0, OBSERVED);
    let mut plain = named.clone();
    plain["resume"]["consumption"]["rateLimits"]
        .as_object_mut()
        .unwrap()
        .remove("account");
    plain["name"] = json!("plain");
    plain["resume"]["consumption"]["rateLimits"]["observedAtMs"] = json!(OBSERVED + 5);
    named["name"] = json!("named");
    let found = reported(&listed(vec![named, plain]));
    assert_eq!(
        keys(found.keys().cloned()),
        ["claude".to_owned(), format!("claude.{WORK}")]
    );
}

#[test]
fn a_malformed_account_makes_the_session_report_nothing() {
    for bad in [
        json!({"key": "0123456789AB", "label": "work"}),
        json!({"key": WORK, "label": "has space"}),
        json!({"key": WORK}),
        json!("work"),
    ] {
        let mut row = account_row("a", WORK, "work", 20.0, OBSERVED);
        row["resume"]["consumption"]["rateLimits"]["account"] = bad.clone();
        assert!(reported(&listed(vec![row])).is_empty(), "{bad}");
    }
}

#[test]
fn one_named_account_is_the_whole_driver_and_the_plain_driver_beside_several_is_only_a_live_one() {
    let plain = claude_row("plain", "active", Some(OBSERVED));
    let work = account_row("a", WORK, "work", 20.0, OBSERVED);
    let home = account_row("b", HOME, "home", 60.0, OBSERVED);
    let shown =
        |rows: Vec<Value>, kept: Vec<Provider>| keys(providers(&present(&listed(rows)), kept));
    let kept_plain = || vec![Provider::named("claude")];
    let kept_account = |key: &str| Provider::from_key(&format!("claude.{key}")).unwrap();
    assert_eq!(
        shown(vec![plain.clone(), work.clone()], vec![]),
        [format!("claude.{WORK}")],
        "one account folds the sessions that name none into itself"
    );
    assert_eq!(
        shown(vec![plain.clone(), work.clone(), home.clone()], vec![]),
        [
            "claude".to_owned(),
            format!("claude.{WORK}"),
            format!("claude.{HOME}")
        ],
        "a live session that names none stays beside several"
    );
    assert_eq!(shown(vec![plain.clone()], vec![]), ["claude"]);
    // Readings kept before accounts were named are not a row once two accounts exist.
    assert_eq!(
        shown(vec![work.clone(), home.clone()], kept_plain()),
        [format!("claude.{WORK}"), format!("claude.{HOME}")]
    );
    // Accounts that only the history holds still count: two kept accounts, one live.
    assert_eq!(
        shown(
            vec![work.clone()],
            vec![kept_account(HOME), Provider::named("claude")]
        ),
        [format!("claude.{WORK}"), format!("claude.{HOME}")]
    );
    // A single account absorbs the kept plain readings as well.
    assert_eq!(shown(vec![work], kept_plain()), [format!("claude.{WORK}")]);
    // No account anywhere: the plain driver, live or only kept.
    assert_eq!(shown(vec![], kept_plain()), ["claude"]);
}

#[test]
fn a_provider_key_round_trips_and_only_a_hex_account_key_names_an_account() {
    for key in [
        "claude",
        &format!("claude.{WORK}"),
        &format!("my-driver_2.{HOME}"),
    ] {
        assert_eq!(Provider::from_key(key).unwrap().key(), key);
    }
    for key in [
        "",
        "claude.",
        "claude.0123456789AB",
        "claude.0123456789abc",
        &format!("claude.{WORK}.extra"),
        &format!("two words.{WORK}"),
    ] {
        assert!(Provider::from_key(key).is_none(), "{key:?}");
    }
    let named = Provider::from_key(&format!("claude.{WORK}")).unwrap();
    assert_eq!(named.id(), "claude");
    assert_eq!(
        named.board_name(),
        "claude",
        "one account reads as the driver"
    );
    assert_eq!(
        named.clone().showing_account(true).board_name(),
        format!("claude.{WORK}")
    );
    assert_eq!(
        named.clone().labelled("work").showing_account(true).label(),
        "Claude · work"
    );
    assert_eq!(
        named,
        Provider::from_key(&format!("claude.{WORK}"))
            .unwrap()
            .labelled("other")
    );
}
