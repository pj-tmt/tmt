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

#[test]
fn member_copy_key_uses_the_selected_row_even_with_conversation_focus() {
    for tab in [OpsTab::Overview, OpsTab::Agents] {
        for reader in [false, true] {
            let mut app = member_actions_with_lead();
            app.shell.tab = tab;
            app.overview_view = crate::board::overview::monitor::View::Agents;
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
    let mut app = member_actions_with_lead();
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
    }) if to == "product-lead" && row == "worker" && text == "keep the recipient explicit")
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
fn member_note_chrome_names_both_recipient_and_subject() {
    for tab in [OpsTab::Overview, OpsTab::Agents] {
        for width in [160, 60] {
            let mut app = member_actions_with_lead();
            app.shell.tab = tab;
            app.overview_view = crate::board::overview::monitor::View::Agents;
            app.perform(&Action::parse("annotate lead").unwrap());
            let lines = paint(&app, width, 36);
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Note to") && line.contains("about worker")),
                "{tab:?} width={width}: {lines:?}"
            );
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
    let mut app = member_actions_with_lead();
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
fn member_factory_scan_moves_to_s_and_actions_include_copy_and_note() {
    let mut app = member_actions_with_lead();
    let footer = paint(&app, 160, 36).join("\n");
    assert!(footer.contains("y copy"), "{footer}");
    assert!(footer.contains("a note to lead"), "{footer}");
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
        let mut app = member_actions_with_lead();
        reference_inventory(&mut app);
        if surface == "squad" {
            app.shell.tab = OpsTab::Overview;
            app.overview_view = crate::board::overview::monitor::View::Squads;
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
                let mut app = member_actions_with_lead();
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
