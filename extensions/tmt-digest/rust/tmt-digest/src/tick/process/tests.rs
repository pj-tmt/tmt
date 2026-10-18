//! Public-process fixtures exercise policy writes and fresh delivery admission.
use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const MEMBER: &str = "10000000-0000-4000-8000-000000000001";
const SETTER: &str = "20000000-0000-4000-8000-000000000001";
const OWNER: &str = "30000000-0000-4000-8000-000000000001";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        // The store refuses a symlinked path, and macOS's temp dir is under the `/var` symlink.
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "tmt-digest-port-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        let script = r#"#!/bin/sh
cd '__ROOT__' || exit 9
case "$*" in
 'identity ls --json') cat inventory.json ;;
 'api') input=$(cat); printf '%s\n' "$input" >> calls
   case "$input" in
    *'"operation":"digest.checklist.history"'*) cat history.json ;;
    *'"operation":"digest.policy.show"'*) cat policies.json ;;
    *'"operation":"digest.stats.show"'*) cat stats.json ;;
    *'"operation":"digest.checklist.flush"'*)
      if test -f slow-flush; then sleep 2.1; fi
      if test -f check-fails; then state=uncertain; else state=not_idle; fi
      cat flush-$state.json ;;
    *) printf '%s\n' '{"ok":true}' ;;
   esac ;;
 *) exit 9 ;;
esac
"#
        .replace("__ROOT__", fixture.0.to_str().unwrap());
        fs::write(fixture.0.join("core"), script).unwrap();
        fs::set_permissions(fixture.0.join("core"), fs::Permissions::from_mode(0o700)).unwrap();
        fixture.write(
            "inventory.json",
            json!({"identities":[{"id":MEMBER,"name":"worker"}]}),
        );
        fixture.policy(0, false);
        fixture.stats(10, 0);
        fixture.settings("20s", None);
        fixture.write(
            "flush-not_idle.json",
            json!({"identityId":MEMBER,"state":"not_idle"}),
        );
        fixture.write(
            "flush-uncertain.json",
            json!({"identityId":MEMBER,"state":"uncertain"}),
        );
        fixture
    }
    fn write(&self, file: &str, value: Value) {
        fs::write(self.0.join(file), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn policy(&self, revision: u64, active: bool) {
        self.write("policies.json", json!({"policies":[{"identityId":MEMBER,"revision":revision,"active":active,"ownerIdentityId":OWNER,"setterIdentityId":SETTER}]}));
    }
    fn stats(&self, held: u64, age: u64) {
        self.observed(held, age, super::super::now_ms().unwrap());
    }
    fn observed(&self, held: u64, age: u64, now: u64) {
        self.write("stats.json", json!({"stats":[{"identityId":MEMBER,"heldCount":held,"oldestHeldAgeMs":if held==0 {Value::Null} else {json!(age)},"observedAtMs":now}]}));
    }
    /// `held` messages, the oldest of which arrived at `arrived`. Core derives the
    /// age and the observation time from one reading of the clock, so the arrival
    /// (`observedAtMs - oldestHeldAgeMs`) is exactly the same at every call.
    fn arrived(&self, held: u64, arrived: u64) {
        let now = super::super::now_ms().unwrap();
        self.observed(held, now - arrived, now);
    }
    fn settings(&self, mode: &str, setter: Option<&str>) {
        let attribution = setter
            .map(|id| format!("setByIdentityId = '{id}'\n"))
            .unwrap_or_default();
        fs::write(
            self.0.join("digest.toml"),
            format!("[members.'{MEMBER}']\nmode = '{mode}'\nsetAtMs = 1\n{attribution}"),
        )
        .unwrap();
    }
    fn port(&self) -> TickProcess {
        TickProcess::new(
            Core::fixture(self.0.join("core")),
            self.0.join("digest.toml"),
            super::super::now_ms().unwrap() + 60_000,
            Instant::now() + Duration::from_secs(60),
        )
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.0.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn writes(&self) -> Vec<Value> {
        self.calls()
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|row| {
                matches!(
                    row["operation"].as_str(),
                    Some("digest.policy.set" | "digest.policy.clear" | "digest.checklist.dueNow")
                )
            })
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn new_hold_uses_known_setter_or_member_and_existing_hold_preserves_attribution() {
    for (revision, configured, expected_owner, expected_setter) in [
        (0, None, MEMBER, MEMBER),
        (0, Some(SETTER), SETTER, SETTER),
        (7, Some(MEMBER), OWNER, SETTER),
    ] {
        let fixture = Fixture::new();
        fixture.policy(revision, revision > 0);
        fixture.settings("20s", configured);
        let mut port = fixture.port();
        port.observe().unwrap();
        fixture.policy(revision + 1, true);
        port.observe().unwrap();
        let writes = fixture.writes();
        assert_eq!(
            writes.len(),
            1,
            "only one renewal per unchanged tick setting"
        );
        assert_eq!(writes[0]["operation"], "digest.policy.set");
        let input = &writes[0]["input"];
        assert_eq!(input["expectedRevision"], revision);
        assert_eq!(input["ownerIdentityId"], expected_owner);
        assert_eq!(input["setterIdentityId"], expected_setter);
        assert!(input["untilMs"].as_u64().unwrap() >= super::super::now_ms().unwrap() + 119_000);
        port.finish().unwrap();
    }
}

#[test]
fn due_interval_captures_before_pinned_flush_and_uncertainty_is_not_retried() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("check-fails"), "").unwrap();
    let mut port = fixture.port();
    port.observe().unwrap();
    port.deliver_if_current(MEMBER).unwrap();
    port.deliver_if_current(MEMBER).unwrap();
    port.observe().unwrap();
    let calls = fixture.calls();
    let due = calls
        .iter()
        .position(|line| line.contains("digest.checklist.dueNow"))
        .unwrap();
    let check = calls
        .iter()
        .position(|line| line.contains("digest.checklist.flush"))
        .unwrap();
    assert!(due < check);
    assert_eq!(
        calls
            .iter()
            .filter(|line| line.contains("digest.checklist.flush"))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|line| line.contains("digest.checklist.dueNow"))
            .count(),
        1
    );
    assert!(
        port.finish()
            .unwrap_err()
            .message
            .contains("DELIVERY_UNCERTAIN")
    );
}

