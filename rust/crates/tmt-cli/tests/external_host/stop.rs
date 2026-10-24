//! `tmt stop` uses the move's verified stop with owned fake providers and a
//! private host driver; no test signals anything it did not start.
use super::move_session::{remembered, start};
use super::*;

fn stop(fixture: &Fixture, selector: &str) -> std::process::Output {
    stop_with(fixture, selector, &[])
}

fn stop_with(fixture: &Fixture, selector: &str, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["stop", selector, "--json"];
    args.extend_from_slice(extra);
    fixture.command_in(None, &args).output().unwrap()
}

/// The refusal a failed `--json` stop prints.
fn refusal(output: &std::process::Output) -> String {
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    error["error"]["message"].as_str().unwrap().to_owned()
}

fn outcome(output: &std::process::Output) -> Value {
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn stop_ends_only_the_recorded_agent_and_keeps_the_binding_and_session() {
    register();
    for (provider, by_uuid) in [("claude", false), ("codex", true)] {
        let fixture = Fixture::new(&format!("stop-{provider}"));
        let (old, process) = start(&fixture, provider, fixture.approve());
        let before = remembered(&fixture);
        let selector = if by_uuid {
            before.entry.identity.id.clone()
        } else {
            "worker".to_owned()
        };

        let stopped = outcome(&stop(&fixture, &selector));
        assert_eq!(stopped["outcome"], "stopped");
        assert_eq!(stopped["identity"]["id"], before.entry.identity.id);
        assert_eq!(stopped["identity"]["name"], "worker");
        assert_eq!(stopped.get("pane"), None, "no pane without --close");
        assert!(fixture.requests("closed").is_empty());
        assert_eq!(process.seen(), Seen::Ended);
        let after = remembered(&fixture);
        assert_eq!(after.entry, before.entry, "the binding stays");
        assert_eq!(after.preferences, before.preferences, "the session stays");

        // A second stop finds the agent gone and signals nothing.
        let again = outcome(&stop(&fixture, &selector));
        assert_eq!(again["outcome"], "already_ended");
        assert_eq!(remembered(&fixture).entry, before.entry);
        drop(old);
        assert_drivers_ended(&fixture);
    }
}

#[test]
fn stop_sends_one_term_and_refuses_an_agent_that_does_not_exit() {
    register();
    let fixture = Fixture::new("stop-ignores-term");
    fs::create_dir_all(fixture.root.join("old-steps")).unwrap();
    fs::write(fixture.root.join("old-steps/ignore-term"), "").unwrap();
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let before = remembered(&fixture);
    let output = stop(&fixture, "worker");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = refusal(&output);
    assert!(error.contains("Exit the old agent in its pane"), "{error}");
    assert_eq!(process.seen(), Seen::Live);
    assert_eq!(
        fs::read_to_string(fixture.root.join("old-steps/terms")).unwrap(),
        "TERM\n"
    );
    let after = remembered(&fixture);
    assert_eq!(after.entry, before.entry);
    assert_eq!(after.preferences, before.preferences);
    drop(old);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

#[test]
fn stop_refuses_a_shared_codex_server_without_a_signal_or_binding_change() {
    use tmt_core::binding::{BindingRepository, session::RuntimeMode};
    register();
    let fixture = Fixture::new("stop-shared-codex");
    let (old, process) = start(&fixture, "codex", fixture.approve());
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let original = remembered(&fixture);
    storage
        .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|records| {
            let mut preferences = original.preferences.clone();
            preferences.remembered.as_mut().unwrap().mode = RuntimeMode::new("shared").unwrap();
            records.set_session_preferences(&original.entry.identity.id, &preferences)
        })
        .unwrap();
    storage.close().unwrap();
    let before = remembered(&fixture);
    let output = stop(&fixture, "worker");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = refusal(&output);
    assert!(
        error.contains("shared/channel servers are not stopped"),
        "{error}"
    );
    assert_eq!(process.seen(), Seen::Live);
    let after = remembered(&fixture);
    assert_eq!(after.entry, before.entry);
    assert_eq!(after.preferences, before.preferences);
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn stop_of_an_unbound_member_signals_nothing_and_an_unknown_or_retired_uuid_is_not_found() {
    register();
    let fixture = Fixture::new("stop-unbound");
    let _ = fixture.approve();
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let idle = tmt_core::identity::create_or_resolve(
        &mut storage,
        "idle",
        tmt_core::identity::Lifetime::Saved,
    )
    .unwrap()
    .identity;
    let gone = tmt_core::identity::create_or_resolve(
        &mut storage,
        "gone",
        tmt_core::identity::Lifetime::Saved,
    )
    .unwrap()
    .identity;
    storage.close().unwrap();
    assert!(
        fixture
            .command_in(None, &["rm", "--force", "gone"])
            .output()
            .unwrap()
            .status
            .success()
    );

    for selector in [idle.name.as_str(), idle.id.as_str()] {
        assert_eq!(outcome(&stop(&fixture, selector))["outcome"], "not_running");
    }
    // A member with no binding has no pane to close either.
    let closed = outcome(&stop_with(&fixture, &idle.name, &["--close"]));
    assert_eq!(
        (closed["outcome"].as_str(), closed["pane"].as_str()),
        (Some("not_running"), Some("none"))
    );
    assert!(fixture.requests("closed").is_empty());
    for selector in [
        gone.id.as_str(),
        "7f0c2f2e-0000-4000-8000-000000000000",
        "nobody",
    ] {
        let output = stop(&fixture, selector);
        assert_eq!(output.status.code(), Some(3), "{selector}: {output:?}");
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"]["code"], "NAME_NOT_FOUND");
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains(selector),
            "{error}"
        );
    }
}

#[test]
fn stop_close_closes_only_the_pane_the_host_says_tmt_opened_and_only_after_the_stop() {
    register();
    for (provider, by_uuid) in [("claude", false), ("codex", true)] {
        let fixture = Fixture::closing(&format!("stop-close-{provider}"));
        fs::write(fixture.root.join("driver/opened"), "true").unwrap();
        let (old, process) = start(&fixture, provider, fixture.approve());
        let before = remembered(&fixture);
        let selector = if by_uuid {
            before.entry.identity.id.clone()
        } else {
            "worker".to_owned()
        };

        let stopped = outcome(&stop_with(&fixture, &selector, &["--close"]));
        assert_eq!(stopped["outcome"], "stopped");
        assert_eq!(stopped["pane"], "closed");
        assert_eq!(process.seen(), Seen::Ended);
        // One close for exactly the verified pane, after the agent was gone.
        let closes = fixture.requests("closed");
        assert_eq!(closes.len(), 1, "{closes:?}");
        let binding = before.entry.binding.as_ref().unwrap();
        assert_eq!(closes[0]["paneId"], binding.pane_id.as_str());
        assert_eq!(closes[0]["panePid"], binding.pane_pid);
        assert_eq!(closes[0]["socket"], binding.server.socket_path.as_str());
        // The identity keeps its binding record and session, so it can resume.
        let after = remembered(&fixture);
        assert_eq!(after.entry, before.entry);
        assert_eq!(after.preferences, before.preferences);
        drop(old);
        assert_drivers_ended(&fixture);
    }
}

#[test]
fn stop_close_leaves_a_pane_the_host_cannot_vouch_for_and_says_so_after_a_real_stop() {
    register();
    // No mark, a false mark, and a host that has no close at all.
    for (case, opened, closing, expected) in [
        ("no-mark", None, true, "TMT did not open that pane"),
        (
            "false-mark",
            Some("false"),
            true,
            "TMT did not open that pane",
        ),
        ("no-close", Some("true"), false, "cannot close a pane"),
    ] {
        let fixture = if closing {
            Fixture::closing(&format!("stop-close-kept-{case}"))
        } else {
            Fixture::new(&format!("stop-close-kept-{case}"))
        };
        if let Some(opened) = opened {
            fs::write(fixture.root.join("driver/opened"), opened).unwrap();
        }
        let (old, process) = start(&fixture, "claude", fixture.approve());
        let before = remembered(&fixture);
        let output = stop_with(&fixture, "worker", &["--close"]);
        assert_eq!(output.status.code(), Some(1), "{case}: {output:?}");
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"]["code"], "PANE_NOT_CLOSED", "{case}");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("was stopped, but its pane was left open")
                && message.contains(expected),
            "{case}: {message}"
        );
        // The agent stopped; nothing was closed; the records are untouched.
        assert_eq!(process.seen(), Seen::Ended, "{case}");
        assert!(fixture.requests("closed").is_empty(), "{case}");
        assert_eq!(remembered(&fixture).entry, before.entry);
        drop(old);
        assert_drivers_ended(&fixture);
    }
}

