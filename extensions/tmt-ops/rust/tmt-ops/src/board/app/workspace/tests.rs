use super::*;
use crate::board::workspace::{
    OpsMember, OpsObservation, OpsRoster, OpsSquad,
    manage::{Kind, Stage},
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use serde_json::json;

pub(in crate::board) const ROOM: &str = "11111111-1111-4111-8111-111111111111";
pub(in crate::board) const MEMBER: &str = "22222222-2222-4222-8222-222222222222";

/// Literal attention evidence and the agent-list mode for member-action scenarios.
pub(in crate::board) fn observe_waiting_member(app: &mut App) {
    app.overview_view = crate::board::overview::monitor::View::Agents;
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.members.get_mut(MEMBER).unwrap().waiting = true;
    for squad in &mut snapshot.roster.squads {
        if let Some(member) = squad.member_states.get_mut(MEMBER) {
            member.waiting = true;
        }
    }
    snapshot.shell_counts = snapshot.roster.shell_counts();
    app.overview_rows.borrow_mut().clear();
}

pub(in crate::board) fn board() -> App {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut loaded = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    loaded.tabs.push(crate::board::ALL.into());
    let mut workspace = OpsWorkspaceSnapshot {
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
                json!({"id":MEMBER,"name":"worker","state":"working","presence":"active","pane":{"location":{"host":"tmux"}}}),
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
        window: crate::config::TokenWindow::MINUTE,
        windows: crate::config::TokenWindow::DEFAULTS,
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
    app.invoker_tmux = true;
    app.overview_rows.borrow_mut().clear();
    app
}

/// Acquire effective tables through Config, as the refresh worker does.
fn configured_member(text: &str, row: serde_json::Value) -> App {
    let mut app = board();
    let target = app.workspace.selected.clone().unwrap();
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    workspace.rows.insert(target, row);
    let path = std::env::temp_dir().join(format!(
        "ops-workspace-bind-{}.toml",
        crate::id::new_v4().unwrap()
    ));
    std::fs::write(&path, text).unwrap();
    let config = Config::read(path.clone()).unwrap();
    workspace.configure(&config).unwrap();
    let view = app.view.as_mut().unwrap();
    view.opener = config.program("opener").unwrap();
    view.clipboard = config.program("clipboard").unwrap();
    std::fs::remove_file(path).unwrap();
    app
}

#[test]
fn configured_back_dispatches_once_and_retains_observed_success_or_failure() {
    let mut app = configured_member(
        "[bind]\nbackspace = \"back\"\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    let selected = app.workspace.selected.clone();
    assert_eq!(
        key(&mut app, KeyCode::Backspace),
        Effect::Act(Request::Back)
    );
    let outcomes: [Result<String, String>; 2] = [
        Ok("Nothing to go back to.".into()),
        Err("Pane '%5' was not found.".into()),
    ];
    for outcome in outcomes {
        let text = outcome.as_ref().unwrap_or_else(|error| error).clone();
        app.finished(outcome);
        assert_eq!(app.notice.as_ref(), Some(&text));
        assert_eq!(app.workspace.selected, selected);
    }
}

#[test]
fn configured_link_copy_and_menu_keep_safe_urls_programs_and_current_scoped_keys() {
    let mut app = configured_member(
        r#"opener = ["browser", "--new-tab"]
clipboard = ["clipboard", "--input"]
[bind]
o = "open {link}"
f2 = "copy {name}"
enter = "menu"
"#,
        json!({"id":MEMBER,"name":"worker","fields":{"link":"https://example.test/pr"}}),
    );
    let selected = app.workspace.selected.clone();
    assert_eq!(
        key(&mut app, KeyCode::Char('o')),
        Effect::Act(Request::Open {
            link: "https://example.test/pr".into(),
            opener: Some(vec!["browser".into(), "--new-tab".into()]),
        })
    );
    let copy = Effect::Act(Request::Copy {
        text: "worker".into(),
        program: Some(vec!["clipboard".into(), "--input".into()]),
    });
    assert_eq!(key(&mut app, KeyCode::F(2)), copy);
    let target = selected.as_ref().unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(target)
        .unwrap()["fields"]["link"] = json!("file:///private/prompt");
    assert_eq!(key(&mut app, KeyCode::Char('o')), Effect::None);
    assert!(
        app.notice
            .as_ref()
            .unwrap()
            .contains("Only http(s) links open")
    );
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.title, "worker");
    assert!(
        menu.entries
            .iter()
            .any(|entry| entry.key == "f2" && entry.label == "copy {name}")
    );
    assert!(
        !menu
            .entries
            .iter()
            .any(|entry| ["click", "double-click", "enter"].contains(&entry.key.as_str()))
    );
    assert_eq!(key(&mut app, KeyCode::Char('j')), Effect::None);
    assert!(
        app.menu.is_some(),
        "the menu owns j rather than the Host action"
    );
    assert_eq!(key(&mut app, KeyCode::F(2)), copy);
    assert!(app.menu.is_none());
    key(&mut app, KeyCode::Enter);
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.menu.is_none());
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn configured_refresh_normalizes_control_keys_and_section_wins_without_owning_search() {
    let mut app = configured_member(
        r#"
[bind]
ctrl-r = "copy {name}"
f5 = "refresh"
[[squad.product.section]]
title = "Tasks"
filter = "task"
[squad.product.section.bind]
ctrl-r = "refresh"
"#,
        json!({"id":MEMBER,"name":"worker","fields":{"task":"ship"}}),
    );
    let refresh = KeyEvent::new(
        KeyCode::Char('R'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert_eq!(app.key(refresh), Effect::Refresh);
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .sections
        .clear();
    assert!(
        matches!(app.key(refresh), Effect::Act(Request::Copy { text, .. }) if text == "worker")
    );
    assert_eq!(key(&mut app, KeyCode::F(5)), Effect::Refresh);
    app.searching = true;
    app.search = "worker".into();
    assert_eq!(app.key(refresh), Effect::None);
    assert_eq!(app.search, "worker");
    assert!(app.input.is_none());
}

#[test]
fn configured_section_action_overrides_global_and_missing_fields_refuse_on_selected_uuid() {
    let text = r#"
[bind]
o = "open {link}"
f5 = "refresh"
[[squad.product.section]]
title = "Tasks"
filter = "task"
[squad.product.section.bind]
o = "run code {name} --task={task}"
"#;
    let mut app = configured_member(
        text,
        json!({"id":MEMBER,"name":"worker","fields":{"link":"https://example.com/pull/42","task":"ship"}}),
    );
    assert_eq!(key(&mut app, KeyCode::F(5)), Effect::Refresh);
    assert_eq!(
        key(&mut app, KeyCode::Char('o')),
        Effect::Act(Request::Run(vec![
            "code".into(),
            "worker".into(),
            "--task=ship".into()
        ]))
    );
    let mut app = configured_member(
        text,
        json!({"id":MEMBER,"name":"worker","fields":{"link":"https://example.com/pull/42"}}),
    );
    assert!(
        matches!(key(&mut app, KeyCode::Char('o')), Effect::Act(Request::Open { link, .. }) if link == "https://example.com/pull/42")
    );
    let mut app = configured_member(text, json!({"id":MEMBER,"name":"worker","fields":{}}));
    let selected = app.workspace.selected.clone();
    assert_eq!(key(&mut app, KeyCode::Char('o')), Effect::None);
    assert_eq!(
        app.notice.as_deref(),
        Some("open: link is empty for this row.")
    );
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn configured_host_actions_use_published_coordinates_and_refuse_missing_fields() {
    let text = r#"
[bind]
enter = "run flat {name}"
[bind.tmux]
enter = "run tmux switch-client -t {pane} {host} {session} {window}"
[bind.herdr]
enter = "run herdr pane focus {pane} {session}"
"#;
    for (pane, expected) in [
        (
            json!({"id":"%1","location":{"host":"tmux","session":"work","window":"2"}}),
            Some(vec![
                "tmux",
                "switch-client",
                "-t",
                "%1",
                "tmux",
                "work",
                "2",
            ]),
        ),
        (json!({"id":"%2","location":{"host":"herdr"}}), None),
        (json!({"id":"%3"}), Some(vec!["flat", "worker"])),
        (
            json!({"id":"%4","location":{"host":"other"}}),
            Some(vec!["flat", "worker"]),
        ),
    ] {
        let mut app = configured_member(text, json!({"id":MEMBER,"name":"worker","pane":pane}));
        let selected = app.workspace.selected.clone();
        let effect = key(&mut app, KeyCode::Enter);
        if let Some(args) = expected {
            assert_eq!(
                effect,
                Effect::Act(Request::Run(args.into_iter().map(str::to_owned).collect()))
            );
        } else {
            assert_eq!(effect, Effect::None);
            assert_eq!(
                app.notice.as_deref(),
                Some("run: session is empty for this row.")
            );
        }
        assert_eq!(app.workspace.selected, selected);
    }
}

#[test]
fn enter_with_no_host_table_talks_the_same_for_every_host() {
    // Without any `[bind.<host>]` table, a member row on tmux, Herdr, an unknown
    // host or no observed location resolves Enter through the one global Member
    // table: it opens the composer. No host gets a menu, a silent refusal or a
    // different default because it has no table of its own.
    for pane in [
        json!({"id":"%1","location":{"host":"tmux","session":"work","window":"2"}}),
        json!({"id":"%2","location":{"host":"herdr"}}),
        json!({"id":"%3","location":{"host":"other"}}),
        json!({"id":"%4"}),
    ] {
        let mut app = configured_member("", json!({"id":MEMBER,"name":"worker","pane":pane}));
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None, "{pane}");
        assert!(app.menu.is_none(), "{pane}: Enter must not open a menu");
        assert!(app.notice.is_none(), "{pane}: {:?}", app.notice);
        assert!(app.input.is_some(), "{pane}: Enter must open the composer");
    }
    // A table for another host never changes the default of a host without one.
    let mut app = configured_member(
        "[bind.tmux]\nenter = \"run tmux-only {pane}\"\n",
        json!({"id":MEMBER,"name":"worker","pane":{"id":"%2","location":{"host":"herdr"}}}),
    );
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.menu.is_none() && app.input.is_some());
}

#[test]
fn configured_section_keeps_unoverridden_host_actions_for_the_same_membership() {
    let mut app = configured_member(
        r#"
[bind]
enter = "run flat {name}"
[bind.tmux]
enter = "run host {pane}"
o = "run host-o {pane}"
[[squad.product.section]]
title = "Tasks"
filter = "task"
[squad.product.section.bind]
enter = "run section {name}"
"#,
        json!({"id":MEMBER,"name":"worker","fields":{"task":"ship"},"pane":{"id":"%1","location":{"host":"tmux"}}}),
    );
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Effect::Act(Request::Run(vec!["section".into(), "worker".into()]))
    );
    assert_eq!(
        key(&mut app, KeyCode::Char('o')),
        Effect::Act(Request::Run(vec!["host-o".into(), "%1".into()]))
    );
}
// Literal observed fixtures for captures and the real-render cost harness.
pub(in crate::board) fn populated_board(squads: usize, agents: usize) -> App {
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
    home.workspace.shell_counts = home.workspace.roster.shell_counts();
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

fn painted_member_row(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
        .unwrap();
    let hits = app.hits.borrow();
    let hit = hits
        .iter()
        .find(|hit| hit.row() == Some(index))
        .expect("selected member is painted");
    (hit.x..hit.x + hit.width)
        .map(|x| terminal.backend().buffer()[(x, hit.y)].symbol())
        .collect()
}

#[test]
fn workspace_presence_is_observed_separately_from_member_state_and_missing_data() {
    for state in [None, Some("idle"), Some("working")] {
        for presence in [None, Some(true), Some(false)] {
            let mut app = board();
            let selected = app.workspace.selected.clone().unwrap();
            let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
            let member = snapshot.roster.members.get_mut(MEMBER).unwrap();
            member.state = state.map(str::to_owned);
            member.presence = presence;
            snapshot.roster.squads[0]
                .member_states
                .get_mut(MEMBER)
                .unwrap()
                .state = state.map(str::to_owned);
            snapshot.rows.get_mut(&selected).unwrap()["state"] = json!(state);
            snapshot.shell_counts = snapshot.roster.shell_counts();
            let row = painted_member_row(&app, 100, 24);
            assert!(
                row.contains("worker") && row.contains(state.unwrap_or("unreported")),
                "{row}"
            );
            let counts = &app.workspace_snapshot().unwrap().shell_counts;
            assert_eq!(counts.online, usize::from(presence == Some(true)));
            assert_eq!(counts.complete, presence.is_some());
            let header = &paint(&app, 100, 24)[0];
            assert!(
                header.contains(if presence == Some(true) {
                    "1 online"
                } else if presence == Some(false) {
                    "0 online"
                } else {
                    "? online"
                }),
                "{header}"
            );
            assert_eq!(app.workspace_selection_state(), SelectionState::Visible);
            assert_eq!(app.workspace.selected, Some(selected));
        }
    }
}

#[test]
fn compact_workspace_labels_keep_name_state_and_complete_source_units() {
    use crate::labels::{Label, Supplied};
    let mut app = board();
    app.labels = Supplied::from_rows(vec![(
        "digest".into(),
        [(
            MEMBER.into(),
            vec![
                Label {
                    text: "Auto".into(),
                    role: tmt_cli_style::Role::Text,
                    action: None,
                },
                Label {
                    text: "17 held".into(),
                    role: tmt_cli_style::Role::Muted,
                    action: None,
                },
                Label {
                    text: "tail-extra-whole".into(),
                    role: tmt_cli_style::Role::Muted,
                    action: None,
                },
            ],
        )]
        .into(),
    )]);
    for width in [40, 60, 100] {
        let row = painted_member_row(&app, width, 36);
        assert!(
            row.contains("worker") && row.contains("● working"),
            "{width}: {row}"
        );
        assert_eq!(row.contains("Auto"), width >= 60, "{width}: {row}");
        assert_eq!(
            row.contains("17"),
            row.contains("17 held"),
            "{width}: {row}"
        );
        assert_eq!(
            row.contains("tail"),
            row.contains("tail-extra-whole"),
            "{width}: {row}"
        );
        if width == 40 {
            assert!(!row.contains("digest"), "{row}");
        }
        if width == 100 {
            let conversation = paint(&app, width, 36).join("\n");
            assert!(
                conversation.contains("17 held") && conversation.contains("tail-extra-whole"),
                "{conversation}"
            );
        }
    }
}

#[test]
fn workspace_rows_wheel_preserves_uuid_and_keyboard_focus_until_explicit_navigation() {
    for focus in [0, 1] {
        let mut app = populated_board(1, 100);
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .layout = crate::config::OpsLayout::Side;
        app.select(80);
        app.focus = focus;
        paint(&app, 160, 36);
        let selected = app.workspace.selected.clone();
        let keyboard = app.focused_pane();
        let rows = app.scrolls.offset(Pane::Rows);
        let replies = app.scrolls.offset(Pane::Replies);
        assert!(rows > 0);
        let hit = *app
            .hits
            .borrow()
            .iter()
            .find(|hit| hit.row() == Some(80))
            .unwrap();
        for _ in 0..3 {
            assert_eq!(
                app.mouse(
                    MouseEvent {
                        kind: MouseEventKind::ScrollUp,
                        column: hit.x + 1,
                        row: hit.y,
                        modifiers: KeyModifiers::NONE,
                    },
                    Instant::now()
                ),
                Effect::None
            );
        }
        paint(&app, 160, 36);
        assert!(app.scrolls.offset(Pane::Rows) < rows);
        assert_eq!(app.scrolls.offset(Pane::Replies), replies);
        assert_eq!(app.workspace.selected, selected);
        assert_eq!(app.focused_pane(), keyboard);
        if focus == 1 {
            key(&mut app, KeyCode::Tab);
        }
        assert_eq!(app.focused_pane(), Some(Pane::Rows));
        key(&mut app, KeyCode::Down);
        let next = app.workspace_rows()[81].target.clone();
        assert_eq!(app.workspace.selected, Some(next));
        paint(&app, 160, 36);
        assert!(
            app.hits.borrow().iter().any(|hit| hit.row() == Some(81)),
            "keyboard navigation reveals the explicit next UUID"
        );
    }
}

#[test]
fn configured_paging_key_runs_its_action_before_default_tree_navigation() {
    let mut app = configured_member(
        "[bind]\npagedown = \"copy {name}\"\n",
        json!({"id":MEMBER,"name":"worker"}),
    );
    paint(&app, 100, 24);
    let selected = app.workspace.selected.clone();
    let offset = app.scrolls.offset(Pane::Rows);
    assert_eq!(
        key(&mut app, KeyCode::PageDown),
        Effect::Act(Request::Copy {
            text: "worker".into(),
            program: None
        })
    );
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.scrolls.offset(Pane::Rows), offset);
}

#[test]
fn frame_rows_share_one_projection_and_release_it_after_nested_paint_or_unwind() {
    let app = board();
    {
        let _frame = app.workspace_frame_rows();
        let rows = app.workspace_rows();
        assert!(Rc::ptr_eq(&rows, &app.workspace_rows()));
        {
            let _nested = app.workspace_frame_rows();
            assert!(Rc::ptr_eq(&rows, &app.workspace_rows()));
        }
        assert!(Rc::ptr_eq(&rows, &app.workspace_rows()));
    }
    assert!(app.frame_workspace_rows.borrow().is_none());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _frame = app.workspace_frame_rows();
        panic!("paint failed");
    }));
    assert!(result.is_err());
    assert!(app.frame_workspace_rows.borrow().is_none());
    let rows = app.workspace_rows();
    assert!(!Rc::ptr_eq(&rows, &app.workspace_rows()));
}

