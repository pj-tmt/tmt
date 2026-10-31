//! Executable selection regressions: reject self before a Core can spawn it.

use super::*;
use std::{
    fs,
    os::unix::fs::symlink,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub(super) struct Fixture(pub(super) PathBuf);

impl Fixture {
    pub(super) fn new() -> Self {
        // The hardlink scenario must share the executable's filesystem, even
        // when the system temporary directory is a separate Linux tmpfs.
        let current = std::env::current_exe().unwrap();
        let path = current.parent().unwrap().join(format!(
            "squad-core-selection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

pub(super) fn session_fixture(fixture: &Fixture) -> Core {
    let selected = fixture.0.join("tmt");
    crate::test_support::write_ready_executable(
        &selected,
        &format!(
            r#"#!/bin/sh
cd '{}'
printf '%s %s\n' "$$" "$*" >> launches
if [ "$*" = api ]; then
  cat > capability-input
  if [ -f bad-capability ]; then printf broken; exit 1; fi
  if [ -f legacy ]; then printf '{{}}'; else printf '{{"session":{{"framing":"ndjson"}}}}'; fi
  exit 0
fi
while IFS= read -r request; do
  printf '%s\n' "$request" >> requests
  case "$request" in
    *'"operation":"stale"'*) printf '%s\n' '{{"error":{{"code":"API_SESSION_STALE","message":"reopen"}}}}'; exit 0 ;;
    *'"operation":"fail"'*) printf '%s\n' '{{"error":{{"code":"CONFLICT","message":"changed","current":{{"revision":7}}}},"requestId":"accepted"}}' ;;
    *'"operation":"truncated"'*) printf partial; exit 7 ;;
    *'"operation":"block"'*) printf ready > admitted; cat > /dev/null; exit 0 ;;
    *'"operation":"gate"'*) printf ready > admitted; while [ ! -f release ]; do sleep 0.01; done; printf '%s\n' "$request" ;;
    *) printf '%s\n' "$request" ;;
  esac
done
printf done > eof
"#,
            fixture.0.display()
        ),
    );
    Core::at(selected).with_api_session().unwrap()
}

fn lines(path: PathBuf) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn board_sessions_preserve_attribution_and_close_at_last_owner() {
    let fixture = Fixture::new();
    let core = session_fixture(&fixture);
    let clone = core.interactive();
    assert_eq!(
        core.api("read", json!({"text":"one\ntwo"})).unwrap(),
        json!({"version":1,"operation":"read","input":{"text":"one\ntwo"}})
    );
    assert_eq!(
        clone.api_write("write", json!({}), Some("actor")).unwrap()["identity"],
        "actor"
    );
    assert_eq!(
        clone.api_write("write", json!({}), None).unwrap()["originator"],
        "anonymous"
    );
    let failure = core.api("fail", json!({})).unwrap_err();
    assert_eq!(failure.code, "CONFLICT");
    assert_eq!(failure.current.as_deref(), Some(&json!({"revision":7})));
    assert_eq!(failure.request.as_deref(), Some("accepted"));
    assert!(!failure.was_cancelled());
    assert_eq!(clone.api("later", json!({})).unwrap()["operation"], "later");
    let launches = lines(fixture.0.join("launches"));
    assert_eq!(
        launches.len(),
        4,
        "each lane negotiates once and owns one persistent child"
    );
    assert_eq!(
        launches
            .iter()
            .filter(|line| line.ends_with(" api"))
            .count(),
        2
    );
    assert_eq!(
        launches
            .iter()
            .filter(|line| line.ends_with(" api --session"))
            .count(),
        2
    );
    drop(core);
    assert!(
        !fixture.0.join("eof").exists(),
        "another worker still owns the session"
    );
    drop(clone);
    assert_eq!(fs::read(fixture.0.join("eof")).unwrap(), b"done");
    assert_eq!(lines(fixture.0.join("requests")).len(), 5);
}

