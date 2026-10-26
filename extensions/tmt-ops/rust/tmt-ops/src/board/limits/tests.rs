use super::*;
use serde_json::json;

const NOW_MS: u64 = 1_791_600_000_000;
const HOUR: u64 = 3_600_000;
const MINUTE: u64 = 60_000;

/// A footer 10 minutes into the week's window and an hour into the short one.
const FOOTER: &str = "ctx 89%   5h 97.0% (4h00m)  7d 56.0% (4d03h)";

fn claude(name: &str, observed: u64) -> Value {
    json!({"name": name, "presence": "active",
           "resume": {"driver": "claude", "usage": {"observedAtMs": observed}}})
}

fn codex(left_used: f64, observed: u64) -> Value {
    json!({"name": "x", "presence": "active", "resume": {"driver": "codex",
        "consumption": {"rateLimits": {"observedAtMs": observed, "windows": [
            {"windowMinutes": 10080, "usedPercent": left_used, "resetsAtMs": NOW_MS + 100 * HOUR}]}}}})
}

fn listed(rows: Vec<Value>) -> Value {
    json!({"identities": rows})
}

struct Panes {
    calls: Vec<String>,
    footer: Option<&'static str>,
}

impl Panes {
    fn showing(footer: &'static str) -> Self {
        Self {
            calls: Vec::new(),
            footer: Some(footer),
        }
    }

    fn capture(&mut self) -> impl FnMut(&str) -> Option<String> + '_ {
        |name| {
            self.calls.push(name.to_owned());
            self.footer.map(str::to_owned)
        }
    }
}

fn standing(snapshot: &Snapshot, provider: Provider) -> Standing {
    snapshot
        .iter()
        .find(|s| s.provider == provider)
        .unwrap()
        .clone()
}

#[test]
fn codex_comes_from_the_listing_with_no_pane_read() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    let snapshot = sampler.sample(
        &listed(vec![codex(25.0, NOW_MS)]),
        NOW_MS,
        Instant::now(),
        &mut panes.capture(),
    );
    assert!(panes.calls.is_empty());
    assert_eq!(snapshot.len(), 1);
    let codex = standing(&snapshot, Provider::named("codex"));
    assert_eq!(codex.latest.unwrap().weekly.left, 75.0);
    assert_eq!(codex.burn, None, "one sample is not a rate");
}

#[test]
fn sampled_history_carries_derived_trend_without_an_extra_capture() {
    let mut sampler = Sampler::new(None);
    let now = Instant::now();
    let mut panes = Panes::showing(FOOTER);
    let mut last = Vec::new();
    for (hours, used) in [(0, 10.0), (4, 14.0), (6, 22.0)] {
        let observed = NOW_MS + hours * HOUR;
        last = sampler.sample(
            &listed(vec![codex(used, observed)]),
            observed,
            now,
            &mut panes.capture(),
        );
    }
    let codex = standing(&last, Provider::named("codex"));
    assert_eq!(codex.burn, Some(2.0));
    assert_eq!(codex.trend, Some(Trend::Up));
    assert_eq!(
        codex.outlook(NOW_MS + 6 * HOUR),
        Outlook::Known(Figures {
            weekly: estimate::Left {
                percent: 78.0,
                resets_in_ms: 94 * HOUR
            },
            short: None,
            burn: Some(2.0),
            trend: Some(Trend::Up),
            runs_out_before_reset: true,
        })
    );
    assert!(matches!(
        codex.outlook(NOW_MS + 8 * HOUR + 1),
        Outlook::Unknown { .. }
    ));
    assert!(panes.calls.is_empty());
}

