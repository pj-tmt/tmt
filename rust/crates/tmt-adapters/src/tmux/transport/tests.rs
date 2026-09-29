use super::*;
use crate::process::CommandFailure;
use crate::scripted_runner::{ScriptedRunner, failure, failure_with_kind};
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};
use tmt_core::limits::MAX_CAPTURE_LINES;

const SOCKET: &str = "/tmp/private.sock";
const PANE: &str = "%9";

fn assert_deadlines_and_cap(runner: &ScriptedRunner, expected_cap: usize, started: Instant) {
    let now = Instant::now();
    let calls = runner.calls.borrow();
    assert!(!calls.is_empty());
    for call in calls.iter() {
        assert_eq!(call.program, "tmux");
        assert_eq!(call.input, Vec::<u8>::new());
        assert_eq!(call.max_output_bytes, expected_cap);
        assert!(call.deadline >= started + Duration::from_secs(1));
        assert!(call.deadline <= now + Duration::from_secs(1));
    }
}

fn assert_socket(calls: &[crate::scripted_runner::Invocation]) {
    for call in calls {
        assert_eq!(&call.args[..2], ["-S", SOCKET]);
    }
}

#[test]
fn send_uses_explicit_socket_protected_payload_owned_buffer_and_one_enter() {
    let runner = ScriptedRunner::new([Ok(""), Ok(""), Ok("")]);
    let tmux = Tmux::new(runner);
    let waited = RefCell::new(Vec::new());
    // Deliberately includes ASCII punctuation, Unicode, and an existing newline.
    let message = "if (!ready)!\n尾";
    let started = Instant::now();

    tmux.send_with_wait(SOCKET, PANE, message, Duration::from_millis(731), |delay| {
        assert_eq!(tmux.runner.calls.borrow().len(), 2);
        waited.borrow_mut().push(delay);
    })
    .unwrap();

    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 3);
    assert_socket(&calls);
    assert_eq!(&calls[0].args[..4], ["-S", SOCKET, "set-buffer", "-b"]);
    assert_eq!(&calls[0].args[5..], ["--", "if (！ready)！\n尾\n"]);
    assert_eq!(&calls[1].args[..4], ["-S", SOCKET, "paste-buffer", "-b"]);
    assert_eq!(calls[1].args[4], calls[0].args[4]);
    assert_eq!(&calls[1].args[5..], ["-d", "-t", PANE, "-p"]);
    assert_eq!(
        &calls[2].args[..],
        ["-S", SOCKET, "send-keys", "-t", PANE, "Enter"]
    );
    assert_eq!(*waited.borrow(), [Duration::from_millis(731)]);
    drop(calls);
    assert_deadlines_and_cap(&tmux.runner, 64 * 1024, started);
}

#[test]
fn send_assigns_unique_buffer_names_across_messages() {
    let runner = ScriptedRunner::new([Ok(""), Ok(""), Ok(""), Ok(""), Ok(""), Ok("")]);
    let tmux = Tmux::new(runner);

    tmux.send_with_wait(SOCKET, PANE, "one\n", Duration::ZERO, |_| {})
        .unwrap();
    tmux.send_with_wait(SOCKET, PANE, "two\n", Duration::ZERO, |_| {})
        .unwrap();

    let calls = tmux.runner.calls.borrow();
    assert_ne!(calls[0].args[4], calls[3].args[4]);
    assert!(calls[0].args[4].starts_with("tmt-"));
    assert_eq!(calls[1].args[4], calls[0].args[4]);
    assert_eq!(calls[4].args[4], calls[3].args[4]);
}

#[test]
fn set_buffer_failure_has_one_literal_fallback_with_the_same_payload() {
    let runner = ScriptedRunner::new([Err(failure(false)), Ok(""), Ok(""), Ok("")]);
    let tmux = Tmux::new(runner);
    let waited = RefCell::new(Vec::new());

    tmux.send_with_wait(
        SOCKET,
        PANE,
        "-n weird!\nLine",
        Duration::from_millis(17),
        |delay| {
            assert_eq!(tmux.runner.calls.borrow().len(), 3);
            waited.borrow_mut().push(delay);
        },
    )
    .unwrap();

    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 4);
    assert_socket(&calls);
    assert_eq!(calls[0].args[2], "set-buffer");
    assert_eq!(
        &calls[1].args[..],
        [
            "-S",
            SOCKET,
            "delete-buffer",
            "-b",
            calls[0].args[4].as_str()
        ]
    );
    assert_eq!(
        &calls[2].args[2..],
        ["send-keys", "-l", "-t", PANE, "--", "-n weird！\nLine\n"]
    );
    assert_eq!(
        &calls[3].args[..],
        ["-S", SOCKET, "send-keys", "-t", PANE, "Enter"]
    );
    assert_eq!(calls[2].args.last(), calls[0].args.last());
    assert_eq!(*waited.borrow(), [Duration::from_millis(17)]);
}

