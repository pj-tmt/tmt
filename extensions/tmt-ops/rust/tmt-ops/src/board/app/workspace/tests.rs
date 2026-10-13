use super::*;
use crate::board::workspace::{
    OpsMember, OpsObservation, OpsRoster, OpsSquad,
    manage::{Kind, Stage},
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use serde_json::json;

const ROOM: &str = "11111111-1111-4111-8111-111111111111";
const MEMBER: &str = "22222222-2222-4222-8222-222222222222";

fn board() -> App {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut loaded = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    loaded.tabs.push(crate::board::ALL.into());
    let mut workspace = WorkspaceSnapshot {
        roster: OpsRoster {
            members: [(
                MEMBER.into(),
                OpsMember {
                    id: MEMBER.into(),
                    name: "worker".into(),
                    state: Some("working".into()),
                    waiting: false,
                    presence: Some(true),
                    usage: None,
                },
            )]
            .into(),
            squads: vec![OpsSquad {
                room_id: ROOM.into(),
                name: "product".into(),
                members: vec![MEMBER.into()],
                lead: None,
                member_states: [(
                    MEMBER.into(),
                    crate::board::workspace::OpsMembershipState {
                        state: Some("working".into()),
                        waiting: false,
                        rank: None,
                    },
                )]
                .into(),
                observation: OpsObservation::Complete,
            }],
            membership_complete: true,
            observation: OpsObservation::Complete,
        },
        rows: [
            (OpsTarget::Pool, json!({"name":"Common pool","kind":"pool"})),
            (
                OpsTarget::Squad(ROOM.into()),
                json!({"name":"product","roomId":ROOM,"revision":2,"memberIds":[MEMBER]}),
            ),
            (
                OpsTarget::Member {
                    room_id: Some(ROOM.into()),
                    identity_id: MEMBER.into(),
                },
                json!({"id":MEMBER,"name":"worker","state":"working"}),
            ),
        ]
        .into(),
        ..Default::default()
    };
    let path = std::env::temp_dir().join(format!(
        "ops-workspace-test-{}-{}.toml",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path, "").unwrap();
    let config = Config::read(path.clone()).unwrap();
    workspace.configure(&config).unwrap();
    std::fs::remove_file(path).unwrap();
    loaded.view.as_mut().unwrap().home = Some(crate::board::home::Home {
        workspace,
        summary: crate::board::home::Counts {
            members: 1,
            working: 1,
            ..Default::default()
        },
        windows: crate::config::TokenWindow::DEFAULTS,
        sections: vec![],
        squads: vec![],
        failures: vec![],
        incomplete: false,
    });
    loaded.view.as_mut().unwrap().me = Some("Owner".into());
    loaded.view.as_mut().unwrap().me_id = Some("33333333-3333-4333-8333-333333333333".into());
    let mut app = App::new(Some(crate::board::ALL.into()));
    app.apply(loaded);
    app.shell.tab = OpsTab::Agents;
    app.workspace.expanded.insert(ROOM.into());
    app.workspace.selected = Some(OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: MEMBER.into(),
    });
    app.workspace_reconcile();
    app
}
// Literal observed fixtures for captures and the real-render cost harness.
fn populated_board(squads: usize, agents: usize) -> App {
    let mut app = board();
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.members.clear();
    home.workspace.roster.squads.clear();
    home.workspace.rows.retain(|key, _| *key == OpsTarget::Pool);
    home.workspace.squad_looks.clear();
    for ordinal in 0..squads {
        let id = format!("55555555-5555-4555-8555-{ordinal:012}");
        let name = format!("squad-{ordinal:02}");
        home.workspace.rows.insert(
            OpsTarget::Squad(id.clone()),
            json!({"name":name,"roomId":id,"revision":1}),
        );
        home.workspace.roster.squads.push(OpsSquad {
            room_id: id,
            name,
            members: vec![],
            lead: None,
            member_states: Default::default(),
            observation: OpsObservation::Complete,
        });
    }
    for ordinal in 0..agents {
        let id = format!("66666666-6666-4666-8666-{ordinal:012}");
        let name = format!("agent-{ordinal:05}");
        let state = if ordinal == 0 { "blocked" } else { "working" };
        let member = OpsMember {
            id: id.clone(),
            name: name.clone(),
            state: Some(state.into()),
            waiting: false,
            presence: Some(true),
            usage: None,
        };
        home.workspace.roster.members.insert(id.clone(), member);
        let room = if squads == 0 {
            None
        } else {
            let squad = &mut home.workspace.roster.squads[ordinal % squads];
            squad.members.push(id.clone());
            squad.member_states.insert(
                id.clone(),
                crate::board::workspace::OpsMembershipState {
                    state: Some(state.into()),
                    waiting: false,
                    rank: None,
                },
            );
            if ordinal < squads {
                squad.lead = Some(id.clone());
            }
            Some(squad.room_id.clone())
        };
        home.workspace.rows.insert(OpsTarget::Member {room_id:room,identity_id:id.clone()},json!({"id":id,"name":name,"state":state,"task":"Bounded observed task","presence":"active"}));
    }
    home.summary.members = agents;
    home.summary.blocked = usize::from(agents > 0);
    home.summary.working = agents.saturating_sub(1);
    app.workspace.expanded = home
        .workspace
        .roster
        .squads
        .iter()
        .map(|squad| squad.room_id.clone())
        .collect();
    app.workspace.pool_expanded = true;
    app.workspace.selected = Some(OpsTarget::Pool);
    app.workspace_reconcile();
    app
}
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn paint(app: &App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}
#[test]
fn removal_review_is_scrollable_frozen_and_cancel_is_the_initial_action() {
    let mut app = board();
    assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
    let screen = paint(&app, 80, 24).join("\n");
    assert!(screen.contains("Source timers to pause: unknown"));
    assert!(screen.contains("identity, process, history"));
    assert!(screen.contains("Apply this exact operation"));
    let frozen = app.workspace_form.as_ref().unwrap().operation.clone();
    app.workspace.selected = Some(OpsTarget::Pool);
    assert_eq!(app.workspace_form.as_ref().unwrap().operation, frozen);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(
        app.workspace_form.is_none(),
        "Enter initially cancels, never applies a row below"
    );
}
#[test]
fn confirmation_requires_current_geometry_and_does_not_replay_while_pending() {
    let mut app = board();
    app.workspace_begin(Kind::Remove);
    assert_eq!(app.workspace_confirm(), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
    paint(&app, 80, 24);
    key(&mut app, KeyCode::Tab); // Cancel -> read
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Tab); // read -> apply
    let effect = key(&mut app, KeyCode::Enter);
    assert!(
        matches!(effect,Effect::Act(Request::WorkspaceOperation(crate::membership::workspace::Operation::Remove {room,member})) if room.id == ROOM && member == MEMBER)
    );
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Pending);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.workspace_confirm(), Effect::None);
}
#[test]
fn resized_or_clipped_confirmation_has_no_stale_pointer_or_key_authority() {
    let mut app = board();
    app.workspace_begin(Kind::Remove);
    paint(&app, 80, 24);
    let apply = app.workspace_form.as_ref().unwrap().controls.get()[1].unwrap();
    app.invalidate_overlay_frames();
    let event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: apply.x,
        row: apply.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.overlay_event(&event), Some(Effect::None));
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
    paint(&app, 20, 5);
    assert_eq!(app.workspace_confirm(), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
    key(&mut app, KeyCode::Esc);
    assert!(app.workspace_form.is_none());
    assert_eq!(
        app.workspace.selected,
        Some(OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into()
        })
    );
}
#[test]
fn short_terminal_review_uses_the_screen_and_scrolls_the_frozen_operation() {
    let mut app = board();
    app.workspace_begin(Kind::Remove);
    let first = paint(&app, 60, 8).join("\n");
    assert!(first.contains("Remove membership"));
    assert!(first.contains("From squad"), "{first}");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Down);
    let next = paint(&app, 60, 8).join("\n");
    assert!(next.contains(ROOM), "{next}");
    key(&mut app, KeyCode::End);
    let last = paint(&app, 60, 8).join("\n");
    let reflowed = last
        .lines()
        .map(|line| line.trim().trim_matches('│').trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(reflowed.contains("only explicit Resume does."), "{last}");
    assert!(last.contains("Read"));
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Review);
    key(&mut app, KeyCode::Esc);
    assert!(app.workspace_form.is_none());
}
#[test]
fn form_digits_and_search_own_input_before_shell_shortcuts() {
    let mut app = board();
    app.workspace_begin(Kind::Create);
    key(&mut app, KeyCode::Char('1'));
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "12");
    assert_eq!(app.shell.tab, OpsTab::Agents);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Char('/'));
    key(&mut app, KeyCode::Char('3'));
    assert_eq!(app.shell.tab, OpsTab::Agents);
}
#[test]
fn hidden_or_removed_selection_cannot_redirect_an_action_to_an_adjacent_member() {
    let mut app = board();
    app.search = "no-match".into();
    app.workspace_reconcile();
    let target = app.workspace.selected.clone();
    assert_eq!(app.workspace_selection_state(), SelectionState::Hidden);
    assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
    assert!(app.workspace_form.is_none());
    app.search.clear();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads[0]
        .members
        .clear();
    app.workspace_reconcile();
    assert_eq!(app.workspace_selection_state(), SelectionState::Missing);
    assert_eq!(app.workspace.selected, target);
    assert!(app.selected_row().is_none());
}
#[test]
fn partial_item_result_survives_as_readable_evidence_with_no_automatic_retry() {
    let mut app = board();
    app.workspace_begin(Kind::Remove);
    app.finished_workspace(Ok(crate::membership::Outcome {complete: false,document: json!({"results":[{"name":"one","added":true},{"name":"two","error":{"code":"OFFLINE","message":"not confirmed"}}],"repair":"tmt ops squad rename product next --repair"})}));
    let Stage::Result(text) = &app.workspace_form.as_ref().unwrap().stage else {
        panic!("result expected")
    };
    assert!(text.contains("Applied: one"));
    assert!(text.contains("Unknown or not applied: two"));
    assert!(text.contains("rename product next --repair"));
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.workspace_form.is_some());
}
#[test]
fn squad_controls_are_clickable_and_stale_controls_never_retarget() {
    let mut app = board();
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    app.workspace_reconcile();
    paint(&app, 160, 36);
    let (area, _, _) = app
        .workspace_action_hits
        .borrow()
        .iter()
        .find(|(_, _, choice)| matches!(choice, Choice::WorkspaceBegin(Kind::Add)))
        .unwrap()
        .clone();
    let event = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    };
    app.workspace.selected = Some(OpsTarget::Pool);
    assert_eq!(app.mouse(event, Instant::now()), Effect::None);
    assert!(app.workspace_form.is_none());
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    paint(&app, 160, 36);
    assert_eq!(app.mouse(event, Instant::now()), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().kind, Kind::Add);
}
#[test]
fn no_squads_is_a_known_empty_workspace_with_an_explicit_creation_action() {
    let mut app = board();
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.members.clear();
    home.workspace.roster.squads.clear();
    home.workspace
        .rows
        .retain(|target, _| *target == OpsTarget::Pool);
    app.workspace.selected = Some(OpsTarget::Pool);
    app.shell.tab = OpsTab::Overview;
    app.workspace_reconcile();
    assert_eq!(app.workspace_rows()[0].count, Some(0));
    assert_eq!(key(&mut app, KeyCode::Char('n')), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().kind, Kind::Create);
}
#[test]
fn selected_member_host_uses_the_frozen_uuid_and_overview_has_membership_actions() {
    let mut app = board();
    app.shell.tab = OpsTab::Overview;
    assert_eq!(
        key(&mut app, KeyCode::Char('j')),
        Effect::Act(Request::Jump(MEMBER.into()))
    );
    assert_eq!(key(&mut app, KeyCode::Char('m')), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().kind, Kind::Assign);
}
#[test]
fn supplied_label_role_and_choice_hit_survive_the_workspace_tree_and_digest_tab() {
    use crate::labels::{Label, Supplied};
    use tmt_cli_style::Role;
    let mut app = board();
    let choose = crate::labels::Choose::parse("digest", &json!({"kind":"choose","argv":["digest","mode","{value}","--identity",MEMBER],"current":"auto","options":[{"value":"auto","label":"Auto"}]})).unwrap();
    app.labels = Supplied::from_rows(vec![(
        "digest".into(),
        [(
            MEMBER.into(),
            vec![
                Label {
                    text: "Auto".into(),
                    role: Role::Text,
                    action: Some(choose),
                },
                Label {
                    text: "1 held".into(),
                    role: Role::Muted,
                    action: None,
                },
            ],
        )]
        .into(),
    )]);
    for tab in [OpsTab::Overview, OpsTab::Agents, OpsTab::Digest] {
        app.shell.tab = tab;
        let screen = paint(&app, 100, 36).join("\n");
        assert!(screen.contains("digest Auto ▾"), "{tab:?}: {screen}");
        assert!(screen.contains("1 held"));
        let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let held = buffer
            .content
            .windows(6)
            .find(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>() == "1 held")
            .unwrap();
        assert_eq!(
            held[0].fg,
            app.look()
                .role(Role::Text)
                .fg
                .unwrap_or(ratatui::style::Color::Reset),
            "selection contrast policy keeps words readable"
        );
        let (area, _) = app.chip_hits.borrow().first().unwrap().to_owned();
        assert!(matches!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE
                },
                Instant::now()
            ),
            Effect::Act(Request::DigestOpen(_))
        ));
    }
}
#[test]
fn in_place_reload_keeps_shell_expansion_and_exact_selection_without_new_recipient_authority() {
    let mut app = board();
    app.shell.tab = OpsTab::Digest;
    app.workspace.pool_expanded = true;
    let resume = app.resume().unwrap();
    let mut replacement = board();
    replacement.shell.tab = OpsTab::Overview;
    replacement.workspace.expanded.clear();
    replacement.workspace.pool_expanded = false;
    replacement.workspace.selected = Some(OpsTarget::Pool);
    replacement.carry(resume);
    replacement.restore_pending();
    assert_eq!(replacement.shell.tab, OpsTab::Digest);
    assert!(replacement.workspace.expanded.contains(ROOM));
    assert!(replacement.workspace.pool_expanded);
    assert_eq!(replacement.workspace.selected, app.workspace.selected);
    let retained = replacement.resume().unwrap();
    let home = replacement.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.squads[0].members.clear();
    replacement.carry(retained);
    replacement.restore_pending();
    assert_eq!(
        replacement.workspace_selection_state(),
        SelectionState::Missing
    );
    assert_eq!(replacement.workspace.selected, app.workspace.selected);
    assert!(replacement.selected_row().is_none());
}
#[test]
fn repeated_create_member_fields_keep_reference_completion_eligibility() {
    let mut app = board();
    app.workspace_begin(Kind::Create);
    app.workspace_submit_field("next".into());
    app.workspace_submit_field("".into());
    app.workspace_submit_field("worker".into());
    assert!(app.workspace_reference_field());
    app.workspace_submit_field("".into());
    let draft = app.workspace_form.as_ref().unwrap();
    assert_eq!(draft.stage, Stage::Review);
    assert!(
        matches!(&draft.operation,Some(crate::membership::workspace::Operation::Create {members,..}) if members == &[MEMBER.to_owned()])
    );
}

