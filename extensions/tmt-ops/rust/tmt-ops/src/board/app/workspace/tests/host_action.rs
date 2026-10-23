use super::*;

#[test]
fn workspace_settings_down_saturation_selects_its_final_actions_entry() {
    let mut app = board();
    // With no squad selected the menu ends at Panels; a squad adds its settings link after a rule.
    app.workspace.selected = Some(OpsTarget::Pool);
    assert_eq!(app.workspace_settings(), Effect::None);
    let last = app.menu.as_ref().unwrap().entries.last().unwrap();
    assert_eq!(last.label, "Panels");
    assert!(matches!(&last.choice, Choice::Panels));
    paint(&app, 160, 36);
    for _ in 0..40 {
        assert_eq!(key(&mut app, KeyCode::Down), Effect::None);
    }
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.selected, menu.entries.len() - 1);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.panels_picker.is_some());
}

#[test]
fn custom_host_binding_wins_in_overview_agents_and_conversation_from_one_table() {
    let tab = OpsTab::Agents;
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
    assert_eq!(app.workspace_scope(), crate::config::OpsKeyScope::Reader);
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

#[test]
fn board_pane_run_defers_only_reserved_context_to_the_effect_lane() {
    let row = json!({"id":MEMBER,"name":"worker","fields":{"board_pane":"%99"},"pane":{"id":"%3","location":{"host":"tmux"}}});
    let mut app = configured_member("[bind]\nj='run echo {board_pane} {name}'\n", row.clone());
    assert_eq!(
        key(&mut app, KeyCode::Char('j')),
        Effect::Act(Request::PreparedRun {
            action: crate::action::Action::parse("run echo {board_pane} {name}").unwrap(),
            ops: app.parameter_observations(&row),
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
    let tab = OpsTab::Agents;
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
    app.workspace.filter = crate::board::workspace::AgentsFilter::All;
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

#[test]
fn host_editor_projects_the_same_factory_j_as_dispatch_in_each_pane() {
    // The tree follows the recipient in either pane; the flat list's reader keeps reader keys.
    for flat in [false, true] {
        for reader in [false, true] {
            let mut app = configured_member(
                "",
                json!({"id":MEMBER,"name":"worker","presence":"active","pane":{"id":"%3","location":{"host":"tmux"}}}),
            );
            app.shell.tab = OpsTab::Agents;
            if flat {
                app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
            }
            paint(&app, 160, 36);
            if reader {
                key(&mut app, KeyCode::Tab);
            }
            let expected = if flat && reader {
                "scroll-down"
            } else {
                "jump"
            };
            assert_eq!(app.bindings()["j"].text, expected);
            let selection = app.host_action_selection();
            let path = std::path::PathBuf::from(&app.workspace_snapshot().unwrap().config_path);
            assert!(
                !path.exists(),
                "configured_member removes its temporary file"
            );
            app.open_host_action(
                Config::read(path.clone()).unwrap(),
                selection,
                Some("%9".into()),
            )
            .unwrap();
            let overlay = app.settings.as_ref().unwrap();
            assert_eq!(overlay.settings.entries[0].value, expected);
            assert_eq!(overlay.settings.entries[0].source, "workspace default");
            assert_eq!(
                overlay.picks[0].value,
                if expected == "jump" { "(current)" } else { "" }
            );
            assert!(!path.exists(), "opening the editor never authors a binding");
        }
    }
}

#[test]
fn nested_parameters_dispatch_current_observations_not_the_opening_preview() {
    let row = json!({"id":MEMBER,"name":"worker","fields":{"task":"opening", "runtime":"custom"},"pane":{"id":"%3","location":{"host":"tmux"}}});
    let mut app = configured_member(
        "[bind]\nf12='run echo {row:/fields/task} {ops:/runningDriver} {runtime}'\n",
        row,
    );
    app.references.agents.insert(
        MEMBER.into(),
        crate::board::inventory::Agent {
            parameters: json!({"runningDriver":"codex"}),
            ..Default::default()
        },
    );
    let opening = app.host_action_selection();
    assert_eq!(opening.ops["runningDriver"], "codex");
    let target = app.workspace.selected.clone().unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&target)
        .unwrap()["fields"]["task"] = json!("literal $(touch nope); two words");
    app.references.agents.get_mut(MEMBER).unwrap().parameters = json!({"runningDriver":"claude"});
    assert_eq!(
        key(&mut app, KeyCode::F(12)),
        Effect::Act(Request::Run(vec![
            "echo".into(),
            "literal $(touch nope); two words".into(),
            "claude".into(),
            "custom".into()
        ]))
    );
    assert_eq!(opening.recipient.unwrap()["fields"]["task"], "opening");
    app.references.agents.get_mut(MEMBER).unwrap().parameters = json!({});
    assert_eq!(key(&mut app, KeyCode::F(12)), Effect::None);
    assert!(app.notice.as_ref().unwrap().contains("ops:/runningDriver"));
}

#[test]
fn the_editor_factory_sample_matches_reader_and_recipient_dependent_bindings() {
    let row = json!({"id":MEMBER,"name":"worker","fields":{},"pane":{"id":"%3","location":{"host":"tmux"}}});
    let mut app = configured_member("", row);
    for focus in [0, 1] {
        app.focus = focus;
        let selection = app.host_action_selection();
        let path = std::env::temp_dir().join(format!(
            "ops-all-key-default-{}.toml",
            crate::id::new_v4().unwrap()
        ));
        app.open_host_action(Config::read(path).unwrap(), selection, None)
            .unwrap();
        let overlay = app.settings.as_ref().unwrap();
        for (event, action) in app
            .bindings()
            .into_iter()
            .filter(|(event, _)| crate::action::valid_event(event))
        {
            assert_eq!(
                overlay
                    .settings
                    .entries
                    .iter()
                    .find(|entry| entry.key == format!("bind.{event}"))
                    .unwrap()
                    .value,
                action.text,
                "{event}"
            );
        }
        app.settings = None;
    }
}

#[test]
fn parameter_timers_follow_an_explicit_mention_occurrence_and_squad_names_come_from_roster() {
    use crate::board::overview::cron_counts::{Job, Summary};
    let row = json!({"id":MEMBER,"name":"worker","fields":{},"pane":{"id":"%3","location":{"host":"tmux"}}});
    let mut app = configured_member("", row.clone());
    app.cron.counts = Summary::of(Some(&[
        Job {
            room: ROOM,
            id: "job-a",
            owner: Some(MEMBER),
            on: true,
            owner_valid: true,
        },
        Job {
            room: "other-room",
            id: "job-b",
            owner: Some(MEMBER),
            on: true,
            owner_valid: true,
        },
        Job {
            room: "other-room",
            id: "job-c",
            owner: Some(MEMBER),
            on: false,
            owner_valid: true,
        },
    ]));
    let other = OpsTarget::Member {
        room_id: Some("other-room".into()),
        identity_id: MEMBER.into(),
    };
    let mut other_squad = app.workspace_snapshot().unwrap().roster.squads[0].clone();
    other_squad.room_id = "other-room".into();
    other_squad.name = "other".into();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads
        .push(other_squad);
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .insert(other.clone(), row.clone());
    let action = crate::action::Action::parse("run echo {ops:/timers/total}").unwrap();
    assert_eq!(
        app.perform_member_host(&action, &other),
        Effect::Act(Request::Run(vec!["echo".into(), "2".into()]))
    );
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    let observation = app.parameter_observations(&row);
    assert_eq!(observation["squad"]["name"], "product");
    assert_eq!(observation["timers"]["total"], 1);
    app.cron.counts = Summary::default();
    assert!(app.parameter_observations(&row).get("timers").is_none());
}

#[test]
fn opening_factory_table_never_bakes_authored_commands_into_reset_inheritance() {
    let row = json!({"id":MEMBER,"name":"worker","fields":{},"pane":{"id":"%3","location":{"host":"tmux"}}});
    let app = configured_member("[bind]\nf12='copy {name}'\n", row);
    assert!(app.bindings().contains_key("f12"));
    let selection = app.host_action_selection();
    assert!(!selection.factory_bindings.contains_key("f12"));
    let path = std::env::temp_dir().join(format!(
        "ops-reset-factory-{}.toml",
        crate::id::new_v4().unwrap()
    ));
    let config = Config::read(path).unwrap();
    assert!(
        config
            .binding(
                selection.scope,
                &[crate::config::BindingLocation::Global],
                "f12",
                &selection.factory_bindings
            )
            .unwrap()
            .action
            .is_none()
    );
}

#[test]
fn member_session_has_one_factory_binding_in_rows_and_reader() {
    for scope in [
        crate::config::OpsKeyScope::Member,
        crate::config::OpsKeyScope::Reader,
    ] {
        let keys = crate::config::workspace_defaults(scope);
        assert_eq!(
            keys.get("R").map(|action| action.text.as_str()),
            Some("member-session")
        );
    }
}

#[test]
fn member_session_opens_a_placeholder_without_running_a_command() {
    let mut app = configured_member("", json!({"id":MEMBER,"name":"worker"}));
    let _no_commands = crate::runner::forbid_commands();
    let _effect = key(&mut app, KeyCode::Char('R'));
    assert!(
        app.modal_open(),
        "Session opens immediately before a worker read can finish"
    );
    assert!(paint(&app, 160, 36).join("\n").contains("Loading session"));
}

#[test]
fn an_authored_session_key_keeps_its_binding_and_typing_keeps_its_text() {
    let mut app = configured_member(
        "[bind]\nR='run echo {name}'\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('R')),
        Effect::Act(Request::Run(vec!["echo".into(), "worker".into()]))
    );
    assert!(app.member_session.is_none());
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('R'));
    assert!(app.member_session.is_none());
    assert!(app.input.as_ref().unwrap().text.as_str().contains('R'));
}

#[test]
fn session_opens_in_conversation_focus_without_starting_a_command() {
    let mut app = board();
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.workspace_scope(), OpsKeyScope::Reader);
    let _ui = crate::runner::forbid_commands();
    assert!(matches!(
        key(&mut app, KeyCode::Char('R')),
        Effect::MemberSession(_)
    ));
    assert!(paint(&app, 160, 36).join("\n").contains("Loading session"));
}
