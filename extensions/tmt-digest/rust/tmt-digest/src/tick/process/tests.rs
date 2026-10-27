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
printf '%s %s\n' "$$" "$*" >> launches
reply() { input=$1; printf '%s\n' "$input" >> calls
   case "$input" in
    *'"operation":"capabilities"'*) cat capabilities.json ;;
    *'"operation":"orchestration.read"'*)
      if test -f slow-receiving-read; then sleep 2.1; fi
      printf '%s\n' '{"receivingRevision":0}' ;;
    *'"operation":"digest.receiving.sync"'*)
      if test -f sync-fails; then printf '%s\n' '{"error":{"code":"ORCH_CONFLICT","message":"changed"}}'; return 5; fi
      cat receiving.json ;;
    *'"operation":"digest.wake.step"'*)
      if test -f wake-fails; then printf '%s\n' '{"error":{"code":"WAKE_FAILED","message":"inspect claim"}}'; return 1; fi
      cat wake.json ;;
    *'"operation":"changes.cursor"'*)
      if test -f cursor-unsupported; then printf '%s\n' '{"error":{"code":"API_INPUT_INVALID","message":"older core"}}'; return 1; fi
      cat cursor.json ;;
    *'"operation":"digest.checklist.history"'*) cat history.json ;;
    *'"operation":"digest.policy.show"'*) cat policies.json ;;
    *'"operation":"digest.stats.show"'*) cat stats.json ;;
    *'"operation":"digest.checklist.flush"'*)
      if test -f slow-flush; then sleep 2.1; fi
      if test -f check-fails; then state=uncertain; else state=not_idle; fi
      cat flush-$state.json ;;
    *) printf '%s\n' '{"ok":true}' ;;
   esac; }
case "$*" in
 'identity ls --json') cat inventory.json ;;
 'api') reply "$(cat)"; exit $? ;;
 'api --session') while IFS= read -r line; do out=$(reply "$line"); printf '%s\n' "$out"; done ;;
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
        fixture.write(
            "capabilities.json",
            json!({"operations":["digest.receiving.sync"]}),
        );
        fixture.write("cursor.json", json!({"cursor":1,"usage":0}));
        fixture.write("wake.json", json!({"schemaVersion":1,"state":"nothing_due","identityId":null,"projection":"incoming_summary","nextAtMs":null}));
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
    /// What Core answers once the last `digest.policy.set` is stored.
    fn recorded_hold(&self) -> Value {
        self.writes()
            .iter()
            .rev()
            .find(|row| row["operation"] != "digest.checklist.dueNow")
            .map_or(Value::Null, |row| row["input"]["maxHoldMs"].clone())
    }
    fn policy(&self, revision: u64, active: bool) {
        self.write("policies.json", json!({"policies":[{"identityId":MEMBER,"revision":revision,"active":active,"ownerIdentityId":OWNER,"setterIdentityId":SETTER,"maxHoldMs":self.recorded_hold()}]}));
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
        self.receiving(Receiving::parse(mode).unwrap());
    }
    fn receiving(&self, mode: Receiving) {
        let value = match mode {
            Receiving::Direct => json!({"mode":"direct"}),
            Receiving::Auto => json!({"mode":"auto"}),
            Receiving::Batch { milliseconds } => json!({"mode":"batch","intervalMs":milliseconds}),
            Receiving::Urgent => json!({"mode":"urgent"}),
            Receiving::Hold => json!({"mode":"hold"}),
        };
        self.write("receiving.json",json!({"revision":1,"projection":{"receivingRevision":1,"members":[{"identityId":MEMBER,"effective":{"receiving":{"available":true,"value":value}}}]}}));
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
fn explicit_capability_absence_keeps_legacy_leases_and_emits_one_upgrade_diagnostic() {
    for configured in ["20s", "auto", "off"] {
        let fixture = Fixture::new();
        fixture.settings(configured, None);
        fixture.write(
            "capabilities.json",
            json!({"operations":["digest.policy.set","digest.policy.show"]}),
        );
        fixture.policy(u64::from(configured == "off"), configured == "off");
        let mut port = fixture.port();
        let rows = port.observe().unwrap();
        assert_eq!(rows[0].mode, Receiving::parse(configured).unwrap());
        let first = fixture.writes();
        let lease = first
            .iter()
            .find(|r| {
                r["operation"] == "digest.policy.set" || r["operation"] == "digest.policy.clear"
            })
            .unwrap();
        if configured == "off" {
            assert_eq!(lease["operation"], "digest.policy.clear");
        } else {
            assert_eq!(lease["operation"], "digest.policy.set");
            assert_eq!(
                lease["input"]["maxHoldMs"],
                if configured == "auto" {
                    300_000
                } else {
                    20_000
                }
            );
        }
        fixture.policy(2, configured != "off");
        port.observe().unwrap();
        let calls = fixture.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|s| s.contains("\"operation\":\"capabilities\""))
                .count(),
            1
        );
        assert!(
            !calls
                .iter()
                .any(|s| s.contains("orchestration.read") || s.contains("digest.receiving.sync"))
        );
        let mut diagnostic = Vec::new();
        port.finish_to(&mut diagnostic).unwrap();
        assert_eq!(
            String::from_utf8(diagnostic).unwrap(),
            format!("{LEGACY_RECEIVING_DIAGNOSTIC}\n")
        );
    }
}

