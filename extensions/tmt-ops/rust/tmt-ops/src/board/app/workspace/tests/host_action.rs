use super::*;

#[test]
fn workspace_settings_down_saturation_selects_its_final_actions_entry() {
    let mut app = board();
    assert_eq!(app.workspace_settings(), Effect::None);
    let last = app.menu.as_ref().unwrap().entries.last().unwrap();
    assert_eq!(last.label, "Actions");
    assert!(
        matches!(&last.choice, Choice::Action(action) if action.verb == crate::action::Verb::Menu)
    );
    paint(&app, 160, 36);
    for _ in 0..40 {
        assert_eq!(key(&mut app, KeyCode::Down), Effect::None);
    }
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.selected, menu.entries.len() - 1);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_ne!(
        app.menu.as_ref().unwrap().title,
        "Workspace settings · minimum usable size governs placement"
    );
}

#[test]
fn custom_host_binding_wins_in_overview_agents_and_conversation_from_one_table() {
    for tab in [OpsTab::Overview, OpsTab::Agents] {
        let mut app = configured_member(
            "[bind]\nj='swap left'\no='open {pr_link}'\n",
            json!({"id":MEMBER,"name":"worker","fields":{"pr_link":"https://example.com/pull/42"},"pane":{"id":"%3","location":{"host":"tmux"}}}),
        );
        app.shell.tab = tab;
        app.focus = 0;
        assert_eq!(
            key(&mut app, KeyCode::Char('j')),
            Effect::Act(Request::Swap {
                target: MEMBER.into(),
                name: "worker".into(),
                direction: "left".into()
            })
        );
        assert!(matches!(
            key(&mut app, KeyCode::Char('o')),
            Effect::Act(Request::Open { .. })
        ));
        assert!(paint(&app, 160, 36).join("\n").contains("j swap left"));
        key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.workspace_scope(),
            if tab == OpsTab::Overview {
                crate::config::OpsKeyScope::OverviewReader
            } else {
                crate::config::OpsKeyScope::Reader
            }
        );
        assert_eq!(
            key(&mut app, KeyCode::Char('j')),
            Effect::Act(Request::Swap {
                target: MEMBER.into(),
                name: "worker".into(),
                direction: "left".into()
            })
        );
        assert!(paint(&app, 160, 36).join("\n").contains("j swap left"));
    }
}

#[test]
fn board_pane_run_defers_only_reserved_context_to_the_effect_lane() {
    let row = json!({"id":MEMBER,"name":"worker","fields":{"board_pane":"%99"},"pane":{"id":"%3","location":{"host":"tmux"}}});
    let mut app = configured_member("[bind]\nj='run echo {board_pane} {name}'\n", row.clone());
    assert_eq!(
        key(&mut app, KeyCode::Char('j')),
        Effect::Act(Request::PreparedRun {
            action: crate::action::Action::parse("run echo {board_pane} {name}").unwrap(),
            row
        })
    );
    assert!(paint(&app, 160, 36).join("\n").contains("j run echo"));
    app.workspace.selected = Some(OpsTarget::Member {
        room_id: None,
        identity_id: "unknown".into(),
    });
    assert_eq!(key(&mut app, KeyCode::Char('j')), Effect::None);
    assert!(app.notice.as_deref().unwrap().contains("visible"));
}

#[test]
fn squad_host_action_keeps_its_binding_context_and_current_lead_recipient() {
    for tab in [OpsTab::Overview, OpsTab::Agents] {
        let mut app = configured_member(
            "[bind]\nj='jump'\n[bind.tmux]\nj='swap left'\n",
            json!({"id":MEMBER,"name":"worker","pane":{"id":"%3","location":{"host":"tmux"}}}),
        );
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .squads[0]
            .lead = Some(MEMBER.into());
        app.shell.tab = tab;
        app.overview_view = crate::board::overview::monitor::View::Squads;
        app.overview_rows.borrow_mut().clear();
        app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
        app.workspace_reconcile();
        paint(&app, 160, 36);
        let selection = app.host_action_selection();
        assert_eq!(selection.row.as_ref().unwrap()["roomId"], ROOM);
        assert_eq!(selection.recipient.as_ref().unwrap()["id"], MEMBER);
        assert_eq!(
            selection.squad, None,
            "squad roots do not apply a member section"
        );
        assert_eq!(
            key(&mut app, KeyCode::Char('j')),
            Effect::Act(Request::Swap {
                target: MEMBER.into(),
                name: "worker".into(),
                direction: "left".into()
            })
        );
        assert!(paint(&app, 160, 36).join("\n").contains("j swap left"));
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .squads[0]
            .lead = None;
        assert!(
            app.host_action_selection().recipient.is_none(),
            "an unobserved lead never becomes the squad record"
        );
    }
}
