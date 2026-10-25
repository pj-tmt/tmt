//! A hook that stops before it can act says where and on which event, never what it held.
use super::*;
use crate::session_learning::tests::{Launch, launch};
use tmt_core::host::HostKind;

/// What a worker leaves behind for one payload: `observe`, then the parent's failure.
fn stop_of(launch: &Launch, provider: &str, payload: &str) -> (Result<Observation, ()>, String) {
    launch
        .learning
        .set_role(with_lifecycle(provider, |lifecycle| {
            hook_role(lifecycle, payload.as_bytes())
        }));
    let result = observe(
        provider,
        payload,
        launch.deadline,
        false,
        Some(&launch.learning),
    );
    if result.is_err() {
        launch
            .learning
            .fail(Reason::WorkerError, launch.deadline)
            .unwrap();
    }
    (result, launch.diagnostic().unwrap().cause())
}

#[test]
fn a_payload_no_decoder_accepts_names_its_event_and_the_decoder_that_owns_it() {
    // A SessionStart is the launch's own run and reports the step it stopped at; any other
    // run, which cannot bind, only names its event on the launch as the launch stands.
    for (provider, payload, cause) in [
        (
            "claude",
            r#"{"hook_event_name":"FutureEvent","session_id":"s"}"#,
            "input: not_admitted, refused:unknown",
        ),
        (
            "claude",
            r#"{"session_id":"s"}"#,
            "input: not_admitted, refused:unknown",
        ),
        (
            "claude",
            "not json at all, with a secret in it",
            "input: not_admitted, refused:unknown",
        ),
        (
            "claude",
            r#"{"hook_event_name":"Stop"}"#,
            "input: not_admitted, refused:turn:Stop",
        ),
        (
            "claude",
            r#"{"hook_event_name":"UserPromptSubmit","prompt":"private"}"#,
            "input: not_admitted, refused:prompt:UserPromptSubmit",
        ),
        (
            "claude",
            r#"{"hook_event_name":"SessionStart","source":"startup"}"#,
            "decode: worker_error, refused:lifecycle:SessionStart",
        ),
        (
            "codex",
            r#"{"hook_event_name":"Notification","message":"private"}"#,
            "input: not_admitted, refused:activity_session:Notification",
        ),
    ] {
        let launch = launch(HostKind::pty(), provider);
        let (result, stored) = stop_of(&launch, provider, payload);
        assert!(result.is_err(), "{payload}");
        assert_eq!(stored, cause, "{payload}");
        assert!(
            !stored.contains("private") && !stored.contains("secret"),
            "no payload text is stored: {stored}"
        );
    }
}

#[test]
fn a_hook_that_finds_no_provider_process_says_so_instead_of_stopping_silently() {
    let launch = launch(HostKind::pty(), "codex");
    let run = Run {
        deadline: launch.deadline,
        learning: Some(&launch.learning),
        event: SessionLearningEvent::Unknown,
    };
    let observed = unsupported_host(run).unwrap();
    assert!(observed.context.is_empty() && observed.workspace.is_none());
    assert_eq!(
        launch.diagnostic().unwrap().cause(),
        "host_discovery: not_admitted, host_unsupported"
    );
    // Without a launch rendezvous the hook still observes nothing and writes nothing.
    let quiet = Run {
        deadline: launch.deadline,
        learning: None,
        event: SessionLearningEvent::Unknown,
    };
    assert!(unsupported_host(quiet).is_ok());
}

/// Runs `check` with the first-party lifecycle of `provider`.
fn with_lifecycle<T>(provider: &str, check: impl FnOnce(&dyn RuntimeLifecycle) -> T) -> T {
    let registry = RuntimeRegistry::first_party();
    check(
        registry
            .lifecycle(&HarnessId::new(provider).unwrap())
            .unwrap(),
    )
}

#[test]
fn only_a_lifecycle_event_is_a_binding_run() {
    for (provider, payload, role) in [
        (
            "claude",
            r#"{"hook_event_name":"SessionStart","session_id":"s-1","source":"startup","cwd":"/src","model":"m"}"#,
            Role::Binding,
        ),
        (
            "claude",
            r#"{"hook_event_name":"SessionEnd","session_id":"s-1","reason":"other"}"#,
            Role::Binding,
        ),
        (
            "claude",
            r#"{"hook_event_name":"UserPromptSubmit","session_id":"s-1","prompt":"private"}"#,
            Role::Observer,
        ),
        (
            "claude",
            r#"{"hook_event_name":"Stop","session_id":"s-1","last_assistant_message":"done"}"#,
            Role::Observer,
        ),
        (
            "claude",
            r#"{"hook_event_name":"PostToolUse","session_id":"s-1","tool_name":"Bash"}"#,
            Role::Observer,
        ),
        // Nothing decodes these, so they cannot bind either.
        (
            "claude",
            r#"{"hook_event_name":"FutureEvent"}"#,
            Role::Observer,
        ),
        ("claude", "not json", Role::Observer),
        // A lifecycle event binds by its name, even when its payload is refused: that refusal is
        // the launch's own to report.
        (
            "claude",
            r#"{"hook_event_name":"SessionStart"}"#,
            Role::Binding,
        ),
    ] {
        assert!(
            with_lifecycle(provider, |lifecycle| hook_role(
                lifecycle,
                payload.as_bytes()
            )) == role,
            "{provider} {payload}"
        );
    }
}