#[test]
fn fresh_settings_can_postpone_or_remove_interval_hold_before_delivery() {
    for changed in ["1h", "off", "auto"] {
        let fixture = Fixture::new();
        fixture.stats(1, 30_000);
        let mut port = fixture.port();
        port.observe().unwrap();
        fixture.policy(8, true);
        fixture.settings(changed, None);
        port.deliver_if_current(MEMBER).unwrap();
        let calls = fixture.calls();
        assert!(
            !calls
                .iter()
                .any(|line| line.contains("digest.checklist.dueNow"))
        );
        let writes = fixture.writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            writes[1]["operation"],
            if changed == "1h" {
                "digest.policy.set"
            } else {
                "digest.policy.clear"
            }
        );
        assert_eq!(
            calls
                .iter()
                .filter(|line| line.contains("digest.checklist.flush"))
                .count(),
            usize::from(changed != "1h")
        );
        port.finish().unwrap();
    }
}

#[test]
fn elapsed_boundary_starts_no_core_process_even_when_wall_clock_is_earlier() {
    let fixture = Fixture::new();
    let mut port = TickProcess::new(
        Core::fixture(fixture.0.join("core")),
        fixture.0.join("digest.toml"),
        u64::MAX,
        Instant::now(),
    );
    assert!(port.observe().unwrap().is_empty());
    port.deliver_if_current(MEMBER).unwrap();
    assert!(fixture.calls().is_empty());
}

