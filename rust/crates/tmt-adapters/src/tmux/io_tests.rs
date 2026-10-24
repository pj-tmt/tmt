use super::*;
use crate::process::{CommandError, CommandFailure, CommandOutput};
use crate::scripted_runner::{ScriptedRunner, failure};

const SERVER_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

#[test]
fn socket_permission_requires_os_evidence_not_diagnostic_words() {
    let directory = crate::test_support::TestDirectory::new();
    let missing = directory.path.join("missing.sock");
    for reason in [
        "Permission denied",
        "Operation not permitted",
        "Permission refusée",
        "No such file or directory",
    ] {
        let mut error = CommandError::new(CommandFailure::Exit {
            code: Some(1),
            signal: None,
        });
        error.output = Some(CommandOutput {
            stdout: Vec::new(),
            stderr: format!("error connecting to {} ({reason})\n", missing.display()).into_bytes(),
        });
        let tmux = Tmux::new(ScriptedRunner::new([Err(error)]));
        assert!(tmux.caller_pane(&full_environment()).unwrap().is_none());
    }
}

#[test]
fn a_failed_lookup_does_not_mistake_live_or_stale_sockets_for_denial() {
    use std::{io::Read, os::unix::net::UnixListener};

    let directory = crate::test_support::TestDirectory::new();
    let socket = directory.path.join("listener.sock");
    crate::test_support::assert_private_socket(&socket, &directory.path);
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let failure = || {
        let mut cause = CommandError::new(CommandFailure::Exit {
            code: Some(1),
            signal: None,
        });
        cause.output = Some(CommandOutput {
            stdout: Vec::new(),
            stderr: format!(
                "error connecting to {} (Permission refusée)\n",
                socket.display()
            )
            .into_bytes(),
        });
        TmuxError::command(TmuxFailure::Command, cause)
    };
    assert!(!failure().socket_permission_denied());
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_nonblocking(true).unwrap();
    assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0, "probe sends no bytes");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(listener);
    assert!(
        !failure().socket_permission_denied(),
        "refused listener is not denial"
    );
}

