use super::*;
use crate::board::{
    app::workspace::tests::{MEMBER, board},
    inventory::Inventory,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::json;

fn remembered() -> App {
    let mut app = board();
    app.references = Inventory::from_listed(
        &[],
        &Ok(json!({"identities":[{
            "id":MEMBER,"name":"worker","driverSupport":"full",
            "location":{"host":"tmux","session":"work","window":"agents"},
            "resume":{"driver":"codex","session":"remembered-session","staleAtMs":null}
        }]})),
    );
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
        .activity
        .last_ms = Some(crate::status::now_ms() - 3_600_000);
    app
}
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.member_session_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap()
}
fn load(app: &mut App) -> Task {
    let Effect::MemberSession(task) = app.open_member_session() else {
        panic!("read task");
    };
    app.finished_member_session(Completed {
        task: task.clone(),
        result: Ok(Observed {
            executable: PathBuf::from("/opt/tmt bin/tmt"),
            preset: None,
        }),
    });
    task
}
fn paint(app: &App, width: u16, height: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(width, height)).unwrap();
    t.draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    t.backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[test]
fn absent_basic_stale_and_failed_observations_never_claim_resume() {
    let mut app = remembered();
    assert!(app.member_session_evidence(MEMBER).remembered());
    for value in [
        json!(null),
        json!({}),
        json!({"driverSupport":"basic","resume":{"driver":"codex","session":"id"}}),
        json!({"driverSupport":"full","resume":{"driver":"codex","session":"id","staleAtMs":123}}),
    ] {
        let mut value = value;
        if let Some(row) = value.as_object_mut() {
            row.insert("id".into(), json!(MEMBER));
            row.insert("name".into(), json!("worker"));
        }
        app.references = Inventory::from_listed(&[], &Ok(json!({"identities":[value]})));
        assert!(!app.member_session_evidence(MEMBER).remembered());
    }
    app = remembered();
    app.references.error = Some("read failed".into());
    assert!(!app.member_session_evidence(MEMBER).remembered());
}
#[test]
fn copy_and_cancel_keep_the_board_and_never_launch() {
    let mut app = remembered();
    let _no_commands = crate::runner::forbid_commands();
    load(&mut app);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    let text = paint(&app, 160, 36);
    assert!(text.contains("The board closes here"));
    let Effect::Act(Request::Copy { text, .. }) = key(&mut app, KeyCode::Char('c')) else {
        panic!("copy only");
    };
    assert_eq!(text, format!("'/opt/tmt bin/tmt' 'resume' '--' '{MEMBER}'"));
    assert!(app.foreground.is_none());
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.member_session.as_ref().unwrap().confirmation.is_none());
    assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.member_session.is_none());
}
#[test]
fn confirmation_requires_current_complete_paint_and_refuses_a_named_draft_or_effect() {
    let mut app = remembered();
    load(&mut app);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Effect::None,
        "no unpainted confirm"
    );
    paint(&app, 160, 36);
    app.in_flight = 1;
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(
        app.member_session
            .as_ref()
            .unwrap()
            .status
            .as_ref()
            .unwrap()
            .contains("pending action")
    );
    app.in_flight = 0;
    paint(&app, 160, 36);
    let Effect::Foreground(command) = key(&mut app, KeyCode::Enter) else {
        panic!("confirmed handoff");
    };
    assert_eq!(command.args, ["resume", "--", MEMBER]);
}
#[test]
fn reopen_selection_or_memory_change_fences_old_reads_and_confirmations() {
    let mut app = remembered();
    let Effect::MemberSession(old) = app.open_member_session() else {
        panic!()
    };
    key(&mut app, KeyCode::Esc);
    let Effect::MemberSession(new) = app.open_member_session() else {
        panic!()
    };
    app.finished_member_session(Completed {
        task: old,
        result: Ok(Observed {
            executable: "/wrong".into(),
            preset: None,
        }),
    });
    assert!(app.member_session.as_ref().unwrap().loaded.is_none());
    app.finished_member_session(Completed {
        task: new,
        result: Ok(Observed {
            executable: "/tmt".into(),
            preset: None,
        }),
    });
    key(&mut app, KeyCode::Enter);
    app.references.agents.get_mut(MEMBER).unwrap().parameters["resume"]["session"] =
        json!("new-session");
    key(&mut app, KeyCode::Down);
    assert!(app.member_session.as_ref().unwrap().confirmation.is_none());
    assert!(matches!(
        app.member_session.as_ref().unwrap().loaded,
        Some(Err(_))
    ));
}
#[test]
fn narrow_layout_drops_session_before_existing_tracks() {
    use crate::board::workspace::columns::{AgentsField, AgentsLayout};
    use ratatui::layout::Rect;
    let mut layout = AgentsLayout::default();
    let wide = layout.slots(Rect::new(0, 0, 120, 1));
    assert!(wide.iter().any(|(f, _)| *f == AgentsField::Session));
    let narrow = layout.slots(Rect::new(0, 0, 75, 1));
    assert!(!narrow.iter().any(|(f, _)| *f == AgentsField::Session));
    assert!(narrow.iter().any(|(f, _)| *f == AgentsField::Model));
}

