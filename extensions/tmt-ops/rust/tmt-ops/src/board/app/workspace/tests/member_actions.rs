//! Member copy/note actions and their explicit renderer evidence.
use super::*;

fn member_actions_with_lead() -> App {
    const LEAD: &str = "88888888-8888-4888-8888-888888888881";
    let mut app = board();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut lead = snapshot.roster.members[MEMBER].clone();
    lead.id = LEAD.into();
    lead.name = "product-lead".into();
    snapshot.roster.members.insert(LEAD.into(), lead);
    snapshot.roster.squads[0].lead = Some(LEAD.into());
    snapshot.roster.squads[0].members.push(LEAD.into());
    snapshot.rows.insert(
        OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: LEAD.into(),
        },
        json!({"id":LEAD,"name":"product-lead","state":"working"}),
    );
    let row = snapshot
        .rows
        .get_mut(app.workspace.selected.as_ref().unwrap())
        .unwrap();
    row["fields"] = json!({"task":"ship the fix"});
    row["waitingOnYou"] = json!([{"requestId":"pending-question","preview":"answer me"}]);
    app.workspace_reconcile();
    app
}

fn member_actions_with_authored_note() -> App {
    let mut app = member_actions_with_lead();
    let path = std::env::temp_dir().join(format!(
        "ops-authored-note-{}.toml",
        crate::id::new_v4().unwrap()
    ));
    std::fs::write(&path, "[bind]\na='annotate lead'\n").unwrap();
    let config = Config::read(path.clone()).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .configure(&config)
        .unwrap();
    std::fs::remove_file(path).unwrap();
    app
}

#[test]
fn agents_shortcuts_keep_authored_keys_without_factory_duplicates() {
    let mut app = configured_member(
        "[bind]\nr='copy authored-r'\nd='copy authored-d'\na='copy authored-a'\nv='copy authored-v'\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    for ch in ['r', 'd', 'a', 'v'] {
        assert!(matches!(
            key(&mut app, KeyCode::Char(ch)),
            Effect::Act(Request::Copy { text, .. }) if text == format!("authored-{ch}")
        ));
    }
}

#[test]
fn agents_shortcuts_keep_unbound_rename_and_remove_in_actions() {
    let mut app = configured_member(
        "[bind]\nr='copy r'\nd='copy d'\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    paint(&app, 160, 40);
    app.context_menu();
    for ui in [OpsUiAction::RenameMember, OpsUiAction::RemoveMember] {
        let entries: Vec<_> = app.menu.as_ref().unwrap().entries.iter().filter(|entry| {
            matches!(&entry.choice, Choice::Action(action) if action.verb == Verb::Ui(ui))
        }).collect();
        assert_eq!(entries.len(), 1, "{ui:?}");
        assert!(
            entries[0].key.is_empty(),
            "unbound actions invent no shortcut"
        );
    }
}

#[test]
fn agents_shortcuts_right_click_selects_the_fresh_row_and_opens_actions() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = member_actions_with_lead();
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    paint(&app, 160, 40);
    let row = app.workspace_rows().iter().position(|row| {
        matches!(&row.target, OpsTarget::Member { identity_id, .. } if identity_id != MEMBER)
    }).unwrap();
    let expected = app.workspace_rows()[row].target.clone();
    let hit = app
        .hits
        .borrow()
        .iter()
        .copied()
        .find(|hit| hit.row() == Some(row))
        .unwrap();
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    );
    assert_eq!(app.workspace.selected.as_ref(), Some(&expected));
    assert!(
        app.menu.is_some(),
        "right-click opens the existing Actions owner"
    );
}

#[test]
fn agents_shortcuts_keep_tab_hints_out_of_the_whole_footer() {
    let app = member_actions_with_lead();
    let text = paint(&app, 160, 40).join("\n");
    assert!(text.contains("f  Squads"), "{text}");
    assert!(text.contains("M  Talk   Measure"), "{text}");
    let footer = crate::board::view::footer::agents_hints(&app, 1000).unwrap();
    assert!(!footer.contains("M talk/measure"), "{footer}");
    assert!(!footer.contains("f filter"), "{footer}");
}