#[test]
fn frame_rows_never_retain_old_roster_filter_or_membership_authority() {
    let mut app = board();
    let selected = app.workspace.selected.clone();
    assert!(
        paint(&app, 160, 36)
            .iter()
            .any(|line| line.contains("worker"))
    );
    assert!(app.frame_workspace_rows.borrow().is_none());
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .members
        .get_mut(MEMBER)
        .unwrap()
        .name = "renamed-worker".into();
    assert!(
        paint(&app, 160, 36)
            .iter()
            .any(|line| line.contains("renamed-worker"))
    );
    app.search = "no-match".into();
    app.workspace_reconcile();
    paint(&app, 80, 24);
    assert_eq!(app.workspace_selection_state(), SelectionState::Hidden);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    app.search.clear();
    app.workspace.expanded.clear();
    app.workspace_reconcile();
    paint(&app, 160, 36);
    assert_eq!(app.workspace_selection_state(), SelectionState::Hidden);
    app.workspace.expanded.insert(ROOM.into());
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
    paint(&app, 160, 36);
    assert_eq!(app.workspace_selection_state(), SelectionState::Missing);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.input.is_none());
    assert_eq!(app.workspace.selected, selected);
    assert!(app.frame_workspace_rows.borrow().is_none());
}

#[test]
fn workspace_shell_is_available_before_data_at_every_theme_depth() {
    for base in tmt_cli_style::Base::ALL {
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            let mut app = App::new(Some(crate::board::ALL.into()));
            app.tabs = vec!["private-squad".into()];
            app.initial_look = Some(crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            });
            let lines = paint(&app, 80, 8);
            assert!(lines[0].contains("TMT   OPS / UI"), "{base:?} {depth:?}");
            for label in ["1 Overview", "2 Agents", "3 Digest"] {
                assert!(lines[2].contains(label), "{base:?} {depth:?}: {label}");
            }
            assert!(!lines[0].contains("private-squad"));
            let hits = app.tab_hits.borrow();
            assert_eq!(
                hits.iter().map(|hit| hit.tab).collect::<Vec<_>>(),
                [0, 1, 2]
            );
            assert!(hits.iter().all(|hit| hit.y == 2 && hit.x + hit.width <= 80));
        }
    }
}

#[test]
fn startup_navigation_uses_shell_tabs_without_recipient_authority() {
    let mut app = App::new(Some(crate::board::ALL.into()));
    app.tabs = vec!["private-squad".into()];
    for (digit, expected) in [
        ('2', OpsTab::Agents),
        ('3', OpsTab::Digest),
        ('1', OpsTab::Overview),
    ] {
        assert_eq!(key(&mut app, KeyCode::Char(digit)), Effect::None);
        assert_eq!(app.shell.tab, expected);
        paint(&app, 80, 8);
        let hit = app
            .tab_hits
            .borrow()
            .iter()
            .find(|hit| hit.tab == 1)
            .copied()
            .unwrap();
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: hit.x,
                    row: hit.y,
                    modifiers: KeyModifiers::NONE,
                },
                Instant::now()
            ),
            Effect::None
        );
        assert_eq!(app.shell.tab, OpsTab::Agents);
        assert_eq!(app.current.as_deref(), Some(crate::board::ALL));
        assert!(app.workspace.selected.is_none());
        assert!(app.input.is_none());
    }
    for code in ['t', 'j', 'x', 'm', 'n', 'o', 'c', 'D'] {
        let effect = key(&mut app, KeyCode::Char(code));
        assert!(!matches!(effect, Effect::Act(_)), "{code}");
        assert!(app.input.is_none(), "{code}");
        assert!(app.workspace_form.is_none(), "{code}");
    }
    assert_eq!(key(&mut app, KeyCode::Char('s')), Effect::None);
    assert!(app.menu.is_none(), "sort has no startup fallback menu");
}

#[test]
fn refreshed_or_resized_rows_have_no_pointer_authority_until_repaint() {
    let mut app = board();
    paint(&app, 160, 36);
    let selected = app.workspace.selected.clone();
    let hit = *app
        .hits
        .borrow()
        .iter()
        .find(|hit| hit.row() == Some(app.selected))
        .unwrap();
    let pointer = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.x,
        row: hit.y,
        modifiers: KeyModifiers::NONE,
    };
    let mut replacement = board();
    let snapshot = &mut replacement
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace;
    const OTHER: &str = "66666666-6666-4666-8666-666666666666";
    let mut member = snapshot.roster.members[MEMBER].clone();
    member.id = OTHER.into();
    member.name = "a-new-member".into();
    snapshot.roster.members.insert(OTHER.into(), member);
    snapshot.roster.squads[0].members.push(OTHER.into());
    let target = OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: OTHER.into(),
    };
    snapshot.rows.insert(
        target.clone(),
        json!({"id":OTHER,"name":"a-new-member","state":"working"}),
    );
    let mut loaded = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    loaded.tabs.push(crate::board::ALL.into());
    loaded.view = Ok(replacement.view.take().unwrap());
    app.apply(loaded);
    assert_eq!(app.workspace.selected, selected);
    assert!(app.hits.borrow().is_empty());
    assert_eq!(app.mouse(pointer, Instant::now()), Effect::None);
    assert_eq!(
        app.workspace.selected, selected,
        "old index must not select the newly inserted member"
    );
    paint(&app, 160, 36);
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| row.target == target)
        .unwrap();
    let next = *app
        .hits
        .borrow()
        .iter()
        .find(|hit| hit.row() == Some(index))
        .unwrap();
    assert_eq!(
        app.mouse(
            MouseEvent {
                column: next.x,
                row: next.y,
                ..pointer
            },
            Instant::now()
        ),
        Effect::None
    );
    assert_eq!(app.workspace.selected, Some(target));
    app.invalidate_frame_geometry(); // The production Resize event uses this same owner.
    let selected = app.workspace.selected.clone();
    assert_eq!(app.mouse(pointer, Instant::now()), Effect::None);
    assert_eq!(app.workspace.selected, selected);
    assert!(!app.scrolls.visible(Pane::Rows));
}