#[test]
fn a_prompt_turn_or_activity_run_leaves_the_session_start_diagnostic_as_it_found_it() {
    for (provider, payload) in [
        (
            "claude",
            r#"{"hook_event_name":"UserPromptSubmit","session_id":"s-1","prompt":"private"}"#,
        ),
        (
            "claude",
            r#"{"hook_event_name":"Stop","session_id":"s-1","last_assistant_message":"private"}"#,
        ),
        (
            "claude",
            r#"{"hook_event_name":"PostToolUse","session_id":"s-1","tool_name":"Bash"}"#,
        ),
        ("claude", "not json at all, with a secret in it"),
        (
            "codex",
            r#"{"hook_event_name":"Notification","message":"private"}"#,
        ),
    ] {
        let launch = launch(HostKind::pty(), provider);
        // What the SessionStart run stored: where it stopped and on what.
        launch
            .learning
            .stage_detail(
                Step::HostDiscovery,
                SessionLearningDetail::HostUnsupported,
                launch.deadline,
            )
            .unwrap();
        launch
            .learning
            .fail(Reason::WorkerError, launch.deadline)
            .unwrap();
        let before = launch.diagnostic().unwrap().cause();
        assert_eq!(before, "host_discovery: worker_error, host_unsupported");
        // The later run: the same stages `execute` and `observe` write, then the parent's failure.
        launch
            .learning
            .set_role(with_lifecycle(provider, |lifecycle| {
                hook_role(lifecycle, payload.as_bytes())
            }));
        let (_, after) = stop_of(&launch, provider, payload);
        assert_eq!(after, before, "{provider} {payload}");
    }
}

#[test]
fn an_activity_only_event_without_an_activity_says_so() {
    let launch = launch(HostKind::pty(), "claude");
    // A notification that is not a permission prompt carries no activity.
    let payload = r#"{"hook_event_name":"Notification","session_id":"s-1","message":"Claude is waiting for your input"}"#;
    assert!(with_lifecycle("claude", |lifecycle| {
        lifecycle.decode_activity(payload.as_bytes()).is_none()
    }));
    launch.learning.set_role(Role::Observer);
    let observed = observe(
        "claude",
        payload,
        launch.deadline,
        true,
        Some(&launch.learning),
    )
    .unwrap();
    assert!(observed.context.is_empty() && observed.workspace.is_none());
    assert_eq!(
        launch.diagnostic().unwrap().cause(),
        "input: not_admitted, stopped:activity_session:Notification:activity_missing"
    );
}

#[test]
fn a_finished_tool_with_nothing_waiting_says_so() {
    let launch = launch(HostKind::pty(), "claude");
    launch.learning.set_role(Role::Observer);
    let harness = HarnessId::new("claude").unwrap();
    let session = ProviderSessionId::new("s-1").unwrap();
    for event in ["PostToolUse", "PostToolUseFailure"] {
        let payload = serde_json::json!({"hook_event_name": event, "session_id": "s-1"});
        let paths = launch.paths.clone();
        let observed = with_lifecycle("claude", |lifecycle| {
            let activity = lifecycle
                .decode_activity(payload.to_string().as_bytes())
                .unwrap();
            observe_activity_event(
                &harness,
                lifecycle,
                &session,
                &activity,
                Run {
                    deadline: launch.deadline,
                    learning: Some(&launch.learning),
                    event: SessionLearningEvent::from_hook_name(Some(event)),
                },
                || Ok(paths),
            )
            .unwrap()
        });
        assert!(observed.context.is_empty() && observed.workspace.is_none());
        // A launch keeps the first thing a run that cannot bind says about it.
        assert_eq!(
            launch.diagnostic().unwrap().cause(),
            "input: not_admitted, stopped:activity_session:PostToolUse:not_waiting"
        );
    }
}

#[test]
fn every_stop_a_hook_path_names_is_stored_under_its_event() {
    for provider_event in [
        SessionLearningEvent::Stop,
        SessionLearningEvent::Interrupt,
        SessionLearningEvent::UserPromptSubmit,
        SessionLearningEvent::Notification,
        SessionLearningEvent::PostToolUse,
        SessionLearningEvent::PostToolUseFailure,
        SessionLearningEvent::Unknown,
    ] {
        for stop in [
            Stop::ActivityMissing,
            Stop::NotWaiting,
            Stop::ObserverOwnsStop,
            Stop::UnboundSession,
            Stop::NotCurrent,
        ] {
            let Some(detail) = SessionLearningDetail::stopped(provider_event, stop) else {
                continue;
            };
            let launch = launch(HostKind::pty(), "claude");
            launch.learning.set_role(Role::Observer);
            Run {
                deadline: launch.deadline,
                learning: Some(&launch.learning),
                event: provider_event,
            }
            .stopped(stop);
            assert_eq!(
                launch.diagnostic().unwrap().cause(),
                format!("input: not_admitted, {}", detail.as_str())
            );
        }
    }
}