/// macOS sandbox rules can deny connect while stat and effective W_OK succeed.
/// Re-exec only this test with a process-local sandbox against our own socket.
#[cfg(target_os = "macos")]
#[test]
fn sandbox_connect_denial_is_a_socket_permission_error() {
    use crate::test_support::{TestChild, TestDirectory, assert_private_socket};
    use std::{
        fs,
        os::unix::{
            fs::{FileTypeExt, PermissionsExt},
            net::UnixListener,
        },
        process::{Command, Stdio},
    };

    const SOCKET_ENV: &str = "TMT_TEST_DENIED_SOCKET";
    const TEST: &str = "tmux::io_tests::sandbox_connect_denial_is_a_socket_permission_error";
    if let Some(socket) = std::env::var_os(SOCKET_ENV) {
        let socket = std::path::Path::new(&socket);
        assert!(fs::metadata(socket).unwrap().file_type().is_socket());
        nix::unistd::faccessat(
            nix::fcntl::AT_FDCWD,
            socket,
            nix::unistd::AccessFlags::W_OK,
            nix::fcntl::AtFlags::AT_EACCESS,
        )
        .expect("filesystem access succeeds despite the connect restriction");
        let denied = std::os::unix::net::UnixStream::connect(socket).unwrap_err();
        assert!(matches!(
            denied.raw_os_error(),
            Some(nix::libc::EPERM | nix::libc::EACCES)
        ));
        let mut cause = CommandError::new(CommandFailure::Exit {
            code: Some(1),
            signal: None,
        });
        cause.output = Some(CommandOutput {
            stdout: Vec::new(),
            // Exact diagnostic shape recorded from the Codex seatbelt sandbox.
            stderr: format!(
                "error connecting to {} (Operation not permitted)\n",
                socket.display()
            )
            .into_bytes(),
        });
        let tmux = Tmux::new(ScriptedRunner::new([Err(cause)]));
        let error = tmux
            .resolve_target("%9", OperationOptions::default())
            .unwrap_err();
        assert_eq!(error.kind, TmuxFailure::SocketPermission);
        assert!(crate::host::HostError::from(error).socket_permission_denied());
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].args,
            ["-u", "display-message", "-p", "-t", "%9", "#{pane_id}"]
        );
        assert!(calls[0].input.is_empty(), "lookup sends no pane input");
        return;
    }

    let directory = TestDirectory::new();
    let socket = directory.path.join("denied.sock");
    assert_private_socket(&socket, &directory.path);
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let output_path = directory.path.join("child.log");
    let output = fs::File::create(&output_path).unwrap();
    let mut child = TestChild::new(
        Command::new("/usr/bin/sandbox-exec")
            .args(["-p", "(version 1)(allow default)(deny network-outbound)"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(SOCKET_ENV, &socket)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap(),
    );
    let status = child.wait_for_exit(Duration::from_secs(10));
    assert!(
        status.success(),
        "{}",
        fs::read_to_string(output_path).unwrap()
    );
    // No connection was accepted, so no tmux protocol or pane bytes were sent.
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn marked_row() -> String {
    [
        SERVER_ID,
        "/tmp/private.sock",
        "321",
        "1700000000",
        "%9",
        "e2e:0.0",
        "/workspace",
        "zsh",
        "900",
        "1",
        "e2e",
        "0",
        "",
    ]
    .join(evidence::SEPARATOR)
}

fn full_environment() -> CallerEnvironment {
    CallerEnvironment {
        tmux: Some("/tmp/private.sock,321,0".into()),
        pane: Some("%9".into()),
        process_id: 900,
        driver_env: Default::default(),
    }
}

/// A reply that names the expected pane, socket and server, as tmux prints it.
fn caller_reply(pane: &str, socket: &str, server: u32) -> String {
    let separator = "__TMT_CALLER_PANE_4f1c__";
    format!("{pane}{separator}{socket}{separator}{server}\n")
}

/// A runner that answers once, with a reply tmux printed or with a failed command.
fn answering(result: Result<String, CommandError>) -> Tmux<ScriptedRunner> {
    let runner = ScriptedRunner::default();
    runner
        .results
        .borrow_mut()
        .push_back(result.map(|text| CommandOutput {
            stdout: text.into_bytes(),
            stderr: Vec::new(),
        }));
    Tmux::new(runner)
}

fn budget_of(tmux: &Tmux<ScriptedRunner>, before: std::time::Instant) -> Duration {
    tmux.runner.calls.borrow()[0]
        .deadline
        .duration_since(before)
}

#[test]
fn only_a_required_caller_pane_waits_five_seconds_for_supplied_evidence() {
    use std::time::Instant;
    // Best-effort: one second, a timeout is no pane.
    let tmux = Tmux::new(ScriptedRunner::new([Err(failure(false))]));
    let before = Instant::now();
    assert_eq!(tmux.caller_pane(&full_environment()).unwrap(), None);
    let after = Instant::now();
    let deadline = tmux.runner.calls.borrow()[0].deadline;
    assert!(deadline >= before + OPERATION_TIMEOUT && deadline <= after + OPERATION_TIMEOUT);

    // Required, with supplied evidence: five seconds, and a timeout is reported as one.
    let tmux = Tmux::new(ScriptedRunner::new([Err(failure(false))]));
    let before = Instant::now();
    let error = tmux.caller_pane_required(&full_environment()).unwrap_err();
    let after = Instant::now();
    let deadline = tmux.runner.calls.borrow()[0].deadline;
    assert!(
        deadline >= before + REQUIRED_CALLER_TIMEOUT && deadline <= after + REQUIRED_CALLER_TIMEOUT
    );
    assert_eq!(
        REQUIRED_CALLER_TIMEOUT.as_secs(),
        5,
        "the message below says 5 s"
    );
    assert!(error.timed_out() && !error.cleanup_failed());
    assert_eq!(error.kind, TmuxFailure::Command);
    assert_eq!(
        error.to_string(),
        "tmux did not answer within 5 s while checking the caller pane (ETIMEDOUT)."
    );
    assert!(budget_of(&tmux, before) > OPERATION_TIMEOUT * 4);

    // Required, without supplied evidence: discovery keeps the shared one second.
    let discovery = CallerEnvironment {
        tmux: None,
        pane: None,
        process_id: 900,
        driver_env: Default::default(),
    };
    let tmux = Tmux::new(ScriptedRunner::new([Err(failure(false))]));
    let before = Instant::now();
    assert_eq!(tmux.caller_pane_required(&discovery).unwrap(), None);
    let after = Instant::now();
    let deadline = tmux.runner.calls.borrow()[0].deadline;
    assert!(deadline >= before + OPERATION_TIMEOUT && deadline <= after + OPERATION_TIMEOUT);
}

#[test]
fn a_required_caller_pane_keeps_absence_for_evidence_that_was_read_or_cannot_be_asked() {
    let good = caller_reply("%9", "/tmp/private.sock", 321);
    let required = |result: Result<String, CommandError>| {
        answering(result).caller_pane_required(&full_environment())
    };
    assert_eq!(required(Ok(good.clone())).unwrap().as_deref(), Some("%9"));
    // Stale pane, another socket, another server, trailing junk: completed observations.
    for wrong in [
        caller_reply("%8", "/tmp/private.sock", 321),
        caller_reply("%9", "/tmp/other.sock", 321),
        caller_reply("%9", "/tmp/private.sock", 999),
        format!("{good}extra line\n"),
        String::new(),
    ] {
        assert_eq!(required(Ok(wrong.clone())).unwrap(), None, "{wrong:?}");
    }
    // tmux ran and said no such pane, or could not be started or read: not a missing budget.
    for kind in [
        CommandFailure::Exit {
            code: Some(1),
            signal: None,
        },
        CommandFailure::Spawn,
        CommandFailure::Io,
    ] {
        let error = crate::scripted_runner::failure_with_kind(kind, false);
        assert_eq!(required(Err(error)).unwrap(), None, "{kind:?}");
    }
    // A failed cleanup still reaches the owner, timeout or not.
    let error = required(Err(failure(true))).unwrap_err();
    assert!(error.cleanup_failed());
}

#[test]
fn complete_caller_evidence_uses_one_small_query_without_ancestry() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok(
        "%9__TMT_CALLER_PANE_4f1c__/tmp/private.sock__TMT_CALLER_PANE_4f1c__321\n",
    )]));
    assert_eq!(
        tmux.caller_pane(&full_environment()).unwrap().as_deref(),
        Some("%9")
    );
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, "tmux");
    assert_eq!(
        &calls[0].args[..5],
        ["-u", "display-message", "-p", "-t", "%9"]
    );
    assert_eq!(calls[0].max_output_bytes, 4096);
}

