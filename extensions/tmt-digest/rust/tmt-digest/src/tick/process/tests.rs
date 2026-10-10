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
        let root = std::env::temp_dir().join(format!(
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
        self.write("stats.json", json!({"stats":[{"identityId":MEMBER,"heldCount":held,"oldestHeldAgeMs":if held==0 {Value::Null} else {json!(age)},"observedAtMs":super::super::now_ms().unwrap()}]}));
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
    port.flush(MEMBER).unwrap();
    assert!(Instant::now() >= boundary);
    assert!(port.observe().unwrap().is_empty());
    port.deliver_if_current(MEMBER).unwrap();
    port.finish().unwrap();
    assert_eq!(fixture.calls().len(), 1);
    assert!(fixture.calls()[0].contains("digest.checklist.flush"));
}
