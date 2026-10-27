//! The persistent transport against a fixture `tmt`: negotiation, failure and cleanup rules.
use super::*;
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        // macOS's temp dir is under the `/var` symlink.
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "tmt-digest-core-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&root).unwrap();
        let script = r#"#!/bin/sh
cd '__ROOT__' || exit 9
printf '%s %s\n' "$$" "$*" >> launches
if [ "$*" = api ]; then
  input=$(cat)
  case "$input" in
    *'"operation":"capabilities"'*)
      if [ -f legacy ]; then printf '{"operations":[]}'; else printf '{"operations":[],"session":{"framing":"ndjson"}}'; fi ;;
    *) printf '%s\n' "$input" ;;
  esac
  exit 0
fi
[ "$*" = 'api --session' ] || exit 9
while IFS= read -r request; do
  printf '%s\n' "$request" >> requests
  case "$request" in
    *'"operation":"fail"'*) printf '%s\n' '{"error":{"code":"CONFLICT","message":"changed"}}' ;;
    *'"operation":"stale"'*) printf '%s\n' '{"error":{"code":"API_SESSION_STALE","message":"reopen"}}'; exit 1 ;;
    *'"operation":"truncated"'*) printf partial; exit 7 ;;
    *'"operation":"hang"'*) sleep 30 ;;
    *) printf '%s\n' "$request" ;;
  esac
done
"#
        .replace("__ROOT__", root.to_str().unwrap());
        fs::write(root.join("tmt"), script).unwrap();
        fs::set_permissions(root.join("tmt"), fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
    fn core(&self) -> Core {
        Core::fixture(self.0.join("tmt")).persistent()
    }
    fn lines(&self, file: &str) -> Vec<String> {
        fs::read_to_string(self.0.join(file))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn launched(&self, suffix: &str) -> Vec<String> {
        let mut found = self.lines("launches");
        found.retain(|line| line.ends_with(suffix));
        found
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn alive(launch: &str) -> bool {
    let pid = launch.split(' ').next().unwrap();
    std::process::Command::new("kill")
        .args(["-0", pid])
        .status()
        .unwrap()
        .success()
}

#[test]
fn requests_share_one_session_after_one_admission() {
    let fixture = Fixture::new();
    let core = fixture.core();
    for index in 0..5 {
        let answer = core.api("read", json!({"index": index})).unwrap();
        assert_eq!(answer["input"]["index"], index);
    }
    assert_eq!(
        core.api("capabilities", json!({})).unwrap()["session"]["framing"],
        "ndjson"
    );
    assert_eq!(
        fixture.launched(" api").len(),
        1,
        "capabilities is asked once"
    );
    assert_eq!(fixture.launched(" api --session").len(), 1);
    assert_eq!(fixture.lines("requests").len(), 5);
}

#[test]
fn an_ordinary_error_keeps_the_session() {
    let fixture = Fixture::new();
    let core = fixture.core();
    core.api("read", json!({})).unwrap();
    let error = core.api("fail", json!({})).unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        ("CONFLICT", "changed")
    );
    core.api("read", json!({})).unwrap();
    assert_eq!(fixture.launched(" api --session").len(), 1);
}

#[test]
fn a_failed_exchange_ends_the_session_and_is_never_replayed() {
    let fixture = Fixture::new();
    let core = fixture.core();
    core.api("read", json!({})).unwrap();
    assert_eq!(
        core.api("truncated", json!({})).unwrap_err().code,
        "CORE_UNAVAILABLE"
    );
    core.api("read", json!({})).unwrap();
    let requests = fixture.lines("requests");
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.contains("\"truncated\""))
            .count(),
        1,
        "{requests:?}"
    );
    let sessions = fixture.launched(" api --session");
    assert_eq!(sessions.len(), 2, "the next request starts a fresh session");
    assert!(!alive(&sessions[0]));
}

#[test]
fn a_stale_session_is_reported_and_reopened_for_later_work() {
    let fixture = Fixture::new();
    let core = fixture.core();
    assert_eq!(
        core.api("stale", json!({})).unwrap_err().code,
        "API_SESSION_STALE"
    );
    core.api("read", json!({})).unwrap();
    assert_eq!(fixture.launched(" api --session").len(), 2);
    assert_eq!(
        fixture
            .lines("requests")
            .iter()
            .filter(|line| line.contains("\"stale\""))
            .count(),
        1
    );
}

#[test]
fn a_cli_without_a_session_keeps_one_process_per_request() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("legacy"), b"").unwrap();
    let core = fixture.core();
    for _ in 0..3 {
        core.api("read", json!({})).unwrap();
    }
    assert_eq!(
        fixture.launched(" api").len(),
        4,
        "capabilities and three requests"
    );
    assert!(fixture.launched(" api --session").is_empty());
}

#[test]
fn a_handle_that_did_not_opt_in_never_starts_a_session() {
    let fixture = Fixture::new();
    let core = Core::fixture(fixture.0.join("tmt"));
    core.api("read", json!({})).unwrap();
    core.api("capabilities", json!({})).unwrap();
    assert_eq!(fixture.launched(" api").len(), 2);
    assert!(fixture.launched(" api --session").is_empty());
}

#[test]
fn dropping_the_handle_ends_the_child() {
    let fixture = Fixture::new();
    let core = fixture.core();
    core.api("read", json!({})).unwrap();
    let session = fixture.launched(" api --session").remove(0);
    assert!(alive(&session));
    drop(core);
    assert!(!alive(&session));
}

#[test]
fn a_silent_child_is_bounded_by_the_deadline_and_ended() {
    let fixture = Fixture::new();
    let core = fixture
        .core()
        .until(Instant::now() + Duration::from_millis(1500));
    core.api("read", json!({})).unwrap();
    let started = Instant::now();
    assert_eq!(
        core.api("hang", json!({})).unwrap_err().code,
        "CORE_UNAVAILABLE"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!alive(&fixture.launched(" api --session").remove(0)));
}