#[test]
fn claude_is_read_from_the_freshest_pane_and_dated_by_its_usage_observation() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    let rows = vec![
        claude("old", NOW_MS - HOUR),
        claude("busy", NOW_MS - 5 * MINUTE),
    ];
    let snapshot = sampler.sample(&listed(rows), NOW_MS, Instant::now(), &mut panes.capture());
    assert_eq!(panes.calls, ["busy"]);
    let reading = standing(&snapshot, Provider::named("claude"))
        .latest
        .unwrap();
    assert_eq!(reading.observed_at_ms, NOW_MS - 5 * MINUTE);
    assert_eq!(reading.weekly.left, 56.0);
    assert_eq!(
        reading.weekly.resets_at_ms,
        NOW_MS - 5 * MINUTE + 5940 * MINUTE
    );
}

#[test]
fn the_next_pane_is_tried_when_a_footer_cannot_be_read() {
    let mut sampler = Sampler::new(None);
    let rows = vec![
        claude("a", NOW_MS),
        claude("b", NOW_MS - MINUTE),
        claude("c", NOW_MS - 2 * MINUTE),
    ];
    let mut calls = Vec::new();
    let snapshot = sampler.sample(&listed(rows), NOW_MS, Instant::now(), &mut |name| {
        calls.push(name.to_owned());
        (name == "c").then(|| FOOTER.to_owned())
    });
    assert_eq!(calls, ["a", "b", "c"]);
    assert_eq!(
        standing(&snapshot, Provider::named("claude"))
            .latest
            .unwrap()
            .observed_at_ms,
        NOW_MS - 2 * MINUTE
    );
}

#[test]
fn claude_panes_are_read_at_most_once_a_minute() {
    let mut sampler = Sampler::new(None);
    let start = Instant::now();
    let mut panes = Panes::showing(FOOTER);
    sampler.sample(
        &listed(vec![claude("a", NOW_MS)]),
        NOW_MS,
        start,
        &mut panes.capture(),
    );
    let later = listed(vec![claude("a", NOW_MS + 5 * MINUTE)]);
    sampler.sample(
        &later,
        NOW_MS,
        start + Duration::from_secs(30),
        &mut panes.capture(),
    );
    assert_eq!(panes.calls.len(), 1, "inside the minute");
    sampler.sample(
        &later,
        NOW_MS,
        start + Duration::from_secs(61),
        &mut panes.capture(),
    );
    assert_eq!(panes.calls.len(), 2);
}

#[test]
fn an_unchanged_claude_session_is_not_read_again() {
    let mut sampler = Sampler::new(None);
    let start = Instant::now();
    let mut panes = Panes::showing(FOOTER);
    let rows = listed(vec![claude("a", NOW_MS)]);
    sampler.sample(&rows, NOW_MS, start, &mut panes.capture());
    sampler.sample(
        &rows,
        NOW_MS,
        start + Duration::from_secs(600),
        &mut panes.capture(),
    );
    assert_eq!(panes.calls.len(), 1, "the footer cannot have changed");
}

#[test]
fn a_failed_read_waits_for_new_session_activity() {
    let mut sampler = Sampler::new(None);
    let start = Instant::now();
    let mut panes = Panes::showing("no footer here");
    let rows = listed(vec![claude("a", NOW_MS)]);
    let snapshot = sampler.sample(&rows, NOW_MS, start, &mut panes.capture());
    sampler.sample(
        &rows,
        NOW_MS,
        start + Duration::from_secs(10),
        &mut panes.capture(),
    );
    assert_eq!(panes.calls.len(), 1, "inside the minute");
    sampler.sample(
        &rows,
        NOW_MS,
        start + Duration::from_secs(600),
        &mut panes.capture(),
    );
    assert_eq!(panes.calls.len(), 1, "no new activity, so no new attempt");
    sampler.sample(
        &listed(vec![claude("a", NOW_MS + MINUTE)]),
        NOW_MS,
        start + Duration::from_secs(660),
        &mut panes.capture(),
    );
    assert_eq!(panes.calls.len(), 2, "new activity tries again");
    let claude = standing(&snapshot, Provider::named("claude"));
    assert_eq!(claude.latest, None, "listed but never read");
}