#[test]
fn explicit_shell_click_and_destination_key_can_leave_reader() {
    let mut app = board();
    paint(&app, 160, 36);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    let selected = app.workspace.selected.clone();
    assert_eq!(key(&mut app, KeyCode::Char('1')), Effect::None);
    assert_eq!(app.shell.tab, OpsTab::Overview);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    assert_eq!(app.workspace.selected, selected);
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Enter);
    paint(&app, 160, 36);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    let hit = *app
        .tab_hits
        .borrow()
        .iter()
        .find(|hit| hit.tab == 0)
        .unwrap();
    assert_eq!(
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hit.x,
                row: hit.y,
                modifiers: KeyModifiers::NONE,
            },
            Instant::now()
        ),
        Effect::None
    );
    assert_eq!(app.shell.tab, OpsTab::Overview);
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn workspace_names_are_inert_display_text_and_tree_never_discloses_waiting_previews() {
    for name in [
        "2024".to_owned(),
        "tab\there\nnext\u{1b}[31m".into(),
        "padded ".into(),
        "界".repeat(300),
    ] {
        let mut app = board();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.roster.squads[0].name = name.clone();
        snapshot.roster.members.get_mut(MEMBER).unwrap().name = name.clone();
        let member = OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        };
        let raw = snapshot.rows.get_mut(&member).unwrap();
        raw["name"] = json!(name);
        raw["waitingOnYou"] = json!([{"requestId":"question","preview":"PRIVATE_WAITING_PREVIEW"}]);
        let squad = OpsTarget::Squad(ROOM.into());
        snapshot.rows.get_mut(&squad).unwrap()["name"] = json!(name);
        app.workspace.selected = Some(squad.clone());
        for base in tmt_cli_style::Base::ALL {
            for depth in [
                tmt_cli_style::Depth::TrueColor,
                tmt_cli_style::Depth::Ansi16,
                tmt_cli_style::Depth::None,
            ] {
                app.view.as_mut().unwrap().look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(base),
                    depth,
                };
                // Keep the global theme selected; the selected squad has its own observation.
                app.shell.tab = OpsTab::Agents;
                for width in [160, 100, 80, 60] {
                    let lines = paint(&app, width, 36);
                    assert!(lines.iter().all(|line| !line.contains('\n')
                        && !line.contains('\t')
                        && !line.contains('\u{1b}')));
                    assert!(!lines.join(" ").contains("PRIVATE_WAITING_PREVIEW"));
                    assert_eq!(app.workspace.selected, Some(squad.clone()));
                    assert!(app.workspace_rows().iter().any(|row| row.target == member));
                }
            }
        }
    }
}

#[test]
fn workspace_authored_labels_use_registered_glyphs_at_each_width() {
    let app = board(); // All dynamic fixture text is ASCII.
    for width in [160, 100, 80, 60] {
        for line in paint(&app, width, 36) {
            assert_eq!(
                crate::board::glyph_guard::glyph_error(&line),
                None,
                "{width}: {line}"
            );
        }
    }
}

#[test]
fn shell_tabs_are_plain_labels_and_clipped_labels_have_no_pointer_authority() {
    let mut app = board();
    for state in ["waiting", "partial", "stale"] {
        let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
        home.summary.waiting = 13;
        home.summary.blocked = 3;
        home.incomplete = state == "partial";
        app.error = (state == "stale").then(|| "Roster unavailable".into());
        app.attention.insert(
            "product".into(),
            crate::attention::Attention {
                waiting: 13,
                blocked: 3,
            },
        );
        app.tab_hits.borrow_mut().clear();
        let line = crate::board::workspace::render::tabs(&app, Rect::new(0, 0, 80, 1));
        assert_eq!(line.to_string(), "  1 Overview   2 Agents   3 Digest  ");
        let hits = app.tab_hits.borrow();
        assert_eq!(hits.len(), 3);
        for (hit, (x, width)) in hits.iter().zip([(1, 12), (14, 10), (25, 10)]) {
            assert_eq!((hit.x, hit.y, hit.width), (x, 0, width));
        }
        for gap_x in [13, 24, 35] {
            assert!(
                !hits
                    .iter()
                    .any(|hit| gap_x >= hit.x && gap_x < hit.x + hit.width)
            );
        }
        drop(hits);
        for gap in [2, 4, 6] {
            assert_eq!(line.spans[gap].content, " ");
            assert_eq!(line.spans[gap].style, ratatui::style::Style::default());
        }
        for (index, span) in line.spans.iter().skip(1).step_by(2).enumerate() {
            let selected = index == 1; // The fixture starts in Agents.
            let expected = if selected {
                app.look()
                    .role(tmt_cli_style::Role::Accent)
                    .patch(app.look().selection())
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                app.look().role(tmt_cli_style::Role::Muted)
            };
            assert_eq!(
                span.style, expected,
                "attention cannot recolor shell labels"
            );
        }
    }
    app.tab_hits.borrow_mut().clear();
    crate::board::workspace::render::tabs(&app, Rect::new(0, 0, 23, 1));
    let hits = app.tab_hits.borrow();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].tab, 0);
}

#[test]
fn startup_help_uses_workspace_bindings_and_keeps_complete_editor_guidance() {
    let app = App::new(Some(crate::board::ALL.into()));
    let help = crate::board::help::model(&app);
    let bindings = help
        .sections
        .iter()
        .find(|section| section.id == "bindings")
        .unwrap();
    for (key, action) in app.bindings() {
        let expected = if action.verb == crate::action::Verb::Ui(crate::action::OpsUiAction::Pin) {
            "pin or unpin the selected agent".into()
        } else {
            action.description()
        };
        assert!(
            bindings
                .entries
                .iter()
                .any(|entry| entry.keys == key && entry.description == expected)
        );
    }
    assert!(
        !help
            .sections
            .iter()
            .any(|section| section.id == "board-picks")
    );
    let editor = help
        .sections
        .iter()
        .find(|section| section.id == "editor")
        .unwrap();
    let text = editor
        .entries
        .iter()
        .map(|entry| entry.description.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    for required in [
        "Enter/Tab insert",
        "bracketed paste never sends",
        "session-only",
        "request and byte range",
        "mentions are text only",
    ] {
        assert!(text.contains(required), "{required}");
    }
}

#[test]
fn retired_shift_arrows_cannot_run_the_unmodified_fold_binding() {
    let mut app = board();
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    let expanded = app.workspace.expanded.clone();
    for code in [KeyCode::Left, KeyCode::Right] {
        assert_eq!(
            app.key(KeyEvent::new(code, KeyModifiers::SHIFT)),
            Effect::None
        );
        assert_eq!(app.workspace.expanded, expanded);
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .contains("tab reorder is retired")
        );
    }
    key(&mut app, KeyCode::Left);
    assert!(!app.workspace.expanded.contains(ROOM));
    key(&mut app, KeyCode::Right);
    assert!(app.workspace.expanded.contains(ROOM));
}

#[test]
fn reader_dispatch_help_and_footer_share_effective_keys_and_preserve_overrides() {
    let mut app = board();
    paint(&app, 160, 36);
    key(&mut app, KeyCode::Tab);
    let target = app.workspace.selected.clone();
    assert_eq!(app.bindings()["j"].text, "scroll-down");
    assert_eq!(app.bindings()["2"].text, "agents");
    assert_eq!(key(&mut app, KeyCode::Char('j')), Effect::None);
    assert_eq!(app.workspace.selected, target);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    let keys = app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .keys
        .get_mut(&OpsKeyScope::Reader)
        .unwrap();
    keys.insert("j".into(), Action::parse("copy $name").unwrap());
    keys.insert("k".into(), Action::parse("jump").unwrap());
    let help = crate::board::help::model(&app);
    let bindings = help
        .sections
        .iter()
        .find(|section| section.id == "bindings")
        .unwrap();
    assert!(
        bindings.entries.iter().any(
            |entry| entry.keys == "j" && entry.description == app.bindings()["j"].description()
        )
    );
    assert!(
        bindings.entries.iter().any(
            |entry| entry.keys == "k" && entry.description == app.bindings()["k"].description()
        )
    );
    let rendered = paint(&app, 160, 36).join("\n");
    assert!(rendered.contains("j copy"), "{rendered}");
    assert!(rendered.contains("k host"), "{rendered}");
    assert!(matches!(
        key(&mut app, KeyCode::Char('j')),
        Effect::Act(Request::Copy { .. })
    ));
    assert_eq!(
        key(&mut app, KeyCode::Char('k')),
        Effect::Act(Request::Jump(MEMBER.into()))
    );
    assert_eq!(app.workspace.selected, target);
}

#[test]
fn a_refresh_cannot_restore_factory_pane_focus_or_turn_reader_j_into_host() {
    let mut app = board();
    paint(&app, 160, 36);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    let selected = app.workspace.selected.clone();
    let mut fresh = board();
    fresh.view.as_mut().unwrap().board = crate::config::Board::simple(
        crate::config::BoardMode::Split,
        crate::config::Direction::LeftRight,
        vec![Pane::Rows],
        &[100],
    );
    let mut loaded = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    loaded.tabs.push(crate::board::ALL.into());
    loaded.view = Ok(fresh.view.take().unwrap());
    app.apply(loaded);
    paint(&app, 160, 36);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    assert_eq!(key(&mut app, KeyCode::Char('j')), Effect::None);
    assert_eq!(app.workspace.selected, selected);
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
        matches!(effect,Effect::Act(Request::WorkspaceOperation(crate::board::workspace::manage::Change::Membership(crate::membership::workspace::Operation::Remove {room,member}))) if room.id == ROOM && member == MEMBER)
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
    app.invalidate_frame_geometry();
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
    for code in ['t', 'j', 'o', 'x', 'm', 'D'] {
        assert_eq!(
            key(&mut app, KeyCode::Char(code)),
            Effect::None,
            "hidden: {code}"
        );
        assert!(app.input.is_none() && app.menu.is_none() && app.workspace_form.is_none());
        assert_eq!(app.workspace.selected, target);
    }
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
    for code in ['t', 'j', 'o', 'x', 'm', 'D'] {
        assert_eq!(
            key(&mut app, KeyCode::Char(code)),
            Effect::None,
            "missing: {code}"
        );
        assert!(app.input.is_none() && app.menu.is_none() && app.workspace_form.is_none());
        assert_eq!(app.workspace.selected, target);
    }
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
    assert!(app.workspace_rows().is_empty());
    assert!(paint(&app, 80, 24).join("\n").contains(" No squads yet."));
    let empty = paint(&app, 80, 24);
    assert!(empty.iter().any(|line| line.starts_with(" No squads yet.")));
    assert!(empty.last().unwrap().starts_with(' '));
    app.workspace.query = "missing".into();
    assert!(
        paint(&app, 80, 24)
            .iter()
            .any(|line| line.starts_with(" No matching squads"))
    );
    app.workspace.query.clear();
    app.overview_view = crate::board::overview::monitor::View::Agents;
    assert!(
        paint(&app, 80, 24)
            .iter()
            .any(|line| line.starts_with(" No agents yet."))
    );
    app.workspace.query = "missing".into();
    assert!(
        paint(&app, 80, 24)
            .iter()
            .any(|line| line.starts_with(" No matching agents"))
    );
    app.workspace.query.clear();
    app.overview_view = crate::board::overview::monitor::View::Squads;
    assert_eq!(key(&mut app, KeyCode::Char('n')), Effect::None);
    assert_eq!(app.workspace_form.as_ref().unwrap().kind, Kind::Create);
}
#[test]
fn selected_member_host_uses_the_frozen_uuid_and_overview_has_membership_actions() {
    let mut app = board();
    crate::board::app::workspace::tests::observe_waiting_member(&mut app);
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
    // This assertion exercises real selection-background contrast, not the
    // invoking terminal's fallback palette (whose Text foreground is unset).
    for base in [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight] {
        supplied_label_choice_hit_at_look(crate::look::Look {
            theme: tmt_cli_style::Theme::new(base),
            depth: tmt_cli_style::Depth::TrueColor,
        });
    }
}

