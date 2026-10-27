//! A hand-started provider in an unbound pane binds the one saved identity that remembers its
//! session (#5401). Every refusal leaves today's behavior: the unbound hint, nothing bound.
//!
//! The pane shell of `f2` is the test process; the provider runs below a `sh` the test owns, as
//! in `nested_provider`. A restart is simulated by detaching the identity's old binding, which
//! leaves its remembered session behind.
use super::*;

const SESSION: &str = "1b4e28ba-2fa1-4d4b-8d4a-0f4f4f4f0001";

fn event(session: &str) -> Value {
    serde_json::json!({"hook_event_name": "SessionStart", "session_id": session,
        "source": "resume", "cwd": "/src", "model": "claude-test"})
}

fn stored(fixture: &Fixture) -> tmt_adapters::storage::IdentityContextSnapshot {
    super::move_session::remembered(fixture)
}

/// `worker` is bound in `f1` and learns `SESSION` from a provider that then exits.
fn learned(fixture: &Fixture) {
    bind(fixture, fixture.approve());
    let mut old = SteppedProvider::start(fixture, "claude", "old-provider");
    let output = old.hook(event(SESSION));
    assert_eq!((output.0, output.2.as_str()), (0, ""), "{output:?}");
    drop(old);
    assert_eq!(
        stored(fixture)
            .preferences
            .remembered
            .unwrap()
            .provider_session
            .as_str(),
        SESSION
    );
}

/// What a restart does to the binding: it is gone, the remembered session stays.
fn restart(fixture: &Fixture) {
    use tmt_core::binding::BindingRepository;
    let identity = stored(fixture).entry.identity.id;
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    storage
        .with_binding_transaction::<_, tmt_adapters::storage::StorageError>(|rows| {
            let binding = rows.entry_by_id(&identity)?.unwrap().binding.unwrap();
            rows.detach_binding(&binding.id)?;
            Ok(())
        })
        .unwrap();
    storage.close().unwrap();
    assert!(stored(fixture).entry.binding.is_none());
}

