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