#[test]
fn marked_pane_uses_only_the_selected_server_and_returns_frozen_evidence() {
    let runner = ScriptedRunner::new([Ok(SERVER_ID)]);
    runner.push_output(format!("{}\n", marked_row()).into_bytes(), Vec::new());
    let tmux = Tmux::new(runner);
    let target = tmux
        .marked_pane(&full_environment(), OperationOptions::default())
        .unwrap()
        .unwrap();

    assert_eq!(target.server.server_id, SERVER_ID);
    assert_eq!(target.server.socket_path, "/tmp/private.sock");
    assert_eq!(target.pane_id, "%9");
    assert_eq!(target.pane_pid, 900);
    let calls = tmux.runner.calls.borrow();
    assert_eq!(&calls[0].args[..3], ["-u", "-S", "/tmp/private.sock"]);
    assert_eq!(&calls[1].args[..3], ["-u", "-S", "/tmp/private.sock"]);
    assert_eq!(
        &calls[1].args[3..8],
        ["list-panes", "-a", "-f", "#{pane_marked}", "-F"]
    );
    assert!(calls.iter().all(|call| call.program == "tmux"));
}

#[test]
fn absent_mark_is_distinct_from_invalid_selected_server_evidence() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok(SERVER_ID), Ok("")]));
    assert_eq!(
        tmux.marked_pane(&full_environment(), OperationOptions::default())
            .unwrap(),
        None
    );

    let tmux = Tmux::new(ScriptedRunner::default());
    let invalid = CallerEnvironment {
        tmux: Some("malformed".into()),
        pane: None,
        process_id: 900,
        driver_env: Default::default(),
    };
    assert_eq!(
        tmux.marked_pane(&invalid, OperationOptions::default())
            .unwrap_err()
            .kind,
        TmuxFailure::Evidence
    );
    assert!(tmux.runner.calls.borrow().is_empty());
}

#[test]
fn malformed_explicit_environment_and_scopes_never_spawn() {
    let tmux = Tmux::new(ScriptedRunner::default());
    for (context, pane) in [("bad", "%9"), ("/tmp/private.sock,321,0", "main:0.0")] {
        let environment = CallerEnvironment {
            tmux: Some(context.into()),
            pane: Some(pane.into()),
            process_id: 900,
            driver_env: Default::default(),
        };
        assert_eq!(tmux.caller_pane(&environment).unwrap(), None);
    }
    let panes = vec!["%9".into(), "#{pane_id}".into()];
    let error = tmux
        .snapshot(OperationOptions {
            pane_ids: Some(&panes),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(error.kind, TmuxFailure::Evidence);
    assert!(tmux.runner.calls.borrow().is_empty());
}

#[test]
fn ancestry_and_snapshot_share_one_deadline_and_reject_ambient_panes() {
    let tmux = Tmux::new(ScriptedRunner::new([
        Ok("900 700\n"),
        Ok("700 0\n"),
        Ok(
            "%9__TMT_CALLER_PANE_4f1c__800__TMT_CALLER_PANE_4f1c__/tmp/private.sock__TMT_CALLER_PANE_4f1c__321\n",
        ),
    ]));
    let environment = CallerEnvironment {
        tmux: None,
        pane: None,
        process_id: 900,
        driver_env: Default::default(),
    };
    assert_eq!(tmux.caller_pane(&environment).unwrap(), None);
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].program, "ps");
    assert_eq!(calls[0].args, ["-o", "pid=,ppid=", "-p", "900"]);
    assert_eq!(calls[1].args, ["-o", "pid=,ppid=", "-p", "700"]);
    assert_eq!(calls[2].program, "tmux");
    assert!(calls.iter().all(|call| call.deadline == calls[0].deadline));
    assert!(calls.iter().all(|call| call.max_output_bytes == 64 * 1024));
}

#[test]
fn unavailable_observations_do_not_hide_failed_cleanup() {
    for failed_cleanup in [false, true] {
        let tmux = Tmux::new(ScriptedRunner::new([Err(failure(failed_cleanup))]));
        let caller = tmux.caller_pane(&full_environment());
        if failed_cleanup {
            assert!(caller.unwrap_err().cleanup_failed());
        } else {
            assert_eq!(caller.unwrap(), None);
        }

        let tmux = Tmux::new(ScriptedRunner::new([Err(failure(failed_cleanup))]));
        let target = tmux.resolve_target("10.3", OperationOptions::default());
        assert_eq!(target.unwrap_err().cleanup_failed(), failed_cleanup);

        let tmux = Tmux::new(ScriptedRunner::new([Err(failure(failed_cleanup))]));
        let probe = tmux.probe(
            "/tmp/private.sock",
            u64::from(std::process::id()),
            OperationOptions::default(),
        );
        if failed_cleanup {
            assert!(probe.unwrap_err().cleanup_failed());
        } else {
            assert!(matches!(probe.unwrap(), EndpointProbe::Unknown));
        }
    }
}

#[test]
fn malformed_probe_does_not_retire_a_live_recorded_process() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok("malformed endpoint snapshot")]));
    assert_eq!(
        tmux.probe(
            "/tmp/private.sock",
            u64::from(std::process::id()),
            OperationOptions::default()
        )
        .unwrap(),
        EndpointProbe::Unknown
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 1);
}