pub(in crate::board) const SCENES: &[&str] = &[
    "session-loading",
    "session-remembered",
    "session-stale",
    "session-basic",
    "session-no-memory",
    "session-failed",
    "session-unknown-host",
    "session-resume",
    "session-move",
    "session-rows-flat",
    "session-rows-tree",
];
pub(in crate::board) fn capture_case(scene: &str, look: crate::look::Look) -> App {
    let mut app = remembered();
    app.view.as_mut().unwrap().look = look;
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    match scene {
        "session-stale" => {
            app.references.agents.get_mut(MEMBER).unwrap().parameters["resume"]["staleAtMs"] =
                json!(1)
        }
        "session-basic" => {
            app.references.agents.get_mut(MEMBER).unwrap().parameters["driverSupport"] =
                json!("basic")
        }
        "session-no-memory" => app
            .references
            .agents
            .get_mut(MEMBER)
            .unwrap()
            .parameters
            .as_object_mut()
            .unwrap()
            .remove("resume")
            .map(|_| ())
            .unwrap(),
        "session-unknown-host" => {
            app.references
                .agents
                .get_mut(MEMBER)
                .unwrap()
                .parameters
                .as_object_mut()
                .unwrap()
                .remove("location");
        }
        _ => {}
    }
    if scene == "session-stale" {
        app.references.agents.get_mut(MEMBER).unwrap().parameters["location"]["host"] =
            json!("herdr");
    }
    if scene == "session-unknown-host" {
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
            .activity
            .last_ms = None;
    }
    if scene.starts_with("session-rows-") {
        let mut policy = crate::config::OverviewPanels::default();
        policy.set_side_shown(false).unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences
            .overview_panels = Some(crate::config::PanelSetting::configured(policy));
        if scene == "session-rows-flat" {
            app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
        }
        return app;
    }
    if scene == "session-loading" {
        app.open_member_session();
    } else {
        load(&mut app);
        if scene == "session-failed" {
            app.member_session.as_mut().unwrap().loaded = Some(Err(SquadError::new(
                "READ_FAILED",
                "Public session read failed; no launch attempted.",
            )));
        } else if scene == "session-resume" || scene == "session-move" {
            if scene == "session-move" {
                key(&mut app, KeyCode::Down);
            }
            key(&mut app, KeyCode::Enter);
        }
    }
    app
}

pub(in crate::board) fn capture_metadata(app: &App) -> serde_json::Value {
    let c = app.member_session.as_ref().unwrap();
    let ui = c.ui.borrow();
    let map = ui.frame.as_ref().unwrap();
    json!({"hits":map.hits.iter().map(|hit| json!({"id":hit.id,"row":hit.row_id,"rect":[hit.rect.x,hit.rect.y,hit.rect.width,hit.rect.height]})).collect::<Vec<_>>(),
        "list":format!("{:?}", map.list),"selected":ui.picker.list.selected(),"confirmationReadable":c.readable.get()})
}

