use super::*;
use crate::board::workspace::{
    manage::Change,
    rename_member::{Completed, Outcome, Request as Rename},
};

fn rename(app: &mut App, name: &str) -> Rename {
    assert_eq!(key(app, KeyCode::Char('r')), Effect::None);
    assert_eq!(
        app.workspace_form.as_ref().unwrap().kind,
        Kind::RenameMember
    );
    app.input.as_mut().unwrap().text = name.into();
    assert_eq!(key(app, KeyCode::Enter), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
    paint(app, 160, 36);
    let Effect::Act(Request::WorkspaceOperation(Change::RenameMember(request))) =
        app.workspace_confirm()
    else {
        panic!("explicit reviewed rename")
    };
    request
}
#[test]
fn only_visible_agents_list_member_has_default_rename_and_hidden_target_cannot_begin() {
    let mut app = board();
    assert_eq!(app.bindings()["r"].text, "rename-member");
    for tab in [OpsTab::Overview, OpsTab::Digest] {
        app.shell.tab = tab;
        assert!(!app.bindings().contains_key("r"));
        app.workspace_begin(Kind::RenameMember);
        assert!(app.workspace_form.is_none());
    }
    app.shell.tab = OpsTab::Agents;
    app.focus = 1;
    assert!(!app.bindings().values().any(|a| a.text == "rename-member"));
    app.focus = 0;
    app.search = "no-visible-match".into();
    app.workspace_reconcile();
    assert!(!app.bindings().values().any(|a| a.text == "rename-member"));
    app.workspace_begin(Kind::RenameMember);
    assert!(app.workspace_form.is_none());
}
#[test]
fn rename_freezes_uuid_literal_expected_name_and_requires_visible_review_once() {
    let mut app = board();
    let selected = app.workspace.selected.clone().unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&selected)
        .unwrap()["name"] = json!(" Worker ");
    key(&mut app, KeyCode::Char('r'));
    assert_eq!(
        app.workspace_form
            .as_ref()
            .unwrap()
            .member
            .as_ref()
            .unwrap()
            .name,
        " Worker ",
        "the visible row supplies the CAS precondition even if the earlier global read still says worker"
    );
    app.workspace_submit_field(" --literal-name ".into());
    assert_eq!(
        app.workspace_confirm(),
        Effect::None,
        "no geometry grants a write"
    );
    let screen = paint(&app, 160, 36).join("\n");
    assert!(screen.contains(MEMBER));
    assert!(screen.contains("Expected current name: \" Worker \""));
    app.workspace.selected = Some(OpsTarget::Pool);
    let Effect::Act(Request::WorkspaceOperation(Change::RenameMember(request))) =
        app.workspace_confirm()
    else {
        panic!("frozen rename")
    };
    assert_eq!(
        request,
        Rename {
            identity_id: MEMBER.into(),
            expected_name: " Worker ".into(),
            name: " --literal-name ".into()
        }
    );
    assert_eq!(app.workspace_confirm(), Effect::None);
}
#[test]
fn closing_pending_rename_retains_result_ownership_and_reopen_never_resubmits() {
    let mut app = board();
    let request = rename(&mut app, "renamed");
    app.workspace_close();
    assert!(!app.workspace_form.as_ref().unwrap().shown);
    assert!(app.reload_blocked());
    assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
    assert_eq!(
        app.workspace_form.as_ref().unwrap().kind,
        Kind::RenameMember
    );
    let mut wrong = request.clone();
    wrong.identity_id = ROOM.into();
    app.finished_member_rename(Completed {
        request: wrong,
        outcome: Ok(Outcome::NotFound),
    });
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Pending);
    app.finished_workspace(Err(crate::core::SquadError::new(
        "API_UNAVAILABLE",
        "An earlier membership operation completed",
    )));
    assert_eq!(
        app.workspace_form.as_ref().unwrap().stage,
        Stage::Pending,
        "an earlier membership result cannot consume the rename completion"
    );
    app.finished_member_rename(Completed {
        request: request.clone(),
        outcome: Ok(Outcome::Applied {
            name: "renamed".into(),
            changed: true,
        }),
    });
    assert!(!app.workspace_form.as_ref().unwrap().shown);
    assert_eq!(app.workspace_begin(Kind::RenameMember), Effect::None);
    let Stage::Result(text) = &app.workspace_form.as_ref().unwrap().stage else {
        panic!("retained result")
    };
    assert!(text.contains("Renamed"));
    assert!(text.contains(MEMBER));
    assert_eq!(app.workspace_confirm(), Effect::None);
    assert_eq!(
        app.workspace.selected.as_ref(),
        Some(&OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into()
        })
    );
}
#[test]
fn conflict_actual_name_and_retirement_never_authorize_an_old_review_or_successor() {
    for outcome in [
        Outcome::Conflict {
            name: "competitor".into(),
        },
        Outcome::NotFound,
    ] {
        let mut app = board();
        let request = rename(&mut app, "desired");
        app.finished_member_rename(Completed {
            request,
            outcome: Ok(outcome.clone()),
        });
        let Stage::Result(text) = &app.workspace_form.as_ref().unwrap().stage else {
            panic!("result")
        };
        assert!(text.contains("Not applied"));
        if matches!(outcome, Outcome::Conflict { .. }) {
            assert!(text.contains("competitor"));
        } else {
            assert!(text.contains("successor"));
        }
        assert_eq!(app.workspace_confirm(), Effect::None);
    }
}
#[test]
fn resize_and_stale_pointer_revoke_apply_until_exact_target_is_visible() {
    let mut app = board();
    key(&mut app, KeyCode::Char('r'));
    app.workspace_submit_field("worker-two".into());
    paint(&app, 160, 36);
    let apply = app.workspace_form.as_ref().unwrap().controls.get()[1].unwrap();
    app.invalidate_frame_geometry();
    let pointer = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: apply.x,
        row: apply.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.overlay_event(&pointer), Some(Effect::None));
    assert_eq!(app.workspace_confirm(), Effect::None);
    paint(&app, 60, 8);
    assert_eq!(app.workspace_confirm(), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
}