#[test]
fn ancestry_requires_one_coherent_candidate_after_grouped_row_deduplication() {
    const MATCH: &str = "%9__TMT_CALLER_PANE_4f1c__700__TMT_CALLER_PANE_4f1c__/tmp/private.sock__TMT_CALLER_PANE_4f1c__321";
    const OTHER: &str = "%10__TMT_CALLER_PANE_4f1c__900__TMT_CALLER_PANE_4f1c__/tmp/private.sock__TMT_CALLER_PANE_4f1c__321";
    for (rows, expected) in [
        (format!("{MATCH}\n{MATCH}\n"), Some("%9")),
        (format!("{MATCH}\n{OTHER}\n"), None),
        (format!("{MATCH}\n{}\n", MATCH.replace("700", "800")), None),
        (format!("{}\n{MATCH}\n", MATCH.replace("700", "800")), None),
    ] {
        let runner = ScriptedRunner::new([Ok("900 700\n"), Ok("700 0\n")]);
        runner.results.borrow_mut().push_back(Ok(CommandOutput {
            stdout: rows.into_bytes(),
            stderr: Vec::new(),
        }));
        let tmux = Tmux::new(runner);
        let environment = CallerEnvironment {
            tmux: None,
            pane: None,
            process_id: 900,
            driver_env: Default::default(),
        };
        assert_eq!(tmux.caller_pane(&environment).unwrap().as_deref(), expected);
        assert_eq!(tmux.runner.calls.borrow().len(), 3);
    }
}

#[test]
fn partial_caller_evidence_cannot_override_explicit_socket_or_pane() {
    for environment in [
        CallerEnvironment {
            tmux: Some("/different.sock,321,0".into()),
            pane: None,
            process_id: 900,
            driver_env: Default::default(),
        },
        CallerEnvironment {
            tmux: None,
            pane: Some("%10".into()),
            process_id: 900,
            driver_env: Default::default(),
        },
    ] {
        let tmux = Tmux::new(ScriptedRunner::new([
            Ok("900 700\n"),
            Ok("700 0\n"),
            Ok(
                "%9__TMT_CALLER_PANE_4f1c__700__TMT_CALLER_PANE_4f1c__/tmp/private.sock__TMT_CALLER_PANE_4f1c__321\n",
            ),
        ]));
        assert_eq!(tmux.caller_pane(&environment).unwrap(), None);
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 3);
        if environment.tmux.is_some() {
            assert_eq!(&calls[2].args[..3], ["-u", "-S", "/different.sock"]);
        }
    }
}

#[test]
fn ancestry_cycle_stops_before_any_pane_query() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok("900 700\n"), Ok("700 900\n")]));
    let environment = CallerEnvironment {
        tmux: None,
        pane: None,
        process_id: 900,
        driver_env: Default::default(),
    };
    assert_eq!(tmux.caller_pane(&environment).unwrap(), None);
    assert!(
        tmux.runner
            .calls
            .borrow()
            .iter()
            .all(|call| call.program == "ps")
    );
}

#[test]
fn server_initialization_stops_after_cleanup_failure() {
    let tmux = Tmux::new(ScriptedRunner::new([Err(failure(true))]));
    assert!(
        tmux.snapshot(OperationOptions::default())
            .unwrap_err()
            .cleanup_failed()
    );
    assert_eq!(
        tmux.runner.calls.borrow().len(),
        1,
        "failed cleanup cannot trigger a metadata mutation"
    );
}

#[test]
fn expired_operation_budget_prevents_even_the_first_query() {
    let tmux = Tmux::new(ScriptedRunner::default());
    let error = tmux
        .snapshot(OperationOptions {
            deadline: Some(Instant::now()),
            pane_ids: None,
        })
        .unwrap_err();
    assert_eq!(error.cause.unwrap().kind, CommandFailure::Timeout);
    assert!(tmux.runner.calls.borrow().is_empty());
}

#[test]
fn failed_metadata_read_never_authorizes_a_write() {
    let tmux = Tmux::new(ScriptedRunner::new([Err(failure(false))]));
    let error = tmux
        .clear_marker("%9", None, OperationOptions::default())
        .unwrap_err();
    assert_eq!(error.kind, TmuxFailure::MetadataRead);
    assert_eq!(
        error.to_string(),
        "Could not read pane metadata (ETIMEDOUT)."
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 1);
}

#[test]
fn nonmatching_clear_preserves_metadata_without_a_write() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok(
        r#"{"version":1,"globalIdentity":{"bindingId":"other"},"opaque":true}"#,
    )]));
    assert!(
        !tmux
            .clear_marker("%9", Some("mine"), OperationOptions::default())
            .unwrap()
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 1);
}

#[test]
fn explicit_socket_metadata_updates_preserve_opaque_data_and_never_fall_back() {
    let tmux = Tmux::new(ScriptedRunner::new([
        Ok(r#"{"version":1,"opaque":{"keep":true}}"#),
        Ok(""),
    ]));
    let marker = BindingMarker {
        name: "Alice".into(),
        canonical_name: "alice".into(),
        identity_id: "identity".into(),
        binding_id: "binding".into(),
        server_id: SERVER_ID.into(),
        pane_pid: 654,
    };
    tmux.set_marker_on(
        Some("/foreign/socket"),
        "%9",
        &marker,
        OperationOptions::default(),
    )
    .unwrap();
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 2);
    for call in calls.iter() {
        assert_eq!(&call.args[..3], ["-u", "-S", "/foreign/socket"]);
    }
    let written: serde_json::Value = serde_json::from_str(calls[1].args.last().unwrap()).unwrap();
    assert_eq!(written["opaque"], serde_json::json!({"keep": true}));
    assert_eq!(written["globalIdentity"]["bindingId"], "binding");

    let tmux = Tmux::new(ScriptedRunner::new([Err(failure(false))]));
    assert!(
        tmux.clear_marker_on(
            Some("/foreign/socket"),
            "%9",
            Some("binding"),
            OperationOptions::default()
        )
        .is_err()
    );
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(&calls[0].args[..3], ["-u", "-S", "/foreign/socket"]);
}

