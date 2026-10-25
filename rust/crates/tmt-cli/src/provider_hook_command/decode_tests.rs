//! A hook that stops before it can act says where and on which event, never what it held.
use super::*;
use crate::session_learning::tests::{Launch, launch};
use tmt_core::host::HostKind;

/// What a worker leaves behind for one payload: `observe`, then the parent's failure.
fn stop_of(launch: &Launch, provider: &str, payload: &str) -> (Result<Observation, ()>, String) {
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
    for (provider, payload, cause) in [
        (
            "claude",
            r#"{"hook_event_name":"FutureEvent","session_id":"s"}"#,
            "decode: worker_error, refused:unknown",
        ),
        (
            "claude",
            r#"{"session_id":"s"}"#,
            "decode: worker_error, refused:unknown",
        ),
        (
            "claude",
            "not json at all, with a secret in it",
            "decode: worker_error, refused:unknown",
        ),
        (
            "claude",
            r#"{"hook_event_name":"Stop"}"#,
            "decode: worker_error, refused:turn:Stop",
        ),
        (
            "claude",
            r#"{"hook_event_name":"UserPromptSubmit","prompt":"private"}"#,
            "decode: worker_error, refused:prompt:UserPromptSubmit",
        ),
        (
            "claude",
            r#"{"hook_event_name":"SessionStart","source":"startup"}"#,
            "decode: worker_error, refused:lifecycle:SessionStart",
        ),
        (
            "codex",
            r#"{"hook_event_name":"Notification","message":"private"}"#,
            "decode: worker_error, refused:activity_session:Notification",
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
    };
    assert!(unsupported_host(quiet).is_ok());
}