#[test]
fn advertised_receiving_errors_never_fall_back_to_configured_leases() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("sync-fails"), "").unwrap();
    let mut port = fixture.port();
    for _ in 0..2 {
        assert_eq!(
            port.observe().err().expect("observation must fail").code,
            "ORCH_CONFLICT"
        );
    }
    assert!(fixture.writes().is_empty());
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|s| s.contains("\"operation\":\"capabilities\""))
            .count(),
        1
    );
    let mut diagnostic = Vec::new();
    port.finish_to(&mut diagnostic).unwrap();
    assert!(diagnostic.is_empty());
}

#[test]
fn malformed_capabilities_are_not_explicit_absence() {
    let fixture = Fixture::new();
    fixture.write(
        "capabilities.json",
        json!({"operations":["digest.receiving.sync",null]}),
    );
    let mut port = fixture.port();
    assert_eq!(
        port.observe().err().expect("observation must fail").code,
        "CORE_RESPONSE_INVALID"
    );
    assert!(fixture.writes().is_empty());
    let mut diagnostic = Vec::new();
    port.finish_to(&mut diagnostic).unwrap();
    assert!(diagnostic.is_empty());
}

#[test]
fn core_resolved_receiving_controls_the_tick_instead_of_configured_member_mode() {
    for resolved in [
        Receiving::Hold,
        Receiving::Urgent,
        Receiving::Batch {
            milliseconds: 90_000,
        },
    ] {
        let fixture = Fixture::new();
        fixture.settings("20s", None);
        fixture.receiving(resolved);
        fixture.policy(0, false);
        let rows = fixture.port().observe().unwrap();
        assert_eq!(rows[0].mode, resolved);
        let writes = fixture.writes();
        let policy = writes
            .iter()
            .find(|r| r["operation"] == "digest.policy.set")
            .unwrap();
        if resolved == Receiving::Hold {
            assert!(
                !writes
                    .iter()
                    .any(|r| r["operation"] == "digest.checklist.dueNow")
            );
            assert!(policy["input"]["maxHoldMs"].is_null());
        } else if resolved == Receiving::Urgent {
            let calls: Vec<Value> = fixture
                .calls()
                .iter()
                .filter_map(|s| serde_json::from_str(s).ok())
                .collect();
            let flush = calls
                .iter()
                .find(|r| r["operation"] == "digest.checklist.flush")
                .unwrap();
            assert!(
                !flush["input"]
                    .get("whenBusy")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            );
        } else {
            assert_eq!(policy["input"]["maxHoldMs"], 90_000);
        }
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
        let due = calls
            .iter()
            .filter(|line| line.contains("digest.checklist.dueNow"))
            .count();
        let writes: Vec<_> = fixture
            .writes()
            .iter()
            .map(|row| row["operation"].as_str().unwrap().to_owned())
            .filter(|operation| operation != "digest.checklist.dueNow")
            .collect();
        // The first reading set the 20 s interval's hold; the fresh one decides the rest.
        let (second, offered, marked) = match changed {
            "1h" => ("digest.policy.set", 0, 0),
            "off" => ("digest.policy.clear", 1, 0),
            // Auto keeps the hold, so what is held is marked due before it is offered.
            _ => ("digest.policy.set", 1, 1),
        };
        assert_eq!(writes, ["digest.policy.set", second], "{changed}");
        assert_eq!(due, marked, "{changed}");
        assert_eq!(
            calls
                .iter()
                .filter(|line| line.contains("digest.checklist.flush"))
                .count(),
            offered,
            "{changed}"
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
fn a_receiving_read_that_crosses_the_budget_starts_no_publication() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("slow-receiving-read"), "").unwrap();
    let mut port = TickProcess::new(
        Core::fixture(fixture.0.join("core")),
        fixture.0.join("digest.toml"),
        u64::MAX,
        Instant::now() + Duration::from_secs(1),
    );
    assert!(port.observe().unwrap().is_empty());
    let calls = fixture.calls();
    assert!(calls.iter().any(|line| line.contains("orchestration.read")));
    assert!(
        !calls
            .iter()
            .any(|line| line.contains("digest.receiving.sync"))
    );
    assert!(!calls.iter().any(|line| line.contains("digest.policy.show")));
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
                "receiving.json",
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
        mode: Receiving::Direct,
        flush_count: 10,
        auto_caps: crate::settings::AutoCaps::default(),
        held_count: 1,
        oldest_held_at_ms: Some(0),
    };
    port.flush(&held, Rule::Off, false).unwrap();
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
        ("auto", 3, 5_000, Rule::AutoHold, Some("not_idle")),
        ("auto", 5, 5_000, Rule::AutoCap, Some("not_idle")),
        ("auto", 1, 301_000, Rule::AutoCap, Some("not_idle")),
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
            if matches!(row.mode, Receiving::Batch { .. }) {
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
    assert_eq!(rows[0].rule, Rule::AutoHold);
    // A different answer from Core is a new decision, once the backlog is offered again.
    fixture.write(
        "flush-not_idle.json",
        json!({"identityId":MEMBER,"state":"delivered"}),
    );
    port.offered.clear();
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
    for (changed, rule) in [("off", Rule::Off), ("auto", Rule::AutoHold)] {
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
    assert_eq!(rows(&fixture), [(Rule::AutoHold, Some("not_idle"))]);
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
    assert_eq!(rows(&fixture)[2], (Rule::AutoHold, Some("delivered")));
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

fn offers(fixture: &Fixture, operation: &str) -> usize {
    fixture
        .calls()
        .iter()
        .filter(|line| line.contains(operation))
        .count()
}

#[test]
fn an_auto_backlog_is_offered_on_arrival_and_again_only_for_news_or_after_the_wait() {
    let fixture = Fixture::new();
    fixture.settings("auto", None);
    let arrived = super::super::now_ms().unwrap() - 2_000;
    fixture.arrived(2, arrived);
    let mut port = fixture.port();
    // The loop looks about once a second; a busy member is not asked each time.
    for _ in 0..4 {
        port.observe().unwrap();
    }
    assert_eq!(offers(&fixture, "digest.checklist.dueNow"), 1);
    assert_eq!(offers(&fixture, "digest.checklist.flush"), 1);
    // The range is marked due before it is offered, never the reverse.
    let calls = fixture.calls();
    let due = calls.iter().position(|l| l.contains("dueNow")).unwrap();
    let flush = calls
        .iter()
        .position(|l| l.contains("checklist.flush"))
        .unwrap();
    assert!(due < flush);
    // Another arrival is news: it is marked due and offered at once.
    fixture.arrived(3, arrived);
    port.observe().unwrap();
    assert_eq!(offers(&fixture, "digest.checklist.dueNow"), 2);
    assert_eq!(offers(&fixture, "digest.checklist.flush"), 2);
    // The same backlog is offered again after the wait, and an emptied one forgets it.
    port.offered.clear();
    port.observe().unwrap();
    assert_eq!(offers(&fixture, "digest.checklist.flush"), 3);
    fixture.stats(0, 0);
    port.observe().unwrap();
    assert!(port.offered.is_empty());
    assert_eq!(offers(&fixture, "digest.checklist.flush"), 3);
    port.finish().unwrap();
}

#[test]
fn only_the_cap_offers_past_a_busy_agent_and_below_it_core_keeps_its_idle_check() {
    for (held, age, when_busy) in [
        (1, 1_000, false),
        (4, 240_000, false),
        (5, 1_000, true),
        (1, 300_000, true),
        (9, 400_000, true),
    ] {
        let fixture = Fixture::new();
        fixture.settings("auto", None);
        fixture.stats(held, age);
        let mut port = fixture.port();
        port.observe().unwrap();
        let flushes: Vec<Value> = fixture
            .calls()
            .iter()
            .filter(|line| line.contains("digest.checklist.flush"))
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(flushes.len(), 1, "{held} held for {age} ms");
        let input = &flushes[0]["input"];
        assert_eq!(input["identityId"], MEMBER);
        assert_eq!(
            input.get("whenBusy").and_then(Value::as_bool),
            when_busy.then_some(true),
            "{held} held for {age} ms"
        );
        assert_eq!(input.as_object().unwrap().len(), 1 + usize::from(when_busy));
    }
    // Interval and off members never pass it.
    for mode in ["off", "1s"] {
        let fixture = Fixture::new();
        fixture.settings(mode, None);
        fixture.stats(20, 900_000);
        let mut port = fixture.port();
        for row in port.observe().unwrap() {
            if matches!(row.mode, Receiving::Batch { .. }) {
                port.deliver_if_current(&row.identity_id).unwrap();
            }
        }
        assert!(offers(&fixture, "digest.checklist.flush") >= 1, "{mode}");
        assert_eq!(offers(&fixture, "whenBusy"), 0, "{mode}");
    }
}

#[test]
fn an_auto_member_gets_a_two_minute_hold_renewed_once_and_off_releases_it() {
    let fixture = Fixture::new();
    fixture.settings("auto", Some(SETTER));
    fixture.stats(0, 0);
    let mut port = fixture.port();
    port.observe().unwrap();
    fixture.policy(1, true);
    port.observe().unwrap();
    port.observe().unwrap();
    let writes = fixture.writes();
    assert_eq!(writes.len(), 1, "one renewal per invocation");
    assert_eq!(writes[0]["operation"], "digest.policy.set");
    let until = writes[0]["input"]["untilMs"].as_u64().unwrap();
    let now = super::super::now_ms().unwrap();
    assert!((now + 119_000..=now + 121_000).contains(&until), "{until}");
    // A hold from a longer interval is cut back to the auto horizon, not kept.
    fixture.settings("off", None);
    let mut released = fixture.port();
    fixture.policy(2, true);
    released.observe().unwrap();
    assert_eq!(
        fixture.writes().last().unwrap()["operation"],
        "digest.policy.clear"
    );
    // An inactive policy is set again the next minute.
    let again = Fixture::new();
    again.settings("auto", None);
    again.stats(0, 0);
    again.policy(3, false);
    again.port().observe().unwrap();
    assert_eq!(again.writes()[0]["operation"], "digest.policy.set");
}

#[test]
fn an_inherited_auto_hold_is_owned_by_the_global_default_setter_and_none_means_the_member() {
    let defaulted = |attribution: &str| {
        let fixture = Fixture::new();
        fs::write(
            fixture.0.join("digest.toml"),
            format!("default = 'auto'\n{attribution}"),
        )
        .unwrap();
        fixture.stats(0, 0);
        fixture
    };
    // (default attribution, pinned owner in Core, revision, expected owner, expected setter)
    for (attribution, revision, owner, setter) in [
        (
            format!("defaultSetByIdentityId = '{OWNER}'\n"),
            0,
            OWNER,
            OWNER,
        ),
        (String::new(), 0, MEMBER, MEMBER),
        // A hold already pinned to the member itself adopts a setter recorded later...
        (
            format!("defaultSetByIdentityId = '{OWNER}'\n"),
            4,
            OWNER,
            OWNER,
        ),
    ] {
        let fixture = defaulted(&attribution);
        if revision > 0 {
            fixture.write("policies.json", json!({"policies":[{"identityId":MEMBER,"revision":revision,"active":true,"ownerIdentityId":MEMBER,"setterIdentityId":MEMBER,"maxHoldMs":fixture.recorded_hold()}]}));
        } else {
            fixture.policy(0, false);
        }
        let mut port = fixture.port();
        port.observe().unwrap();
        let writes = fixture.writes();
        assert_eq!(writes.len(), 1, "{attribution}");
        assert_eq!(
            writes[0]["input"]["ownerIdentityId"], owner,
            "{attribution}"
        );
        assert_eq!(
            writes[0]["input"]["setterIdentityId"], setter,
            "{attribution}"
        );
        assert_eq!(writes[0]["input"]["expectedRevision"], revision);
    }
    // ...but a pinned owner other than the member is never replaced, and an unchanged
    // adopted hold is not rewritten on every look.
    let fixture = defaulted(&format!("defaultSetByIdentityId = '{SETTER}'\n"));
    // A hold from before the limit was recorded is rewritten once, and Core then reports it.
    fixture.policy(5, true);
    let mut port = fixture.port();
    port.observe().unwrap();
    fixture.policy(6, true);
    port.observe().unwrap();
    let writes = fixture.writes();
    assert_eq!(writes.len(), 1, "one renewal for the invocation");
    assert_eq!(writes[0]["input"]["ownerIdentityId"], OWNER);
    assert_eq!(writes[0]["input"]["setterIdentityId"], SETTER);
}

#[test]
fn every_hold_records_the_limit_of_the_rule_that_decides_it() {
    // (setting, the limit the rule enforces on the oldest held message)
    for (setting, limit) in [
        ("auto", Some(300_000)),
        ("20s", Some(20_000)),
        ("1h", Some(3_600_000)),
    ] {
        let fixture = Fixture::new();
        fixture.policy(0, false);
        fixture.settings(setting, None);
        let mut port = fixture.port();
        port.observe().unwrap();
        let sets: Vec<_> = fixture
            .writes()
            .into_iter()
            .filter(|row| row["operation"] == "digest.policy.set")
            .collect();
        assert_eq!(sets.len(), 1, "{setting}");
        assert_eq!(sets[0]["input"]["maxHoldMs"].as_u64(), limit, "{setting}");
    }
    // An auto member's own wait is the limit, and a changed wait updates the active hold.
    let fixture = Fixture::new();
    fixture.policy(0, false);
    fs::write(
        fixture.0.join("digest.toml"),
        format!("[members.'{MEMBER}']\nmode = 'auto'\nmaxWait = '90s'\nsetAtMs = 1\n"),
    )
    .unwrap();
    fixture.receiving(Receiving::Auto);
    fixture.port().observe().unwrap();
    let set = fixture
        .writes()
        .into_iter()
        .find(|row| row["operation"] == "digest.policy.set")
        .unwrap();
    assert_eq!(set["input"]["maxHoldMs"].as_u64(), Some(90_000));
    // Off holds nothing, so there is nothing to bound.
    let fixture = Fixture::new();
    fixture.policy(3, true);
    fixture.settings("off", None);
    fixture.port().observe().unwrap();
    let clear = fixture
        .writes()
        .into_iter()
        .find(|row| row["operation"] == "digest.policy.clear")
        .unwrap();
    assert!(clear["input"].get("maxHoldMs").is_none());
}

#[test]
fn the_switch_off_sends_one_by_one_despite_a_member_setting_and_on_holds_again() {
    let fixture = Fixture::new();
    fixture.settings("auto", Some(SETTER));
    let off = |on: bool| {
        let path = fixture.0.join("digest.toml");
        let stored = fs::read_to_string(&path).unwrap();
        let stored = stored.replace("enabled = false\n", "");
        fs::write(
            &path,
            if on {
                stored
            } else {
                format!("enabled = false\n{stored}")
            },
        )
        .unwrap();
        fixture.receiving(if on {
            Receiving::Auto
        } else {
            Receiving::Direct
        });
    };
    // The member's own `auto` is holding three messages under an active hold.
    fixture.stats(3, 1000);
    fixture.policy(1, true);
    off(false);
    fixture.port().observe().unwrap();
    let operations: Vec<_> = fixture
        .writes()
        .iter()
        .map(|row| row["operation"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(operations, ["digest.policy.clear"], "the hold is released");
    assert!(
        fixture
            .calls()
            .iter()
            .any(|line| line.contains("digest.checklist.flush")),
        "the backlog is offered for delivery now"
    );
    // The stored override survived and applies again once the switch is on.
    assert!(
        fs::read_to_string(fixture.0.join("digest.toml"))
            .unwrap()
            .contains("mode = 'auto'")
    );
    off(true);
    fixture.policy(2, false);
    fixture.port().observe().unwrap();
    assert!(
        fixture
            .writes()
            .iter()
            .any(|row| row["operation"] == "digest.policy.set"),
        "the member's own auto holds again"
    );
}

#[test]
fn wake_capability_uses_the_existing_once_per_run_read_and_explicit_absence_makes_no_step() {
    for supported in [false, true] {
        let f = Fixture::new();
        f.stats(0, 0);
        let operations = if supported {
            vec!["digest.receiving.sync", "digest.wake.step"]
        } else {
            vec!["digest.receiving.sync"]
        };
        f.write("capabilities.json", json!({"operations":operations}));
        let mut port = f.port();
        port.observe().unwrap();
        port.observe().unwrap();
        assert_eq!(offers(&f, "capabilities"), 1);
        assert_eq!(
            offers(&f, "digest.wake.step"),
            if supported { 2 } else { 0 }
        );
    }
}
#[test]
fn an_advertised_wake_error_ends_the_observation_without_legacy_fallback() {
    let f = Fixture::new();
    f.stats(0, 0);
    f.write(
        "capabilities.json",
        json!({"operations":["digest.receiving.sync","digest.wake.step"]}),
    );
    fs::write(f.0.join("wake-fails"), "").unwrap();
    let mut port = f.port();
    let error = port.observe().err().expect("advertised wake fails");
    assert_eq!(error.code, "WAKE_FAILED");
    assert_eq!(offers(&f, "digest.wake.step"), 1);
    assert_eq!(offers(&f, "digest.checklist.flush"), 0);
    assert!(f.calls().last().unwrap().contains("digest.wake.step"));
}

#[test]
fn a_persistent_tick_reads_through_one_session_process_and_leaves_no_child() {
    let fixture = Fixture::new();
    fixture.write(
        "capabilities.json",
        json!({"operations":["digest.receiving.sync"],"session":{"framing":"ndjson"}}),
    );
    let mut port = TickProcess::new(
        Core::fixture(fixture.0.join("core")).persistent(),
        fixture.0.join("digest.toml"),
        super::super::now_ms().unwrap() + 60_000,
        Instant::now() + Duration::from_secs(60),
    );
    for _ in 0..3 {
        assert_eq!(port.observe().unwrap()[0].identity_id, MEMBER);
    }
    drop(port);
    let launches = fs::read_to_string(fixture.0.join("launches")).unwrap();
    let launched = |suffix: &str| {
        launches
            .lines()
            .filter(|line| line.ends_with(suffix))
            .count()
    };
    assert_eq!(
        launched(" api"),
        1,
        "only the capabilities admission is a one-shot request"
    );
    assert_eq!(launched(" api --session"), 1, "{launches}");
    assert_eq!(
        launched("identity ls --json"),
        1,
        "the roster is listed once while the cursor is unchanged"
    );
    let operations = fixture
        .calls()
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|row| row["operation"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    for operation in [
        "digest.receiving.sync",
        "digest.policy.show",
        "digest.stats.show",
    ] {
        assert_eq!(
            operations.iter().filter(|name| *name == operation).count(),
            3,
            "{operation}: {operations:?}"
        );
    }
    let session = launches
        .lines()
        .find(|line| line.ends_with(" api --session"))
        .unwrap();
    let pid = session.split(' ').next().unwrap();
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", pid])
            .status()
            .unwrap()
            .success(),
        "the session child {pid} must not outlive the tick"
    );
}

const OTHER: &str = "10000000-0000-4000-8000-000000000002";

fn session_capabilities(fixture: &Fixture) {
    fixture.write(
        "capabilities.json",
        json!({"operations":["digest.receiving.sync"],"session":{"framing":"ndjson"}}),
    );
}
fn launches(fixture: &Fixture, suffix: &str) -> usize {
    fs::read_to_string(fixture.0.join("launches"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.ends_with(suffix))
        .count()
}
fn operations(fixture: &Fixture, name: &str) -> Vec<Value> {
    fixture
        .calls()
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["operation"] == name)
        .collect()
}
fn persistent_port(fixture: &Fixture) -> TickProcess {
    TickProcess::new(
        Core::fixture(fixture.0.join("core")).persistent(),
        fixture.0.join("digest.toml"),
        super::super::now_ms().unwrap() + 60_000,
        Instant::now() + Duration::from_secs(60),
    )
}

#[test]
fn an_unchanged_cursor_lists_the_roster_once_per_tick() {
    let fixture = Fixture::new();
    session_capabilities(&fixture);
    let mut port = persistent_port(&fixture);
    for _ in 0..3 {
        assert_eq!(port.observe().unwrap()[0].identity_id, MEMBER);
    }
    assert_eq!(launches(&fixture, "identity ls --json"), 1);
    assert_eq!(operations(&fixture, "changes.cursor").len(), 3);
}

#[test]
fn a_moved_cursor_lists_again_and_a_retired_member_leaves_the_requests() {
    let fixture = Fixture::new();
    session_capabilities(&fixture);
    fixture.write(
        "inventory.json",
        json!({"identities":[{"id":MEMBER,"name":"worker"},{"id":OTHER,"name":"other"}]}),
    );
    let both = |revision: u64| {
        json!({"policies":[
            {"identityId":MEMBER,"revision":revision,"active":false,"ownerIdentityId":OWNER,"setterIdentityId":SETTER,"maxHoldMs":null},
            {"identityId":OTHER,"revision":revision,"active":false,"ownerIdentityId":OWNER,"setterIdentityId":SETTER,"maxHoldMs":null}]})
    };
    fixture.write("policies.json", both(0));
    let now = super::super::now_ms().unwrap();
    fixture.write(
        "stats.json",
        json!({"stats":[
            {"identityId":MEMBER,"heldCount":0,"oldestHeldAgeMs":null,"observedAtMs":now},
            {"identityId":OTHER,"heldCount":0,"oldestHeldAgeMs":null,"observedAtMs":now}]}),
    );
    let mode = json!({"available":true,"value":{"mode":"batch","intervalMs":20000}});
    fixture.write(
        "receiving.json",
        json!({"revision":1,"projection":{"receivingRevision":1,"members":[
            {"identityId":MEMBER,"effective":{"receiving":mode}},
            {"identityId":OTHER,"effective":{"receiving":mode}}]}}),
    );
    let mut port = persistent_port(&fixture);
    assert_eq!(port.observe().unwrap().len(), 2);
    assert_eq!(port.observe().unwrap().len(), 2);
    assert_eq!(launches(&fixture, "identity ls --json"), 1);
    // OTHER is retired: the roster and Core's rows change, and the cursor moves.
    fixture.write(
        "inventory.json",
        json!({"identities":[{"id":MEMBER,"name":"worker"}]}),
    );
    fixture.policy(0, false);
    fixture.stats(0, 0);
    fixture.receiving(Receiving::Batch {
        milliseconds: 20_000,
    });
    fixture.write("cursor.json", json!({"cursor":2,"usage":0}));
    let rows = port.observe().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].identity_id, MEMBER);
    assert_eq!(launches(&fixture, "identity ls --json"), 2);
    let shown = operations(&fixture, "digest.policy.show");
    assert_eq!(
        shown.last().unwrap()["input"]["identities"],
        json!([MEMBER])
    );
    assert_eq!(shown[0]["input"]["identities"], json!([MEMBER, OTHER]));
}

#[test]
fn without_a_session_every_observation_lists_and_reads_no_cursor() {
    let fixture = Fixture::new();
    let mut port = fixture.port();
    for _ in 0..3 {
        port.observe().unwrap();
    }
    assert_eq!(launches(&fixture, "identity ls --json"), 3);
    assert!(operations(&fixture, "changes.cursor").is_empty());
}

#[test]
fn an_older_core_without_the_cursor_lists_every_observation_and_asks_once() {
    let fixture = Fixture::new();
    session_capabilities(&fixture);
    fs::write(fixture.0.join("cursor-unsupported"), b"").unwrap();
    let mut port = persistent_port(&fixture);
    for _ in 0..3 {
        port.observe().unwrap();
    }
    assert_eq!(launches(&fixture, "identity ls --json"), 3);
    assert_eq!(operations(&fixture, "changes.cursor").len(), 1);
}