#[test]
fn member_copy_key_uses_the_selected_row_even_with_conversation_focus() {
    for tab in [OpsTab::Agents] {
        for reader in [false, true] {
            let mut app = member_actions_with_authored_note();
            app.shell.tab = tab;
            app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
            paint(&app, 160, 36);
            if reader {
                app.focus = app
                    .focus_panes()
                    .iter()
                    .position(|pane| *pane == Pane::Replies)
                    .unwrap();
                assert_eq!(app.focused_pane(), Some(Pane::Replies));
            }
            assert!(app.workspace_selected_member().is_some());
            let copied = key(&mut app, KeyCode::Char('y'));
            assert!(
                matches!(
                    &copied,
                    Effect::Act(Request::Copy { text, .. }) if text == "worker: ship the fix (working)"
                ),
                "{tab:?} reader={reader}: {copied:?}; notice={:?}",
                app.notice
            );
            key(&mut app, KeyCode::Char('a'));
            assert!(matches!(app.input.as_ref().map(|input| &input.compose),
                Some(Compose::Annotate { to, row }) if to == "product-lead" && row == "worker"));
        }
    }
}

#[test]
fn explicit_member_note_goes_to_the_lead_instead_of_answering_a_pending_request() {
    let mut app = member_actions_with_authored_note();
    app.perform(&Action::parse("annotate lead").unwrap());
    assert!(app.menu.is_none());
    assert_eq!(
        app.input.as_ref().unwrap().compose,
        Compose::Annotate {
            to: "product-lead".into(),
            row: "worker".into()
        }
    );
    app.paste("keep the recipient explicit");
    assert!(
        matches!(key(&mut app, KeyCode::Enter), Effect::Act(Request::Annotate {
        to, row, text, ..
    }) if to == "product-lead" && row == "worker" && text == "[product · worker] keep the recipient explicit")
    );
}

#[test]
fn member_note_without_an_observed_lead_never_falls_back_to_member_talk() {
    let mut app = board();
    app.perform(&Action::parse("annotate lead").unwrap());
    assert!(app.input.is_none());
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|notice| notice.contains("lead"))
    );
}

#[test]
fn member_note_placeholder_names_the_lead_and_keeps_the_subject_in_the_send_owner() {
    for tab in [OpsTab::Agents] {
        for width in [160, 60] {
            let mut app = member_actions_with_authored_note();
            app.shell.tab = tab;
            app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
            app.perform(&Action::parse("annotate lead").unwrap());
            let lines = paint(&app, width, 36);
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("note to product-lead")),
                "{tab:?} width={width}: {lines:?}"
            );
            assert!(!lines.iter().any(|line| line.contains("about worker")));
            assert!(matches!(&app.input.as_ref().unwrap().compose,
                Compose::Annotate { to, row } if to == "product-lead" && row == "worker"));
        }
    }
}

#[test]
fn authored_member_keys_keep_their_meaning_and_hidden_rows_grant_no_action() {
    let mut app = configured_member(
        "[bind]\na='copy {name}'\ny='copy authored-y'\ns='copy authored-s'\n",
        json!({"id":MEMBER,"name":"worker","state":"working"}),
    );
    for (ch, expected) in [('a', "worker"), ('y', "authored-y"), ('s', "authored-s")] {
        assert!(
            matches!(key(&mut app, KeyCode::Char(ch)), Effect::Act(Request::Copy { text, .. }) if text == expected)
        );
        assert!(app.input.is_none());
    }
    let mut app = member_actions_with_authored_note();
    app.search = "no-visible-match".into();
    app.workspace_reconcile();
    assert!(!app.workspace_selection_visible());
    for ch in ['y', 'a'] {
        assert_eq!(key(&mut app, KeyCode::Char(ch)), Effect::None);
        assert!(app.input.is_none());
        assert!(
            app.notice
                .as_deref()
                .is_some_and(|text| text.contains("visible"))
        );
    }
}