#[test]
fn thirty_members_use_serialized_public_process_opportunities_with_measured_elapsed_time() {
    let fixtures: Vec<_> = (1..=30)
        .map(|i| {
            let fixture = Fixture::new();
            let id = format!("10000000-0000-4000-8000-{i:012}");
            for file in [
                "inventory.json",
                "policies.json",
                "stats.json",
                "digest.toml",
                "flush-not_idle.json",
                "flush-uncertain.json",
            ] {
                let path = fixture.0.join(file);
                let text = fs::read_to_string(&path).unwrap().replace(MEMBER, &id);
                fs::write(path, text).unwrap();
            }
            (fixture, id)
        })
        .collect();
    let started = Instant::now();
    for (fixture, id) in &fixtures {
        let mut port = fixture.port();
        let rows = port.observe().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].deadline(super::super::now_ms().unwrap()).is_some());
        port.deliver_if_current(id).unwrap();
        port.finish().unwrap();
        let calls = fixture.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|line| line.contains("digest.checklist.dueNow"))
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|line| line.contains("digest.checklist.flush"))
                .count(),
            1
        );
    }
    eprintln!(
        "30 serialized public-process fixture opportunities: {:?} (fixture Core; no live transport)",
        started.elapsed()
    );
}

#[test]
fn already_started_core_call_finishes_after_boundary_then_no_new_call_starts() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("slow-flush"), "").unwrap();
    let boundary = Instant::now() + Duration::from_secs(2);
    let mut port = TickProcess::new(
        Core::fixture(fixture.0.join("core")).until(boundary + Duration::from_secs(15)),
        fixture.0.join("digest.toml"),
        u64::MAX,
        boundary,
    );
    let held = TickObservation {
        identity_id: MEMBER.into(),
        mode: Mode::Off,
        flush_count: 10,
        held_count: 1,
        oldest_held_at_ms: Some(0),
    };
    port.flush(&held, Rule::Off).unwrap();
    assert!(Instant::now() >= boundary);
    assert!(port.observe().unwrap().is_empty());
    port.deliver_if_current(MEMBER).unwrap();
    port.finish().unwrap();
    assert_eq!(fixture.calls().len(), 1);
    assert!(fixture.calls()[0].contains("digest.checklist.flush"));
}

#[test]
fn unavailable_history_reports_partial_without_disabling_policy_or_authorizing_delivery() {
    let fixture = Fixture::new();
    let mut port = fixture.port();
    port.attach_history(Err(Error::new(
        "DIGEST_HISTORY_IO",
        "fixture unavailable history",
    )));
    port.observe().unwrap();
    assert_eq!(fixture.writes().len(), 1);
    assert!(
        !fixture
            .calls()
            .iter()
            .any(|line| line.contains("digest.checklist.flush"))
    );
    assert!(
        port.finish()
            .unwrap_err()
            .message
            .contains("fixture unavailable history")
    );
}

#[test]
fn usage_collection_failure_keeps_policy_and_delivery_admission_independent() {
    let fixture = Fixture::new();
    let now = super::super::now_ms().unwrap();
    fixture.write("history.json",json!({"identityId":MEMBER,"observedAtMs":now,"throughSequence":0,"deliveredDigests":0,"checklists":[],"nextAfter":null}));
    let store = crate::history::Store::open(&fixture.0, true).unwrap();
    let mut port = fixture.port();
    port.attach_history(Ok(store));
    port.observe().unwrap();
    assert_eq!(fixture.writes().len(), 1);
    assert!(port.failed.is_empty());
    port.deliver_if_current(MEMBER).unwrap();
    assert!(port.failed.is_empty());
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|line| line.contains("digest.checklist.flush"))
            .count(),
        1
    );
    let usage: Vec<Value> = fixture
        .calls()
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|row: &Value| row["operation"] == "consumption.history")
        .collect();
    assert_eq!(usage.len(), 2);
    assert!(
        usage.iter().all(|row| row["input"]
            == json!({"identityIds":[MEMBER],"windowsMs":[600000],"maxBuckets":120}))
    );
    assert!(
        port.finish()
            .unwrap_err()
            .message
            .contains("Invalid usage asOfMs")
    );
}