#[test]
fn a_driver_that_reports_its_own_limits_is_never_pane_read() {
    let mut sampler = Sampler::new(None);
    let mut row = codex(25.0, NOW_MS);
    row["resume"]["usage"] = json!({"observedAtMs": NOW_MS});
    let mut panes = Panes::showing(FOOTER);
    sampler.sample(
        &listed(vec![row]),
        NOW_MS,
        Instant::now(),
        &mut panes.capture(),
    );
    assert!(panes.calls.is_empty());
}

#[test]
fn any_driver_with_usage_and_no_limits_gets_its_footer_read() {
    let mut sampler = Sampler::new(None);
    let mut row = claude("a", NOW_MS);
    row["resume"]["driver"] = json!("newcomer");
    let mut panes = Panes::showing(FOOTER);
    let snapshot = sampler.sample(
        &listed(vec![row]),
        NOW_MS,
        Instant::now(),
        &mut panes.capture(),
    );
    assert_eq!(panes.calls, ["a"]);
    assert_eq!(snapshot[0].provider.label(), "Newcomer");
    assert_eq!(snapshot[0].latest.unwrap().weekly.left, 56.0);
}

#[test]
fn providers_are_shown_in_driver_order() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    let snapshot = sampler.sample(
        &listed(vec![codex(25.0, NOW_MS), claude("a", NOW_MS)]),
        NOW_MS,
        Instant::now(),
        &mut panes.capture(),
    );
    let order: Vec<_> = snapshot.iter().map(|s| s.provider.label()).collect();
    assert_eq!(order, ["Claude", "Codex"]);
}

#[test]
fn a_provider_without_sessions_or_history_is_not_shown() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    let snapshot = sampler.sample(
        &listed(vec![]),
        NOW_MS,
        Instant::now(),
        &mut panes.capture(),
    );
    assert!(snapshot.is_empty());
}

#[test]
fn history_keeps_a_provider_shown_after_its_sessions_leave() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    sampler.sample(
        &listed(vec![codex(25.0, NOW_MS)]),
        NOW_MS,
        Instant::now(),
        &mut panes.capture(),
    );
    let snapshot = sampler.sample(
        &listed(vec![]),
        NOW_MS + HOUR,
        Instant::now(),
        &mut panes.capture(),
    );
    assert_eq!(
        standing(&snapshot, Provider::named("codex"))
            .latest
            .unwrap()
            .observed_at_ms,
        NOW_MS
    );
}

#[test]
fn the_burn_follows_the_samples_across_ticks() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    for (hours, used) in [(0, 20.0), (1, 21.0), (3, 23.0)] {
        let at = NOW_MS + hours * HOUR;
        sampler.sample(
            &listed(vec![codex(used, at)]),
            at,
            Instant::now(),
            &mut panes.capture(),
        );
    }
    let snapshot = sampler.sample(
        &listed(vec![codex(23.0, NOW_MS + 3 * HOUR)]),
        NOW_MS + 3 * HOUR,
        Instant::now(),
        &mut panes.capture(),
    );
    assert_eq!(
        standing(&snapshot, Provider::named("codex")).burn,
        Some(1.0)
    );
}

const WORK: &str = "0123456789ab";
const HOME: &str = "ba9876543210";

fn account_row(
    driver: &str,
    name: &str,
    key: &str,
    label: &str,
    used: f64,
    observed: u64,
) -> Value {
    json!({"name": name, "presence": "active", "resume": {"driver": driver,
        "usage": {"observedAtMs": observed},
        "consumption": {"rateLimits": {"observedAtMs": observed,
            "account": {"key": key, "label": label},
            "windows": [{"windowMinutes": 10080, "usedPercent": used,
                         "resetsAtMs": NOW_MS + 100 * HOUR}]}}}})
}

fn names(snapshot: &Snapshot) -> Vec<(String, String)> {
    snapshot
        .iter()
        .map(|standing| (standing.provider.label(), standing.provider.board_name()))
        .collect()
}