#[test]
fn member_authored_note_keeps_help_footer_and_actions() {
    let mut app = member_actions_with_authored_note();
    let footer = paint(&app, 160, 36).join("\n");
    assert!(footer.contains("y copy"), "{footer}");
    assert!(footer.contains("a note"), "{footer}");
    let help = crate::board::help::model(&app);
    let annotate = help.sections[0]
        .entries
        .iter()
        .find(|entry| entry.keys == "a")
        .unwrap();
    assert_eq!(
        annotate.description,
        "write a note to the selected agent's lead"
    );
    app.context_menu();
    let entries = &app.menu.as_ref().unwrap().entries;
    for (key, verb) in [
        ("y", crate::action::Verb::Copy),
        ("a", crate::action::Verb::Annotate),
    ] {
        assert!(entries.iter().any(|entry| entry.key == key
            && matches!(&entry.choice, Choice::Action(action) if action.verb == verb)));
    }
    app.menu = None;
    key(&mut app, KeyCode::Char('s'));
    assert!(app.discovery.is_some());
    assert!(app.input.is_none());
}

fn reference_inventory(app: &mut App) {
    app.references = crate::board::inventory::Inventory::from_listed(
        &[crate::squad::Squad {
            name: "tmt-ux".into(),
            room_id: ROOM.into(),
        }],
        &Ok(json!({"identities":[
            {"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","name":"ux-1"},
            {"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","name":"tmt-ux-lead"}
        ]})),
    );
}

#[test]
fn workspace_member_squad_and_note_inputs_share_fuzzy_completion_without_retargeting() {
    for surface in ["member", "squad", "note"] {
        let mut app = member_actions_with_authored_note();
        reference_inventory(&mut app);
        if surface == "squad" {
            app.shell.tab = OpsTab::Agents;
            app.workspace.filter = crate::board::workspace::AgentsFilter::All;
            app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
            app.workspace_reconcile();
        }
        assert!(app.workspace_selection_visible(), "{surface}");
        app.perform(
            &Action::parse(if surface == "note" {
                "annotate lead"
            } else {
                "talk"
            })
            .unwrap(),
        );
        let compose = app.input.as_ref().expect(surface).compose.clone();
        for ch in "@ux1".chars() {
            assert_eq!(key(&mut app, KeyCode::Char(ch)), Effect::None);
        }
        assert_eq!(
            app.mentions.completion.matches[0].label, "ux-1",
            "{surface}"
        );
        assert_eq!(key(&mut app, KeyCode::Tab), Effect::None);
        let input = app.input.as_ref().unwrap();
        assert_eq!(input.text.as_str(), "@ux-1 ", "{surface}");
        assert_eq!(input.compose, compose, "{surface}");
    }
}