#[test]
fn squad_theme_scope_is_observed_in_agents_and_digest_while_overview_keeps_the_shared_theme() {
    let mut app = board();
    let look = crate::look::Look {
        theme: tmt_cli_style::Theme::new(tmt_cli_style::theme::Base::parse("tmt-light").unwrap()),
        depth: tmt_cli_style::Depth::TrueColor,
    };
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .insert(ROOM.into(), look);
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        app.shell.tab = tab;
        assert_eq!(app.look().theme.base.name(), "tmt-light");
        assert_eq!(app.workspace_squad().unwrap().name, "product");
    }
    app.shell.tab = OpsTab::Overview;
    assert_ne!(app.look().theme.base.name(), "tmt-light");
    assert!(app.workspace_squad().is_none());
}

#[test]
fn pool_annotation_refuses_before_opening_the_shared_composer() {
    let mut app = board();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.squads[0].members.clear();
    let target = OpsTarget::Member {
        room_id: None,
        identity_id: MEMBER.into(),
    };
    snapshot.rows.insert(
        target.clone(),
        json!({"id":MEMBER,"name":"worker","pending":"decision"}),
    );
    app.workspace.pool_expanded = true;
    app.workspace.selected = Some(target.clone());
    app.workspace_reconcile();
    let send = app.send_target(RowTarget::Workspace(target), true).unwrap();
    assert_eq!(
        app.compose_row(send, crate::action::Verb::Annotate, String::new()),
        Effect::None
    );
    assert!(app.input.is_none());
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("Notes require a squad context")
    );
}