#[test]
fn configured_r_keeps_its_meaning_and_unassigned_member_uses_the_same_rename() {
    let mut explicit = configured_member(
        "[bind]\nr = \"copy {name}\"\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    assert!(
        matches!(key(&mut explicit, KeyCode::Char('r')), Effect::Act(Request::Copy {text,..}) if text == "worker")
    );
    assert!(explicit.workspace_form.is_none());

    let mut app = board();
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.squads.clear();
    let target = OpsTarget::Member {
        room_id: None,
        identity_id: MEMBER.into(),
    };
    home.workspace
        .rows
        .insert(target.clone(), json!({"id":MEMBER,"name":"worker"}));
    app.workspace.pool_expanded = true;
    app.workspace.selected = Some(target);
    app.workspace_reconcile();
    let request = rename(&mut app, "pool-worker");
    assert_eq!(request.identity_id, MEMBER);
    assert!(app.workspace_form.as_ref().unwrap().room.is_none());
}

#[test]
fn initial_enter_and_escape_cancel_without_clicking_through_to_the_member() {
    for close in [KeyCode::Enter, KeyCode::Esc] {
        let mut app = board();
        let selected = app.workspace.selected.clone();
        key(&mut app, KeyCode::Char('r'));
        app.input.as_mut().unwrap().text = "different".into();
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
        assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
        paint(&app, 160, 36);
        assert_eq!(key(&mut app, close), Effect::None);
        assert!(app.workspace_form.is_none());
        assert_eq!(app.workspace.selected, selected);
        assert!(app.input.is_none());
    }
}