#[test]
#[ignore = "explicit member-action look packet, never writes frozen fixtures"]
fn capture_member_actions_packet() {
    let output = std::env::var_os("TMT_MEMBER_ACTIONS_PACKET").expect("task-owned packet path");
    let _no_commands = crate::runner::forbid_commands();
    let mut frames = Vec::new();
    for (base, depth) in [
        ("tmt", tmt_cli_style::Depth::TrueColor),
        ("tmt-light", tmt_cli_style::Depth::TrueColor),
        ("tmt", tmt_cli_style::Depth::None),
    ] {
        for width in [160, 100, 60] {
            for scene in ["copy", "note", "pool", "completion"] {
                let mut app = member_actions_with_authored_note();
                reference_inventory(&mut app);
                let look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
                    depth,
                };
                let view = app.view.as_mut().unwrap();
                view.look = look;
                for scoped in view
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .squad_looks
                    .values_mut()
                {
                    *scoped = look;
                }
                assert_eq!(app.look().depth, depth);
                assert_eq!(app.look().theme.base.name(), base);
                match scene {
                    "copy" => {
                        assert!(matches!(
                            key(&mut app, KeyCode::Char('y')),
                            Effect::Act(Request::Copy { .. })
                        ));
                        // Inject the existing worker's successful result; no host clipboard access.
                        app.finished(Ok("Copied".into()));
                    }
                    "note" | "completion" => {
                        key(&mut app, KeyCode::Char('a'));
                        assert!(
                            matches!(app.input.as_ref().map(|input| &input.compose), Some(Compose::Annotate { to, row }) if to == "product-lead" && row == "worker")
                        );
                        app.paste(if scene == "completion" {
                            "Please ask @tul"
                        } else {
                            "Please review this member's task."
                        });
                    }
                    "pool" => {
                        let snapshot =
                            &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                        snapshot.roster.squads[0].members.retain(|id| id != MEMBER);
                        let target = OpsTarget::Member {
                            room_id: None,
                            identity_id: MEMBER.into(),
                        };
                        snapshot.rows.insert(
                            target.clone(),
                            json!({"id":MEMBER,"name":"worker","state":"working"}),
                        );
                        app.workspace.pool_expanded = true;
                        app.workspace.selected = Some(target);
                        app.workspace_reconcile();
                        key(&mut app, KeyCode::Char('a'));
                        assert!(app.input.is_none());
                        assert!(app.notice.as_deref().unwrap().contains("squad context"));
                    }
                    _ => unreachable!(),
                }
                let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
                terminal
                    .draw(|frame| {
                        crate::board::view::prepare(&mut app, frame.area());
                        crate::board::view::render(frame, &app);
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                frames.push(json!({"scene":scene,"width":width,"height":36,"base":base,"depth":format!("{depth:?}"),
                    "lines":buffer.content.chunks(usize::from(width)).map(|row|row.iter().map(|cell|cell.symbol()).collect::<String>()).collect::<Vec<_>>(),
                    "cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>()
                }));
            }
        }
    }
    std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
}

#[test]
fn agents_shortcuts_right_click_rejects_stale_loading_overlay_and_empty_hits() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for state in ["stale", "loading", "overlay", "empty", "search"] {
        let mut app = member_actions_with_lead();
        app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
        paint(&app, 160, 40);
        let hit = app
            .hits
            .borrow()
            .iter()
            .copied()
            .find(|hit| hit.row().is_some())
            .unwrap();
        let selected = app.workspace.selected.clone();
        let mut event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        };
        match state {
            "stale" => app.invalidate_frame_geometry(),
            "loading" => app.current = Some("new-loading-view".into()),
            "overlay" => app.help = true,
            "empty" => {
                event.column = 159;
                event.row = 39;
            }
            "search" => app.searching = true,
            _ => unreachable!(),
        }
        assert_eq!(app.mouse(event, Instant::now()), Effect::None, "{state}");
        assert_eq!(app.workspace.selected, selected, "{state}");
        assert!(app.menu.is_none(), "{state}");
    }
}

#[test]
fn agents_shortcuts_keyboard_settings_reaches_named_unbound_member_actions() {
    let mut app = board();
    key(&mut app, KeyCode::Char(','));
    let actions = app
        .menu
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|entry| entry.label == "Actions")
        .unwrap();
    app.menu.as_mut().unwrap().selected = actions;
    key(&mut app, KeyCode::Enter);
    let entries = &app.menu.as_ref().unwrap().entries;
    for ui in [OpsUiAction::RenameMember, OpsUiAction::RemoveMember] {
        let entry = entries.iter().find(|entry| matches!(&entry.choice, Choice::Action(action) if action.verb == Verb::Ui(ui))).unwrap();
        assert_eq!(entry.label, ui.menu_label().unwrap());
        assert!(entry.key.is_empty());
    }
    app.menu = None;
    key(&mut app, KeyCode::Char('m'));
    assert_eq!(app.workspace_form.as_ref().unwrap().kind, Kind::Assign);
}

