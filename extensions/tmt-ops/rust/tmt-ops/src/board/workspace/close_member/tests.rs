use super::*;
use crate::board::{
    app::workspace::tests::{MEMBER, board},
    app::{Effect, Request as Action},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
const NAME: &str = "worker";
fn request(mode: Mode) -> Request {
    Request {
        identity_id: MEMBER.into(),
        expected_name: NAME.into(),
        mode,
        pane_reason: None,
    }
}
fn removed() -> Value {
    json!({"outcome":"removed","identityId":MEMBER,"name":NAME})
}
fn fault(code: &str, signalled: Option<bool>) -> SquadError {
    let mut error = SquadError::new(code, "Core refused the operation");
    error.signalled = signalled;
    error
}
#[test]
fn only_verified_stop_successes_automatically_remove_with_the_same_uuid_name() {
    for (outcome, pane) in [
        ("stopped", Some("closed")),
        ("stopped", Some("gone")),
        ("stopped", Some("kept")),
        ("already-ended", Some("kept")),
        ("not-running", None),
    ] {
        let mut stop = json!({"outcome":outcome,"identityId":MEMBER,"name":NAME});
        if let Some(pane) = pane {
            stop["pane"] = json!(pane);
        }
        if pane == Some("kept") {
            stop["paneReason"] = json!("The pane was opened by you.");
        }
        let mut calls = Vec::new();
        let result = request(Mode::CloseAndDelete).run(|operation, input| {
            calls.push((operation.to_owned(), input));
            Ok(if operation == "identity.stop" {
                stop.clone()
            } else {
                removed()
            })
        });
        assert_eq!(
            calls,
            vec![
                (
                    "identity.stop".into(),
                    json!({"identityId":MEMBER,"expectedName":NAME,"close":true})
                ),
                (
                    "identity.remove".into(),
                    json!({"identityId":MEMBER,"expectedName":NAME})
                ),
            ]
        );
        assert_eq!(
            result,
            Outcome::Removed {
                pane_reason: (pane == Some("kept")).then(|| "The pane was opened by you.".into())
            }
        );
    }
}
#[test]
fn every_stop_fault_keeps_its_evidence_and_never_dispatches_remove() {
    for code in [
        "RUNTIME_CONFLICT",
        "IDENTITY_CONFLICT",
        "RESUME_UNAVAILABLE",
        "RESUME_UNSUPPORTED",
        "PROCESS_ERROR",
        "TMUX_PERMISSION_DENIED",
        "API_UNAVAILABLE",
        "UNFAMILIAR_ERROR",
    ] {
        for signalled in [Some(true), Some(false), None] {
            let error = fault(code, signalled);
            let mut calls = Vec::new();
            let result = request(Mode::CloseAndDelete).run(|operation, _| {
                calls.push(operation.to_owned());
                Err(error.clone())
            });
            assert_eq!(calls, ["identity.stop"]);
            assert_eq!(
                result,
                Outcome::Refused {
                    step: Step::Stop,
                    error,
                    pane_reason: None
                }
            );
        }
    }
}
#[test]
fn changed_missing_or_malformed_stop_never_authorizes_delete() {
    for value in [
        json!({"outcome":"conflict","identityId":MEMBER,"name":"new-name"}),
        json!({"outcome":"not-found","identityId":MEMBER}),
        json!({"outcome":"stopped","identityId":"another-uuid","name":NAME,"pane":"closed"}),
        json!({"outcome":"stopped","identityId":MEMBER,"name":"new-name","pane":"closed"}),
        json!({"outcome":"stopped","identityId":MEMBER,"name":NAME}),
        json!({"outcome":"stopped","identityId":MEMBER,"name":NAME,"pane":"kept"}),
        json!({"outcome":"future","identityId":MEMBER,"name":NAME}),
    ] {
        let mut calls = 0;
        let result = request(Mode::CloseAndDelete).run(|operation, _| {
            assert_eq!(operation, "identity.stop");
            calls += 1;
            Ok(value.clone())
        });
        assert_eq!(calls, 1);
        assert!(!matches!(result, Outcome::Removed { .. }));
    }
}
#[test]
fn explicit_delete_only_uses_remove_and_preserves_its_final_guard() {
    for response in [
        Ok(removed()),
        Ok(json!({"outcome":"conflict","identityId":MEMBER,"name":"replacement"})),
        Ok(json!({"outcome":"not-found","identityId":MEMBER})),
        Err(fault("AGENT_RUNNING", None)),
        Err(fault("IDENTITY_CONFLICT", None)),
    ] {
        let mut calls = Vec::new();
        let result = request(Mode::DeleteOnly).run(|operation, input| {
            calls.push((operation.to_owned(), input));
            response.clone()
        });
        assert_eq!(
            calls,
            vec![(
                "identity.remove".into(),
                json!({"identityId":MEMBER,"expectedName":NAME})
            )]
        );
        match response {
            Ok(v) if v["outcome"] == "removed" => {
                assert!(matches!(result, Outcome::Removed { .. }))
            }
            Ok(_) => assert_eq!(result, Outcome::Changed { pane_reason: None }),
            Err(error) => assert_eq!(
                result,
                Outcome::Refused {
                    step: Step::Delete,
                    error,
                    pane_reason: None
                }
            ),
        }
    }
}
#[test]
fn confirmed_stop_and_kept_pane_reason_survive_a_failed_remove() {
    let result=request(Mode::CloseAndDelete).run(|operation,_| {
        if operation=="identity.stop" {Ok(json!({"outcome":"stopped","identityId":MEMBER,"name":NAME,"pane":"kept","paneReason":"Host could not close the pane."}))}
        else {Err(fault("AGENT_RUNNING",None))}
    });
    assert!(
        matches!(result,Outcome::Refused{step:Step::Delete,pane_reason:Some(reason),..} if reason=="Host could not close the pane.")
    );
}
fn paint(app: &App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn begin(app: &mut App) -> Request {
    assert_eq!(key(app, KeyCode::Char('x')), Effect::None);
    paint(app, 100, 36);
    assert_eq!(key(app, KeyCode::Right), Effect::None);
    paint(app, 100, 36);
    let Effect::Act(Action::WorkspaceOperation(Change::CloseMember(request))) =
        key(app, KeyCode::Enter)
    else {
        panic!("confirmed close dispatches typed lifecycle work")
    };
    assert_eq!(
        key(app, KeyCode::Enter),
        Effect::None,
        "pending cannot resubmit"
    );
    request
}
#[test]
fn any_refused_stop_allows_an_explicit_delete_but_never_replays_stop() {
    for (code, signalled) in [
        ("RESUME_UNAVAILABLE", Some(false)),
        ("RUNTIME_CONFLICT", Some(false)),
        ("PROCESS_ERROR", Some(true)),
        ("API_UNAVAILABLE", None),
    ] {
        let mut app = board();
        let _commands = crate::runner::forbid_commands();
        let request = begin(&mut app);
        app.finished_close_member(Completed {
            request,
            outcome: Outcome::Refused {
                step: Step::Stop,
                error: fault(code, signalled),
                pane_reason: None,
            },
        });
        let screen = paint(&app, 100, 36);
        assert!(screen.contains("Delete identity"));
        if code == "RESUME_UNAVAILABLE" {
            assert!(screen.contains("No recorded agent session"));
        }
        for _ in 0..3 {
            if app.workspace_form.as_ref().unwrap().review.focus
                == tmt_tui::components::Focus::Buttons(2)
            {
                break;
            }
            key(&mut app, KeyCode::Right);
            paint(&app, 100, 36);
        }
        let Effect::Act(Action::WorkspaceOperation(Change::CloseMember(request))) =
            key(&mut app, KeyCode::Enter)
        else {
            panic!("explicit Delete must dispatch guarded removal")
        };
        assert_eq!(request.mode, Mode::DeleteOnly);
        assert_eq!(request.identity_id, MEMBER);
        app.finished_close_member(Completed {
            request,
            outcome: Outcome::Refused {
                step: Step::Delete,
                error: fault("AGENT_RUNNING", None),
                pane_reason: None,
            },
        });
        let screen = paint(&app, 100, 36);
        assert!(screen.contains("Still running or unproven"));
        assert!(
            app.workspace_form.is_some(),
            "refused deletion keeps the identity's review"
        );
    }
}
#[test]
fn close_pending_survives_hide_reopen_and_late_unrelated_completion_is_ignored() {
    let mut app = board();
    let _commands = crate::runner::forbid_commands();
    let request = begin(&mut app);
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert!(!app.modal_open());
    assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Pending);
    let mut other = request.clone();
    other.identity_id = "33333333-3333-4333-8333-333333333333".into();
    app.finished_close_member(Completed {
        request: other,
        outcome: Outcome::Removed { pane_reason: None },
    });
    assert!(app.workspace_form.is_some());
    app.finished_close_member(Completed {
        request,
        outcome: Outcome::Removed {
            pane_reason: Some("User-opened pane.".into()),
        },
    });
    assert!(app.workspace_form.is_none());
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|text| text.contains("Pane left open: User-opened pane."))
    );
}

