//! Real App transitions; no worker or ambient terminal is involved.
use super::*;

#[test]
fn escape_returns_one_level_and_never_quits_the_board() {
    for (width, height) in [(160, 36), (100, 24), (60, 18)] {
        let mut app = board();
        let target = app.workspace.selected.clone();
        let expanded = app.workspace.expanded.clone();
        paint(&app, width, height);
        assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
        assert_eq!(
            app.perform(&crate::action::Action::parse("inspect").unwrap()),
            Effect::None
        );
        let screen = paint(&app, width, height).join("\n");
        assert!(screen.contains("── worker "), "{screen}");
        assert!(screen.contains("Esc back"), "{screen}");
        assert!(
            !screen.contains("press j:"),
            "focused host metadata must not describe scrolling as Host: {screen}"
        );
        if width == 160 {
            assert!(screen.contains("j host"), "{screen}");
            assert_eq!(
                app.bindings()["pageup"].verb,
                Verb::Ui(OpsUiAction::ScrollPageUp)
            );
        }
        assert_eq!(app.focused_pane(), Some(Pane::Replies));
        assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
        assert_eq!(app.focused_pane(), Some(Pane::Rows));
        assert_eq!(app.workspace.selected, target);
        assert_eq!(app.workspace.expanded, expanded);
        for _ in 0..3 {
            assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
        }
        assert_eq!(key(&mut app, KeyCode::Char('q')), Effect::Quit);
    }
}

#[test]
fn destination_bindings_leave_conversation_and_dismiss_read_only_owners() {
    // The selected UUID survives entering and dismissing every overlay. Reads
    // emitted by opening recovery/discovery are deliberately not executed here.
    for owner in [
        "conversation",
        "help",
        "menu",
        "timers",
        "snapshots",
        "discovery",
        "checklist",
    ] {
        let mut app = board();
        let target = app.workspace.selected.clone();
        paint(&app, 160, 36);
        match owner {
            "conversation" => {
                app.perform(&crate::action::Action::parse("inspect").unwrap());
            }
            "help" => {
                key(&mut app, KeyCode::Char('?'));
            }
            "menu" => {
                app.context_menu();
            }
            "timers" => {
                key(&mut app, KeyCode::Char('c'));
            }
            "snapshots" => {
                app.open_recovery();
            }
            "discovery" => {
                app.open_discovery();
            }
            "checklist" => {
                app.open_checklist();
            }
            _ => unreachable!(),
        }
        paint(&app, 160, 36);
        assert_eq!(key(&mut app, KeyCode::Char('3')), Effect::None, "{owner}");
        assert_eq!(app.shell.tab, OpsTab::Digest, "{owner}");
        assert!(!app.modal_open(), "{owner}");
        assert_eq!(app.focused_pane(), Some(Pane::Rows), "{owner}");
        assert_eq!(app.workspace.selected, target, "{owner}");
        assert_eq!(key(&mut app, KeyCode::Char('1')), Effect::None, "{owner}");
        assert_eq!(app.shell.tab, OpsTab::Overview, "{owner}");
    }
}

#[test]
fn typing_search_and_message_capture_q_and_digits_and_keep_the_draft() {
    let mut app = board();
    paint(&app, 160, 36);
    key(&mut app, KeyCode::Enter);
    for ch in "q123".chars() {
        assert_eq!(key(&mut app, KeyCode::Char(ch)), Effect::None);
    }
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "q123");
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.input.is_none());
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "q123");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('/'));
    for ch in "q123".chars() {
        assert_eq!(key(&mut app, KeyCode::Char(ch)), Effect::None);
    }
    assert_eq!(app.search, "q123");
    assert_eq!(app.shell.tab, OpsTab::Agents);
    key(&mut app, KeyCode::Esc);
    assert!(!app.searching);
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
}

#[test]
fn covered_editor_is_not_typing_owner_and_destination_keeps_its_uuid_draft() {
    let mut app = board();
    paint(&app, 160, 36);
    key(&mut app, KeyCode::Enter);
    for ch in "kept".chars() {
        key(&mut app, KeyCode::Char(ch));
    }
    app.help_state.borrow_mut().open();
    app.help = true;
    assert_eq!(
        app.workspace_input_owner(),
        crate::board::shell::OpsInputOwner::Popup
    );
    assert_eq!(key(&mut app, KeyCode::Char('2')), Effect::None);
    assert!(!app.help);
    assert!(app.input.is_none());
    assert_eq!(app.shell.tab, OpsTab::Agents);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "kept");
}