#[test]
fn stale_or_missing_response_is_never_replayed_and_only_later_work_reopens() {
    let fixture = Fixture::new();
    let core = session_fixture(&fixture);
    assert_eq!(
        core.api("stale", json!({})).unwrap_err().code,
        "API_SESSION_STALE"
    );
    assert_eq!(lines(fixture.0.join("launches")).len(), 2);
    assert_eq!(core.api("later", json!({})).unwrap()["operation"], "later");
    assert_eq!(lines(fixture.0.join("launches")).len(), 3);
    assert_eq!(
        core.api("truncated", json!({})).unwrap_err().code,
        "SQUAD_CORE_UNAVAILABLE"
    );
    assert_eq!(lines(fixture.0.join("launches")).len(), 3);
    assert_eq!(core.api("final", json!({})).unwrap()["operation"], "final");
    drop(core);
    let requests: Vec<Value> = lines(fixture.0.join("requests"))
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        requests
            .iter()
            .map(|request| request["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["stale", "later", "truncated", "final"]
    );
    assert_eq!(lines(fixture.0.join("launches")).len(), 4);
}

#[test]
fn buffering_session_times_out_without_replay_and_later_work_opens_a_new_child() {
    let fixture = Fixture::new();
    let core = session_fixture(&fixture);
    let error = core
        .api_within("block", json!({}), Duration::from_secs(2))
        .unwrap_err();
    assert_eq!(error.code, "SQUAD_CORE_UNAVAILABLE");
    assert!(error.message.contains("did not finish in time"));
    assert!(fixture.0.join("admitted").exists());
    assert_eq!(core.api("later", json!({})).unwrap()["operation"], "later");
    drop(core);
    let requests: Vec<Value> = lines(fixture.0.join("requests"))
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        requests
            .iter()
            .map(|value| value["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["block", "later"]
    );
    assert_eq!(lines(fixture.0.join("launches")).len(), 3);
}

#[test]
fn interaction_completes_while_background_exchange_is_held() {
    let fixture = Fixture::new();
    let core = session_fixture(&fixture);
    let background = core.clone();
    let held = std::thread::spawn(move || background.api("gate", json!({})));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !fixture.0.join("admitted").exists() {
        assert!(Instant::now() < deadline, "background request admission");
        std::thread::sleep(Duration::from_millis(10));
    }
    let interactive = core.interactive();
    let (send, recv) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _ = send.send(interactive.api_within("later", json!({}), Duration::from_secs(2)));
    });
    let result = recv.recv_timeout(Duration::from_secs(3));
    // Release even on failure so a regression cannot strand the test process.
    fs::write(fixture.0.join("release"), b"").unwrap();
    thread.join().unwrap();
    assert_eq!(held.join().unwrap().unwrap()["operation"], "gate");
    assert_eq!(result.unwrap().unwrap()["operation"], "later");
    drop(core);
    let launches = lines(fixture.0.join("launches"));
    assert_eq!(
        launches
            .iter()
            .filter(|line| line.ends_with(" api --session"))
            .count(),
        2
    );
}

#[test]
fn old_core_is_negotiated_once_and_keeps_the_existing_one_shot_door() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("legacy"), b"").unwrap();
    let core = session_fixture(&fixture);
    assert_eq!(core.api("read", json!({})).unwrap(), json!({}));
    assert_eq!(
        core.api_write("write", json!({}), Some("actor")).unwrap(),
        json!({})
    );
    drop(core);
    assert_eq!(
        lines(fixture.0.join("launches")).len(),
        3,
        "one probe, then two ordinary API processes"
    );
    let request: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("capability-input")).unwrap()).unwrap();
    assert_eq!(request["identity"], "actor");
    assert!(!fixture.0.join("requests").exists());
}

#[test]
fn failed_capability_admission_never_submits_the_effect() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("bad-capability"), b"").unwrap();
    let core = session_fixture(&fixture);
    assert!(core.api_write("write", json!({}), Some("actor")).is_err());
    assert!(!fixture.0.join("requests").exists());
    assert_eq!(lines(fixture.0.join("launches")).len(), 1);
    fs::remove_file(fixture.0.join("bad-capability")).unwrap();
    assert_eq!(core.api("later", json!({})).unwrap()["operation"], "later");
    drop(core);
    let requests = lines(fixture.0.join("requests"));
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].contains("write"));
}

#[test]
fn cancelling_one_admitted_read_drains_the_exchange_and_keeps_the_session() {
    let fixture = Fixture::new();
    let core = session_fixture(&fixture);
    let cancel = runner::Cancellation::default();
    let reader = core.cancellable(cancel.clone());
    let read = std::thread::spawn(move || reader.api("gate", json!({})));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !fixture.0.join("admitted").exists() {
        assert!(Instant::now() < deadline, "fixture request admission");
        std::thread::sleep(Duration::from_millis(10));
    }
    cancel.cancel();
    assert!(read.join().unwrap().unwrap_err().was_cancelled());
    fs::write(fixture.0.join("release"), b"").unwrap();
    assert_eq!(core.api("later", json!({})).unwrap()["operation"], "later");
    drop(core);
    let requests = lines(fixture.0.join("requests"));
    assert_eq!(requests.len(), 2, "the cancelled call must not replay");
    assert_eq!(
        lines(fixture.0.join("launches")).len(),
        2,
        "capability plus one session"
    );
}

