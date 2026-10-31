//! `identity.stop` / `identity.remove` and the `tmt rm` guard, with owned fake
//! providers and a private host driver; no test signals anything it did not start.
use super::move_session::{remembered, start};
use super::*;

/// One `tmt api` call: its exit code and its one JSON document.
fn api(fixture: &Fixture, operation: &str, input: Value) -> (Option<i32>, Value) {
    use std::io::Write;
    let mut child = fixture
        .command_in(None, &["api"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let body = serde_json::json!({"version": 1, "operation": operation, "input": input});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"));
    (output.status.code(), value)
}

fn stop(fixture: &Fixture, id: &str, name: &str, close: bool) -> (Option<i32>, Value) {
    api(
        fixture,
        "identity.stop",
        serde_json::json!({"identityId": id, "expectedName": name, "close": close}),
    )
}

fn remove(fixture: &Fixture, id: &str, name: &str) -> (Option<i32>, Value) {
    api(
        fixture,
        "identity.remove",
        serde_json::json!({"identityId": id, "expectedName": name}),
    )
}

fn found(fixture: &Fixture, id: &str) -> bool {
    let (_, resolved) = api(
        fixture,
        "references.resolve",
        serde_json::json!({"identityIds": [id]}),
    );
    resolved["identities"][0]["found"] == true && resolved["identities"][0]["retired"] == false
}

#[test]
fn api_stop_closes_the_pane_tmt_opened_then_remove_deletes_the_identity() {
    register();
    let fixture = Fixture::closing("api-stop-closed");
    fs::write(fixture.root.join("driver/opened"), "true").unwrap();
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let id = remembered(&fixture).entry.identity.id;

    let (code, stopped) = stop(&fixture, &id, "worker", true);
    assert_eq!(code, Some(0), "{stopped}");
    assert_eq!(
        stopped,
        serde_json::json!({"outcome":"stopped","identityId":id,"name":"worker","pane":"closed"})
    );
    assert_eq!(process.seen(), Seen::Ended);
    assert_eq!(fixture.requests("closed").len(), 1);

    let (code, removed) = remove(&fixture, &id, "worker");
    assert_eq!(code, Some(0), "{removed}");
    assert_eq!(
        removed,
        serde_json::json!({"outcome":"removed","identityId":id,"name":"worker"})
    );
    assert!(!found(&fixture, &id));
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn api_stop_with_a_kept_pane_is_still_a_stop_and_names_why() {
    register();
    let fixture = Fixture::closing("api-stop-kept");
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let id = remembered(&fixture).entry.identity.id;

    let (code, stopped) = stop(&fixture, &id, "worker", true);
    assert_eq!(code, Some(0), "{stopped}");
    assert_eq!(stopped["outcome"], "stopped");
    assert_eq!(stopped["pane"], "kept");
    let reason = stopped["paneReason"].as_str().unwrap();
    assert!(
        reason.contains("pane was left open") && reason.contains("TMT did not open that pane"),
        "{reason}"
    );
    assert_eq!(process.seen(), Seen::Ended);
    assert!(fixture.requests("closed").is_empty());

    // The stopped agent is proven ended, so the identity can go.
    assert_eq!(remove(&fixture, &id, "worker").1["outcome"], "removed");
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn a_refused_stop_signals_nothing_and_the_running_identity_is_never_removed() {
    use tmt_core::binding::{BindingRepository, session::RuntimeMode};
    register();
    let fixture = Fixture::new("api-stop-refused");
    let (old, process) = start(&fixture, "codex", fixture.approve());
    let original = remembered(&fixture);
    let id = original.entry.identity.id.clone();
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    storage
        .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|records| {
            let mut preferences = original.preferences.clone();
            preferences.remembered.as_mut().unwrap().mode = RuntimeMode::new("shared").unwrap();
            records.set_session_preferences(&id, &preferences)
        })
        .unwrap();
    storage.close().unwrap();
    let before = remembered(&fixture);

    let (code, refused) = stop(&fixture, &id, "worker", true);
    assert_eq!(code, Some(1), "{refused}");
    assert_eq!(refused["error"]["code"], "RESUME_UNSUPPORTED");
    assert_eq!(refused["error"]["signalled"], false);
    assert_eq!(process.seen(), Seen::Live);

    let (code, kept) = remove(&fixture, &id, "worker");
    assert_eq!(code, Some(1), "{kept}");
    assert_eq!(kept["error"]["code"], "AGENT_RUNNING");
    assert!(
        kept["error"]["message"]
            .as_str()
            .unwrap()
            .contains("tmt stop worker"),
        "{kept}"
    );
    // The CLI refuses the same way, and --force does not override it.
    let output = fixture
        .command_in(None, &["rm", "--force", "worker", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "AGENT_RUNNING");
    assert_eq!(process.seen(), Seen::Live);
    let after = remembered(&fixture);
    assert_eq!(after.entry, before.entry);
    assert_eq!(after.preferences, before.preferences);
    assert!(found(&fixture, &id));

    // Once the agent has ended, both remove as before.
    drop(old);
    assert_eq!(process.seen(), Seen::Ended);
    assert_eq!(remove(&fixture, &id, "worker").1["outcome"], "removed");
    assert!(!found(&fixture, &id));
    assert_drivers_ended(&fixture);
}

#[test]
fn a_stop_whose_term_is_ignored_reports_it_was_signalled() {
    register();
    let fixture = Fixture::new("api-stop-ignored");
    fs::create_dir_all(fixture.root.join("old-steps")).unwrap();
    fs::write(fixture.root.join("old-steps/ignore-term"), "").unwrap();
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let id = remembered(&fixture).entry.identity.id;
    let (code, refused) = stop(&fixture, &id, "worker", false);
    assert_eq!(code, Some(1), "{refused}");
    assert_eq!(refused["error"]["code"], "PROCESS_ERROR");
    assert_eq!(refused["error"]["signalled"], true);
    assert_eq!(process.seen(), Seen::Live);
    assert_eq!(
        remove(&fixture, &id, "worker").1["error"]["code"],
        "AGENT_RUNNING"
    );
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn tmt_rm_refuses_a_running_agent_and_removes_after_it_ended() {
    register();
    let fixture = Fixture::new("rm-running");
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let rm = |fixture: &Fixture| {
        fixture
            .command_in(None, &["rm", "--force", "worker", "--json"])
            .output()
            .unwrap()
    };
    let output = rm(&fixture);
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "AGENT_RUNNING");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("still running") && message.contains("tmt stop worker"),
        "{message}"
    );
    assert_eq!(process.seen(), Seen::Live);

    assert!(
        fixture
            .command_in(None, &["stop", "worker", "--json"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let output = rm(&fixture);
    assert!(output.status.success(), "{output:?}");
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn a_stale_name_or_unknown_uuid_writes_nothing() {
    register();
    let fixture = Fixture::new("api-stop-guard");
    let (old, process) = start(&fixture, "claude", fixture.approve());
    let before = remembered(&fixture);
    let id = before.entry.identity.id.clone();
    let unknown = "7f0c2f2e-0000-4000-8000-000000000000";
    for (code, value) in [
        stop(&fixture, &id, "Worker", true),
        remove(&fixture, &id, "old"),
    ] {
        assert_eq!(code, Some(0), "{value}");
        assert_eq!(
            value,
            serde_json::json!({"outcome":"conflict","identityId":id,"name":"worker"})
        );
    }
    for (code, value) in [
        stop(&fixture, unknown, "worker", false),
        remove(&fixture, unknown, "worker"),
    ] {
        assert_eq!(code, Some(0), "{value}");
        assert_eq!(
            value,
            serde_json::json!({"outcome":"not-found","identityId":unknown})
        );
    }
    let (code, invalid) = stop(&fixture, "worker", "worker", false);
    assert_eq!(code, Some(1));
    assert_eq!(invalid["error"]["code"], "API_INPUT_INVALID");
    assert_eq!(process.seen(), Seen::Live);
    assert_eq!(remembered(&fixture).entry, before.entry);
    drop(old);
    assert_drivers_ended(&fixture);
}

#[test]
fn an_unbound_member_stops_as_not_running_and_removes() {
    register();
    let fixture = Fixture::new("api-unbound");
    let _ = fixture.approve();
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let idle = tmt_core::identity::create_or_resolve(
        &mut storage,
        "idle",
        tmt_core::identity::Lifetime::Saved,
    )
    .unwrap()
    .identity;
    storage.close().unwrap();
    let (code, stopped) = stop(&fixture, &idle.id, "idle", true);
    assert_eq!(code, Some(0));
    assert_eq!(
        stopped,
        serde_json::json!({"outcome":"not-running","identityId":idle.id,"name":"idle"})
    );
    // A saved identity needs no force through the API.
    assert_eq!(remove(&fixture, &idle.id, "idle").1["outcome"], "removed");
    assert!(!found(&fixture, &idle.id));
    // The retired UUID with its old name is not found by either operation.
    for (code, value) in [
        stop(&fixture, &idle.id, "idle", true),
        remove(&fixture, &idle.id, "idle"),
    ] {
        assert_eq!(code, Some(0), "{value}");
        assert_eq!(
            value,
            serde_json::json!({"outcome":"not-found","identityId":idle.id})
        );
    }
}
