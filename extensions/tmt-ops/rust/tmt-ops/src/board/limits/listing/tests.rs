use super::*;
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