/// A store the port writes and a reader sees the way `tmt digest decisions` does.
fn recorded(fixture: &Fixture, port: &mut TickProcess) {
    port.attach_history(Ok(crate::history::Store::open(&fixture.0, true).unwrap()));
}
fn decisions(fixture: &Fixture) -> Vec<Decision> {
    crate::history::Store::open(&fixture.0, false)
        .unwrap()
        .unwrap()
        .decisions(0, crate::history::MAX)
        .unwrap()
}

#[test]
fn every_rule_records_one_content_free_row_with_cores_answer() {
    // (mode, held, age of the oldest, expected rule, expected Core answer)
    for (mode, held, age, rule, outcome) in [
        ("auto", 3, 5_000, Rule::AutoIdlePoint, Some("not_idle")),
        ("off", 3, 5_000, Rule::Off, Some("not_idle")),
        ("20s", 1, 30_000, Rule::IntervalDeadline, Some("not_idle")),
        ("20s", 10, 1_000, Rule::FlushCount, Some("not_idle")),
        ("20s", 1, 1_000, Rule::NotDue, None),
    ] {
        let fixture = Fixture::new();
        fixture.settings(mode, None);
        fixture.stats(held, age);
        let mut port = fixture.port();
        recorded(&fixture, &mut port);
        for row in port.observe().unwrap() {
            if matches!(row.mode, Mode::Interval { .. }) {
                match row.rule(super::super::now_ms().unwrap()) {
                    Some(Rule::NotDue) => port.not_due(&row),
                    Some(_) => port.deliver_if_current(&row.identity_id).unwrap(),
                    None => {}
                }
            }
        }
        let rows = decisions(&fixture);
        assert_eq!(rows.len(), 1, "{mode} held {held}: {rows:?}");
        let row = &rows[0];
        assert_eq!(row.rule, rule, "{mode}");
        assert_eq!(row.outcome, outcome, "{mode}");
        assert_eq!(row.identity_id, MEMBER);
        assert_eq!((row.held_count, row.flush_count), (held, 10));
        assert!(row.oldest_held_age_ms.unwrap() >= age);
        assert_eq!(
            row.interval_ms,
            matches!(
                rule,
                Rule::IntervalDeadline | Rule::FlushCount | Rule::NotDue
            )
            .then_some(20_000)
        );
    }
}

#[test]
fn a_member_holding_nothing_records_nothing_and_a_repeat_is_not_recorded_twice() {
    let fixture = Fixture::new();
    fixture.settings("auto", None);
    fixture.stats(0, 0);
    let mut port = fixture.port();
    recorded(&fixture, &mut port);
    port.observe().unwrap();
    assert!(decisions(&fixture).is_empty());
    // The loop sees the same decision about once a second; one row stays.
    fixture.stats(2, 1_000);
    for _ in 0..3 {
        port.observe().unwrap();
    }
    let rows = decisions(&fixture);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rule, Rule::AutoIdlePoint);
    // A different answer from Core is a new decision.
    fixture.write(
        "flush-not_idle.json",
        json!({"identityId":MEMBER,"state":"delivered"}),
    );
    port.observe().unwrap();
    let outcomes: Vec<_> = decisions(&fixture).iter().map(|row| row.outcome).collect();
    assert_eq!(outcomes, [Some("not_idle"), Some("delivered")]);
}

#[test]
fn an_uncertain_flush_is_recorded_and_a_later_postponement_is_a_new_decision() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("check-fails"), "").unwrap();
    fixture.stats(1, 30_000);
    let mut port = fixture.port();
    recorded(&fixture, &mut port);
    port.observe().unwrap();
    port.deliver_if_current(MEMBER).unwrap();
    // The fresh reading before the opportunity finds the interval raised.
    let other = Fixture::new();
    other.stats(1, 30_000);
    let mut postponed = other.port();
    recorded(&other, &mut postponed);
    postponed.observe().unwrap();
    other.policy(8, true);
    other.settings("1h", None);
    postponed.deliver_if_current(MEMBER).unwrap();
    assert_eq!(
        decisions(&fixture)
            .iter()
            .map(|row| (row.rule, row.outcome))
            .collect::<Vec<_>>(),
        [(Rule::IntervalDeadline, Some("uncertain"))]
    );
    let rows = decisions(&other);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].rule, rows[0].outcome), (Rule::NotDue, None));
    assert_eq!(rows[0].interval_ms, Some(3_600_000));
}