#[test]
fn binding_session_requires_a_budget_and_preserves_failed_probe_cleanup() {
    use tmt_core::binding::BindingEndpoint;
    let server = ServerEvidence {
        host: tmt_core::host::HostKind::Tmux,
        server_id: SERVER_ID.into(),
        socket_path: "/foreign/socket".into(),
        server_pid: 321,
        server_start_time: "start".into(),
    };
    let tmux = Tmux::new(ScriptedRunner::default());
    let mut session = BindingSession::new(&tmux);
    assert!(!session.budget_available());
    assert_eq!(
        session.probe_binding(&server, &["%9".into()]).unwrap(),
        EndpointProbe::Unknown
    );
    assert!(tmux.runner.calls.borrow().is_empty());
    session.begin_coordination();
    let oversize = (0..1025)
        .map(|index| format!("%{index}"))
        .collect::<Vec<_>>();
    assert_eq!(
        session.probe_binding(&server, &oversize).unwrap(),
        EndpointProbe::Unknown
    );
    assert!(tmux.runner.calls.borrow().is_empty());

    let tmux = Tmux::new(ScriptedRunner::new([Err(failure(true))]));
    let mut session = BindingSession::new(&tmux);
    session.begin_coordination();
    assert!(
        session
            .probe_binding(&server, &["%9".into()])
            .unwrap_err()
            .cleanup_failed()
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 1);
}

#[test]
fn empty_scope_reads_server_only_and_never_enumerates_panes() {
    let tmux = Tmux::new(ScriptedRunner::new([
        Ok(SERVER_ID),
        Ok(
            "123e4567-e89b-42d3-a456-426614174000__TMT_FIELD_4f1c__/tmp/private.sock__TMT_FIELD_4f1c__321__TMT_FIELD_4f1c__1700000000\n",
        ),
    ]));
    let snapshot = tmux
        .snapshot(OperationOptions {
            pane_ids: Some(&[]),
            ..Default::default()
        })
        .unwrap();
    assert!(snapshot.panes.is_empty());
    assert_eq!(snapshot.server.server_id, SERVER_ID);
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].args[1], "display-message");
    assert!(
        calls
            .iter()
            .all(|call| !call.args.iter().any(|arg| arg == "list-panes"))
    );
}

#[test]
fn a_marker_refresh_rewrites_only_this_bindings_marker_and_keeps_opaque_data() {
    let renamed = BindingMarker {
        name: "Ada".into(),
        canonical_name: "ada".into(),
        identity_id: "identity".into(),
        binding_id: "binding".into(),
        server_id: SERVER_ID.into(),
        pane_pid: 654,
    };
    let tmux = Tmux::new(ScriptedRunner::new([
        Ok(
            r#"{"version":1,"globalIdentity":{"name":"Alice","canonicalName":"alice","identityId":"identity","bindingId":"binding","serverId":"123e4567-e89b-42d3-a456-426614174000","panePid":654},"opaque":1}"#,
        ),
        Ok(""),
    ]));
    assert!(
        tmux.refresh_marker_on(None, "%9", &renamed, OperationOptions::default())
            .unwrap()
    );
    let calls = tmux.runner.calls.borrow();
    let written: serde_json::Value = serde_json::from_str(calls[1].args.last().unwrap()).unwrap();
    assert_eq!(written["globalIdentity"]["name"], "Ada");
    assert_eq!(written["globalIdentity"]["canonicalName"], "ada");
    assert_eq!(written["opaque"], 1);

    // Another binding took the pane after the probe: no write.
    let tmux = Tmux::new(ScriptedRunner::new([Ok(
        r#"{"version":1,"globalIdentity":{"bindingId":"other"}}"#,
    )]));
    assert!(
        !tmux
            .refresh_marker_on(None, "%9", &renamed, OperationOptions::default())
            .unwrap()
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 1);
}

fn tmux_exit(stderr: &str) -> CommandError {
    let mut error = CommandError::new(CommandFailure::Exit {
        code: Some(1),
        signal: None,
    });
    error.output = Some(CommandOutput {
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    });
    error
}

fn refused_server_id_set() -> CommandError {
    tmux_exit("already set: @tmt.server-id\n")
}

fn unset_server_id() -> CommandError {
    tmux_exit("invalid option: @tmt.server-id\n")
}

