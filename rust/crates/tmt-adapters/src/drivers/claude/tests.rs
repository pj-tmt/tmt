use super::*;
use serde_json::{Value, json};

fn event(starting: bool, transition: SessionTransition, session: &str) -> ClaudeObservation {
    ClaudeObservation {
        starting,
        transition,
        session: ProviderSessionId::new(session).unwrap(),
        model: None,
    }
}

#[test]
fn clear_keeps_the_binding_runtime_and_stale_end_cannot_end_the_new_session() {
    let process = RuntimeIncarnation::new(42, "start-A").unwrap();
    let first = event(true, SessionTransition::Started, "one")
        .propose(
            &BindingSessionState::default(),
            &process,
            RuntimeLiveness::Unknown,
        )
        .unwrap();
    assert!(
        event(true, SessionTransition::Cleared, "two")
            .propose(&first, &process, RuntimeLiveness::Alive)
            .is_none()
    );
    let clearing = event(false, SessionTransition::Cleared, "one")
        .propose(&first, &process, RuntimeLiveness::Alive)
        .unwrap();
    assert_eq!(clearing.state, RuntimeState::Unknown);
    let second = event(true, SessionTransition::Cleared, "two")
        .propose(&clearing, &process, RuntimeLiveness::Alive)
        .unwrap();
    assert_eq!(
        second
            .key
            .as_ref()
            .unwrap()
            .provider_session
            .as_ref()
            .unwrap()
            .as_str(),
        "two"
    );
    assert!(
        event(true, SessionTransition::Cleared, "old-delayed-start")
            .propose(&second, &process, RuntimeLiveness::Alive)
            .is_none()
    );
    assert!(
        event(false, SessionTransition::Ended, "one")
            .propose(&second, &process, RuntimeLiveness::Alive)
            .is_none()
    );
    assert!(
        event(true, SessionTransition::Compacted, "one")
            .propose(&second, &process, RuntimeLiveness::Alive)
            .is_none()
    );
    let compacted = event(true, SessionTransition::Compacted, "two")
        .propose(&second, &process, RuntimeLiveness::Alive)
        .unwrap();
    assert_eq!(compacted.key, second.key);
}

#[test]
fn only_proven_previous_process_loss_allows_new_incarnation_admission() {
    let old = RuntimeIncarnation::new(42, "start-A").unwrap();
    let new = RuntimeIncarnation::new(42, "start-B").unwrap();
    let first = event(true, SessionTransition::Started, "one")
        .propose(
            &BindingSessionState::default(),
            &old,
            RuntimeLiveness::Unknown,
        )
        .unwrap();
    for liveness in [RuntimeLiveness::Alive, RuntimeLiveness::Unknown] {
        assert!(
            event(true, SessionTransition::Resumed, "one")
                .propose(&first, &new, liveness)
                .is_none()
        );
    }
    let resumed = event(true, SessionTransition::Resumed, "one")
        .propose(&first, &new, RuntimeLiveness::Gone)
        .unwrap();
    assert_eq!(resumed.key.as_ref().unwrap().incarnation, new);
    let ended = event(false, SessionTransition::Ended, "one")
        .propose(&resumed, &new, RuntimeLiveness::Alive)
        .unwrap();
    assert!(
        event(true, SessionTransition::Started, "one")
            .propose(&ended, &new, RuntimeLiveness::Alive)
            .is_none()
    );
}

#[test]
fn hook_start_correlates_a_launched_child_and_resume_preserves_its_owner() {
    let process = RuntimeIncarnation::new(42, "start-A").unwrap();
    let owner = RuntimeIncarnation::new(41, "owner-A").unwrap();
    let launched = BindingSessionState::default()
        .admit_launched(
            ObservedSessionKey {
                incarnation: process.clone(),
                provider_session: None,
            },
            owner.clone(),
            SessionTransition::Started,
            RuntimeLiveness::Alive,
            RuntimeLiveness::Alive,
        )
        .unwrap();
    let started = event(true, SessionTransition::Started, "one")
        .propose(&launched, &process, RuntimeLiveness::Alive)
        .unwrap();
    assert_eq!(started.launch_owner.as_ref(), Some(&owner));
    let changing = event(false, SessionTransition::Resumed, "one")
        .propose(&started, &process, RuntimeLiveness::Alive)
        .unwrap();
    assert_eq!(changing.state, RuntimeState::Unknown);
    let resumed = event(true, SessionTransition::Resumed, "two")
        .propose(&changing, &process, RuntimeLiveness::Alive)
        .unwrap();
    assert_eq!(resumed.state, RuntimeState::Running);
    assert_eq!(resumed.launch_owner.as_ref(), Some(&owner));
    assert_eq!(
        resumed
            .key
            .as_ref()
            .unwrap()
            .provider_session
            .as_ref()
            .unwrap()
            .as_str(),
        "two"
    );
    assert!(
        event(false, SessionTransition::Ended, "one")
            .propose(&resumed, &process, RuntimeLiveness::Alive)
            .is_none()
    );
}