#[test]
fn payload_preserves_empty_existing_newline_and_crlf_inputs() {
    for (message, expected) in [
        ("", "\n"),
        ("already\n", "already\n"),
        ("crlf\r\n", "crlf\r\n"),
    ] {
        let tmux = Tmux::new(ScriptedRunner::new([Ok(""), Ok(""), Ok("")]));
        tmux.send_with_wait(SOCKET, PANE, message, Duration::ZERO, |_| {})
            .unwrap();
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls[0].args.last().map(String::as_str), Some(expected));
    }
}

#[test]
fn paste_failure_preserves_primary_uncertainty_and_never_replays() {
    let runner = ScriptedRunner::new([
        Ok(""),
        Err(failure(false)),
        Err(failure_with_kind(
            CommandFailure::Exit {
                code: Some(7),
                signal: None,
            },
            false,
        )),
    ]);
    let tmux = Tmux::new(runner);

    let error = tmux
        .send_with_wait(SOCKET, PANE, "body", Duration::ZERO, |_| {})
        .unwrap_err();
    assert_eq!(error.stage, DeliveryStage::Paste);
    assert!(error.uncertain());
    assert!(!error.cleanup_failed());
    assert_eq!(error.cause.kind, TmuxFailure::Command);
    assert_eq!(
        error.cause.cause.as_ref().unwrap().kind,
        CommandFailure::Timeout
    );
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2].args[2], "delete-buffer");
}

#[test]
fn paste_cleanup_failure_remains_observable_without_fallback() {
    let runner = ScriptedRunner::new([Ok(""), Err(failure(false)), Err(failure(true))]);
    let tmux = Tmux::new(runner);

    let error = tmux
        .send_with_wait(SOCKET, PANE, "body", Duration::ZERO, |_| {})
        .unwrap_err();
    assert_eq!(error.stage, DeliveryStage::Paste);
    assert!(error.uncertain());
    assert!(error.cleanup_failed());
    assert!(error.cleanup_error.is_some());
    assert_eq!(tmux.runner.calls.borrow().len(), 3);
}

#[test]
fn literal_failure_never_sends_enter_or_replays_payload() {
    let runner = ScriptedRunner::new([
        Err(failure(false)),
        Ok(""),
        Err(failure_with_kind(
            CommandFailure::Exit {
                code: Some(7),
                signal: None,
            },
            false,
        )),
    ]);
    let tmux = Tmux::new(runner);

    let error = tmux
        .send_with_wait(SOCKET, PANE, "body", Duration::ZERO, |_| {})
        .unwrap_err();
    assert_eq!(error.stage, DeliveryStage::Literal);
    assert!(error.uncertain());
    assert_eq!(tmux.runner.calls.borrow().len(), 3);
}

#[test]
fn failed_child_cleanup_suppresses_literal_fallback_and_keeps_both_causes() {
    let runner = ScriptedRunner::new([Err(failure(true)), Err(failure(true))]);
    let tmux = Tmux::new(runner);

    let error = tmux
        .send_with_wait(SOCKET, PANE, "body", Duration::ZERO, |_| {})
        .unwrap_err();
    assert_eq!(error.stage, DeliveryStage::Prepare);
    assert!(!error.uncertain());
    assert!(error.cleanup_failed());
    assert!(error.cause.cleanup_failed());
    assert!(error.cleanup_error.is_some());
    assert_eq!(tmux.runner.calls.borrow().len(), 2);
}

#[test]
fn primary_cleanup_failure_alone_suppresses_literal_fallback() {
    let runner = ScriptedRunner::new([Err(failure(true)), Ok("")]);
    let tmux = Tmux::new(runner);

    let error = tmux
        .send_with_wait(SOCKET, PANE, "body", Duration::ZERO, |_| {})
        .unwrap_err();
    assert_eq!(error.stage, DeliveryStage::Prepare);
    assert!(error.cause.cleanup_failed());
    assert!(error.cleanup_error.is_none());
    assert!(error.cleanup_failed());
    assert_eq!(tmux.runner.calls.borrow().len(), 2);
}

#[test]
fn secondary_cleanup_failure_alone_suppresses_literal_fallback() {
    let runner = ScriptedRunner::new([Err(failure(false)), Err(failure(true))]);
    let tmux = Tmux::new(runner);

    let error = tmux
        .send_with_wait(SOCKET, PANE, "body", Duration::ZERO, |_| {})
        .unwrap_err();
    assert_eq!(error.stage, DeliveryStage::Prepare);
    assert!(!error.cause.cleanup_failed());
    assert!(error.cleanup_error.is_some());
    assert!(error.cleanup_failed());
    assert_eq!(tmux.runner.calls.borrow().len(), 2);
}