#[test]
fn one_named_account_reads_exactly_like_the_plain_driver() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    let rows = vec![
        account_row("claude", "a", WORK, "work", 20.0, NOW_MS),
        claude("plain", NOW_MS - MINUTE),
    ];
    let snapshot = sampler.sample(&listed(rows), NOW_MS, Instant::now(), &mut panes.capture());
    assert_eq!(
        names(&snapshot),
        [("Claude".to_owned(), "claude".to_owned())]
    );
    assert_eq!(snapshot[0].latest.unwrap().weekly.left, 80.0);
    assert!(
        panes.calls.is_empty(),
        "the account's own limits need no footer"
    );
}

#[test]
fn several_accounts_are_named_and_the_sessions_without_one_are_the_unknown_account() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    let rows = vec![
        account_row("claude", "a", WORK, "work", 20.0, NOW_MS),
        account_row("claude", "b", HOME, "home", 60.0, NOW_MS),
        claude("plain", NOW_MS - MINUTE),
    ];
    let snapshot = sampler.sample(&listed(rows), NOW_MS, Instant::now(), &mut panes.capture());
    assert_eq!(
        names(&snapshot),
        [
            ("Claude · unknown account".to_owned(), "claude".to_owned()),
            ("Claude · work".to_owned(), format!("claude.{WORK}")),
            ("Claude · home".to_owned(), format!("claude.{HOME}")),
        ]
    );
    assert_eq!(snapshot[0].latest, None);
    assert_eq!(snapshot[1].latest.unwrap().weekly.left, 80.0);
    assert_eq!(snapshot[2].latest.unwrap().weekly.left, 40.0);
    assert!(panes.calls.is_empty());
}

#[test]
fn a_changed_label_keeps_the_account_and_its_history() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    let mut last = Vec::new();
    for (hours, used, label) in [(0, 20.0, "work"), (1, 21.0, "work"), (3, 23.0, "renamed")] {
        let at = NOW_MS + hours * HOUR;
        let rows = vec![account_row("codex", "a", WORK, label, used, at)];
        last = sampler.sample(&listed(rows), at, Instant::now(), &mut panes.capture());
    }
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].provider.account_label(), Some("renamed"));
    assert_eq!(
        last[0].burn,
        Some(1.0),
        "the same account, so the same history"
    );
}

#[test]
fn an_account_sampled_after_plain_readings_inherits_their_history() {
    let mut sampler = Sampler::new(None);
    let mut panes = Panes::showing(FOOTER);
    let mut last = Vec::new();
    for hours in [0, 1] {
        let at = NOW_MS + hours * HOUR;
        let used = 20.0 + hours as f64;
        last = sampler.sample(
            &listed(vec![codex(used, at)]),
            at,
            Instant::now(),
            &mut panes.capture(),
        );
    }
    assert_eq!(last[0].provider.account_key(), None);
    let at = NOW_MS + 3 * HOUR;
    let rows = vec![account_row("codex", "a", WORK, "work", 23.0, at)];
    let last = sampler.sample(&listed(rows), at, Instant::now(), &mut panes.capture());
    assert_eq!(last.len(), 1, "one account is the whole driver");
    assert_eq!(last[0].provider.account_key(), Some(WORK));
    assert_eq!(last[0].provider.label(), "Codex");
    assert_eq!(
        last[0].burn,
        Some(1.0),
        "the pace survives gaining the account"
    );
}