fn supplied_label_fixture(look: crate::look::Look) -> App {
    use crate::labels::{Label, Supplied};
    use tmt_cli_style::Role;
    let mut app = board();
    let view = app.view.as_mut().unwrap();
    view.look = look;
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
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
        .unwrap()["digest"] =
        json!({"active":true,"digestUntilMs":crate::status::now_ms() + 60_000,"heldCount":99});
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
    app
}

fn supplied_label_choice_hit_at_look(look: crate::look::Look) {
    use tmt_cli_style::Role;
    let mut app = supplied_label_fixture(look);
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        app.shell.tab = tab;
        for width in [100, 160] {
            let screen = paint(&app, width, 36).join("\n");
            let mode = if matches!(tab, OpsTab::Digest | OpsTab::Overview) {
                "Auto ▾"
            } else {
                "digest Auto ▾"
            };
            assert!(screen.contains(mode), "{tab:?}: {screen}");
            assert!(screen.contains("1 held"));
            assert!(
                !screen.contains("99 held"),
                "supplied labels replace the policy chip: {screen}"
            );
            let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let chips = app.chip_hits.borrow().clone();
            let mut held = Vec::new();
            // Locate supplier labels beside their admitted Auto choice, not an
            // unrelated "1 held" in the Digest summary or conversation history.
            for (area, _) in &chips {
                for x in area.right()..width.saturating_sub(5).min(area.right() + 12) {
                    let start = usize::from(area.y) * usize::from(width) + usize::from(x);
                    let cells = &buffer.content[start..start + 6];
                    if cells.iter().map(|cell| cell.symbol()).collect::<String>() == "1 held" {
                        held.push(cells);
                        break;
                    }
                }
            }
            assert!(!held.is_empty(), "the supplied label remains visible");
            // At 160 columns the Agents tree clips this suffix, but its
            // conversation header still shows the supplier's muted-role label.
            // Selected rows use the contrast policy; other surfaces keep the role.
            if tab == OpsTab::Digest {
                assert!(
                    held.iter()
                        .any(|cells| Some(cells[0].bg) == look.selection().bg)
                );
            }
            for cells in held {
                let selected = Some(cells[0].bg) == look.selection().bg;
                let role = if selected { Role::Text } else { Role::Muted };
                let expected = look.role(role).fg.unwrap();
                for cell in cells {
                    assert_eq!(
                        cell.fg, expected,
                        "supplied label role and selection contrast: {tab:?}/{width}/{selected}"
                    );
                }
            }
            assert!(!chips.is_empty());
            for (area, _) in chips {
                // Both the tree chip and the shared conversation chip resolve the
                // same exact membership. The whole label, including its last cell,
                // is actionable; neighbouring text never becomes a dropdown.
                for column in [area.x, area.right() - 1] {
                    // Selection invalidates the previous frame's authorities.
                    // Repaint before the next independent edge activation.
                    terminal
                        .draw(|frame| {
                            crate::board::view::prepare(&mut app, frame.area());
                            crate::board::view::render(frame, &app);
                        })
                        .unwrap();
                    let Effect::Act(Request::DigestOpen(ask)) = app.mouse(
                        MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column,
                            row: area.y,
                            modifiers: KeyModifiers::NONE,
                        },
                        Instant::now(),
                    ) else {
                        panic!("visible chip must open its own member");
                    };
                    assert_eq!(ask.squad, "product");
                    assert_eq!(ask.member, "worker");
                    assert_eq!(
                        app.workspace.selected,
                        Some(OpsTarget::Member {
                            room_id: Some(ROOM.into()),
                            identity_id: MEMBER.into(),
                        })
                    );
                }
                if area.x > 0 {
                    assert!(!matches!(
                        app.mouse(
                            MouseEvent {
                                kind: MouseEventKind::Down(MouseButton::Left),
                                column: area.x - 1,
                                row: area.y,
                                modifiers: KeyModifiers::NONE
                            },
                            Instant::now(),
                        ),
                        Effect::Act(Request::DigestOpen(_))
                    ));
                }
            }
        }
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
        matches!(&draft.operation,Some(crate::board::workspace::manage::Change::Membership(crate::membership::workspace::Operation::Create {members,..})) if members == &[MEMBER.to_owned()])
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

#[test]
fn workspace_annotations_recheck_sender_membership_and_lead_identity() {
    const LEAD: &str = "88888888-8888-4888-8888-888888888881";
    const REPLACEMENT: &str = "88888888-8888-4888-8888-888888888882";
    let opening = || {
        let mut app = board();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        let mut lead = snapshot.roster.members[MEMBER].clone();
        lead.id = LEAD.into();
        lead.name = "lead".into();
        snapshot.roster.members.insert(LEAD.into(), lead);
        snapshot.roster.squads[0].lead = Some(LEAD.into());
        snapshot.roster.squads[0].members.push(LEAD.into());
        snapshot.rows.insert(
            OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: LEAD.into(),
            },
            json!({"id":LEAD,"name":"lead"}),
        );
        snapshot
            .rows
            .get_mut(app.workspace.selected.as_ref().unwrap())
            .unwrap()["pending"] =
            json!("Keep the acquired pending note until an explicit status change");
        app.workspace_reconcile();
        app.perform(&Action::parse("annotate").unwrap());
        assert_eq!(
            app.input.as_ref().unwrap().compose,
            Compose::Annotate {
                to: "lead".into(),
                row: "worker".into()
            }
        );
        app.paste("  observed\nmessage  ");
        app
    };
    let mut app = opening();
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Effect::Act(Request::Annotate {
            me: "Owner".into(),
            squad: "product".into(),
            to: "lead".into(),
            row: "worker".into(),
            text: "  observed\nmessage  ".into()
        })
    );
    app.finished(Ok("Annotated".into()));
    assert_eq!(
        app.selected_row().unwrap()["pending"],
        "Keep the acquired pending note until an explicit status change"
    );
    for change in [
        "sender",
        "sender UUID",
        "membership",
        "identity removed",
        "lead name",
        "lead UUID",
        "loading",
    ] {
        let mut app = opening();
        match change {
            "sender" => app.view.as_mut().unwrap().me = Some("Other".into()),
            "sender UUID" => app.view.as_mut().unwrap().me_id = Some(REPLACEMENT.into()),
            "loading" => app.current = Some("unloaded".into()),
            _ => {
                let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                match change {
                    "membership" => snapshot.roster.squads[0].members.retain(|id| id != MEMBER),
                    "identity removed" => {
                        snapshot.roster.members.remove(MEMBER);
                    }
                    "lead name" => {
                        snapshot.roster.members.get_mut(LEAD).unwrap().name = "new-lead".into()
                    }
                    _ => {
                        let mut lead = snapshot.roster.members.remove(LEAD).unwrap();
                        lead.id = REPLACEMENT.into();
                        snapshot.roster.members.insert(REPLACEMENT.into(), lead);
                        snapshot.roster.squads[0].lead = Some(REPLACEMENT.into());
                        snapshot.roster.squads[0].members.retain(|id| id != LEAD);
                        snapshot.roster.squads[0].members.push(REPLACEMENT.into());
                    }
                }
            }
        }
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None, "{change}");
        assert!(
            app.notice.as_deref().unwrap().contains("nothing sent"),
            "{change}"
        );
    }
}

#[test]
fn workspace_replies_keep_the_exact_request_and_recheck_opening_authority() {
    const FIRST: &str = "req_44444444-4444-4444-8444-444444444441";
    const SECOND: &str = "req_44444444-4444-4444-8444-444444444442";
    let opening = || {
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
            .unwrap()["waitingOnYou"] = json!([
            {"requestId":FIRST,"preparedAtMs":20,"preview":"first question"},
            {"requestId":SECOND,"preparedAtMs":30,"preview":"second question"}
        ]);
        app.perform(&Action::parse("reply").unwrap());
        assert_eq!(app.menu.as_ref().unwrap().entries.len(), 2);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert!(matches!(
            &app.input.as_ref().unwrap().compose,
            Compose::Reply { request, .. } if request == SECOND
        ));
        app
    };
    let mut app = opening();
    key(&mut app, KeyCode::Char('y'));
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Effect::Act(Request::Reply {
            me: "Owner".into(),
            request: SECOND.into(),
            from: "worker".into(),
            text: "y".into(),
        })
    );

    for change in [
        "request",
        "sender",
        "sender UUID",
        "loading",
        "membership",
        "name",
    ] {
        let mut app = opening();
        key(&mut app, KeyCode::Char('y'));
        let selected = app.workspace.selected.clone().unwrap();
        match change {
            "sender" => app.view.as_mut().unwrap().me = Some("Other".into()),
            "sender UUID" => {
                app.view.as_mut().unwrap().me_id =
                    Some("99999999-9999-4999-8999-999999999999".into())
            }
            "loading" => app.current = Some("unloaded".into()),
            _ => {
                let rows = &mut app
                    .view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .rows;
                match change {
                    "request" => rows.get_mut(&selected).unwrap()["waitingOnYou"] = json!([]),
                    "membership" => {
                        rows.remove(&selected);
                    }
                    _ => rows.get_mut(&selected).unwrap()["name"] = json!("renamed"),
                }
            }
        }
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None, "{change}");
        assert!(app.notice.as_deref().unwrap().contains("nothing sent"));
    }
    let mut app = opening();
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.notice.as_deref(), Some("Nothing sent."));
    let mut app = opening();
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.input.is_none() && app.menu.is_none());
}

#[test]
fn a_successful_reply_refresh_keeps_the_exact_member_and_feedback_until_the_next_key() {
    const REQUEST: &str = "req_44444444-4444-4444-8444-444444444443";
    for width in [160, 100, 80, 60] {
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
            .unwrap()["waitingOnYou"] = json!([
            {"requestId":REQUEST,"preparedAtMs":20,"preview":"An explicitly selected question"}
        ]);
        app.perform(&Action::parse("reply").unwrap());
        assert!(app.menu.is_none());
        assert!(
            matches!(&app.input.as_ref().unwrap().compose, Compose::Reply { request, .. } if request == REQUEST)
        );
        paint(&app, width, 36);
        key(&mut app, KeyCode::Char('y'));
        assert_eq!(
            key(&mut app, KeyCode::Enter),
            Effect::Act(Request::Reply {
                me: "Owner".into(),
                request: REQUEST.into(),
                from: "worker".into(),
                text: "y".into(),
            })
        );
        assert_eq!(app.workspace.selected.as_ref(), Some(&selected));
        assert!(app.notice.is_none(), "a requested reply is not completion");
        app.finished(Ok("Replied".into()));
        let mut fresh = board();
        fresh
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&selected)
            .unwrap()["waitingOnYou"] = json!([]);
        let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
        snapshot.tabs = fresh.tabs;
        snapshot.view = Ok(fresh.view.take().unwrap());
        app.apply(snapshot);
        assert_eq!(app.workspace.selected, Some(selected.clone()));
        assert_eq!(app.selected_row().unwrap()["waitingOnYou"], json!([]));
        assert_eq!(app.notice.as_deref(), Some("Replied"));
        let screen = paint(&app, width, 36).join("\n");
        assert!(screen.contains("Replied"), "{width}: {screen}");
        key(&mut app, KeyCode::Down);
        assert!(app.notice.is_none());
    }
}