#[test]
fn server_id_adopts_the_winner_after_a_refused_set() {
    let tmux = Tmux::new(ScriptedRunner::new([
        Err(unset_server_id()),
        Err(refused_server_id_set()),
        Ok(SERVER_ID),
    ]));
    let deadline = Instant::now() + Duration::from_millis(500);
    assert_eq!(
        tmux.ensure_server_id_on(
            Some("/tmp/private.sock"),
            OperationOptions {
                deadline: Some(deadline),
                ..Default::default()
            },
        )
        .unwrap(),
        SERVER_ID
    );
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].args, calls[2].args);
    assert_eq!(
        &calls[1].args[..7],
        [
            "-u",
            "-S",
            "/tmp/private.sock",
            "set-option",
            "-s",
            "-o",
            SERVER_ID_OPTION
        ]
    );
    assert!(valid_server_id(&calls[1].args[7]));
    assert!(calls.iter().all(|call| call.deadline == deadline));
}

#[test]
fn server_id_requires_valid_readback_after_either_set_outcome() {
    for set in [Ok(""), Err(refused_server_id_set())] {
        let tmux = Tmux::new(ScriptedRunner::new([
            Err(unset_server_id()),
            set,
            Ok("invalid"),
        ]));
        assert_eq!(
            tmux.ensure_server_id_on(None, OperationOptions::default())
                .unwrap_err()
                .kind,
            TmuxFailure::Evidence
        );
        assert_eq!(tmux.runner.calls.borrow().len(), 3);
    }
    let tmux = Tmux::new(ScriptedRunner::new([
        Err(unset_server_id()),
        Err(refused_server_id_set()),
        Err(failure(false)),
    ]));
    let error = tmux
        .ensure_server_id_on(None, OperationOptions::default())
        .unwrap_err();
    assert!(matches!(error.cause.unwrap().kind, CommandFailure::Timeout));
}

#[test]
fn server_id_preserves_operational_set_failures_without_readback() {
    use crate::scripted_runner::failure_with_kind;
    for error in [
        failure_with_kind(CommandFailure::Timeout, false),
        failure_with_kind(CommandFailure::Spawn, false),
        failure_with_kind(CommandFailure::Io, false),
        failure_with_kind(CommandFailure::OutputLimit, false),
        failure_with_kind(
            CommandFailure::Exit {
                code: None,
                signal: Some(9),
            },
            false,
        ),
        failure_with_kind(
            CommandFailure::Exit {
                code: Some(1),
                signal: None,
            },
            true,
        ),
    ] {
        let expected = error.kind;
        let tmux = Tmux::new(ScriptedRunner::new([Err(unset_server_id()), Err(error)]));
        let error = tmux
            .ensure_server_id_on(None, OperationOptions::default())
            .unwrap_err();
        assert_eq!(error.cause.unwrap().kind, expected);
        assert_eq!(tmux.runner.calls.borrow().len(), 2);
    }
}

#[test]
fn server_id_reads_an_existing_value_without_setting_it() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok(SERVER_ID)]));
    assert_eq!(
        tmux.ensure_server_id_on(None, OperationOptions::default())
            .unwrap(),
        SERVER_ID
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 1);
}

#[test]
fn server_id_reads_back_after_successful_initialization() {
    let tmux = Tmux::new(ScriptedRunner::new([
        Err(unset_server_id()),
        Ok(""),
        Ok(SERVER_ID),
    ]));
    assert_eq!(
        tmux.ensure_server_id_on(None, OperationOptions::default())
            .unwrap(),
        SERVER_ID
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 3);
}

#[test]
fn explicit_target_resolution_preserves_execution_failures() {
    for kind in [
        CommandFailure::Timeout,
        CommandFailure::Io,
        CommandFailure::Spawn,
        CommandFailure::OutputLimit,
        CommandFailure::Exit {
            code: None,
            signal: Some(15),
        },
    ] {
        let tmux = Tmux::new(ScriptedRunner::new([Err(CommandError::new(kind))]));
        let error = tmux
            .resolve_target("10.3", OperationOptions::default())
            .unwrap_err();
        assert_eq!(error.kind, TmuxFailure::Command);
        assert_eq!(error.cause.unwrap().kind, kind);
        assert_eq!(tmux.runner.calls.borrow().len(), 1);

        let tmux = Tmux::new(ScriptedRunner::new([Err(CommandError::new(kind))]));
        assert_eq!(tmux.caller_pane(&full_environment()).unwrap(), None);
    }
}

#[test]
fn explicit_target_resolution_keeps_success_and_unconfirmed_missing_distinct() {
    for (output, expected) in [("%9\n", Some("%9")), ("", None), ("invalid", None)] {
        let tmux = Tmux::new(ScriptedRunner::new([Ok(output)]));
        assert_eq!(
            tmux.resolve_target("10.3", OperationOptions::default())
                .unwrap()
                .as_deref(),
            expected
        );
    }
    let directory = crate::test_support::TestDirectory::new();
    let missing = directory.path.join("missing.sock");
    for stderr in [
        "can't find window: 10\n".to_owned(),
        format!(
            "error connecting to {} (Permission denied)\n",
            missing.display()
        ),
        format!(
            "error connecting to {} (Operation not permitted)\n",
            missing.display()
        ),
    ] {
        let mut error = CommandError::new(CommandFailure::Exit {
            code: Some(1),
            signal: None,
        });
        error.output = Some(CommandOutput {
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        });
        let tmux = Tmux::new(ScriptedRunner::new([Err(error)]));
        let target = tmux.resolve_target("10.3", OperationOptions::default());
        assert_eq!(target.unwrap(), None);
    }
}