#[test]
fn stop_close_asks_the_host_once_and_reports_a_refusal_without_retrying() {
    register();
    let fixture = Fixture::closing("stop-close-refused");
    fs::write(fixture.root.join("driver/opened"), "true").unwrap();
    fs::write(
        fixture.root.join("driver/close-answer"),
        r#"{"error":{"code":"failed","message":"host would not close it"}}"#,
    )
    .unwrap();
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let output = stop_with(&fixture, "worker", &["--close"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "PANE_NOT_CLOSED");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("was stopped, but its pane could not be closed"),
        "{message}"
    );
    assert_eq!(process.seen(), Seen::Ended);
    assert_eq!(fixture.requests("closed").len(), 1, "one close, no retry");
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn stop_close_has_nothing_to_close_when_the_agent_and_its_pane_are_both_gone() {
    register();
    let fixture = Fixture::closing("stop-close-gone");
    fs::write(fixture.root.join("driver/opened"), "true").unwrap();
    let (old, process) = start(&fixture, "claude", fixture.approve());
    drop(old);
    assert_eq!(process.seen(), Seen::Ended);
    fs::write(fixture.root.join("driver/absent"), "").unwrap();
    let gone = outcome(&stop_with(&fixture, "worker", &["--close"]));
    assert_eq!(gone["outcome"], "already_ended");
    assert_eq!(gone["pane"], "gone");
    assert!(fixture.requests("closed").is_empty());
    assert_drivers_ended(&fixture);
}