#[test]
fn a_composer_with_its_own_key_legend_does_not_advertise_list_shortcuts() {
    for width in [160, 100, 80, 60] {
        let mut app = board();
        key(&mut app, KeyCode::Enter);
        let lines = paint(&app, width, 36);
        assert!(app.input_band.get().is_some());
        assert!(lines.iter().any(|line| line.contains("Enter send")));
        assert!(
            lines.last().unwrap().trim().is_empty(),
            "{width}: {:?}",
            lines.last()
        );
        key(&mut app, KeyCode::Char('j'));
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "j");
        key(&mut app, KeyCode::Esc);
        assert!(app.input.is_none());
        let lines = paint(&app, width, 36);
        assert!(lines.last().unwrap().contains("Nothing sent."));
        // Escape restores list focus while the UUID draft stays in the visible
        // passive composer. Clearing the notice reveals effective list hints.
        app.notice = None;
        assert_eq!(app.focused_pane(), Some(Pane::Rows));
        let lines = paint(&app, width, 36);
        assert!(
            lines[lines.len() - 2].contains("j host"),
            "{width}: {:?}",
            lines.last()
        );
    }
}

#[test]
fn compact_overview_talk_opens_agents_but_wide_overview_keeps_the_shared_conversation() {
    for width in [160, 100, 80, 60] {
        let mut app = board();
        crate::board::app::workspace::tests::observe_waiting_member(&mut app);
        app.shell.tab = OpsTab::Overview;
        paint(&app, width, 36);
        let selected = app.workspace.selected.clone();
        app.perform(&crate::action::Action::parse("talk").unwrap());
        assert_eq!(app.focused_pane(), Some(Pane::Replies));
        assert_eq!(
            app.shell.tab,
            if width >= 140 {
                OpsTab::Overview
            } else {
                OpsTab::Agents
            }
        );
        let lines = paint(&app, width, 36);
        assert!(app.input_band.get().is_some());
        assert!(lines.iter().any(|line| line.contains(if width >= 140 {
            "› worker as you…"
        } else {
            "Message → worker"
        })));
        assert_eq!(app.workspace.selected, selected);
        key(&mut app, KeyCode::Char('2')); // Message input owns digits.
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "2");
        assert_eq!(
            app.shell.tab,
            if width >= 140 {
                OpsTab::Overview
            } else {
                OpsTab::Agents
            }
        );
    }
}

#[test]
fn overview_reader_resize_and_refresh_keep_reader_keys_instead_of_host_scope() {
    let mut app = board();
    app.shell.tab = OpsTab::Overview;
    app.set_body_width(160);
    paint(&app, 160, 36);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    let selected = app.workspace.selected.clone();
    app.invalidate_frame_geometry();
    app.set_body_width(160);
    assert_eq!(app.shell.tab, OpsTab::Overview);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    assert_eq!(app.bindings()["j"].text, "scroll-down");
    app.set_body_width(80);
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(app.focused_pane(), Some(Pane::Replies));
    paint(&app, 80, 36);
    assert_eq!(key(&mut app, KeyCode::Char('j')), Effect::None);
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn selected_squad_discloses_existing_usage_facts_without_private_lead_fields() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        for width in [160, 100, 80, 60] {
            let mut app = crate::board::home::tests::oracle::metered(crate::look::Look::default());
            app.shell.tab = OpsTab::Agents;
            app.view.as_mut().unwrap().home.as_mut().unwrap().squads[0]
                .lead
                .as_mut()
                .unwrap()["fields"] = json!({"task":"private lead task", "pr_link":"private PR"});
            let index = app
                .workspace_rows()
                .iter()
                .position(|row| row.target == OpsTarget::Squad(ROOM.into()))
                .unwrap();
            app.select(index);
            paint(&app, width, 36);
            key(&mut app, KeyCode::Tab);
            let text = paint(&app, width, 36).join("\n");
            assert!(text.contains("Lead usage:"), "{width}: {text}");
            assert!(text.contains("~21 tok"), "{width}: {text}");
            assert!(text.contains("Lead share: ~100%"), "{width}: {text}");
            assert!(!text.contains("private lead task") && !text.contains("private PR"));
        }
    });
}

#[test]
fn selected_tree_rows_keep_uniform_reverse_fallback_and_clipped_hit_bounds() {
    use ratatui::style::Modifier;
    for base in tmt_cli_style::Base::ALL {
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            let mut app = board();
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            app.view.as_mut().unwrap().look = look;
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .squad_looks
                .insert(ROOM.into(), look);
            for width in [160, 80, 31, 1] {
                let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
                terminal
                    .draw(|frame| crate::board::view::render(frame, &app))
                    .unwrap();
                let hits = app.hits.borrow();
                assert!(
                    hits.iter()
                        .all(|hit| hit.x + hit.width <= width && hit.y < 18)
                );
                let Some(hit) = hits.iter().find(|hit| hit.row() == Some(app.selected)) else {
                    continue;
                };
                let buffer = terminal.backend().buffer();
                let selection = look.selection();
                for x in hit.x..hit.x + hit.width {
                    let cell = &buffer[(x, hit.y)];
                    if let Some(background) = selection.bg {
                        assert_eq!(cell.bg, background);
                    } else {
                        assert_eq!(
                            cell.fg,
                            selection.fg.unwrap_or_default(),
                            "{base:?}/{depth:?} at {x},{}",
                            hit.y
                        );
                        assert!(cell.modifier.contains(Modifier::REVERSED));
                        assert!(!cell.modifier.contains(Modifier::DIM));
                    }
                }
            }
        }
    }
}

#[test]
fn passive_composer_preserves_footer_bindings_notice_error_modal_and_reload_precedence() {
    for width in [160, 100, 60] {
        let mut app = board();
        assert!(paint(&app, width, 18).last().unwrap().contains("j host"));
        assert!(app.input_band.get().is_some() && app.input.is_none());
        key(&mut app, KeyCode::Enter);
        assert!(paint(&app, width, 18).last().unwrap().trim().is_empty());
        app.reload = crate::board::reload::Watch::due();
        let lines = paint(&app, width, 18);
        assert!(lines.last().unwrap().contains("reload waiting"));
        assert!(!lines.last().unwrap().contains("j host"));
        key(&mut app, KeyCode::Esc);
        assert!(
            paint(&app, width, 18)
                .last()
                .unwrap()
                .contains("Nothing sent.")
        );
        app.reload = Default::default();
        app.notice = None;
        app.error = Some("Acquired source unavailable".into());
        assert!(
            paint(&app, width, 18)
                .last()
                .unwrap()
                .contains("Acquired source unavailable")
        );
        app.error = None;
        app.help = true;
        assert!(paint(&app, width, 18).last().unwrap().trim().is_empty());
        app.help = false;
        assert!(paint(&app, width, 18).last().unwrap().contains("j host"));
    }
}

#[test]
fn member_footer_names_the_default_host_action_without_an_open_alias() {
    let app = board();
    let lines = paint(&app, 160, 36);
    assert!(lines[lines.len() - 2].contains("j host"));
    assert!(!lines[lines.len() - 2].contains("j open"));
}

#[test]
fn workspace_settings_cancel_returns_to_the_exact_reader_without_factory_actions() {
    for width in [160, 100, 80, 60] {
        let mut app = board();
        paint(&app, width, 36);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focused_pane(), Some(Pane::Replies));
        let selected = app.workspace.selected.clone();
        key(&mut app, KeyCode::Char(','));
        let menu = app.menu.as_ref().unwrap();
        assert!(menu.entries.iter().all(|entry| match &entry.choice {
            Choice::Action(action) => action.verb != Verb::View,
            _ => true,
        }));
        assert!(menu.entries.iter().any(|entry| matches!(
            entry.choice,
            Choice::WorkspaceLayout(crate::config::OpsLayout::Auto)
        )));
        paint(&app, width, 36);
        key(&mut app, KeyCode::Esc);
        assert!(app.menu.is_none());
        assert_eq!(app.workspace.selected, selected);
        assert_eq!(app.focused_pane(), Some(Pane::Replies));
        assert_eq!(
            key(&mut app, KeyCode::Char('j')),
            Effect::None,
            "the restored Reader must scroll rather than jump to host"
        );
        assert_eq!(app.workspace.selected, selected);
    }
}

#[test]
fn retired_source_actions_cannot_open_legacy_ui_or_redirect_the_selected_target() {
    for text in [
        "pick-tab",
        "view",
        "notes",
        "home-message",
        "home-write",
        "home-pick",
        "ask-lead",
        "toggle detail",
        "tab",
        "jump lead",
    ] {
        let mut app = board();
        let selected = app.workspace.selected.clone();
        let action = crate::action::Action::parse(text).unwrap();
        assert_eq!(app.perform(&action), Effect::None, "{text}");
        assert!(
            app.notice
                .as_ref()
                .is_some_and(|notice| notice.contains("retired")
                    || notice.contains("implicit lead targeting")),
            "{text}: {:?}",
            app.notice
        );
        assert_eq!(app.workspace.selected, selected, "{text}");
        assert!(app.input.is_none() && app.menu.is_none(), "{text}");
    }
}

#[test]
fn repeated_membership_selection_keeps_its_room_context_and_exact_row_data() {
    const OTHER_ROOM: &str = "77777777-7777-4777-8777-777777777777";
    let mut app = board();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut squad = snapshot.roster.squads[0].clone();
    squad.room_id = OTHER_ROOM.into();
    squad.name = "another-squad".into();
    squad.member_states.get_mut(MEMBER).unwrap().state = Some("blocked".into());
    snapshot.roster.squads.push(squad);
    let first = OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: MEMBER.into(),
    };
    let second = OpsTarget::Member {
        room_id: Some(OTHER_ROOM.into()),
        identity_id: MEMBER.into(),
    };
    snapshot.rows.get_mut(&first).unwrap()["note"] = json!("first room observation");
    snapshot.rows.insert(
        second.clone(),
        json!({"id":MEMBER,"name":"worker","state":"blocked","note":"second room observation"}),
    );
    app.workspace.expanded.insert(OTHER_ROOM.into());
    for target in [first, second] {
        let index = app
            .workspace_rows()
            .iter()
            .position(|row| row.target == target)
            .unwrap();
        app.select(index);
        for width in [160, 80, 60] {
            paint(&app, width, 36);
            assert_eq!(app.workspace.selected.as_ref(), Some(&target));
            assert_eq!(
                app.selected_row().unwrap()["note"],
                if matches!(&target, OpsTarget::Member {room_id: Some(id), ..} if id == ROOM) {
                    "first room observation"
                } else {
                    "second room observation"
                }
            );
            let hit = *app
                .hits
                .borrow()
                .iter()
                .find(|hit| hit.row() == Some(index))
                .unwrap();
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: hit.x,
                    row: hit.y,
                    modifiers: KeyModifiers::NONE,
                },
                Instant::now(),
            );
            assert_eq!(app.workspace.selected.as_ref(), Some(&target));
        }
    }
}