#[test]
fn public_show_runs_on_the_lane_while_loading_paints_and_cancel_fences_its_result() {
    use std::{os::unix::net::UnixDatagram, sync::mpsc::channel, time::Duration};
    let root = std::env::temp_dir().join(format!("ops-session-{}", crate::id::new_v4().unwrap()));
    std::fs::create_dir(&root).unwrap();
    struct Root(PathBuf);
    impl Drop for Root {
        fn drop(&mut self) {
            if self.0.exists() {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }
    let _cleanup = Root(root.clone());
    let gate = UnixDatagram::bind(root.join("control")).unwrap();
    gate.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let driver = root.join("driver.py");
    std::fs::write(
        &driver,
        r#"import json, pathlib, socket, sys
root=pathlib.Path(__file__).parent
(root/'args').write_text(json.dumps(sys.argv[1:]))
s=socket.socket(socket.AF_UNIX,socket.SOCK_DGRAM)
s.bind(str(root/'reply'))
s.settimeout(4)
s.sendto(b'ready',str(root/'control'))
s.recv(32)
print(json.dumps({'identity':'worker','launch':None}))
"#,
    )
    .unwrap();
    let executable = root.join("core");
    crate::test_support::write_ready_executable(
        &executable,
        &format!(
            "#!/bin/sh\nexec /usr/bin/python3 '{}' \"$@\"\n",
            driver.display()
        ),
    );
    let (events, input) = channel();
    let lane = crate::board::lane::Lane::spawn(Core::at(executable), None, events);
    let mut app = remembered();
    let Effect::MemberSession(task) = app.open_member_session() else {
        panic!()
    };
    lane.submit(crate::board::lane::Job::MemberSession(task));
    let mut bytes = [0; 32];
    let n = gate.recv(&mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"ready");
    {
        let _ui = crate::runner::forbid_commands();
        assert!(paint(&app, 160, 36).contains("Loading session"));
        key(&mut app, KeyCode::Esc);
        assert!(app.member_session.is_none());
    }
    gate.send_to(b"release", root.join("reply")).unwrap();
    let crate::board::BoardEvent::MemberSession(completed) =
        input.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!()
    };
    app.finished_member_session(completed);
    assert!(
        app.member_session.is_none(),
        "cancelled read cannot reopen or launch"
    );
    assert!(app.foreground.is_none());
    let args: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("args")).unwrap()).unwrap();
    assert_eq!(args, json!(["resume", "--show", "--", MEMBER]));
    lane.finish();
    drop(gate);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_kept_conversation_draft_is_named_and_never_lost_to_handoff() {
    let mut app = remembered();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.paste("kept exact draft");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.composer.has_drafts());
    load(&mut app);
    key(&mut app, KeyCode::Enter);
    paint(&app, 160, 36);
    key(&mut app, KeyCode::Down);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(
        app.member_session
            .as_ref()
            .unwrap()
            .status
            .as_ref()
            .unwrap()
            .contains("conversation draft")
    );
    assert!(app.composer.has_drafts());
    assert!(app.foreground.is_none());
}
#[test]
fn missing_freshness_or_changed_target_never_admits_the_held_confirmation() {
    let mut app = remembered();
    app.references.agents.get_mut(MEMBER).unwrap().parameters["resume"]
        .as_object_mut()
        .unwrap()
        .remove("staleAtMs");
    assert!(!app.member_session_evidence(MEMBER).remembered());
    app = remembered();
    load(&mut app);
    key(&mut app, KeyCode::Enter);
    paint(&app, 160, 36);
    app.workspace.selected = None;
    key(&mut app, KeyCode::Down);
    assert!(app.member_session.as_ref().unwrap().confirmation.is_none());
    assert!(app.foreground.is_none());
}
#[test]
fn a_review_starts_on_cancel_and_tiny_frames_never_enable_the_handoff() {
    let mut app = remembered();
    load(&mut app);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.member_session
            .as_ref()
            .unwrap()
            .ui
            .borrow()
            .picker
            .list
            .selected(),
        Some("cancel")
    );
    let text = paint(&app, 60, 8);
    assert!(text.contains("enlarge to continue"));
    key(&mut app, KeyCode::Down);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(app.member_session.as_ref().unwrap().confirmation.is_some());
    assert!(app.foreground.is_none());
}
#[test]
fn copied_shell_metacharacters_remain_one_literal_word_per_argument() {
    let command = Foreground::new(PathBuf::from("/opt/tmt's bin/$()tmt"), MEMBER, true);
    assert_eq!(
        command.line(),
        format!("'/opt/tmt'\\''s bin/$()tmt' 'resume' '--move' '--' '{MEMBER}'")
    );
    assert_eq!(command.args, ["resume", "--move", "--", MEMBER]);
}

#[test]
fn flat_token_click_follows_the_painted_cell_when_session_is_admitted_or_dropped() {
    use crate::board::{app::Choice, shell::OpsTab, workspace::AgentsFilter};
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use std::time::Instant;
    for (width, has_session) in [(110, true), (82, false)] {
        let mut app = remembered();
        app.shell.tab = OpsTab::Agents;
        app.workspace.filter = AgentsFilter::Agents;
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let lines: Vec<String> = (0..36)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let header = lines
            .iter()
            .find(|line| line.contains("STATE") && line.contains("TOK 1h"))
            .unwrap();
        assert_eq!(header.contains("HOST / SESSION"), has_session);
        let unit_at = header.find("TOK 1h").unwrap();
        // The last heading cell and the right-aligned figure share this column.
        let figure_x = unicode_width::UnicodeWidthStr::width(&header[..unit_at]) as u16 + 5;
        let row_y = lines
            .iter()
            .position(|line| line.contains("worker") && line.contains("product"))
            .unwrap() as u16;
        assert_eq!(buffer[(figure_x, row_y)].symbol(), "—");
        let hit = app
            .workspace_strip_hits
            .borrow()
            .iter()
            .find_map(|(area, choice)| {
                (*choice == Choice::TokMoney && area.y == row_y).then_some(*area)
            })
            .unwrap();
        assert_eq!(
            hit.x + hit.width - 1,
            figure_x,
            "{width}: row hit must end at its painted figure"
        );
        let click = |app: &mut App, x| {
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: x,
                    row: row_y,
                    modifiers: KeyModifiers::NONE,
                },
                Instant::now(),
            )
        };
        if has_session {
            let row = &lines[usize::from(row_y)];
            let at = row.find("tmux · remembered").unwrap();
            let session_x = unicode_width::UnicodeWidthStr::width(&row[..at]) as u16;
            assert!(
                !matches!(
                    click(&mut app, session_x),
                    Effect::Act(Request::WorkspacePreferences { .. })
                ),
                "Session is not a token-unit control"
            );
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
        }
        let Effect::Act(Request::WorkspacePreferences { expected, next, .. }) =
            click(&mut app, figure_x)
        else {
            panic!("{width}: clicking the painted token figure must switch its unit");
        };
        assert_ne!(expected.savings_unit, next.savings_unit);
        let mut rest = next;
        rest.savings_unit = expected.savings_unit;
        assert_eq!(rest, expected, "only the unit changes");
    }
}