#[test]
fn fork_requires_admission_in_the_callers_existing_binding() {
    let old = RuntimeIncarnation::new(42, "start-A").unwrap();
    let fork = RuntimeIncarnation::new(43, "start-B").unwrap();
    let first = event(true, SessionTransition::Started, "one")
        .propose(
            &BindingSessionState::default(),
            &old,
            RuntimeLiveness::Unknown,
        )
        .unwrap();
    let event = event(true, SessionTransition::Forked, "fork");
    assert!(
        event
            .propose(&first, &fork, RuntimeLiveness::Alive)
            .is_none()
    );
    assert!(
        event
            .propose(&first, &fork, RuntimeLiveness::Unknown)
            .is_none()
    );
    let admitted = event.propose(&first, &fork, RuntimeLiveness::Gone).unwrap();
    assert_eq!(admitted.key.as_ref().unwrap().incarnation, fork);
    assert_eq!(admitted.last_transition, Some(SessionTransition::Forked));
    // A separately verified binding starts from its own state, not the old
    // pane's session. No identity or binding transfer exists in this mapper.
    let independent = event
        .propose(
            &BindingSessionState::default(),
            &fork,
            RuntimeLiveness::Unknown,
        )
        .unwrap();
    assert_eq!(independent, admitted);
}

#[test]
fn lifecycle_mapping_keeps_clear_and_resume_nonterminal() {
    for (event, key, source, expected, starting) in [
        (
            "SessionStart",
            "source",
            "startup",
            SessionTransition::Started,
            true,
        ),
        (
            "SessionStart",
            "source",
            "resume",
            SessionTransition::Resumed,
            true,
        ),
        (
            "SessionStart",
            "source",
            "clear",
            SessionTransition::Cleared,
            true,
        ),
        (
            "SessionStart",
            "source",
            "compact",
            SessionTransition::Compacted,
            true,
        ),
        (
            "SessionStart",
            "source",
            "fork",
            SessionTransition::Forked,
            true,
        ),
        (
            "SessionEnd",
            "reason",
            "clear",
            SessionTransition::Cleared,
            false,
        ),
        (
            "SessionEnd",
            "reason",
            "resume",
            SessionTransition::Resumed,
            false,
        ),
        (
            "SessionEnd",
            "reason",
            "other",
            SessionTransition::Ended,
            false,
        ),
        (
            "SessionEnd",
            "reason",
            "logout",
            SessionTransition::Ended,
            false,
        ),
        (
            "SessionEnd",
            "reason",
            "prompt_input_exit",
            SessionTransition::Ended,
            false,
        ),
    ] {
        let payload = json!({
            "hook_event_name": event,
            "session_id": "exact-session",
            key: source,
            "transcript_path": "/not/read",
            "cwd": "/not/identity/evidence"
        });
        let decoded = decode_hook(payload.to_string().as_bytes()).unwrap();
        assert_eq!(decoded.transition, expected);
        assert_eq!(decoded.starting, starting);
        assert_eq!(decoded.session.as_str(), "exact-session");
    }
}

#[test]
fn malformed_unknown_and_oversized_events_do_not_guess_a_transition() {
    assert_eq!(decode_hook(b"{oops"), Err(HookInputError::Invalid));
    assert_eq!(
        decode_hook(&vec![b' '; HOOK_INPUT_LIMIT + 1]),
        Err(HookInputError::TooLarge)
    );
    for payload in [
        json!({"hook_event_name":"Stop", "session_id":"session"}),
        json!({"hook_event_name":"SessionStart", "session_id":"session", "source":"future"}),
        json!({"hook_event_name":"SessionEnd", "session_id":"session"}),
    ] {
        assert_eq!(
            decode_hook(payload.to_string().as_bytes()),
            Err(HookInputError::Unsupported)
        );
    }
    assert_eq!(
        decode_hook(br#"{"hook_event_name":"SessionStart","session_id":"","source":"startup"}"#),
        Err(HookInputError::Invalid)
    );
}

#[test]
fn context_is_bounded_structured_data_without_decision_fields() {
    let context = "TMT identity: \"Reader\"\nRole: \"quoted \\\" text\"";
    let output: Value = serde_json::from_str(&encode_context(context).unwrap()).unwrap();
    assert_eq!(
        output,
        json!({"hookSpecificOutput": {
            "hookEventName":"SessionStart", "additionalContext": context
        }})
    );
    assert!(encode_context("").is_none());
    assert!(encode_context(&"x".repeat(CONTEXT_LIMIT + 1)).is_none());
    assert!(encode_context(&"x".repeat(CONTEXT_LIMIT)).is_some());
}

#[test]
fn hook_command_quotes_stable_launcher_without_resolving_it() {
    let entry = crate::runtime::hook_protocol::command_entry(NAME, "/tmp/Ben's tools/tmt");
    assert_eq!(
        entry,
        json!({"hooks":[{
            "type":"command", "command":"'/tmp/Ben'\\''s tools/tmt' __hook claude", "timeout":3
        }]})
    );
}