/// One literal fixture owner for private capture packets and the frozen HOME oracle.
/// It configures only an isolated test model; rendering stays on the actual App path.
pub(in crate::board) fn capture_case(scene: &str, look: crate::look::Look) -> App {
    let mut app = match scene {
        "squads-eight" => populated_board(8, 150),
        "squads-thirty-six" => populated_board(36, 150),
        _ => board(),
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
        "named-layer" => app.view_layer = crate::board::ViewLayer::parse("popup").unwrap(),
        "named-layer-long" => {
            app.view_layer = crate::board::ViewLayer::parse(&"x".repeat(32)).unwrap()
        }
        "named-layer-long-partial" => {
            app.view_layer = crate::board::ViewLayer::parse(&"x".repeat(32)).unwrap();
            home.workspace.roster.observation = OpsObservation::Partial;
        }
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
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.shell_counts = snapshot.roster.shell_counts();
    app.workspace_reconcile();
    match scene {
        "member-links" | "member-links-menu" => {
            app.references = crate::board::inventory::Inventory::from_listed(
                &[],
                &Ok(json!({"identities":[{"id":MEMBER,"name":"worker","canonicalName":"worker"}]})),
            );
            app.view.as_mut().unwrap().replies = vec![
                json!({"recipientId":MEMBER,"requestId":"mention-response","response":"Review #tmt:worker, then ask #tmt:missing. CJK: 請#tmt:worker。"}),
            ];
            app.perform(&crate::action::Action::parse("talk").unwrap());
            app.input.as_mut().unwrap().text = "Draft for #tmt:worker stays raw.".into();
            assert!(
                app.input.as_ref().unwrap().draft_key.is_some(),
                "capture uses a real UUID draft"
            );
            key(&mut app, KeyCode::Esc);
            assert_eq!(
                app.composer_projection()
                    .and_then(|projection| projection.text)
                    .map(|editor| editor.as_str()),
                Some("Draft for #tmt:worker stays raw.")
            );
            app.notice = None;
            app.workspace_reconcile();
            app.focus = 1;
        }
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
            app.input.as_mut().unwrap().text =
                "Draft remains with the exact recipient while the workspace resizes.".into();
        }
        _ => {}
    }
    app
}

/// Explicit, read-only evidence capture. This never rewrites a frozen fixture
/// and never calls Core, tmux, the filesystem-backed services or the action lane.
#[test]
#[ignore = "explicit WP2 workspace capture packet"]
fn capture_workspace_packet() {
    let output = std::env::var("TMT_WORKSPACE_CAPTURE").expect("task-owned evidence path");
    let mut captures = Vec::new();
    for scene in [
        "member-links",
        "member-links-menu",
        "named-layer-long",
        "named-layer-long-partial",
        "named-layer",
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
                    let look = crate::look::Look {
                        theme: tmt_cli_style::Theme::new(
                            tmt_cli_style::theme::Base::parse(base).unwrap(),
                        ),
                        depth,
                    };
                    let mut app = capture_case(scene, look);
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    if scene == "member-links-menu" {
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                        crate::board::member_links::capture_menu(&mut app);
                    }
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
    assert_eq!(captures.len(), 608);
    std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
}

#[test]
fn overview_never_claims_zero_from_incomplete_rosters_and_reports_the_global_observed_states() {
    for squads in [0, 8, 36] {
        let mut app = populated_board(squads, 150);
        app.shell.tab = OpsTab::Overview;
        let complete = paint(&app, 160, 36).join("\n");
        assert!(complete.contains("● 150 online"), "{complete}");
        assert!(complete.contains("✗ 1 blocked"), "{complete}");
        assert!(complete.contains("◆ 0 waiting"), "{complete}");
        assert!(
            crate::board::overview::monitor::scene(&app, app.workspace_snapshot().unwrap()).waiting
                == 0,
            "blocked is not a pending question"
        );
        assert_eq!(
            crate::board::overview::monitor::scene(&app, app.workspace_snapshot().unwrap())
                .summary
                .working,
            149
        );
        let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
        home.workspace.roster.membership_complete = false;
        home.workspace.roster.observation = OpsObservation::Partial;
        app.overview_rows.borrow_mut().clear(); // A real roster receipt invalidates the same cache.
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
                        let width = if index % 2 == 0 { 100 } else { 160 };
                        terminal.backend_mut().resize(width, 50);
                        terminal.resize(Rect::new(0, 0, width, 50)).unwrap();
                        assert_eq!(terminal.size().unwrap().width, width);
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
    assert_eq!(app.shell.tab, OpsTab::Overview);
    assert_eq!(app.workspace.selected, original);
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    paint(&app, 60, 8);
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.focused_pane(),
        Some(Pane::Rows),
        "Overview cannot focus its hidden conversation"
    );
}

mod member_rename;
mod membership_context;
mod navigation;

#[test]
fn agents_table_cells_and_hits_keep_state_color_scoped_across_sizes_and_focus() {
    use ratatui::style::{Color, Modifier};
    let capture_path = std::env::var("TMT_AGENTS_CAPTURE").ok();
    let mut captures = Vec::new();
    for width in [160, 100, 60] {
        for height in [44, 24, 18] {
            for (base, depth) in [
                (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
                (
                    tmt_cli_style::Base::TmtLight,
                    tmt_cli_style::Depth::TrueColor,
                ),
                (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
            ] {
                for selected_squad in [false, true] {
                    for focus in [0, 1] {
                        let look = crate::look::Look {
                            theme: tmt_cli_style::Theme::new(base),
                            depth,
                        };
                        let mut app = board();
                        app.view.as_mut().unwrap().look = look;
                        let snapshot =
                            &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                        snapshot.squad_looks.clear();
                        snapshot.roster.members.get_mut(MEMBER).unwrap().state =
                            Some("blocked".into());
                        snapshot.roster.squads[0]
                            .member_states
                            .get_mut(MEMBER)
                            .unwrap()
                            .state = Some("blocked".into());
                        let member = OpsTarget::Member {
                            room_id: Some(ROOM.into()),
                            identity_id: MEMBER.into(),
                        };
                        snapshot.rows.get_mut(&member).unwrap()["state"] = json!("blocked");
                        snapshot.shell_counts = snapshot.roster.shell_counts();
                        if selected_squad {
                            app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
                        }
                        app.focus = focus;
                        app.workspace_reconcile();
                        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                        let buffer = terminal.backend().buffer();
                        let lines: Vec<String> = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                            .collect();
                        if app.scrolls.visible(Pane::Rows) {
                            assert!(
                                lines.iter().any(|line| line.contains("SQUAD / AGENT")),
                                "{width}x{height}: {lines:?}"
                            );
                            let selected_hit = app
                                .hits
                                .borrow()
                                .iter()
                                .find(|hit| {
                                    hit.row().is_some_and(|index| {
                                        Some(&app.workspace_rows()[index].target)
                                            == app.workspace.selected.as_ref()
                                    })
                                })
                                .copied()
                                .unwrap();
                            let text = (selected_hit.x..selected_hit.x + selected_hit.width)
                                .map(|x| buffer[(x, selected_hit.y)].symbol())
                                .collect::<String>();
                            assert!(
                                text.contains(if selected_squad {
                                    "▾ product"
                                } else {
                                    "└ worker"
                                }),
                                "{width}x{height}: {text}"
                            );
                        } else {
                            assert!(
                                lines.iter().any(|line| line.contains(if selected_squad {
                                    "Roster preview"
                                } else {
                                    "Message → worker"
                                })),
                                "{width}x{height}: {lines:?}"
                            );
                            assert!(app.hits.borrow().iter().all(|hit| hit.row().is_none()));
                        }
                        let selected = app.workspace.selected.clone();
                        assert_eq!(app.workspace_selection_state(), SelectionState::Visible);
                        assert!(
                            app.hits
                                .borrow()
                                .iter()
                                .filter_map(Hit::row)
                                .all(|index| index < app.workspace_rows().len())
                        );
                        for hit in app.cron_scope_hits.borrow().iter() {
                            assert_eq!(hit.area.intersection(buffer.area), hit.area);
                        }
                        if !selected_squad && focus == 0 {
                            let hit = app
                                .hits
                                .borrow()
                                .iter()
                                .find(|hit| {
                                    hit.row().is_some_and(|index| {
                                        app.workspace_rows()[index].target == member
                                    })
                                })
                                .cloned()
                                .unwrap();
                            for x in hit.x..hit.x + hit.width {
                                let cell = &buffer[(x, hit.y)];
                                let state = app
                                    .workspace_state_cells
                                    .borrow()
                                    .iter()
                                    .any(|area| area.contains((x, hit.y).into()));
                                if depth != tmt_cli_style::Depth::None {
                                    assert_eq!(cell.bg, look.selection().bg.unwrap());
                                    assert_eq!(
                                        cell.fg,
                                        look.role(if state {
                                            tmt_cli_style::Role::Blocked
                                        } else {
                                            tmt_cli_style::Role::Text
                                        })
                                        .fg
                                        .unwrap_or(Color::Reset),
                                        "{width}x{height} x{x}"
                                    );
                                } else {
                                    assert!(cell.modifier.contains(Modifier::REVERSED));
                                }
                            }
                        }
                        if capture_path.is_some() {
                            captures.push(json!({"width":width,"height":height,"base":format!("{base:?}"),"depth":format!("{depth:?}"),"squad":selected_squad,"focus":focus,
                            "lines":lines,"cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>(),
                            "hits":format!("{:?}",app.hits.borrow()),"timerHits":app.cron_scope_hits.borrow().iter().map(|hit|json!({"area":format!("{:?}",hit.area),"target":format!("{:?}",hit.target),"room":hit.room,"owner":hit.owner})).collect::<Vec<_>>(),"selected":format!("{selected:?}")}));
                        }
                    }
                }
            }
        }
    }
    if let Some(path) = capture_path {
        assert_eq!(captures.len(), 108);
        std::fs::write(path, serde_json::to_vec(&captures).unwrap()).unwrap();
    }
}

#[test]
fn agents_filter_uses_the_effective_key_and_painted_controls_without_retargeting() {
    use crate::board::workspace::AgentsFilter;
    let mut app = board();
    let selected = app.workspace.selected.clone();
    let expansion = app.workspace.expanded.clone();
    let preferences = app.workspace_snapshot().unwrap().preferences.clone();
    paint(&app, 160, 44);
    assert_eq!(key(&mut app, KeyCode::Char('f')), Effect::None);
    assert_eq!(app.workspace.filter, AgentsFilter::NeedsYou);
    assert_eq!(app.workspace_selection_state(), SelectionState::Hidden);
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.workspace.expanded, expansion);
    assert!(
        app.workspace_strip_hits.borrow().is_empty(),
        "filter invalidates stale controls"
    );
    paint(&app, 160, 44);
    let all = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find(|(_, choice)| *choice == Choice::AgentsFilter(AgentsFilter::All))
        .unwrap()
        .0;
    assert_eq!(
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: all.x,
                row: all.y,
                modifiers: KeyModifiers::NONE
            },
            Instant::now()
        ),
        Effect::None
    );
    assert_eq!(app.workspace.filter, AgentsFilter::All);
    assert_eq!(app.workspace_selection_state(), SelectionState::Visible);
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.workspace_snapshot().unwrap().preferences, preferences);
    app.shell.tab = OpsTab::Overview;
    paint(&app, 160, 44);
    assert!(
        app.workspace_strip_hits
            .borrow()
            .iter()
            .all(|(_, choice)| matches!(choice, Choice::SavingsUnit | Choice::OverviewView(_))),
        "Overview paints only its own view controls and any admitted estimate"
    );
    assert_eq!(key(&mut app, KeyCode::Char('f')), Effect::None);
    assert_eq!(
        app.workspace.filter,
        AgentsFilter::All,
        "Overview has its own filter"
    );
}