#[test]
fn close_controls_require_current_geometry_and_cancel_never_becomes_destructive() {
    let mut app = board();
    let _commands = crate::runner::forbid_commands();
    key(&mut app, KeyCode::Char('x'));
    for (width, height) in [(60, 36), (100, 36), (160, 50)] {
        paint(&app, width, height);
        let draft = app.workspace_form.as_ref().unwrap();
        assert!(draft.close.as_ref().unwrap().hits.borrow().buttons[1].is_some());
        assert_eq!(draft.review.focus, tmt_tui::components::Focus::Buttons(0));
        app.invalidate_frame_geometry();
        assert_eq!(
            app.workspace_confirm(),
            Effect::None,
            "invalidated controls cannot submit"
        );
    }
    let compact = paint(&app, 60, 8);
    assert!(compact.contains("Enlarge to review"));
    assert_eq!(app.workspace_confirm(), Effect::None);
    paint(&app, 12, 8);
    assert!(
        app.workspace_form
            .as_ref()
            .unwrap()
            .close
            .as_ref()
            .unwrap()
            .hits
            .borrow()
            .buttons
            .get(1)
            .is_none_or(Option::is_none)
    );
    assert_eq!(app.workspace_confirm(), Effect::None);
    key(&mut app, KeyCode::Esc);
    assert!(!app.modal_open());
}
#[test]
fn close_keeps_the_reviewed_identity_and_refuses_a_different_viewer() {
    let mut app = board();
    let _commands = crate::runner::forbid_commands();
    key(&mut app, KeyCode::Char('x'));
    paint(&app, 100, 36);
    app.workspace.selected = Some(crate::board::workspace::OpsTarget::Pool);
    key(&mut app, KeyCode::Right);
    paint(&app, 100, 36);
    let Effect::Act(Action::WorkspaceOperation(Change::CloseMember(request))) =
        key(&mut app, KeyCode::Enter)
    else {
        panic!("frozen review must retain its own target")
    };
    assert_eq!(request.identity_id, MEMBER);
    assert_eq!(request.expected_name, NAME);
    let mut app = board();
    key(&mut app, KeyCode::Char('x'));
    paint(&app, 100, 36);
    key(&mut app, KeyCode::Right);
    paint(&app, 100, 36);
    app.view.as_mut().unwrap().me = Some("different-viewer".into());
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|text| text.contains("viewer changed"))
    );
}