#[test]
fn missing_target_exit_does_not_hide_failed_cleanup() {
    let tmux = Tmux::new(ScriptedRunner::new([Err(
        crate::scripted_runner::failure_with_kind(
            CommandFailure::Exit {
                code: Some(1),
                signal: None,
            },
            true,
        ),
    )]));
    assert!(
        tmux.resolve_target("10.3", OperationOptions::default())
            .unwrap_err()
            .cleanup_failed()
    );
}

#[test]
fn utf8_is_one_client_flag_for_bootstrap_and_keeps_command_local_unset() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok(""), Ok("")]));
    let deadline = Instant::now() + Duration::from_secs(1);
    let bootstrap = [
        "-S",
        "/tmp/selected",
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        "first",
    ];
    let unset = [
        "-S",
        "/tmp/selected",
        "set-option",
        "-p",
        "-u",
        "-t",
        "%9",
        "@tmt.badge",
    ];
    for args in [bootstrap.as_slice(), unset.as_slice()] {
        tmux.execute(
            args.iter().map(|arg| (*arg).into()).collect(),
            OperationOptions {
                deadline: Some(deadline),
                ..Default::default()
            },
            TmuxFailure::Command,
        )
        .unwrap();
    }
    let calls = tmux.runner.calls.borrow();
    for (call, original) in calls.iter().zip([bootstrap.as_slice(), unset.as_slice()]) {
        assert_eq!(call.args[0], "-u");
        assert_eq!(call.args[1..], *original);
        assert_eq!(call.deadline, deadline);
    }
    assert_eq!(calls[0].args.iter().filter(|arg| *arg == "-u").count(), 1);
    assert_eq!(calls[1].args.iter().filter(|arg| *arg == "-u").count(), 2);
}

#[test]
fn a_recorded_tmux_server_is_observed_on_its_socket_without_metadata_writes() {
    use tmt_core::binding::BindingEndpoint;
    #[derive(Clone, Default)]
    struct Recorded(std::rc::Rc<ScriptedRunner>);
    impl CommandRunner for Recorded {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            self.0.execute(request)
        }
    }
    let runner = Recorded::default();
    for _ in 0..3 {
        runner.0.push_output(marked_row().into_bytes(), Vec::new());
    }
    let server = ServerEvidence {
        host: tmt_core::host::HostKind::Tmux,
        server_id: SERVER_ID.into(),
        socket_path: "/tmp/private.sock".into(),
        server_pid: 321,
        server_start_time: "1700000000".into(),
    };
    let host = crate::host::Host::for_server_with(&server, runner.clone());
    let snapshot = host.snapshot(OperationOptions::default());
    assert_eq!(
        &runner.0.calls.borrow()[0].args[..4],
        ["-u", "-S", "/tmp/private.sock", "list-panes"],
        "a recorded server read must not initialize metadata on the ambient socket"
    );
    assert_eq!(snapshot.unwrap().server, server);
    assert_eq!(
        host.observe_snapshot(OperationOptions::default())
            .unwrap()
            .server,
        server
    );
    let mut session = host.session();
    session.begin_coordination();
    assert_eq!(
        session.current_snapshot(&["%9".into()]).unwrap().server,
        server
    );
    let calls = runner.0.calls.borrow();
    assert_eq!(calls.len(), 3);
    for call in calls.iter() {
        assert_eq!(
            &call.args[..4],
            ["-u", "-S", "/tmp/private.sock", "list-panes"]
        );
        assert!(!call.args.iter().any(|arg| arg == "set-option"));
    }
}

#[test]
fn selected_socket_failure_never_falls_back_to_or_adopts_an_ambient_server() {
    for result in [Err(failure(false)), Ok("unmarked server")] {
        let tmux = Tmux::new(ScriptedRunner::new([result])).on_socket("/tmp/selected.sock");
        assert!(tmux.snapshot(OperationOptions::default()).is_err());
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            &calls[0].args[..4],
            ["-u", "-S", "/tmp/selected.sock", "list-panes"]
        );
    }
}

#[test]
fn a_selected_server_empty_scope_reads_no_panes_or_metadata_writes() {
    let runner = ScriptedRunner::default();
    runner.push_output(marked_row().into_bytes(), Vec::new());
    let tmux = Tmux::new(runner).on_socket("/tmp/private.sock");
    let snapshot = tmux
        .snapshot(OperationOptions {
            pane_ids: Some(&[]),
            ..Default::default()
        })
        .unwrap();
    assert!(
        snapshot.panes.is_empty(),
        "an empty scope must not enumerate the server"
    );
    assert_eq!(snapshot.server.server_id, SERVER_ID);
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        &calls[0].args[..4],
        ["-u", "-S", "/tmp/private.sock", "display-message"]
    );
}

#[derive(Clone, Default)]
struct CallerRunner(std::rc::Rc<ScriptedRunner>);

impl CommandRunner for CallerRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        self.0.execute(request)
    }
}