#[test]
fn agents_wide_table_uses_hour_readings_once_and_marks_missing_members() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut app = crate::board::home::tests::oracle::metered(crate::look::Look::default());
        app.shell.tab = OpsTab::Agents;
        app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
        let reading = app.overview_member_usage(MEMBER).unwrap();
        for missing in [false, true] {
            let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
            snapshot.preferences.layout = crate::config::OpsLayout::Stack;
            snapshot.roster.squads[0].members.push(MEMBER.into());
            if missing {
                snapshot.roster.squads[0]
                    .members
                    .push("missing-observation".into());
            }
            app.workspace_reconcile();
            let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let row = app
                .hits
                .borrow()
                .iter()
                .find(|hit| {
                    hit.row().is_some_and(|index| {
                        app.workspace_rows()[index].target == OpsTarget::Squad(ROOM.into())
                    })
                })
                .copied()
                .unwrap();
            let slots = app
                .agents_layout
                .borrow_mut()
                .slots(Rect::new(1, row.y, 158, 1));
            for (field, expected) in [
                (
                    crate::board::workspace::columns::AgentsField::Lead,
                    "worker".to_owned(),
                ),
                (
                    crate::board::workspace::columns::AgentsField::Tokens,
                    format!(
                        "{}{}",
                        if missing || reading.partial { "~" } else { "" },
                        crate::source::tokens(reading.tokens as f64)
                    ),
                ),
            ] {
                let slot = slots
                    .iter()
                    .find(|(candidate, _)| *candidate == field)
                    .unwrap()
                    .1;
                let shown = (slot.x..slot.right())
                    .map(|x| terminal.backend().buffer()[(x, row.y)].symbol())
                    .collect::<String>();
                assert_eq!(shown.trim(), expected);
            }
        }
    });
}

#[test]
fn roster_preview_click_selects_the_exact_membership_and_invalidates_old_hits() {
    let mut app = board();
    let member = app.workspace.selected.clone().unwrap();
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    app.workspace_reconcile();
    paint(&app, 160, 44);
    let area = app
        .workspace_action_hits
        .borrow()
        .iter()
        .find(|(_, _, choice)| *choice == Choice::WorkspaceTarget(member.clone()))
        .unwrap()
        .0;
    assert_eq!(
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.right() - 1,
                row: area.y,
                modifiers: KeyModifiers::NONE
            },
            Instant::now()
        ),
        Effect::None
    );
    assert_eq!(app.workspace.selected, Some(member));
    assert_eq!(app.focused_pane(), Some(Pane::Rows));
    assert!(app.workspace_action_hits.borrow().is_empty());
    assert!(app.workspace_state_cells.borrow().is_empty());
}

#[test]
fn default_attention_colors_stay_in_the_painted_state_word_while_other_states_are_neutral() {
    use tmt_cli_style::{Base, Depth, Role, Theme};
    for base in [Base::Tmt, Base::TmtLight] {
        let look = crate::look::Look {
            theme: Theme::new(base),
            depth: Depth::TrueColor,
        };
        for (state, role) in [
            ("working", Role::Text),
            ("idle", Role::Text),
            ("offline", Role::Text),
            ("waiting", Role::Waiting),
            ("blocked", Role::Blocked),
        ] {
            for selected in [false, true] {
                let mut app = board();
                app.view.as_mut().unwrap().look = look;
                let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                snapshot.squad_looks.clear();
                snapshot.roster.members.get_mut(MEMBER).unwrap().state = Some(state.into());
                snapshot.roster.squads[0]
                    .member_states
                    .get_mut(MEMBER)
                    .unwrap()
                    .state = Some(state.into());
                let target = OpsTarget::Member {
                    room_id: Some(ROOM.into()),
                    identity_id: MEMBER.into(),
                };
                snapshot.rows.get_mut(&target).unwrap()["state"] = json!(state);
                assert!(
                    snapshot.rows[&target]["colors"]["state"].is_null(),
                    "fixture has no authored state token"
                );
                if !selected {
                    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
                }
                app.workspace_reconcile();
                let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
                terminal
                    .draw(|frame| crate::board::view::render(frame, &app))
                    .unwrap();
                let hit = app
                    .hits
                    .borrow()
                    .iter()
                    .find(|hit| {
                        hit.row()
                            .is_some_and(|index| app.workspace_rows()[index].target == target)
                    })
                    .copied()
                    .unwrap();
                let state_area = app
                    .workspace_state_cells
                    .borrow()
                    .iter()
                    .find(|area| area.y == hit.y)
                    .copied()
                    .unwrap();
                let painted_state = (state_area.x..state_area.right())
                    .map(|x| terminal.backend().buffer()[(x, hit.y)].symbol())
                    .collect::<String>();
                assert!(
                    painted_state.contains(state),
                    "the whole state word is visible: {painted_state}"
                );
                let painted_row = (hit.x..hit.x + hit.width)
                    .map(|x| terminal.backend().buffer()[(x, hit.y)].symbol())
                    .collect::<String>();
                assert!(painted_row.contains("worker"));
                for x in hit.x..hit.x + hit.width {
                    if terminal.backend().buffer()[(x, hit.y)]
                        .symbol()
                        .trim()
                        .is_empty()
                    {
                        continue;
                    }
                    assert_eq!(
                        terminal.backend().buffer()[(x, hit.y)].fg,
                        look.role(if state_area.contains((x, hit.y).into()) {
                            role
                        } else {
                            Role::Text
                        })
                        .fg
                        .unwrap(),
                        "{base:?}/{role:?}/selected={selected} x{x}"
                    );
                }
            }
        }
    }
}

#[test]
fn confirmed_hide_moves_only_its_visible_squad_and_keeps_global_attention() {
    let mut app = board();
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    app.workspace_reconcile();
    let old = app.workspace_snapshot().unwrap().preferences.clone();
    let global = app.workspace_snapshot().unwrap().shell_counts.clone();
    let effect = app.workspace_action(OpsUiAction::HideSquad);
    let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) = effect else {
        panic!("Hide must use the existing preference effect");
    };
    assert_eq!(expected, old);
    assert_eq!(
        app.workspace_snapshot().unwrap().preferences,
        old,
        "no optimistic save"
    );
    assert_eq!(app.workspace.selected, Some(OpsTarget::Squad(ROOM.into())));
    app.finished(Err("stale file".into()));
    assert_eq!(app.workspace_snapshot().unwrap().preferences, old);
    assert_eq!(app.workspace.selected, Some(OpsTarget::Squad(ROOM.into())));
    app.finished_workspace_preferences(next, "Saved".into());
    assert_eq!(app.workspace.selected, Some(OpsTarget::Pool));
    assert_eq!(app.workspace_snapshot().unwrap().hidden_count, 1);
    assert_eq!(app.workspace_snapshot().unwrap().shell_counts, global);
    assert_eq!(app.workspace_rows().len(), 1);
    app.shell.tab = OpsTab::Overview;
    app.workspace_reconcile();
    assert!(app.workspace_rows().is_empty());
    assert_eq!(
        crate::board::overview::monitor::scene(&app, app.workspace_snapshot().unwrap())
            .summary
            .total,
        0
    );
    app.workspace_action(OpsUiAction::RevealHidden);
    assert!(
        crate::board::overview::monitor::scene(&app, app.workspace_snapshot().unwrap()).waiting
            == 0,
        "revealing a squad does not manufacture waiting evidence"
    );
    assert_eq!(
        crate::board::overview::monitor::scene(&app, app.workspace_snapshot().unwrap())
            .summary
            .total,
        1
    );
    assert!(
        app.workspace_snapshot()
            .unwrap()
            .preferences
            .hidden
            .contains(ROOM)
    );
}

#[test]
fn scoped_hidden_squad_is_visible_and_escape_clear_is_carried_across_reload() {
    let mut app = board();
    let mut preferences = app.workspace_snapshot().unwrap().preferences.clone();
    preferences.hidden.insert(ROOM.into());
    app.finished_workspace_preferences(preferences, "Saved".into());
    app.workspace.visibility.scope = vec![ROOM.into()];
    app.workspace_startup = Some(ROOM.into());
    app.workspace_reconcile();
    assert_eq!(app.workspace.selected, Some(OpsTarget::Squad(ROOM.into())));
    assert!(
        !app.workspace_rows()
            .iter()
            .any(|row| row.target == OpsTarget::Pool)
    );
    let scope = app.resume().unwrap();
    assert_eq!(scope.visibility.scope, vec![ROOM.to_owned()]);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.workspace.visibility.scope.is_empty());
    assert_eq!(
        app.workspace.selected,
        Some(OpsTarget::Squad(ROOM.into())),
        "clear has no neighbor recipient authority"
    );
    assert!(!app.workspace_selection_visible());
    let cleared = app.resume().unwrap();
    let mut reloaded = board();
    reloaded.workspace_startup = Some(ROOM.into());
    reloaded.workspace.visibility.scope = vec![ROOM.into()];
    reloaded.carry(cleared);
    reloaded.restore_pending();
    reloaded.workspace_reconcile();
    assert!(reloaded.workspace.visibility.scope.is_empty());
    assert!(reloaded.workspace_startup.is_none());
}

#[test]
fn scope_escape_respects_search_overlay_editor_and_authored_binding() {
    let mut app = board();
    app.workspace.visibility.scope = vec![ROOM.into()];
    app.searching = true;
    app.search = "worker".into();
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.searching);
    assert_eq!(app.workspace.visibility.scope, vec![ROOM.to_owned()]);
    app.help = true;
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.help);
    assert_eq!(app.workspace.visibility.scope, vec![ROOM.to_owned()]);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.input.is_some());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.input.is_none());
    assert_eq!(app.workspace.visibility.scope, vec![ROOM.to_owned()]);
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot
        .keys
        .get_mut(&OpsKeyScope::Member)
        .unwrap()
        .insert("esc".into(), Action::parse("help").unwrap());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.help);
    assert_eq!(app.workspace.visibility.scope, vec![ROOM.to_owned()]);
}

#[test]
fn show_is_a_saved_removal_and_common_pool_has_no_hide_action() {
    let mut app = board();
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    app.workspace.visibility.reveal_hidden = true;
    let mut preferences = app.workspace_snapshot().unwrap().preferences.clone();
    preferences.hidden.insert(ROOM.into());
    preferences.pinned.insert(ROOM.into());
    app.finished_workspace_preferences(preferences, "Saved".into());
    let Effect::Act(Request::WorkspacePreferences { next, .. }) =
        app.workspace_action(OpsUiAction::ShowSquad)
    else {
        panic!("Show must save");
    };
    assert!(next.hidden.is_empty());
    assert!(next.pinned.contains(ROOM));
    app.workspace.selected = Some(OpsTarget::Pool);
    app.workspace_reconcile();
    assert!(!app.workspace_bindings().values().any(|action| matches!(
        action.verb,
        Verb::Ui(OpsUiAction::HideSquad | OpsUiAction::ShowSquad)
    )));
    assert_eq!(app.workspace_action(OpsUiAction::HideSquad), Effect::None);
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("Common pool cannot be hidden")
    );
}