#[test]
fn scripted_core_port_preserves_fault_evidence_and_never_adds_originator_or_retries() {
    use std::fs;
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let fixture = Fixture(
        std::env::temp_dir().join(format!("ops-close-api-{}", crate::id::new_v4().unwrap())),
    );
    fs::create_dir(&fixture.0).unwrap();
    let executable = fixture.0.join("core");
    fs::write(
        fixture.0.join("peer.py"),
        r#"import json,pathlib,sys
root=pathlib.Path(__file__).parent
assert sys.argv[1:]==['api']
request=json.load(sys.stdin)
with (root/'calls').open('a') as log: log.write(json.dumps(request)+'\n')
responses=json.loads((root/'responses').read_text())
response=responses[request['operation']]
print(json.dumps(response))
sys.exit(1 if 'error' in response else 0)
"#,
    )
    .unwrap();
    crate::test_support::write_ready_executable(
        &executable,
        r#"#!/bin/sh
exec python3 "$(dirname "$0")/peer.py" "$@"
"#,
    );
    fs::write(fixture.0.join("responses"),json!({
        "identity.stop":{"error":{"code":"PROCESS_ERROR","message":"TERM did not prove exit","signalled":true}},
        "identity.remove":{"error":{"code":"AGENT_RUNNING","message":"Still running or unproven"}},
    }).to_string()).unwrap();
    let core = Core::at(executable);
    let done = request(Mode::CloseAndDelete).apply(&core);
    assert!(
        matches!(done.outcome,Outcome::Refused{step:Step::Stop,ref error,..} if error.signalled==Some(true))
    );
    let done = request(Mode::DeleteOnly).apply(&core);
    assert!(
        matches!(done.outcome,Outcome::Refused{step:Step::Delete,ref error,..} if error.code=="AGENT_RUNNING")
    );
    let calls: Vec<Value> = fs::read_to_string(fixture.0.join("calls"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls,
        vec![
            json!({"version":1,"operation":"identity.stop","input":{"identityId":MEMBER,"expectedName":NAME,"close":true}}),
            json!({"version":1,"operation":"identity.remove","input":{"identityId":MEMBER,"expectedName":NAME}}),
        ]
    );
    fs::write(fixture.0.join("responses"),json!({
        "identity.stop":{"outcome":"stopped","identityId":MEMBER,"name":NAME,"pane":"kept","paneReason":"User-opened pane"},
        "identity.remove":removed(),
    }).to_string()).unwrap();
    assert_eq!(
        request(Mode::CloseAndDelete).apply(&core).outcome,
        Outcome::Removed {
            pane_reason: Some("User-opened pane".into())
        }
    );
    let calls: Vec<Value> = fs::read_to_string(fixture.0.join("calls"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls.len(),
        4,
        "one stop and one guarded remove, no capability or retry call"
    );
    assert_eq!(calls[2], calls[0]);
    assert_eq!(calls[3], calls[1]);
}

#[test]
fn a_remove_conflict_keeps_the_confirmed_stop_and_kept_pane_reason_visible() {
    let result=request(Mode::CloseAndDelete).run(|operation,_| {
        Ok(if operation=="identity.stop" {json!({"outcome":"stopped","identityId":MEMBER,"name":NAME,"pane":"kept","paneReason":"User-opened pane."})}
        else {json!({"outcome":"conflict","identityId":MEMBER,"name":"renamed"})})
    });
    assert_eq!(
        result,
        Outcome::Changed {
            pane_reason: Some("User-opened pane.".into())
        }
    );
    let mut app = board();
    let _commands = crate::runner::forbid_commands();
    let request = begin(&mut app);
    app.finished_close_member(Completed {
        request,
        outcome: result,
    });
    let screen = paint(&app, 100, 36);
    assert!(screen.contains("Pane left open: User-opened pane."));
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    assert_eq!(app.workspace_begin(Kind::RemoveMember), Effect::None);
    assert!(
        matches!(
            app.workspace_form
                .as_ref()
                .unwrap()
                .close
                .as_ref()
                .unwrap()
                .result,
            Some(Outcome::Changed { .. })
        ),
        "a terminal result is not permission to begin a fresh destructive review outside Agents"
    );
}

#[test]
fn refused_jump_uses_the_frozen_uuid_and_keeps_the_explicit_delete_review() {
    let mut app = board();
    let _commands = crate::runner::forbid_commands();
    app.references = crate::board::inventory::Inventory::from_listed(&Ok(json!({
        "identities":[{"id":MEMBER,"name":NAME,"pane":"fixture-pane"}]
    })));
    let request = begin(&mut app);
    app.finished_close_member(Completed {
        request,
        outcome: Outcome::Refused {
            step: Step::Stop,
            error: fault("RUNTIME_CONFLICT", Some(false)),
            pane_reason: None,
        },
    });
    paint(&app, 100, 36);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT)),
        Effect::None
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('j')),
        Effect::Act(Action::Jump(MEMBER.into()))
    );
    assert!(app.workspace_form.as_ref().unwrap().shown);
    assert_eq!(
        app.workspace_form
            .as_ref()
            .unwrap()
            .close
            .as_ref()
            .unwrap()
            .hits
            .borrow()
            .buttons
            .len(),
        3
    );
}
