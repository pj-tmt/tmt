use super::*;
use crate::process::{CommandError, CommandFailure, CommandOutput};
use crate::scripted_runner::{ScriptedRunner, failure};

const SERVER_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

#[test]
fn socket_permission_is_typed_without_treating_other_tmux_exits_as_denial() {
    let failed = |stderr: &str| {
        let mut error = CommandError::new(CommandFailure::Exit {
            code: Some(1),
            signal: None,
        });
        error.output = Some(CommandOutput {
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        });
        error
    };
    let denied = Tmux::new(ScriptedRunner::new([Err(failed(
        "error connecting to /tmp/private.sock (Permission denied)\n",
    ))]));
    assert!(
        denied
            .caller_pane(&full_environment())
            .unwrap_err()
            .socket_permission_denied()
    );

    let blocked = Tmux::new(ScriptedRunner::new([Err(failed(
        "error connecting to /tmp/private.sock (Operation not permitted)\n",
    ))]));
    assert!(
        blocked
            .caller_pane(&full_environment())
            .unwrap_err()
            .socket_permission_denied()
    );

    let absent = Tmux::new(ScriptedRunner::new([Err(failed(
        "error connecting to /tmp/private.sock (No such file or directory)\n",
    ))]));
    assert!(absent.caller_pane(&full_environment()).unwrap().is_none());
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
        herdr_pane: None,
        herdr_socket: None,
    }
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
    assert_eq!(&calls[0].args[..4], ["display-message", "-p", "-t", "%9"]);
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
    assert_eq!(&calls[0].args[..2], ["-S", "/tmp/private.sock"]);
    assert_eq!(&calls[1].args[..2], ["-S", "/tmp/private.sock"]);
    assert_eq!(
        &calls[1].args[2..7],
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
        herdr_pane: None,
        herdr_socket: None,
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
            herdr_pane: None,
            herdr_socket: None,
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
        herdr_pane: None,
        herdr_socket: None,
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
        if failed_cleanup {
            assert!(target.unwrap_err().cleanup_failed());
        } else {
            assert_eq!(target.unwrap(), None);
        }

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
            herdr_pane: None,
            herdr_socket: None,
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
            herdr_pane: None,
            herdr_socket: None,
        },
        CallerEnvironment {
            tmux: None,
            pane: Some("%10".into()),
            process_id: 900,
            driver_env: Default::default(),
            herdr_pane: None,
            herdr_socket: None,
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
            assert_eq!(&calls[2].args[..2], ["-S", "/different.sock"]);
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
        herdr_pane: None,
        herdr_socket: None,
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
        assert_eq!(&call.args[..2], ["-S", "/foreign/socket"]);
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
    assert_eq!(&calls[0].args[..2], ["-S", "/foreign/socket"]);
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
    assert_eq!(calls[1].args[0], "display-message");
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
    tmux_exit("already set: @tmux-team.server-id\n")
}

fn unset_server_id() -> CommandError {
    tmux_exit("invalid option: @tmux-team.server-id\n")
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
        &calls[1].args[..6],
        [
            "-S",
            "/tmp/private.sock",
            "set-option",
            "-s",
            "-o",
            SERVER_ID_OPTION
        ]
    );
    assert!(valid_server_id(&calls[1].args[6]));
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
            tmux.ensure_server_id(OperationOptions::default())
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
        .ensure_server_id(OperationOptions::default())
        .unwrap_err();
    assert!(matches!(error.cause.unwrap().kind, CommandFailure::Timeout));
}

#[test]
fn server_id_preserves_operational_set_failures_without_readback() {
    use crate::scripted_runner::failure_with_kind;
    let mut denied = refused_server_id_set();
    denied.output.as_mut().unwrap().stderr =
        b"error connecting to /tmp/private.sock (Permission denied)\n".to_vec();
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
        denied,
    ] {
        let expected = error.kind;
        let tmux = Tmux::new(ScriptedRunner::new([Err(unset_server_id()), Err(error)]));
        let error = tmux
            .ensure_server_id(OperationOptions::default())
            .unwrap_err();
        assert_eq!(error.cause.unwrap().kind, expected);
        assert_eq!(tmux.runner.calls.borrow().len(), 2);
    }
}

#[test]
fn server_id_reads_an_existing_value_without_setting_it() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok(SERVER_ID)]));
    assert_eq!(
        tmux.ensure_server_id(OperationOptions::default()).unwrap(),
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
        tmux.ensure_server_id(OperationOptions::default()).unwrap(),
        SERVER_ID
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 3);
}