#[test]
fn no_op_fault_and_uncertain_results_never_replay_and_escape_untrusted_text() {
    use crate::core::SquadError;
    for (outcome, label) in [
        (
            Ok(Outcome::Applied {
                name: "worker".into(),
                changed: false,
            }),
            "Name unchanged",
        ),
        (
            Err(SquadError::new(
                "NAME_ALREADY_ACTIVE",
                "Destination is active",
            )),
            "Not applied",
        ),
        (
            Err(SquadError::new("STORAGE_NOT_WRITABLE", "Storage refused")),
            "Not applied",
        ),
        (
            Err(SquadError::new(
                "SQUAD_IDENTITY_RENAME_INVALID",
                "Lost\u{1b}[2J result\nnext",
            )),
            "Unknown",
        ),
    ] {
        let mut app = board();
        let request = rename(&mut app, "worker");
        app.finished_member_rename(Completed { request, outcome });
        let Stage::Result(text) = &app.workspace_form.as_ref().unwrap().stage else {
            panic!("retained result")
        };
        assert!(text.starts_with(label), "{text}");
        assert!(!text.contains('\u{1b}'));
        assert_eq!(app.workspace_confirm(), Effect::None);
        let notice = app.notice.clone();
        key(&mut app, KeyCode::Esc);
        assert!(app.workspace_form.is_none());
        assert_eq!(app.notice, notice);
    }
}

#[test]
fn public_refresh_updates_name_but_retains_uuid_selection_and_session_draft() {
    let mut app = board();
    app.perform(&Action::parse("talk").unwrap());
    key(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Char('i'));
    key(&mut app, KeyCode::Esc);
    app.focus = 0;
    let selected = app.workspace.selected.clone().unwrap();
    let request = rename(&mut app, "new-worker");
    app.finished_member_rename(Completed {
        request,
        outcome: Ok(Outcome::Applied {
            name: "new-worker".into(),
            changed: true,
        }),
    });
    assert_eq!(
        app.selected_row().unwrap()["name"],
        "worker",
        "completion never edits the roster locally"
    );
    let mut fresh = board();
    let workspace = &mut fresh
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace;
    workspace.roster.members.get_mut(MEMBER).unwrap().name = "new-worker".into();
    workspace.rows.get_mut(&selected).unwrap()["name"] = json!("new-worker");
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    snapshot.tabs = fresh.tabs;
    snapshot.view = Ok(fresh.view.take().unwrap());
    app.apply(snapshot);
    assert_eq!(app.workspace.selected.as_ref(), Some(&selected));
    assert_eq!(app.selected_row().unwrap()["name"], "new-worker");
    key(&mut app, KeyCode::Esc);
    app.perform(&Action::parse("talk").unwrap());
    assert_eq!(app.input.as_ref().unwrap().text, "hi");
}

#[test]
fn closed_rename_stays_reachable_through_actions_after_its_member_disappears() {
    let mut app = board();
    let request = rename(&mut app, "next");
    app.workspace_close();
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.members.clear();
    home.workspace.roster.squads[0].members.clear();
    home.workspace
        .rows
        .retain(|target, _| !matches!(target, OpsTarget::Member { .. }));
    app.workspace.selected = Some(OpsTarget::Pool);
    app.workspace_reconcile();
    assert!(
        !app.bindings().contains_key("r"),
        "no new Rename authority without a visible member"
    );
    assert!(app.reload_blocked());
    for (outcome, label) in [
        (None, "Pending Rename"),
        (Some(Outcome::NotFound), "Rename result"),
    ] {
        if let Some(outcome) = outcome {
            app.finished_member_rename(Completed {
                request: request.clone(),
                outcome: Ok(outcome),
            });
        }
        app.context_menu();
        let menu = app.menu.as_mut().unwrap();
        menu.selected = menu
            .entries
            .iter()
            .position(|entry| entry.label == label)
            .expect("owned pending/result inspection never needs a new member");
        paint(&app, 80, 24);
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
        assert!(app.workspace_form.as_ref().unwrap().shown);
        assert_eq!(
            app.workspace_confirm(),
            Effect::None,
            "inspection cannot resubmit"
        );
        assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    }
    assert!(app.workspace_form.is_none());
    assert!(
        !app.reload_blocked(),
        "closing the read result releases its existing reload owner"
    );
}