#[test]
fn submit_failure_is_uncertain_and_does_not_replay_enter() {
    let runner = ScriptedRunner::new([Ok(""), Ok(""), Err(failure(false))]);
    let tmux = Tmux::new(runner);
    let waits = RefCell::new(0);

    let error = tmux
        .send_with_wait(SOCKET, PANE, "body", Duration::from_millis(12), |_| {
            *waits.borrow_mut() += 1;
        })
        .unwrap_err();
    assert_eq!(error.stage, DeliveryStage::Submit);
    assert!(error.uncertain());
    assert_eq!(*waits.borrow(), 1);
    assert_eq!(tmux.runner.calls.borrow().len(), 3);
}

#[test]
fn invalid_send_targets_text_and_capture_lines_spawn_nothing() {
    for (socket, pane, message) in [
        ("", PANE, "body"),
        ("/tmp/\0sock", PANE, "body"),
        (SOCKET, "main:1.0", "body"),
        (SOCKET, PANE, "nul\0body"),
    ] {
        let runner = ScriptedRunner::default();
        let tmux = Tmux::new(runner);
        let error = tmux
            .send_with_wait(socket, pane, message, Duration::ZERO, |_| {})
            .unwrap_err();
        assert_eq!(error.stage, DeliveryStage::Prepare);
        assert!(!error.uncertain());
        assert!(tmux.runner.calls.borrow().is_empty());
    }

    let runner = ScriptedRunner::default();
    let tmux = Tmux::new(runner);
    let error = tmux.capture_on("", PANE, 0).unwrap_err();
    assert_eq!(error.kind, TmuxFailure::Evidence);
    assert!(tmux.runner.calls.borrow().is_empty());

    let runner = ScriptedRunner::default();
    let tmux = Tmux::new(runner);
    let error = tmux.capture_on(SOCKET, "main:1.0", 0).unwrap_err();
    assert_eq!(error.kind, TmuxFailure::Evidence);
    assert!(tmux.runner.calls.borrow().is_empty());

    let runner = ScriptedRunner::default();
    let tmux = Tmux::new(runner);
    let error = tmux
        .capture_on(SOCKET, PANE, MAX_CAPTURE_LINES + 1)
        .unwrap_err();
    assert_eq!(error.kind, TmuxFailure::Evidence);
    assert!(tmux.runner.calls.borrow().is_empty());
}

#[test]
fn capture_uses_explicit_socket_preserves_complete_output_and_replaces_invalid_utf8() {
    let runner = ScriptedRunner::default();
    let mut bytes = "line one\n尾\n".as_bytes().to_vec();
    bytes.push(0xff);
    runner.push_output(bytes, Vec::new());
    let tmux = Tmux::new(runner);
    let started = Instant::now();

    let output = tmux.capture_on(SOCKET, PANE, 0).unwrap();
    assert_eq!(output, "line one\n尾\n�");
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        &calls[0].args[..],
        ["-S", SOCKET, "capture-pane", "-t", PANE, "-p", "-S", "-0"]
    );
    drop(calls);
    assert_deadlines_and_cap(&tmux.runner, 4 * 1024 * 1024, started);
}

#[test]
fn capture_propagates_output_overflow_and_timeout_without_partial_success() {
    for kind in [CommandFailure::OutputLimit, CommandFailure::Timeout] {
        let runner = ScriptedRunner::new([Err(failure_with_kind(kind, false))]);
        let tmux = Tmux::new(runner);
        let error = tmux.capture_on(SOCKET, PANE, 100).unwrap_err();
        assert_eq!(error.kind, TmuxFailure::Command);
        assert_eq!(error.cause.as_ref().unwrap().kind, kind);
        assert_eq!(tmux.runner.calls.borrow().len(), 1);
    }
}

#[test]
fn capture_accepts_maximum_lines_and_a_complete_four_mib_body() {
    let runner = ScriptedRunner::default();
    runner.push_output(vec![b'x'; 4 * 1024 * 1024], Vec::new());
    let tmux = Tmux::new(runner);

    let output = tmux.capture_on(SOCKET, PANE, MAX_CAPTURE_LINES).unwrap();
    assert_eq!(output.len(), 4 * 1024 * 1024);
    assert!(output.bytes().all(|byte| byte == b'x'));
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 1);
    let expected_lines = format!("-{MAX_CAPTURE_LINES}");
    assert_eq!(
        &calls[0].args[..],
        [
            "-S",
            SOCKET,
            "capture-pane",
            "-t",
            PANE,
            "-p",
            "-S",
            expected_lines.as_str(),
        ]
    );
    assert_eq!(calls[0].max_output_bytes, 4 * 1024 * 1024);
}