#[test]
fn configured_destination_and_reader_entry_share_the_effective_table() {
    let mut app = configured_member(
        "[bind]\nf2='digest-dashboard'\nv='copy {name}'\n",
        json!({"id":MEMBER,"name":"worker","presence":"active"}),
    );
    paint(&app, 160, 36);
    app.perform(&crate::action::Action::parse("inspect").unwrap());
    assert_eq!(
        key(&mut app, KeyCode::Char('v')),
        Effect::Act(Request::Copy {
            text: "worker".into(),
            program: None,
            feedback: crate::board::app::CopyFeedback::Destination,
        })
    );
    assert_eq!(key(&mut app, KeyCode::F(2)), Effect::None);
    assert_eq!(app.shell.tab, OpsTab::Digest);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    key(&mut app, KeyCode::Char('2'));
    app.perform(&crate::action::Action::parse("inspect").unwrap());
    key(&mut app, KeyCode::Char('?'));
    assert_eq!(key(&mut app, KeyCode::F(2)), Effect::None);
    assert!(!app.help);
    assert_eq!(app.shell.tab, OpsTab::Digest);
}

#[test]
fn reader_entry_without_a_response_says_why_without_changing_selection() {
    let mut app = board();
    let target = app.workspace.selected.clone();
    assert!(!paint(&app, 160, 36).join("\n").contains("v reader"));
    app.context_menu();
    assert!(!app.menu.as_ref().unwrap().entries.iter().any(
        |entry| matches!(&entry.choice, Choice::Action(action) if action.verb == Verb::ViewReply)
    ));
    key(&mut app, KeyCode::Esc);
    assert_eq!(key(&mut app, KeyCode::Char('v')), Effect::None);
    assert!(app.row_details.reader.is_none());
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|text| text.contains("response"))
    );
    assert_eq!(app.workspace.selected, target);
}

#[test]
fn reader_entry_uses_the_visible_response_without_requiring_retired_row_expansion() {
    let mut app = board();
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":MEMBER,"requestId":"req_44444444-4444-4444-8444-444444444444",
        "response":"Actual held response", "submittedAtMs":crate::status::now_ms(),
    })];
    paint(&app, 160, 36);
    let target = app.selected_detail_target().unwrap();
    assert!(!app.row_details.expanded.contains(&target));
    assert_eq!(key(&mut app, KeyCode::Char('v')), Effect::None);
    assert!(app.row_details.reader.is_some());
    let screen = paint(&app, 160, 36).join("\n");
    assert!(screen.contains("reply from worker"), "{screen}");
    assert!(screen.contains("Actual held response"), "{screen}");
    key(&mut app, KeyCode::Esc);
    assert!(app.row_details.reader.is_none());
    let screen = paint(&app, 160, 36).join("\n");
    assert!(screen.contains("v reader"), "{screen}");
    app.context_menu();
    assert!(app.menu.as_ref().unwrap().entries.iter().any(
        |entry| matches!(&entry.choice, Choice::Action(action) if action.verb == Verb::ViewReply)
    ));
}