#[test]
fn retained_automatic_reads_never_admit_actions_or_superseded_generations() {
    let fixture = Fixture::new();
    let core = session_fixture(&fixture);
    let held = Arc::new(RetainedReads::default());
    held.reset(true);
    let request = json!({"version":1,"operation":"references.resolve","input":{}});
    let resource = json!({"identities":[]});
    held.keep(&request, &resource);
    let background = core.with_retained_reads(Arc::clone(&held));
    assert_eq!(
        background.api("references.resolve", json!({})).unwrap(),
        resource
    );
    assert!(
        !fixture.0.join("launches").exists(),
        "unchanged covered projection uses no Core read"
    );
    let cancellation = runner::Cancellation::default();
    cancellation.cancel();
    assert!(
        background
            .cancellable(cancellation)
            .api("references.resolve", json!({}))
            .unwrap_err()
            .was_cancelled()
    );
    assert!(
        background
            .api_within("references.resolve", json!({}), Duration::ZERO)
            .is_err()
    );
    assert_eq!(
        background
            .interactive()
            .api("references.resolve", json!({}))
            .unwrap(),
        request,
        "selected reads are fresh"
    );
    assert_eq!(
        background.api_write("write", json!({}), None).unwrap()["originator"],
        "anonymous"
    );
    held.reset(false);
    assert_eq!(
        background.api("references.resolve", json!({})).unwrap(),
        request,
        "failed/unsupported cursor disables retention"
    );
    drop(background);
    drop(core);
    assert_eq!(lines(fixture.0.join("requests")).len(), 3);
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn assert_self_refused(result: Result<Core, SquadError>) {
    let error = match result {
        Ok(_) => panic!("self-selection must fail before any Core invocation"),
        Err(error) => error,
    };
    assert_eq!(error.code, "SQUAD_CORE_UNAVAILABLE");
    assert!(error.message.contains("TMT_EXECUTABLE"));
    assert!(error.message.contains("selects Squad itself"));
}

#[test]
fn supplied_self_is_refused_before_spawn() {
    assert_self_refused(Core::discover_with(
        Some(std::env::current_exe().unwrap()),
        None,
    ));
}

#[test]
fn supplied_symlink_and_hardlink_to_self_are_refused_before_spawn() {
    let fixture = Fixture::new();
    let current = std::env::current_exe().unwrap();
    let symbolic = fixture.0.join("symbolic-tmt");
    let hard = fixture.0.join("hard-tmt");
    symlink(&current, &symbolic).unwrap();
    fs::hard_link(&current, &hard).unwrap();
    for selected in [symbolic, hard] {
        assert_self_refused(Core::discover_with(Some(selected), None));
    }
}

#[test]
fn path_self_alias_is_refused_instead_of_trying_another_executable() {
    let fixture = Fixture::new();
    let first = fixture.0.join("first");
    let second = fixture.0.join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    symlink(std::env::current_exe().unwrap(), first.join("tmt")).unwrap();
    symlink("/bin/sh", second.join("tmt")).unwrap();
    assert_self_refused(Core::discover_with(
        None,
        Some(std::env::join_paths([first, second]).unwrap()),
    ));
}

#[test]
fn normal_supplied_and_path_core_still_return_configuration() {
    let fixture = Fixture::new();
    let selected = fixture.0.join("tmt");
    crate::test_support::write_ready_executable(
        &selected,
        "#!/bin/sh\nif [ \"$*\" != 'config show --json' ]; then exit 2; fi\nprintf '{\"config\":\"fixture\",\"lookup\":\"%s\"}' \"${TMT_OPS_CONFIG_LOOKUP-unset}\"\n",
    );
    for (supplied, search) in [
        (Some(selected.clone()), None),
        (None, Some(fixture.0.as_os_str().to_owned())),
    ] {
        let core = Core::discover_with(supplied, search).unwrap();
        assert_eq!(core.executable(), selected);
        assert_eq!(
            core.json(&["config", "show"]).unwrap(),
            json!({"config": "fixture", "lookup": "unset"})
        );
        assert_eq!(
            core.config_show().unwrap(),
            json!({"config": "fixture", "lookup": "1"})
        );
        assert_eq!(
            core.json(&["config", "show"]).unwrap(),
            json!({"config": "fixture", "lookup": "unset"})
        );
    }
}

#[test]
fn local_cancellation_is_distinct_from_an_identical_public_error() {
    let fixture = Fixture::new();
    let selected = fixture.0.join("tmt");
    crate::test_support::write_ready_executable(
        &selected,
        "#!/bin/sh\nprintf '%s' '{\"error\":{\"code\":\"SQUAD_CORE_UNAVAILABLE\",\"message\":\"The board load was superseded.\"}}'\nexit 1\n",
    );
    let core = Core::at(selected);
    let real_error = core.json(&["config", "show"]).unwrap_err();
    assert!(!real_error.was_cancelled());
    let token = runner::Cancellation::default();
    token.cancel();
    let cancelled = core
        .cancellable(token)
        .json(&["config", "show"])
        .unwrap_err();
    assert!(cancelled.was_cancelled());
    assert_eq!(cancelled.to_json(), real_error.to_json());
    assert_eq!(cancelled.human(), real_error.human());
    assert!(!SquadError::new(&cancelled.code, &cancelled.message).was_cancelled());
    assert!(!SquadError::hinted("SQUAD_CORE_UNAVAILABLE", "failed", " ", "retry").was_cancelled());
}

#[test]
fn the_version_is_the_bare_or_prefixed_line_and_anything_else_is_unknown() {
    for (stdout, expected) in [
        ("5.0.0-alpha.378\n", Some("5.0.0-alpha.378")),
        ("tmt 5.0.0-alpha.378\n", Some("5.0.0-alpha.378")),
        ("  5.0.0+build.7  ", Some("5.0.0+build.7")),
        ("", None),
        ("tmt \n", None),
        ("5.0.0\n6.0.0\n", None),
        ("tmt 5.0.0 (dev)\n", None),
        ("error: unknown flag\n", None),
    ] {
        assert_eq!(
            version_of(stdout.as_bytes()),
            expected.map(str::to_owned),
            "{stdout:?}"
        );
    }
    assert_eq!(version_of(&[0xff, b'1']), None, "not text");
    assert_eq!(
        version_of("1".repeat(65).as_bytes()),
        None,
        "an unbounded line"
    );
}

#[test]
fn a_core_reads_the_cli_version_once_from_dash_dash_version_and_a_failure_is_unknown() {
    let dir = Fixture::new();
    let log = dir.0.join("argv");
    let fake = dir.0.join("tmt");
    crate::test_support::write_ready_executable(
        &fake,
        &format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nif [ \"$1\" = --version ] && [ -z \"$FAIL\" ]; then echo 'tmt 5.0.0-alpha.9'; else echo nope; exit 2; fi\n",
            log = log.display()
        ),
    );
    let core = Core::at(fake.clone());
    assert_eq!(core.cli_version().as_deref(), Some("5.0.0-alpha.9"));
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "--version\n",
        "one plain argv"
    );
    let failing = dir.0.join("tmt-fail");
    crate::test_support::write_ready_executable(&failing, "#!/bin/sh\necho 5.0.0\nexit 3\n");
    assert_eq!(
        Core::at(failing).cli_version(),
        None,
        "a nonzero exit is unknown"
    );
}

#[test]
fn close_delete_stop_fault_preserves_signalled_evidence_without_guessing() {
    for signalled in [true, false] {
        let failure = resource_error(&json!({"error": {
            "code": "PROCESS_ERROR", "message": "stop was not proven",
            "signalled": signalled,
        }}))
        .unwrap_err();
        assert_eq!(
            failure.to_json()["error"]["signalled"],
            json!(signalled),
            "the refusal dialog must distinguish a possible TERM from nothing signalled"
        );
    }
    for evidence in [None, Some(json!("false")), Some(json!(0))] {
        let mut error = json!({"code":"PROCESS_ERROR","message":"outcome unknown"});
        if let Some(value) = evidence {
            error["signalled"] = value;
        }
        let failure = resource_error(&json!({"error":error})).unwrap_err();
        assert!(
            failure.to_json()["error"].get("signalled").is_none(),
            "missing or malformed evidence must remain unknown, never false"
        );
    }
}