/// Explicit, read-only evidence capture. This never rewrites a frozen fixture
/// and never calls Core, tmux, the filesystem-backed services or the action lane.
#[test]
#[ignore = "explicit WP2 workspace capture packet"]
fn capture_workspace_packet() {
    let output = std::env::var("TMT_WORKSPACE_CAPTURE").expect("task-owned evidence path");
    let mut captures = Vec::new();
    for scene in [
        "overview",
        "agents",
        "digest",
        "empty",
        "pool",
        "dense",
        "squads-eight",
        "squads-thirty-six",
        "partial",
        "stale",
        "help",
        "review",
        "result",
        "conversation",
    ] {
        for width in [160, 100, 80, 60] {
            for height in [36, 8] {
                for (base, depth) in [
                    ("tmt", tmt_cli_style::Depth::TrueColor),
                    ("tmt-light", tmt_cli_style::Depth::TrueColor),
                    ("terminal", tmt_cli_style::Depth::Ansi16),
                    ("tmt", tmt_cli_style::Depth::None),
                ] {
                    let mut app = match scene {
                        "squads-eight" => populated_board(8, 150),
                        "squads-thirty-six" => populated_board(36, 150),
                        _ => board(),
                    };
                    let look = crate::look::Look {
                        theme: tmt_cli_style::Theme::new(
                            tmt_cli_style::theme::Base::parse(base).unwrap(),
                        ),
                        depth,
                    };
                    let view = app.view.as_mut().unwrap();
                    view.look = look;
                    let home = view.home.as_mut().unwrap();
                    for squad in &home.workspace.roster.squads {
                        home.workspace
                            .squad_looks
                            .insert(squad.room_id.clone(), look);
                    }
                    match scene {
                        "overview" => app.shell.tab = OpsTab::Overview,
                        "digest" => app.shell.tab = OpsTab::Digest,
                        "empty" => {
                            home.summary = Default::default();
                            home.workspace.roster.members.clear();
                            home.workspace.roster.squads.clear();
                            home.workspace.rows.retain(|key, _| *key == OpsTarget::Pool);
                            app.workspace.selected = Some(OpsTarget::Pool);
                        }
                        "pool" => {
                            home.workspace.roster.squads[0].members.clear();
                            home.workspace.rows.insert(
                                OpsTarget::Member {
                                    room_id: None,
                                    identity_id: MEMBER.into(),
                                },
                                json!({"id":MEMBER,"name":"worker","state":"working"}),
                            );
                            app.workspace.pool_expanded = true;
                            app.workspace.selected = Some(OpsTarget::Member {
                                room_id: None,
                                identity_id: MEMBER.into(),
                            });
                        }
                        "dense" => {
                            for ordinal in 1..=150 {
                                let id = format!("44444444-4444-4444-8444-{ordinal:012}");
                                let name = format!("worker-{ordinal:03}");
                                home.workspace.roster.members.insert(
                                    id.clone(),
                                    OpsMember {
                                        id: id.clone(),
                                        name: name.clone(),
                                        state: Some("idle".into()),
                                        waiting: false,
                                        presence: Some(true),
                                        usage: None,
                                    },
                                );
                                home.workspace.roster.squads[0].members.push(id.clone());
                                home.workspace.roster.squads[0].member_states.insert(
                                    id.clone(),
                                    crate::board::workspace::OpsMembershipState {
                                        state: Some("idle".into()),
                                        waiting: false,
                                        rank: None,
                                    },
                                );
                                home.workspace.rows.insert(
                                    OpsTarget::Member {
                                        room_id: Some(ROOM.into()),
                                        identity_id: id.clone(),
                                    },
                                    json!({"id":id,"name":name,"state":"idle"}),
                                );
                            }
                            home.summary.members = 151;
                            home.summary.idle = 150;
                            app.workspace.selected = Some(OpsTarget::Member {
                                room_id: Some(ROOM.into()),
                                identity_id: "44444444-4444-4444-8444-000000000150".into(),
                            });
                        }
                        "partial" => {
                            home.workspace.roster.observation = OpsObservation::Partial;
                            home.workspace.roster.membership_complete = false;
                            home.workspace.roster.squads[0].observation = OpsObservation::Partial;
                            app.shell.tab = OpsTab::Overview;
                        }
                        "stale" => {
                            home.workspace.roster.observation = OpsObservation::Stale;
                            app.error = Some("Roster read failed; retained data is stale".into());
                        }
                        _ => {}
                    }
                    app.workspace_reconcile();
                    match scene {
                        "help" => {
                            key(&mut app, KeyCode::Char('?'));
                        }
                        "review" | "result" => {
                            app.workspace_begin(Kind::Remove);
                            if scene == "result" {
                                app.workspace_form.as_mut().unwrap().stage = Stage::Result("Source timers paused. Membership removal was not confirmed. Refresh before another operation; no automatic retry.".into());
                            }
                        }
                        "conversation" => {
                            let target = app.workspace.selected.clone().unwrap();
                            let send = app
                                .send_target(RowTarget::Workspace(target), false)
                                .unwrap();
                            app.compose_row(send, crate::action::Verb::Talk, "product".into());
                            app.input.as_mut().unwrap().text = "Draft remains with the exact recipient while the workspace resizes.".into();
                        }
                        _ => {}
                    }
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| crate::board::view::render(frame, &app))
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let cells = buffer.content.iter().map(|cell| json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                    let lines = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect::<Vec<_>>();
                    captures.push(json!({"scene":scene,"width":width,"height":height,"base":base,"depth":format!("{depth:?}"),"cells":cells,"lines":lines,"hits":format!("{:?}",app.hits.borrow()),"actionHits":format!("{:?}",app.workspace_action_hits.borrow()),"selected":format!("{:?}",app.workspace.selected),"selectionState":format!("{:?}",app.workspace_selection_state()),"focus":format!("{:?}",app.focused_pane()),"inputBand":format!("{:?}",app.input_band.get())}));
                }
            }
        }
    }
    assert_eq!(captures.len(), 448);
    std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
}