#[test]
fn a_settings_change_before_the_opportunity_records_the_rule_it_became() {
    for (changed, rule) in [("off", Rule::Off), ("auto", Rule::AutoIdlePoint)] {
        let fixture = Fixture::new();
        fixture.stats(2, 30_000);
        let mut port = fixture.port();
        recorded(&fixture, &mut port);
        port.observe().unwrap();
        fixture.policy(8, true);
        fixture.settings(changed, None);
        port.deliver_if_current(MEMBER).unwrap();
        let rows = decisions(&fixture);
        assert_eq!(rows.len(), 1, "{changed}");
        assert_eq!((rows[0].rule, rows[0].outcome), (rule, Some("not_idle")));
        assert_eq!(rows[0].interval_ms, None);
    }
}

fn rows(fixture: &Fixture) -> Vec<(Rule, Option<&'static str>)> {
    decisions(fixture)
        .iter()
        .map(|row| (row.rule, row.outcome))
        .collect()
}

#[test]
fn a_state_that_lasts_across_ticks_writes_one_row_and_a_new_episode_or_answer_writes_another() {
    let fixture = Fixture::new();
    fixture.settings("auto", None);
    let arrived = super::super::now_ms().unwrap() - 60_000;
    // Three one-minute ticks: the same busy agent holding the same message.
    for _ in 0..3 {
        fixture.arrived(2, arrived);
        let mut tick = fixture.port();
        recorded(&fixture, &mut tick);
        tick.observe().unwrap();
        tick.observe().unwrap();
    }
    assert_eq!(rows(&fixture), [(Rule::AutoIdlePoint, Some("not_idle"))]);
    // The same rule and answer for a newer oldest message is a new episode.
    fixture.arrived(1, arrived + 30_000);
    let mut tick = fixture.port();
    recorded(&fixture, &mut tick);
    tick.observe().unwrap();
    assert_eq!(rows(&fixture).len(), 2);
    // A different answer for that episode is another decision.
    fixture.write(
        "flush-not_idle.json",
        json!({"identityId":MEMBER,"state":"delivered"}),
    );
    fixture.arrived(1, arrived + 30_000);
    let mut tick = fixture.port();
    recorded(&fixture, &mut tick);
    tick.observe().unwrap();
    assert_eq!(rows(&fixture)[2], (Rule::AutoIdlePoint, Some("delivered")));
}

#[test]
fn a_member_waiting_on_its_deadline_across_ticks_writes_one_not_due_row() {
    let fixture = Fixture::new();
    fixture.settings("1h", None);
    let arrived = super::super::now_ms().unwrap() - 1_000;
    for _ in 0..3 {
        fixture.arrived(1, arrived);
        let mut tick = fixture.port();
        recorded(&fixture, &mut tick);
        for row in tick.observe().unwrap() {
            tick.not_due(&row);
        }
    }
    assert_eq!(rows(&fixture), [(Rule::NotDue, None)]);
    // The deadline passing is the next decision, with the same oldest message.
    fixture.settings("1s", None);
    fixture.arrived(1, arrived);
    let mut tick = fixture.port();
    recorded(&fixture, &mut tick);
    let row = tick.observe().unwrap().remove(0);
    tick.deliver_if_current(&row.identity_id).unwrap();
    assert_eq!(
        rows(&fixture),
        [
            (Rule::NotDue, None),
            (Rule::IntervalDeadline, Some("not_idle"))
        ]
    );
}