#[test]
fn a_captured_caller_socket_is_the_only_server_used_by_its_host() {
    use tmt_core::binding::BindingEndpoint;
    assert!(std::env::var_os("TMUX").is_none());
    assert!(std::env::var_os("TMUX_PANE").is_none());
    let directory = crate::test_support::TestDirectory::new();
    let socket = directory.path.join("private.sock");
    crate::test_support::assert_private_socket(&socket, &directory.path);
    let socket = socket.to_str().unwrap();
    let caller = CallerEnvironment {
        tmux: Some(format!("{socket},321,0").into()),
        ..full_environment()
    };
    let runner = CallerRunner::default();
    runner.0.push_output(b"%9\n".to_vec(), vec![]);
    runner.0.push_output(SERVER_ID.as_bytes().to_vec(), vec![]);
    for _ in 0..2 {
        runner.0.push_output(
            marked_row()
                .replace("/tmp/private.sock", socket)
                .into_bytes(),
            vec![],
        );
    }
    runner.0.push_output(SERVER_ID.as_bytes().to_vec(), vec![]);
    runner.0.push_output(
        marked_row()
            .replace("/tmp/private.sock", socket)
            .into_bytes(),
        vec![],
    );
    let host = crate::host::Host::for_caller_with(&caller, runner.clone());
    assert_eq!(
        host.resolve_target("1.0", OperationOptions::default())
            .unwrap()
            .as_deref(),
        Some("%9")
    );
    assert_eq!(&runner.0.calls.borrow()[0].args[..3], ["-u", "-S", socket]);
    assert_eq!(
        host.snapshot(OperationOptions::default())
            .unwrap()
            .server
            .socket_path,
        socket
    );
    assert_eq!(
        host.observe_snapshot(OperationOptions::default())
            .unwrap()
            .server
            .socket_path,
        socket
    );
    let mut session = host.session();
    session.begin_coordination();
    assert_eq!(
        session
            .current_snapshot(&["%9".into()])
            .unwrap()
            .server
            .socket_path,
        socket
    );
    let calls = runner.0.calls.borrow();
    assert_eq!(calls.len(), 6);
    for call in calls.iter() {
        assert_eq!(call.program, "tmux");
        assert_eq!(&call.args[..3], ["-u", "-S", socket]);
        assert_eq!(call.args.iter().filter(|arg| *arg == "-S").count(), 1);
        assert!(!call.args.iter().any(|arg| arg == "set-option"));
    }
    assert_eq!(
        calls[1].args[3], "show-options",
        "caller snapshots retain metadata adoption"
    );
}

#[test]
fn malformed_captured_caller_sockets_never_reach_a_server() {
    use std::os::unix::ffi::OsStringExt;
    for tmux in [
        "broken".into(),
        "/tmp/private.sock,no-pid,0".into(),
        OsString::from_vec(vec![255]),
    ] {
        let runner = CallerRunner::default();
        let caller = CallerEnvironment {
            tmux: Some(tmux),
            ..full_environment()
        };
        let host = crate::host::Host::for_caller_with(&caller, runner.clone());
        assert!(
            host.resolve_target("1.0", OperationOptions::default())
                .is_err()
        );
        assert!(host.snapshot(OperationOptions::default()).is_err());
        assert!(runner.0.calls.borrow().is_empty());
    }
}

#[test]
fn an_absent_captured_caller_socket_keeps_default_selection() {
    for tmux in [None, Some("".into())] {
        let runner = CallerRunner::default();
        runner.0.push_output(b"%9\n".to_vec(), vec![]);
        let caller = CallerEnvironment {
            tmux,
            pane: None,
            ..full_environment()
        };
        let host = crate::host::Host::for_caller_with(&caller, runner.clone());
        assert_eq!(
            host.resolve_target("1.0", OperationOptions::default())
                .unwrap()
                .as_deref(),
            Some("%9")
        );
        assert_eq!(
            &runner.0.calls.borrow()[0].args[..2],
            ["-u", "display-message"]
        );
    }
}

#[test]
fn a_captured_caller_socket_failure_has_no_ambient_fallback() {
    let directory = crate::test_support::TestDirectory::new();
    let socket = directory.path.join("private.sock");
    crate::test_support::assert_private_socket(&socket, &directory.path);
    let socket = socket.to_str().unwrap();
    let runner = CallerRunner::default();
    runner
        .0
        .results
        .borrow_mut()
        .push_back(Err(CommandError::new(CommandFailure::Timeout)));
    let caller = CallerEnvironment {
        tmux: Some(format!("{socket},321,0").into()),
        ..full_environment()
    };
    let host = crate::host::Host::for_caller_with(&caller, runner.clone());
    assert!(
        host.resolve_target("1.0", OperationOptions::default())
            .is_err()
    );
    let calls = runner.0.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(&calls[0].args[..3], ["-u", "-S", socket]);
}

#[test]
fn an_explicit_workspace_socket_overrides_the_captured_caller() {
    let directory = crate::test_support::TestDirectory::new();
    let socket = directory.path.join("recipient.sock");
    crate::test_support::assert_private_socket(&socket, &directory.path);
    let socket = socket.to_str().unwrap();
    for tmux in [
        Some("/different/caller.sock,321,0".into()),
        Some("malformed".into()),
        None,
    ] {
        let caller = CallerEnvironment {
            tmux,
            ..full_environment()
        };
        let runner = CallerRunner::default();
        runner.0.push_output(
            format!("0{}work\n", evidence::SEPARATOR).into_bytes(),
            vec![],
        );
        let host = crate::host::Host::for_caller_with(&caller, runner.clone());
        assert_eq!(
            host.workspace_session_names(socket, Instant::now() + Duration::from_secs(1))
                .unwrap(),
            Some(vec!["work".into()])
        );
        let calls = runner.0.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(&calls[0].args[..3], ["-u", "-S", socket]);
        assert_eq!(calls[0].args.iter().filter(|arg| *arg == "-S").count(), 1);
    }
}