#[test]
fn overview_never_claims_zero_from_incomplete_rosters_and_reports_the_global_observed_states() {
    for squads in [0, 8, 36] {
        let mut app = populated_board(squads, 150);
        app.shell.tab = OpsTab::Overview;
        let complete = paint(&app, 160, 36).join("\n");
        assert!(complete.contains("149 working"), "{complete}");
        assert!(complete.contains("1 blocked"), "{complete}");
        let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
        home.workspace.roster.membership_complete = false;
        home.workspace.roster.observation = OpsObservation::Partial;
        let partial = paint(&app, 100, 36).join("\n");
        assert!(partial.contains("totals unknown"), "{partial}");
        assert!(!partial.contains("Nothing needs you"));
        app.workspace.selected = Some(OpsTarget::Pool);
        assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
        assert!(app.workspace_form.is_none());
    }
}

/// The actual App/render/diff path, with no parallel renderer. Matched baseline
/// and candidate runs remain evidence for a reviewer-agreed budget, not CI SLOs.
#[test]
#[ignore = "explicit release workspace performance evidence"]
fn workspace_frame_cost() {
    if cfg!(debug_assertions) {
        panic!("run this evidence harness with --release");
    }
    for agents in [150, 1_000, 10_000] {
        let mut app = populated_board(36, agents);
        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
        for _ in 0..20 {
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
        }
        for event in ["steady", "scroll", "resize"] {
            let mut samples = Vec::new();
            for index in 0..100 {
                let start = Instant::now();
                match event {
                    "scroll" => {
                        key(&mut app, KeyCode::Down);
                    }
                    "resize" => {
                        terminal
                            .resize(Rect::new(0, 0, if index % 2 == 0 { 100 } else { 160 }, 50))
                            .unwrap();
                    }
                    _ => {}
                }
                terminal
                    .draw(|frame| crate::board::view::render(frame, &app))
                    .unwrap();
                samples.push(start.elapsed().as_micros());
            }
            samples.sort_unstable();
            println!(
                "workspace agents={agents} squads=36 event={event} p50={}us p95={}us cells={}",
                samples[50],
                samples[95],
                terminal.backend().buffer().content.len()
            );
        }
    }
}

#[test]
fn workspace_focus_ignores_retired_home_composition_and_jumps_from_the_tree() {
    let mut app = board();
    app.view.as_mut().unwrap().board = crate::config::Board::simple(
        crate::config::BoardMode::Split,
        crate::config::Direction::LeftRight,
        vec![Pane::Replies],
        &[100],
    );
    paint(&app, 160, 36);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    assert_eq!(
        key(&mut app, KeyCode::Char('j')),
        Effect::Act(Request::Jump(MEMBER.into()))
    );
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    assert_eq!(key(&mut app, KeyCode::Char('j')), Effect::None);
    let original = app.workspace.selected.clone();
    key(&mut app, KeyCode::Char('1'));
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(app.workspace.selected, original);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    key(&mut app, KeyCode::Char('1'));
    paint(&app, 60, 8);
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.focused_pane(),
        Some(Pane::Rows),
        "Overview cannot focus its hidden conversation"
    );
}