#[test]
fn visibility_capture_matrix_keeps_shell_global_and_views_scoped() {
    let output = std::env::var("TMT_VISIBILITY_CAPTURE").ok();
    let mut captures = Vec::new();
    for (width, height) in [(160, 44), (100, 24), (60, 24)] {
        for (base, depth) in [
            (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
            (
                tmt_cli_style::Base::TmtLight,
                tmt_cli_style::Depth::TrueColor,
            ),
            (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
        ] {
            for tab in [OpsTab::Agents, OpsTab::Overview] {
                for mode in ["hidden", "scoped", "revealed"] {
                    let mut app = populated_board(2, 4);
                    let view = app.view.as_mut().unwrap();
                    view.look = crate::look::Look {
                        theme: tmt_cli_style::Theme::new(base),
                        depth,
                    };
                    let snapshot = &mut view.home.as_mut().unwrap().workspace;
                    snapshot.squad_looks.clear();
                    let hidden = snapshot.roster.squads[0].room_id.clone();
                    let other = snapshot.roster.squads[1].room_id.clone();
                    let mut preferences = snapshot.preferences.clone();
                    preferences.hidden.insert(hidden.clone());
                    // Deleted rooms remain in config but never inflate Hidden:N.
                    preferences
                        .hidden
                        .insert("99999999-9999-4999-8999-999999999999".into());
                    snapshot.set_preferences(preferences);
                    snapshot.shell_counts = snapshot.roster.shell_counts();
                    let global = snapshot.shell_counts.clone();
                    assert_eq!(global.blocked, 1);
                    app.workspace.visibility.scope = if mode == "scoped" {
                        vec![hidden.clone()]
                    } else {
                        vec![]
                    };
                    app.workspace.visibility.reveal_hidden = mode == "revealed";
                    app.workspace.selected = Some(OpsTarget::Squad(if mode == "scoped" {
                        hidden
                    } else {
                        other
                    }));
                    app.shell.tab = tab;
                    app.workspace_reconcile();
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| crate::board::view::render(frame, &app))
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let lines = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect::<Vec<_>>();
                    let text = lines.join("\n");
                    assert!(
                        text.contains(if mode == "scoped" {
                            "Scope: squad-00"
                        } else {
                            "Hidden: 1"
                        }),
                        "{width} {mode} {text}"
                    );
                    if mode == "hidden" {
                        assert!(text.contains("Hidden: 1 · ✗ 1 blocked · show"));
                        let x = lines[1].chars().position(|c| c == '✗').unwrap() as u16;
                        assert_eq!(
                            buffer[(x, 1)].fg,
                            app.look()
                                .role(tmt_cli_style::Role::Blocked)
                                .fg
                                .unwrap_or(ratatui::style::Color::Reset)
                        );
                    }
                    if mode == "scoped" {
                        assert!(text.contains("Esc clears"));
                        assert!(!text.contains("Hidden:"));
                    }
                    let rows = app.workspace_rows();
                    if mode == "hidden" {
                        assert!(rows.iter().all(|row| !matches!(&row.target,OpsTarget::Squad(id) if id.ends_with("000000000000"))));
                    }
                    if tab == OpsTab::Agents {
                        assert!(text.contains(if mode == "revealed" {
                            "Agents · 2 squads"
                        } else {
                            "Agents · 1 squad"
                        }));
                        if mode == "scoped" {
                            assert!(!text.contains("+ Common pool"));
                        }
                    }
                    if tab == OpsTab::Overview {
                        let scene = crate::board::overview::monitor::scene(
                            &app,
                            app.workspace_snapshot().unwrap(),
                        );
                        assert!(
                            scene.waiting == 0,
                            "these observed agents have no pending question"
                        );
                        assert_eq!(scene.summary.total, if mode == "revealed" { 4 } else { 2 });
                        assert_eq!(scene.squads.len(), if mode == "revealed" { 2 } else { 1 });
                        assert_eq!(scene.summary.needs_you, usize::from(mode != "hidden"));
                    }
                    assert_eq!(app.workspace_snapshot().unwrap().shell_counts, global);
                    let before = app.workspace.selected.clone();
                    let control = app.workspace_strip_hits.borrow().iter().find(|(_,choice)| matches!(choice,Choice::Action(action) if matches!(action.verb,Verb::Ui(OpsUiAction::ClearScope | OpsUiAction::RevealHidden)))).cloned();
                    if mode != "revealed" {
                        assert!(control.is_some());
                    }
                    captures.push(json!({"width":width,"height":height,"base":format!("{base:?}"),"depth":format!("{depth:?}"),"tab":format!("{tab:?}"),"mode":mode,"lines":lines,
                        "cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>(),
                        "selected":format!("{before:?}"),"hits":format!("{:?}",app.hits.borrow())}));
                    if let Some((area, choice)) = control {
                        let _ = choice;
                        app.mouse(
                            MouseEvent {
                                kind: MouseEventKind::Down(MouseButton::Left),
                                column: area.x,
                                row: area.y,
                                modifiers: KeyModifiers::NONE,
                            },
                            std::time::Instant::now(),
                        );
                        assert_eq!(app.workspace.selected, before);
                        assert!(
                            app.workspace_strip_hits.borrow().is_empty(),
                            "old geometry invalidated"
                        );
                        terminal
                            .draw(|frame| crate::board::view::render(frame, &app))
                            .unwrap();
                        assert!(!area.is_empty());
                    }
                }
            }
        }
    }
    assert_eq!(captures.len(), 54);
    if let Some(output) = output {
        std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
    }
}

#[test]
fn scope_header_fits_whole_names_and_neutralizes_untrusted_roster_names() {
    let mut app = populated_board(3, 3);
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let names = [
        "alpha-long-squad-name-one",
        "bravo-long-squad-name-two",
        "charlie-long-squad-three",
    ];
    for (squad, name) in snapshot.roster.squads.iter_mut().zip(names) {
        squad.name = name.into();
    }
    let ids: Vec<_> = snapshot
        .roster
        .squads
        .iter()
        .map(|squad| squad.room_id.clone())
        .collect();
    app.workspace.visibility.scope = vec![ids[2].clone(), ids[0].clone(), ids[1].clone()];
    let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let line = (0..60)
        .map(|x| terminal.backend().buffer()[(x, 1)].symbol())
        .collect::<String>();
    assert!(line.contains(names[2]));
    assert!(line.contains("+2 · Esc clears"));
    assert!(!line.contains("alpha"));
    assert!(!line.contains("bravo"));
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads[2]
        .name = "safe\u{1b}]52;c;secret\u{7}\u{202e}name".into();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let line = (0..60)
        .map(|x| terminal.backend().buffer()[(x, 1)].symbol())
        .collect::<String>();
    assert!(line.contains("Scope: safename"));
    assert!(!line.contains("secret"));
    assert!(!line.contains('\u{1b}'));
    assert!(!line.contains('\u{202e}'));
}

#[test]
fn cold_view_hints_cannot_set_or_clear_session_visibility() {
    let hint = crate::board::view_state::ViewState {
        controls: None,
        scrolls: Vec::new(),
        shell: OpsTab::Digest,
        expanded: Default::default(),
        pool_expanded: true,
        selected: Some(OpsTarget::Pool),
    };
    for explicit in [false, true] {
        let mut app = board();
        app.workspace.visibility.scope = vec![ROOM.into()];
        app.workspace.visibility.reveal_hidden = true;
        let visibility = app.workspace.visibility.clone();
        app.carry_view_hint(&hint, explicit);
        assert_eq!(app.workspace.visibility, visibility);
    }
}

#[test]
fn hidden_line_reports_both_attention_kinds_but_omits_absent_and_revealed_attention() {
    let mut app = board();
    app.shell.tab = OpsTab::Agents;
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.members.get_mut(MEMBER).unwrap().waiting = true;
    snapshot.roster.members.get_mut(MEMBER).unwrap().state = Some("blocked".into());
    let mut preferences = snapshot.preferences.clone();
    preferences.hidden.insert(ROOM.into());
    snapshot.set_preferences(preferences.clone());
    let paint = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
        (0..60)
            .map(|x| terminal.backend().buffer()[(x, 1)].symbol())
            .collect::<String>()
    };
    assert!(paint(&app).contains("Hidden: 1 · ◆ 1 waiting · ✗ 1 blocked · show"));
    app.workspace.visibility.reveal_hidden = true;
    let line = paint(&app);
    assert!(line.contains("Hidden: 1 · shown"));
    assert!(!line.contains("blocked"));
    assert!(!line.contains("waiting"));
    app.workspace.visibility.reveal_hidden = false;
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.members.get_mut(MEMBER).unwrap().waiting = false;
    snapshot.roster.members.get_mut(MEMBER).unwrap().state = Some("working".into());
    snapshot.set_preferences(preferences);
    let line = paint(&app);
    assert!(line.contains("Hidden: 1 · show"));
    assert!(!line.contains("blocked"));
    assert!(!line.contains("waiting"));
}

#[test]
fn nondefault_layer_caption_changes_only_the_header_and_never_overlaps_attention() {
    for width in [160, 100, 60] {
        for base in ["tmt", "tmt-light"] {
            for depth in [tmt_cli_style::Depth::TrueColor, tmt_cli_style::Depth::None] {
                let look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(
                        tmt_cli_style::theme::Base::parse(base).unwrap(),
                    ),
                    depth,
                };
                let default = capture_case("agents", look);
                let mut named = capture_case("agents", look);
                named.view_layer = crate::board::ViewLayer::parse("popup").unwrap();
                let draw = |app: &App| {
                    let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
                    terminal
                        .draw(|frame| crate::board::view::render(frame, app))
                        .unwrap();
                    terminal.backend().buffer().clone()
                };
                let ordinary = draw(&default);
                let popup = draw(&named);
                let line = |buffer: &ratatui::buffer::Buffer| {
                    buffer.content[..usize::from(width)]
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>()
                };
                let shown = line(&popup);
                assert!(shown.contains(if width == 60 {
                    "OPS / UI · p…"
                } else {
                    "OPS / UI · popup"
                }));
                assert!(
                    shown.contains("online")
                        && shown.contains("waiting")
                        && shown.contains("blocked")
                );
                assert!(!line(&ordinary).contains("· popup"));
                assert_eq!(
                    &ordinary.content[usize::from(width)..],
                    &popup.content[usize::from(width)..]
                );
                named.view_layer = crate::board::ViewLayer::parse(&"x".repeat(32)).unwrap();
                let long = line(&draw(&named));
                assert!(
                    long.contains("online") && long.contains("waiting") && long.contains("blocked")
                );
                if width >= 100 {
                    assert!(long.contains(&"x".repeat(32)));
                } else {
                    assert!(long.contains('…'));
                    assert!(!long.contains(&"x".repeat(32)));
                }
            }
        }
    }
}

#[test]
fn named_header_keeps_unknown_attention_before_the_long_layer_caption() {
    let app = capture_case("named-layer-long-partial", crate::look::Look::default());
    for width in [60, 100] {
        let line = paint(&app, width, 36)[0].clone();
        assert!(line.contains("online") && line.contains("waiting") && line.contains("blocked"));
        assert!(line.contains("totals unknown"));
        if width == 100 {
            assert!(line.contains("OPS / UI · ") && line.contains('…'));
        } else {
            assert!(!line.contains(" · x"));
        }
    }
}

#[test]
fn overview_retains_supplier_values_without_recreating_the_removed_mode_cell() {
    let look = crate::look::Look {
        theme: tmt_cli_style::Theme::default(),
        depth: tmt_cli_style::Depth::TrueColor,
    };
    let mut app = supplied_label_fixture(look);
    app.overview_view = crate::board::overview::monitor::View::Agents;
    app.overview_rows.borrow_mut().clear();
    app.workspace_reconcile();
    app.shell.tab = OpsTab::Overview;
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.members.get_mut(MEMBER).unwrap().waiting = true;
    snapshot.roster.squads[0]
        .member_states
        .get_mut(MEMBER)
        .unwrap()
        .waiting = true;
    let target = app.workspace.selected.as_ref().unwrap().clone();
    snapshot.rows.get_mut(&target).unwrap()["pending"] = json!("Approve this change?");
    app.overview_rows.borrow_mut().clear();
    app.workspace_reconcile();
    for width in [100, 160] {
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        let list = app
            .workspace_placement(app.conversation.borrow().area)
            .unwrap()
            .list
            .unwrap();
        assert!(
            app.chip_hits
                .borrow()
                .iter()
                .all(|(area, _)| area.intersection(list).is_empty()),
            "no retired table dropdown retains an authority"
        );
        let values = crate::board::overview::monitor::values(
            &app,
            app.workspace_snapshot().unwrap(),
            &app.workspace_rows()[0],
        );
        assert!(
            values
                .digest_labels
                .iter()
                .any(|label| label.text == "1 held")
        );
    }
}