#[test]
fn agents_shortcuts_strip_labels_remain_disabled_and_have_no_click_targets() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for state in ["loading", "overlay", "search", "input"] {
        let mut app = configured_member(
            "[bind]\nf='copy'\nF='agents-filter'\nM='copy'\nt='talk-measure'\n",
            json!({"id":MEMBER,"name":"worker"}),
        );
        paint(&app, 160, 40);
        let target = app
            .workspace_strip_hits
            .borrow()
            .iter()
            .find_map(|(area, choice)| matches!(choice, Choice::RightTab(_)).then_some(*area))
            .unwrap();
        match state {
            "loading" => app.current = Some("new-loading-view".into()),
            "overlay" => app.help = true,
            "search" => app.searching = true,
            "input" => {
                key(&mut app, KeyCode::Enter);
                assert!(app.input.is_some());
            }
            _ => unreachable!(),
        }
        let text = paint(&app, 160, 40).join("\n");
        assert_eq!(
            crate::board::keycap::key(&app, OpsUiAction::AgentsFilter, "f").as_deref(),
            Some("F"),
            "{state}"
        );
        assert_eq!(
            crate::board::keycap::key(&app, OpsUiAction::TalkMeasure, "M").as_deref(),
            Some("t"),
            "{state}"
        );
        if state != "overlay" {
            assert!(text.contains("F  Squads"), "{state}: {text}");
            assert!(text.contains("t  Talk"), "{state}: {text}");
        }
        let disabled =
            crate::board::keycap::control_span(&app, "w", OpsUiAction::WorkspaceSnapshots).style;
        for (key, ui) in [
            ("F", OpsUiAction::AgentsFilter),
            ("t", OpsUiAction::TalkMeasure),
        ] {
            assert_eq!(
                crate::board::keycap::control_span(&app, key, ui).style,
                disabled,
                "{state}"
            );
        }
        assert!(
            !app.workspace_strip_hits
                .borrow()
                .iter()
                .any(|(_, choice)| matches!(choice, Choice::RightTab(_) | Choice::AgentsFilter(_))),
            "{state}"
        );
        let tab = app.right_tab;
        let filter = app.workspace.filter;
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: target.x,
                row: target.y,
                modifiers: KeyModifiers::NONE,
            },
            Instant::now(),
        );
        assert_eq!(app.right_tab, tab, "{state}");
        assert_eq!(app.workspace.filter, filter, "{state}");
    }
}

#[test]
fn agents_shortcuts_reader_keeps_configured_labels_and_unbound_controls_honest() {
    for (config, expected) in [
        ("", Some(("f", "M"))),
        (
            "[bind]\nf='help'\nM='copy'\nF='agents-filter'\nt='talk-measure'\n",
            Some(("F", "t")),
        ),
        ("[bind]\nf='help'\nM='copy'\n", None),
    ] {
        let mut app = configured_member(config, json!({"id":MEMBER,"name":"worker"}));
        paint(&app, 160, 40);
        app.focus = app
            .focus_panes()
            .iter()
            .position(|pane| *pane == Pane::Replies)
            .unwrap();
        assert!(app.workspace_scope().is_reader());
        let text = paint(&app, 160, 40).join("\n");
        let f = crate::board::keycap::key(&app, OpsUiAction::AgentsFilter, "f");
        let m = crate::board::keycap::key(&app, OpsUiAction::TalkMeasure, "M");
        assert_eq!(f.as_deref(), expected.map(|keys| keys.0));
        assert_eq!(m.as_deref(), expected.map(|keys| keys.1));
        if let Some((f, m)) = expected {
            assert!(text.contains(&format!("{f}  Squads")), "{text}");
            assert!(text.contains(&format!("{m}  Talk")), "{text}");
            let help = crate::board::help::model(&app);
            for key in [f, m] {
                assert!(
                    help.sections
                        .iter()
                        .any(|section| section.entries.iter().any(|entry| entry.keys == key)),
                    "{key}"
                );
            }
        }
        let before = app.workspace.filter;
        app.perform(&Action::parse("agents-filter").unwrap());
        assert_eq!(
            app.workspace.filter, before,
            "Reader filter shortcut remains ineligible"
        );
    }
}

#[test]
fn agents_shortcuts_named_menu_choices_enter_the_existing_frozen_uuid_reviews() {
    for (ui, kind) in [
        (OpsUiAction::RenameMember, Kind::RenameMember),
        (OpsUiAction::RemoveMember, Kind::RemoveMember),
    ] {
        let mut app = board();
        app.context_menu();
        let choice = app
            .menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find_map(|entry| {
                matches!(&entry.choice,Choice::Action(action) if action.verb==Verb::Ui(ui))
                    .then(|| entry.choice.clone())
            })
            .unwrap();
        app.menu = None;
        app.choose(choice);
        assert_eq!(app.workspace_form.as_ref().unwrap().kind, kind);
        assert_eq!(
            app.workspace.selected,
            Some(OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: MEMBER.into()
            })
        );
    }
}