/// A provider below its own pane shell in `f2`, which the test makes the second pane.
fn in_f2(fixture: &Fixture, name: &str, ancestors: &[&'static str]) -> SteppedProvider {
    fs::write(fixture.root.join("driver/two-panes"), "").unwrap();
    let provider = SteppedProvider::start_nested_in(fixture, "claude", name, "f2", ancestors);
    fs::write(
        fixture.root.join("driver/pid-2"),
        provider.child.id().to_string(),
    )
    .unwrap();
    provider
}

/// The stored rows, as `stored_rows` lists them, with only the differing ones shown on failure.
fn assert_rows_unchanged(before: &[String], after: &[String]) {
    let changed: Vec<_> = before
        .iter()
        .filter(|row| !after.contains(row))
        .map(|row| format!("- {row}"))
        .chain(
            after
                .iter()
                .filter(|row| !before.contains(row))
                .map(|row| format!("+ {row}")),
        )
        .collect();
    assert!(
        changed.is_empty(),
        "stored rows changed:\n{}",
        changed.join("\n")
    );
}

fn is_unbound_hint(output: &(i32, String, String)) -> bool {
    let context: Option<Value> = serde_json::from_str(&output.1).ok();
    output.0 == 0
        && output.2.is_empty()
        && context
            .is_some_and(|context| context["hookSpecificOutput"]["hookEventName"] == "SessionStart")
}

#[test]
fn a_hand_started_provider_binds_the_identity_that_remembers_its_session() {
    register();
    let fixture = Fixture::new("self-bind");
    learned(&fixture);
    restart(&fixture);
    let mut main = in_f2(&fixture, "main-steps", &["sh"]);
    let process = live_provider(&main);
    let output = main.hook(event(SESSION));
    assert_eq!((output.0, output.2.as_str()), (0, ""), "{output:?}");
    let after = stored(&fixture);
    let binding = after.entry.binding.expect("bound to the provider's pane");
    assert_eq!(binding.pane_id, "fake-2");
    let key = binding.session.key.expect("the session is admitted");
    assert_eq!(key.incarnation, process.incarnation);
    assert_eq!(key.provider_session.unwrap().as_str(), SESSION);
    assert!(binding.session.start_refusal.is_none());
    assert_eq!(
        worker(&fixture.json(&["ls", "--json"]))["session"]["unbound"],
        false
    );
    drop(main);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

#[test]
fn a_session_nobody_remembers_gets_the_hint_and_no_binding() {
    register();
    let fixture = Fixture::new("self-bind-unknown");
    learned(&fixture);
    restart(&fixture);
    let rows = stored_rows(&fixture);
    let mut main = in_f2(&fixture, "main-steps", &["sh"]);
    let process = live_provider(&main);
    let output = main.hook(event("a-session-no-identity-remembers"));
    assert!(is_unbound_hint(&output), "{output:?}");
    assert_rows_unchanged(&rows, &stored_rows(&fixture));
    assert!(stored(&fixture).entry.binding.is_none());
    drop(main);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

/// The identity still has its binding in `f1`, so the session is not free to move.
#[test]
fn an_identity_that_is_active_elsewhere_is_never_taken_over() {
    register();
    let fixture = Fixture::new("self-bind-active");
    learned(&fixture);
    let before = stored(&fixture).entry.binding.unwrap();
    let rows = stored_rows(&fixture);
    let mut main = in_f2(&fixture, "main-steps", &["sh"]);
    let process = live_provider(&main);
    let output = main.hook(event(SESSION));
    assert!(is_unbound_hint(&output), "{output:?}");
    // The refused bind may refresh the live binding's verification time, nothing else.
    let after = stored(&fixture).entry.binding.unwrap();
    assert_eq!(
        (&after.id, &after.pane_id, &after.session),
        (&before.id, &before.pane_id, &before.session)
    );
    assert_eq!(after.pane_id, "fake-1");
    let bindings = |rows: &[String]| {
        rows.iter()
            .filter(|row| row.starts_with("bindings: "))
            .count()
    };
    assert_eq!(bindings(&stored_rows(&fixture)), bindings(&rows));
    drop(main);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

/// A nested provider is refused before any identity is chosen, matching session or not.
#[test]
fn a_nested_provider_binds_nothing() {
    register();
    let fixture = Fixture::new("self-bind-nested");
    learned(&fixture);
    restart(&fixture);
    let mut worker_steps = in_f2(&fixture, "worker-steps", &["sh", "claude", "claude"]);
    let process = live_provider(&worker_steps);
    let rows = stored_rows(&fixture);
    let output = worker_steps.hook(event(SESSION));
    assert_eq!(output.0, 0);
    assert_eq!(output.2, HOOK_UNAVAILABLE);
    assert!(output.1.is_empty(), "{output:?}");
    assert_rows_unchanged(&rows, &stored_rows(&fixture));
    assert!(stored(&fixture).entry.binding.is_none());
    drop(worker_steps);
    assert_eq!(process.seen(), Seen::Ended);
    assert_drivers_ended(&fixture);
}

/// The provider is the pane's own process, which the ordinary start admission refuses to adopt.
/// The self-bind has already bound the pane, as `tmt this` would: the pane stays bound with no
/// session key, and the refusal reaches the stored diagnostic.
#[test]
fn a_refused_start_leaves_the_pane_bound_with_the_reason_recorded() {
    register();
    let fixture = Fixture::new("self-bind-pane-process");
    learned(&fixture);
    restart(&fixture);
    fs::write(fixture.root.join("driver/two-panes"), "").unwrap();
    let mut claude = SteppedProvider::start_in(&fixture, "claude", "f2-steps", "f2", false);
    fs::write(
        fixture.root.join("driver/pid-2"),
        claude.child.id().to_string(),
    )
    .unwrap();
    let process = live_provider(&claude);
    let pid = process.incarnation.pid().to_string().into_bytes();
    let output = claude.hook_with_env(event(SESSION), &[("CLAUDE_PID", &pid)]);
    assert_eq!((output.0, output.2.as_str()), (0, HOOK_UNAVAILABLE));
    let after = stored(&fixture);
    let binding = after.entry.binding.expect("bound, not left unbound");
    assert_eq!(binding.pane_id, "fake-2");
    assert!(binding.session.key.is_none());
    let refusal = binding.session.start_refusal.expect("the reason is kept");
    assert_eq!(refusal.harness.as_str(), "claude");
    assert_eq!(refusal.session.as_str(), SESSION);
    assert_eq!(refusal.reason.as_str(), "pane_process");
    drop(claude);
    assert_drivers_ended(&fixture);
}