#[test]
fn accounts_survive_the_sessions_that_reported_them() {
    let directory =
        std::env::temp_dir().join(format!("ops-sampler-accounts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let mut panes = Panes::showing(FOOTER);
    let rows = vec![
        account_row("codex", "a", WORK, "work", 20.0, NOW_MS),
        account_row("codex", "b", HOME, "home", 60.0, NOW_MS),
    ];
    let mut sampler = Sampler::new(Some(directory.clone()));
    sampler.sample(&listed(rows), NOW_MS, Instant::now(), &mut panes.capture());
    // A new board with every session gone: the kept readings and their labels come back.
    let mut later = Sampler::new(Some(directory.clone()));
    let snapshot = later.sample(
        &listed(vec![]),
        NOW_MS + HOUR,
        Instant::now(),
        &mut panes.capture(),
    );
    assert_eq!(
        names(&snapshot),
        [
            ("Codex · work".to_owned(), format!("codex.{WORK}")),
            ("Codex · home".to_owned(), format!("codex.{HOME}")),
        ]
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn readings_kept_before_accounts_were_named_are_not_a_row_beside_two_accounts() {
    let directory = std::env::temp_dir().join(format!("ops-sampler-plain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let mut panes = Panes::showing(FOOTER);
    // Before the upgrade: one plain history, read from a Claude footer.
    let mut old = Sampler::new(Some(directory.clone()));
    let first = old.sample(
        &listed(vec![claude("a", NOW_MS)]),
        NOW_MS,
        Instant::now(),
        &mut panes.capture(),
    );
    assert_eq!(names(&first), [("Claude".to_owned(), "claude".to_owned())]);
    old.sample(
        &listed(vec![]),
        NOW_MS + 6 * MINUTE,
        Instant::now(),
        &mut panes.capture(),
    );
    // After it: two accounts, and no live session that names none.
    let rows = vec![
        account_row("claude", "a", WORK, "work", 20.0, NOW_MS + HOUR),
        account_row("claude", "b", HOME, "home", 60.0, NOW_MS + HOUR),
    ];
    let mut new = Sampler::new(Some(directory.clone()));
    let snapshot = new.sample(
        &listed(rows.clone()),
        NOW_MS + HOUR,
        Instant::now(),
        &mut panes.capture(),
    );
    assert_eq!(
        names(&snapshot),
        [
            ("Claude · work".to_owned(), format!("claude.{WORK}")),
            ("Claude · home".to_owned(), format!("claude.{HOME}")),
        ],
        "no third row with a mixed balance"
    );
    assert!(
        snapshot
            .iter()
            .all(|standing| standing.latest.unwrap().observed_at_ms == NOW_MS + HOUR),
        "the old readings are no account's history either"
    );
    // A live session that names none is the unknown account.
    let mut with_plain = rows;
    with_plain.push(claude("c", NOW_MS + HOUR));
    let snapshot = new.sample(
        &listed(with_plain),
        NOW_MS + HOUR,
        Instant::now(),
        &mut panes.capture(),
    );
    assert_eq!(snapshot.len(), 3);
    assert_eq!(snapshot[0].provider.label(), "Claude · unknown account");
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn several_accounts_are_read_from_the_standings_and_count_accounts_only_the_history_holds() {
    let directory =
        std::env::temp_dir().join(format!("ops-sampler-several-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let mut panes = Panes::showing(FOOTER);
    let both = vec![
        account_row("claude", "a", WORK, "work", 20.0, NOW_MS),
        account_row("claude", "b", HOME, "home", 60.0, NOW_MS),
    ];
    let mut earlier = Sampler::new(Some(directory.clone()));
    earlier.sample(&listed(both), NOW_MS, Instant::now(), &mut panes.capture());
    earlier.sample(
        &listed(vec![]),
        NOW_MS + 6 * MINUTE,
        Instant::now(),
        &mut panes.capture(),
    );
    // Only work has a live seat; home is held by the history alone.
    let mut sampler = Sampler::new(Some(directory.clone()));
    let live = vec![account_row(
        "claude",
        "a",
        WORK,
        "work",
        21.0,
        NOW_MS + HOUR,
    )];
    let snapshot = sampler.sample(
        &listed(live),
        NOW_MS + HOUR,
        Instant::now(),
        &mut panes.capture(),
    );
    assert_eq!(
        several_accounts(&snapshot),
        std::collections::BTreeSet::from(["claude".to_owned()])
    );
    // One account anywhere is not several.
    let single = Sampler::new(None).sample(
        &listed(vec![account_row("claude", "a", WORK, "work", 21.0, NOW_MS)]),
        NOW_MS,
        Instant::now(),
        &mut panes.capture(),
    );
    assert!(several_accounts(&single).is_empty());
    let _ = std::fs::remove_dir_all(directory);
}
