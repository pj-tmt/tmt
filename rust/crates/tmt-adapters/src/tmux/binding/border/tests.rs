use super::*;
use crate::scripted_runner::{ScriptedRunner, failure};
use std::time::{Duration, Instant};
use tmt_core::{binding::session::BindingSessionState, endpoint::ServerEvidence, host::HostKind};

fn binding() -> Binding {
    Binding {
        id: "binding".into(),
        identity_id: "identity".into(),
        server: ServerEvidence {
            host: HostKind::Tmux,
            server_id: "server".into(),
            socket_path: "/tmp/private.sock".into(),
            server_pid: 321,
            server_start_time: "1700000000".into(),
        },
        pane_id: "%9".into(),
        pane_pid: 900,
        pane_incarnation: None,
        session: BindingSessionState::default(),
    }
}

#[test]
fn inherited_format_is_preserved_and_ownership_follows_successful_pane_only_write() {
    let tmux = Tmux::new(ScriptedRunner::new([
        Ok(""),
        Ok("  #[align=left]#{pane_index} , tail  \n"),
        Ok(""),
        Ok(""),
        Ok("top\n"),
    ]));
    let deadline = Instant::now() + Duration::from_millis(400);
    tmux.update_badge_border(
        &binding(),
        true,
        true,
        OperationOptions {
            deadline: Some(deadline),
            pane_ids: None,
        },
    )
    .unwrap();
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 5);
    assert!(
        calls
            .iter()
            .all(|call| call.deadline == deadline && call.args[..2] == ["-S", "/tmp/private.sock"])
    );
    let composed = format!("{BADGE}  #[align=left]#{{pane_index}} , tail  ");
    assert_eq!(
        calls[2].args[2..],
        ["set-option", "-p", "-o", "-t", "%9", FORMAT, &composed]
    );
    assert_eq!(
        calls[3].args[2..],
        ["set-option", "-p", "-t", "%9", OWNER, &composed]
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.args.iter().any(|arg| arg == "-g"))
    );
}

#[test]
fn user_local_empty_or_nonempty_and_existing_badge_formats_are_not_written() {
    for local in ["\n", "user content\n", "#{?@tmt.badge,label,}\n"] {
        let tmux = Tmux::new(ScriptedRunner::new([Ok(local), Ok("bottom\n")]));
        tmux.update_badge_border(&binding(), true, true, OperationOptions::default())
            .unwrap();
        assert!(
            tmux.runner
                .calls
                .borrow()
                .iter()
                .all(|call| call.args[2] == "show-options")
        );
    }
    let tmux = Tmux::new(ScriptedRunner::new([
        Ok(""),
        Ok("before #{@tmt.badge} after\n"),
        Ok("top\n"),
    ]));
    tmux.update_badge_border(&binding(), true, true, OperationOptions::default())
        .unwrap();
    assert_eq!(tmux.runner.calls.borrow().len(), 3);
    assert!(
        tmux.runner
            .calls
            .borrow()
            .iter()
            .all(|call| call.args[2] == "show-options")
    );
}

#[test]
fn cleanup_is_server_fenced_and_refuses_missing_or_foreign_ownership() {
    for owner in ["", "not-owned\n"] {
        let tmux = Tmux::new(ScriptedRunner::new([Ok(owner)]));
        tmux.update_badge_border(&binding(), false, false, OperationOptions::default())
            .unwrap();
        assert_eq!(tmux.runner.calls.borrow().len(), 1);
    }
    let runner = ScriptedRunner::new([]);
    runner.push_output(format!("{BADGE}user\n").into_bytes(), Vec::new());
    runner.push_output(Vec::new(), Vec::new());
    let tmux = Tmux::new(runner);
    tmux.update_badge_border(&binding(), false, false, OperationOptions::default())
        .unwrap();
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls[1].args[2..6], ["if-shell", "-F", "-t", "%9"]);
    assert!(calls[1].args[6].contains("#{==:#{pane-border-format},#{@tmt.border}}"));
    assert_eq!(
        calls[1].args[7],
        "set-option -p -u -t %9 pane-border-format ; set-option -p -u -t %9 @tmt.border"
    );
}

#[test]
fn failed_set_only_if_unset_never_publishes_ownership_and_spent_budget_spawns_nothing() {
    let tmux = Tmux::new(ScriptedRunner::new([
        Ok(""),
        Ok("user\n"),
        Err(failure(false)),
    ]));
    assert!(
        tmux.update_badge_border(&binding(), true, true, OperationOptions::default())
            .is_err()
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 3);
    assert!(
        !tmux
            .runner
            .calls
            .borrow()
            .iter()
            .any(|call| call.args.iter().any(|arg| arg == OWNER))
    );
    let tmux = Tmux::new(ScriptedRunner::default());
    assert!(
        tmux.update_badge_border(
            &binding(),
            true,
            true,
            OperationOptions {
                deadline: Some(Instant::now()),
                pane_ids: None
            }
        )
        .is_err()
    );
    assert!(tmux.runner.calls.borrow().is_empty());
}

#[test]
fn malformed_option_output_refuses_and_hint_names_selected_window_without_changing_it() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok("unterminated")]));
    assert!(
        tmux.update_badge_border(&binding(), true, true, OperationOptions::default())
            .is_err()
    );
    assert_eq!(tmux.runner.calls.borrow().len(), 1);
    assert_eq!(
        border_hint(&binding()),
        "hint: pane borders are off; enable them with tmux -S '/tmp/private.sock' set-option -w -t %9 pane-border-status top"
    );
}

#[test]
fn automatic_refresh_publishes_the_format_without_querying_hint_status() {
    let tmux = Tmux::new(ScriptedRunner::new([Ok(""), Ok("user\n"), Ok(""), Ok("")]));
    tmux.update_badge_border(&binding(), true, false, OperationOptions::default())
        .unwrap();
    let calls = tmux.runner.calls.borrow();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[2].args[2], "set-option");
    assert_eq!(calls[3].args[6], OWNER);
    assert!(
        !calls
            .iter()
            .any(|call| call.args.iter().any(|arg| arg == "pane-border-status"))
    );
}