#[test]
fn pending_operations_hide_reopen_and_keep_their_single_outcome_without_resubmission() {
    use crate::board::workspace::{manage::Change, rename_member};
    for (kind, fields) in [
        (Kind::Create, vec!["next", "", "worker", ""]),
        (Kind::Add, vec!["worker"]),
        (Kind::Lead, vec!["worker"]),
        (Kind::RenameSquad, vec!["renamed"]),
        (Kind::RenameMember, vec!["renamed"]),
        (Kind::Remove, vec![]),
        (Kind::Assign, vec!["product", "add"]),
    ] {
        let mut app = board();
        assert_eq!(app.workspace_begin(kind), Effect::None);
        for text in fields {
            assert_eq!(app.workspace_submit_field(text.into()), Effect::None);
        }
        assert_eq!(
            app.workspace_form.as_ref().unwrap().stage,
            Stage::Review,
            "{kind:?}"
        );
        paint(&app, 160, 36);
        let mut submissions = 0;
        let mut observe = |effect: Effect| {
            if matches!(effect, Effect::Act(Request::WorkspaceOperation(_))) {
                submissions += 1;
            }
            effect
        };
        let Effect::Act(Request::WorkspaceOperation(request)) = observe(app.workspace_confirm())
        else {
            panic!("{kind:?}: one explicit reviewed request");
        };

        assert_eq!(
            observe(key(&mut app, KeyCode::Esc)),
            Effect::None,
            "{kind:?}"
        );
        let retained = app.workspace_form.as_ref().unwrap();
        assert!(!retained.shown);
        assert_eq!(retained.stage, Stage::Pending);
        assert!(app.reload_blocked());
        // Destination selection hides, but never submits or forgets, the owned operation.
        assert_eq!(observe(app.workspace_begin(kind)), Effect::None);
        assert_eq!(observe(key(&mut app, KeyCode::Char('3'))), Effect::None);
        assert_eq!(app.shell.tab, OpsTab::Digest);
        assert!(!app.workspace_form.as_ref().unwrap().shown);
        assert!(app.reload_blocked());
        assert_eq!(
            observe(app.workspace_begin(if kind == Kind::Create {
                Kind::Remove
            } else {
                Kind::Create
            })),
            Effect::None
        );
        assert_eq!(app.workspace_form.as_ref().unwrap().kind, kind);
        assert_eq!(
            app.workspace_form.as_ref().unwrap().operation.as_ref(),
            Some(&request)
        );
        assert!(!app.workspace_form.as_ref().unwrap().shown);
        assert_eq!(observe(app.context_menu()), Effect::None);
        let index = app
            .menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|entry| entry.label == format!("Pending {}", kind.label()))
            .unwrap();
        app.menu.as_mut().unwrap().selected = index;
        assert_eq!(observe(key(&mut app, KeyCode::Enter)), Effect::None);
        assert!(app.workspace_form.as_ref().unwrap().shown);
        assert_eq!(observe(app.workspace_confirm()), Effect::None);
        assert!(
            app.workspace_form
                .as_ref()
                .unwrap()
                .controls
                .get()
                .iter()
                .all(Option::is_none),
            "stale confirmation must stay revoked"
        );
        assert_eq!(observe(key(&mut app, KeyCode::Esc)), Effect::None);
        match request {
            Change::RenameMember(request) => app.finished_member_rename(rename_member::Completed {
                request,
                outcome: Ok(rename_member::Outcome::Applied {
                    name: "renamed".into(),
                    changed: true,
                }),
            }),
            Change::Membership(_) => app.finished_workspace(Err(crate::core::SquadError::new(
                "API_UNAVAILABLE",
                "Outcome not confirmed",
            ))),
        }
        assert!(!app.workspace_form.as_ref().unwrap().shown);
        assert!(matches!(
            app.workspace_form.as_ref().unwrap().stage,
            Stage::Result(_)
        ));
        assert_eq!(observe(app.context_menu()), Effect::None);
        let index = app
            .menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|entry| entry.label == format!("{} result", kind.label()))
            .unwrap();
        app.menu.as_mut().unwrap().selected = index;
        assert_eq!(observe(key(&mut app, KeyCode::Enter)), Effect::None);
        assert!(app.workspace_form.as_ref().unwrap().shown);
        assert_eq!(observe(app.workspace_confirm()), Effect::None);

        assert_eq!(observe(key(&mut app, KeyCode::Esc)), Effect::None);
        assert!(app.workspace_form.is_none());
        assert!(!app.reload_blocked());
        assert_eq!(observe(key(&mut app, KeyCode::Esc)), Effect::None);
        assert_eq!(
            submissions, 1,
            "{kind:?}: one effect leaves App for the lane"
        );
    }
}

#[test]
fn q_steps_back_in_overlays_and_release_events_do_not_switch_tabs() {
    let mut app = board();
    paint(&app, 160, 36);
    key(&mut app, KeyCode::Char('?'));
    assert_eq!(key(&mut app, KeyCode::Char('q')), Effect::None);
    assert!(!app.help);
    app.perform(&crate::action::Action::parse("inspect").unwrap());
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    let mut release = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE);
    release.kind = ratatui::crossterm::event::KeyEventKind::Release;
    assert_eq!(app.key(release), Effect::None);
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(key(&mut app, KeyCode::Char('q')), Effect::Quit);
}
