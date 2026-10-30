//! A launched member that switches conversations continues in the same process (#5703).
use super::*;

/// A `driver` hook event of session s-1.
fn event(driver: &str, name: &str) -> Value {
    match driver {
        "claude" => claude_event(name),
        _ => codex_event(name, (name != "SessionStart").then_some("switch-turn")),
    }
}

/// A member running session s-1 in an owned external-host world, with a held Digest item from
/// core-1; `launched` records its launch owner and gives its hooks the launch claim, as
/// `tmt run` does. Without it the member was bound by hand.
fn member(name: &str, driver: &'static str, launched: bool) -> (Fixture, SteppedProvider) {
    register();
    let fixture = Fixture::delivering(name);
    let record = fixture.approve();
    bind(&fixture, record);
    let agent = worker(&fixture.json(&["ls", "--json"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let person = fixture.json(&["identity", "create", "ben", "--json"])["identity"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.json(&[
        "identity",
        "meta",
        "set",
        "user",
        "true",
        "--identity",
        "ben",
        "--json",
    ]);
    fixture.json(&["identity", "create", "core-1", "--json"]);
    let mut provider = SteppedProvider::start(&fixture, driver, "switch-steps");
    stop_capture::ok(provider.hook(event(driver, "SessionStart")));
    if launched {
        let ProcessObservation::Live(owner) = observe_runtime_process(
            &UnixCommandRunner,
            u64::from(std::process::id()),
            deadline(),
        )
        .unwrap() else {
            panic!("test owner is not live");
        };
        let launch = record_admitted_launch(&fixture, &agent, driver, &owner);
        provider.launch = Some(serde_json::to_string(&launch).unwrap());
    }
    let now = tmt_adapters::request_runtime::wall_time_ms();
    let policy = passthrough::api(
        &fixture,
        "ok",
        "digest.policy.set",
        serde_json::json!({"identityId":agent,"ownerIdentityId":person,"setterIdentityId":person,"expectedRevision":0,"untilMs":now+60_000}),
    );
    assert_eq!(policy["active"], true, "{policy:#}");
    let held = fixture.json(&[
        "talk",
        "worker",
        "Core decision item",
        "--identity",
        "core-1",
        "--detach",
        "--json",
    ]);
    assert_eq!(held["digest"], true, "{held:#}");
    (fixture, provider)
}

/// `/clear` ends s-1; the start of its new session is never seen.
fn clear_end(fixture: &Fixture, provider: &mut SteppedProvider) {
    let mut end = claude_event("SessionEnd");
    end["reason"] = "clear".into();
    stop_capture::ok(provider.hook(end));
    let waiting = session_state(fixture);
    assert_eq!(
        (waiting[0].as_str(), waiting[1].as_str()),
        ("unknown", "cleared")
    );
}

fn cleared_member(name: &str, launched: bool) -> (Fixture, SteppedProvider) {
    let (fixture, mut provider) = member(name, "claude", launched);
    clear_end(&fixture, &mut provider);
    (fixture, provider)
}

fn in_new_session(name: &str) -> Value {
    let mut event = claude_event(name);
    event["session_id"] = "s-2".into();
    event
}

#[test]
fn a_cleared_member_whose_start_was_lost_continues_on_its_next_stop_and_gets_its_digest() {
    let (fixture, mut provider) = cleared_member("switch-lost-start", true);
    let before = session_state(&fixture);
    let mut stop = in_new_session("Stop");
    stop["stop_hook_active"] = false.into();
    stop["last_assistant_message"] = "done".into();
    let feedback = stop_capture::ok(provider.digest_hook(stop));
    let after = session_state(&fixture);
    assert_eq!(after[0], "running", "{after:?}");
    assert_eq!(after[2..4], before[2..4], "the same process: {after:?}");
    assert_eq!(after[4], "s-2", "{after:?}");
    let feedback: Value = serde_json::from_str(&feedback).unwrap();
    assert_eq!(feedback["decision"], "block", "{feedback:#}");
    assert!(
        feedback["reason"]
            .as_str()
            .unwrap()
            .contains("Core decision item"),
        "{feedback:#}"
    );
}

#[test]
fn a_clear_start_that_beats_its_end_is_held_until_the_process_names_its_session() {
    let (fixture, mut provider) = member("switch-early-start", "claude", true);
    let mut start = in_new_session("SessionStart");
    start["source"] = "clear".into();
    stop_capture::ok(provider.hook(start));
    let held = session_state(&fixture);
    assert_eq!(
        (held[0].as_str(), held[4].as_str()),
        ("unknown", "s-1"),
        "the start alone switches nothing: {held:?}"
    );
    clear_end(&fixture, &mut provider);
    let mut prompt = in_new_session("UserPromptSubmit");
    prompt["prompt"] = "next".into();
    stop_capture::ok(provider.hook(prompt));
    let after = session_state(&fixture);
    assert_eq!(
        (after[0].as_str(), after[4].as_str()),
        ("running", "s-2"),
        "{after:?}"
    );
}

#[test]
fn another_session_of_the_same_process_cannot_take_over_a_running_member() {
    // A side conversation or subagent reporting from the same process, with the launch claim.
    let (fixture, mut provider) = member("switch-side-session", "claude", true);
    let mut stop = in_new_session("Stop");
    stop["stop_hook_active"] = false.into();
    let _ = provider.digest_hook(stop);
    let mut prompt = in_new_session("UserPromptSubmit");
    prompt["prompt"] = "side".into();
    let _ = provider.hook(prompt);
    let after = session_state(&fixture);
    assert_eq!(
        (after[0].as_str(), after[4].as_str()),
        ("running", "s-1"),
        "{after:?}"
    );
}

#[test]
fn a_member_bound_by_hand_never_switches_sessions() {
    let (fixture, mut provider) = cleared_member("switch-no-claim", false);
    let mut stop = in_new_session("Stop");
    stop["stop_hook_active"] = false.into();
    let _ = provider.digest_hook(stop);
    let after = session_state(&fixture);
    assert_eq!(
        (after[0].as_str(), after[1].as_str(), after[4].as_str()),
        ("unknown", "cleared", "s-1"),
        "{after:?}"
    );
}

#[test]
fn a_codex_thread_started_before_the_old_one_ended_continues_on_its_next_stop() {
    // gui-3's order on 10-28: the new thread's start is refused while the old one still runs,
    // then the old thread ends, leaving the process awaiting its next session.
    let (fixture, mut provider) = member("switch-codex", "codex", true);
    let mut start = event("codex", "SessionStart");
    start["session_id"] = "s-2".into();
    let _ = provider.hook(start);
    let mut end = event("codex", "SessionEnd");
    end["reason"] = "other".into();
    stop_capture::ok(provider.hook(end));
    let waiting = session_state(&fixture);
    assert_eq!(
        (waiting[0].as_str(), waiting[1].as_str()),
        ("unknown", "ended"),
        "{waiting:?}"
    );
    let mut stop = event("codex", "Stop");
    stop["session_id"] = "s-2".into();
    let feedback = stop_capture::ok(provider.digest_hook(stop));
    let after = session_state(&fixture);
    assert_eq!(
        (after[0].as_str(), after[4].as_str()),
        ("running", "s-2"),
        "{after:?}"
    );
    let feedback: Value = serde_json::from_str(&feedback).unwrap();
    assert!(
        feedback["reason"]
            .as_str()
            .unwrap()
            .contains("Core decision item"),
        "{feedback:#}"
    );
}
