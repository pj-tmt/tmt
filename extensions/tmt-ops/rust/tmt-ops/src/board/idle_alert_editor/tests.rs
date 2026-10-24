pub(in crate::board) mod captures;
use super::*;
use crate::board::app::workspace::tests::board;
#[test]
fn common_pool_never_opens_a_setting_or_starts_io() {
    let _commands = crate::runner::forbid_commands();
    let mut app = board();
    app.workspace.selected = Some(OpsTarget::Pool);
    assert_eq!(app.open_idle_alert(None), Effect::None);
    assert!(app.idle_alert_editor.is_none());
}
use crate::board::app::workspace::tests::{MEMBER, ROOM, configured_member};
use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
fn config() -> Config {
    Config::read(
        std::env::temp_dir().join(format!("idle-editor-{}.toml", crate::id::new_v4().unwrap())),
    )
    .unwrap()
}
fn form() -> App {
    let mut app = board();
    let Effect::IdleAlert(task) = app.open_idle_alert(None) else {
        panic!("member must resolve its squad")
    };
    app.finished_idle_alert(Completed {
        task,
        result: Ok(Box::new(config())),
        recovered: None,
    });
    app
}
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.overlay_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap()
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
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[test]
fn no_tab_arrows_presets_typed_value_cancel_and_invalid_save() {
    let mut app = form();
    let _commands = crate::runner::forbid_commands();
    assert_eq!(
        app.idle_alert_editor.as_ref().unwrap().after.as_str(),
        "30m"
    );
    app.idle_alert_editor.as_mut().unwrap().notice = Some("✓ Saved · squad idle alert".into());
    key(&mut app, KeyCode::Right);
    assert!(app.idle_alert_editor.as_ref().unwrap().enabled);
    assert!(app.idle_alert_editor.as_ref().unwrap().notice.is_none());
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    assert_eq!(
        app.idle_alert_editor.as_ref().unwrap().after.as_str(),
        "15m"
    );
    let c = app.idle_alert_editor.as_mut().unwrap();
    c.after = Editor::from("90x");
    let screen = paint(&app, 160, 36);
    assert!(screen.contains(INVALID), "{screen}");
    key(&mut app, KeyCode::Down);
    paint(&app, 160, 36);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(!app.idle_alert_editor.as_ref().unwrap().pending);
    key(&mut app, KeyCode::Esc);
    assert!(app.idle_alert_editor.is_none());
}
#[test]
fn unchanged_duration_rules_are_used_even_when_alert_is_off() {
    let mut app = form();
    let c = app.idle_alert_editor.as_mut().unwrap();
    for text in ["5m", "15m", "20m", "2h", "24h"] {
        c.after = Editor::from(text);
        assert!(c.draft().is_ok(), "{text}");
    }
    for text in ["4m", "25h", "1h30m", "90x", ""] {
        c.after = Editor::from(text);
        assert_eq!(c.draft().unwrap_err(), INVALID, "{text}");
    }
}
#[test]
fn default_i_is_agents_list_only_and_authored_i_wins() {
    let mut app = board();
    assert_eq!(app.workspace_bindings()["i"].text, "idle-alert");
    app.focus = 1;
    assert!(!app.workspace_bindings().contains_key("i"));
    let app = configured_member(
        "[bind]\ni='copy {name}'\n",
        serde_json::json!({"id":MEMBER,"name":"worker"}),
    );
    assert_eq!(app.workspace_bindings()["i"].text, "copy {name}");
}
#[test]
fn stale_completion_is_ignored_and_new_draft_survives_partial_save() {
    let mut app = form();
    let old = app.idle_alert_editor.as_ref().unwrap().task();
    key(&mut app, KeyCode::Esc);
    app.open_idle_alert(None);
    app.finished_idle_alert(Completed {
        task: old,
        result: Ok(Box::new(config())),
        recovered: None,
    });
    assert!(app.idle_alert_editor.as_ref().unwrap().config.is_none());
    let mut app = form();
    let c = app.idle_alert_editor.as_mut().unwrap();
    c.enabled = true;
    c.after = Editor::from("45m");
    let task = c.task();
    app.finished_idle_alert(Completed {
        task,
        result: Err(crate::core::SquadError::new(
            "PARTIAL",
            "After saved, Alert not changed",
        )),
        recovered: Some(Box::new(config())),
    });
    let c = app.idle_alert_editor.as_ref().unwrap();
    assert_eq!(c.after.as_str(), "45m");
    assert!(c.enabled);
    assert_eq!(
        c.notice.as_deref(),
        Some("✗ Refused · After saved, Alert not changed")
    );
}
#[test]
fn flat_multi_squad_member_chooses_explicitly_and_scoped_member_does_not() {
    let mut app = board();
    let roster = &mut app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster;
    let s = &roster.squads[0];
    roster.squads.push(crate::board::workspace::OpsSquad {
        room_id: "99999999-9999-4999-8999-999999999999".into(),
        name: "other".into(),
        members: s.members.clone(),
        lead: None,
        member_states: s.member_states.clone(),
        observation: s.observation,
        idle_alert: s.idle_alert,
    });
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    app.workspace.selected = Some(OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: MEMBER.into(),
    });
    app.workspace_reconcile();
    assert_eq!(app.open_idle_alert(None), Effect::None);
    assert_eq!(app.menu.as_ref().unwrap().entries.len(), 2);
    assert!(app.idle_alert_editor.is_none());
    assert!(matches!(
        app.open_idle_alert(Some(ROOM.into())),
        Effect::IdleAlert(_)
    ));
}
#[test]
fn form_geometry_never_admits_a_clipped_save_or_retains_old_hits() {
    let app = form();
    let _commands = crate::runner::forbid_commands();
    for width in [160, 100, 80, 60] {
        let screen = paint(&app, width, 36);
        assert!(screen.contains("Idle alert · product"), "{width}: {screen}");
        assert!(app.idle_alert_editor.as_ref().unwrap().readable.get());
    }
    paint(&app, 60, 8);
    let c = app.idle_alert_editor.as_ref().unwrap();
    assert!(!c.readable.get());
    assert!(
        !c.hits
            .borrow()
            .iter()
            .any(|(_, c)| matches!(c, Control::Save))
    );
}
struct Fixture {
    root: std::path::PathBuf,
    core: crate::core::Core,
}
impl Fixture {
    fn new(text: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("ops-idle-dialog-{}", crate::id::new_v4().unwrap()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("ops.toml"), text).unwrap();
        let script = root.join("core.py");
        std::fs::write(&script,r#"import json,pathlib,sys
r=pathlib.Path(__file__).parent
a=sys.argv[1:]
if a[:2]==['room','show']:
 print(json.dumps({'room':{'id':a[2],'name':'squad-product','retired':(r/'retired').exists()}}))
elif a[:2]==['config','show']:print(json.dumps({'paths':{'global':str(r/'config.json')},'resolved':{}}))
elif a[:1]==['api']:json.load(sys.stdin);print(json.dumps({'dataRoot':str(r/'data')}))
else:print(json.dumps({'error':{'code':'UNEXPECTED','message':str(a)}}));sys.exit(1)
"#).unwrap();
        let executable = root.join("core");
        crate::test_support::write_ready_executable(
            &executable,
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 {} \"$@\"\n",
                crate::launch::shell_word(script.to_str().unwrap())
            ),
        );
        Self {
            root,
            core: crate::core::Core::at(executable),
        }
    }
    fn task(&self, enabled: bool, after: &str) -> Task {
        Task {
            owner: 1,
            room: ROOM.into(),
            name: "product".into(),
            save: Some((
                Box::new(Config::read(self.root.join("ops.toml")).unwrap()),
                enabled,
                after.into(),
            )),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn lane_save_persists_the_same_settings_and_invalid_or_changed_file_preserves_bytes() {
    let f = Fixture::new(
        "# keep this comment\n[squad.product.idle_alert]\nenabled = false\nafter = '30m'\n",
    );
    let original = std::fs::read(f.root.join("ops.toml")).unwrap();
    let bad = complete(&f.core, f.task(true, "1h30m"));
    assert!(bad.result.is_err());
    assert_eq!(std::fs::read(f.root.join("ops.toml")).unwrap(), original);
    let stale = f.task(true, "2h");
    std::fs::write(
        f.root.join("ops.toml"),
        [original.as_slice(), b"# concurrent edit\n"].concat(),
    )
    .unwrap();
    let concurrent = std::fs::read(f.root.join("ops.toml")).unwrap();
    assert!(complete(&f.core, stale).result.is_err());
    assert_eq!(std::fs::read(f.root.join("ops.toml")).unwrap(), concurrent);
    let saved = complete(&f.core, f.task(true, "2h")).result.unwrap();
    assert!(saved.idle_alert("product").unwrap().enabled);
    let fresh = Config::read(f.root.join("ops.toml"))
        .unwrap()
        .idle_alert("product")
        .unwrap();
    assert!(fresh.enabled);
    assert_eq!(fresh.after.as_secs(), 7200);
    assert!(
        std::fs::read_to_string(f.root.join("ops.toml"))
            .unwrap()
            .contains("# concurrent edit")
    );
    std::fs::write(f.root.join("retired"), "").unwrap();
    let before = std::fs::read(f.root.join("ops.toml")).unwrap();
    assert!(complete(&f.core, f.task(false, "15m")).result.is_err());
    assert_eq!(std::fs::read(f.root.join("ops.toml")).unwrap(), before);
}
#[test]
fn second_write_failure_reports_after_saved_rereads_and_never_claims_saved() {
    // First write grows 5m -> 30m by one byte to exactly the file limit.
    // Second grows true -> false by one more byte and is refused by the real writer.
    let mut text = "[squad.product.idle_alert]\nenabled = true\nafter = \"5m\"\n#".to_owned();
    text.push_str(&"x".repeat(1024 * 1024 - 1 - text.len()));
    let f = Fixture::new(&text);
    let done = complete(&f.core, f.task(false, "30m"));
    assert_eq!(
        done.result.err().unwrap().message,
        "After saved, Alert not changed"
    );
    let saved = done.recovered.unwrap().idle_alert("product").unwrap();
    assert!(saved.enabled);
    assert_eq!(saved.after.as_secs(), 1800);
    assert_eq!(
        std::fs::metadata(f.root.join("ops.toml")).unwrap().len(),
        1024 * 1024
    );
}

#[test]
fn compact_cell_hover_matches_header_highlight_and_click_targets_its_squad() {
    use ratatui::crossterm::event::MouseEvent;
    let mut app = board();
    app.view.as_mut().unwrap().look = crate::look::Look::default();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads[0]
        .idle_alert = Some(crate::board::workspace::IdleAlertSetting {
        enabled: true,
        after_ms: 1_800_000,
    });
    let _commands = crate::runner::forbid_commands();
    let screen = paint(&app, 160, 36);
    assert!(screen.contains("MODEL / ALERT"));
    assert!(screen.contains("◷ 30m"));
    let area = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find_map(|(r, c)| matches!(c,Choice::IdleAlert(room) if room==ROOM).then_some(*r))
        .unwrap();
    let pointer = MouseEvent {
        kind: MouseEventKind::Moved,
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    };
    app.mouse(pointer, std::time::Instant::now());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 36)).unwrap();
    terminal
        .draw(|f| crate::board::view::render(f, &app))
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(area.x, area.y)].fg,
        app.look().role(tmt_cli_style::Role::Accent).fg.unwrap()
    );
    let event = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        ..pointer
    };
    let Effect::IdleAlert(task) = app.mouse(event, std::time::Instant::now()) else {
        panic!("painted cell must open its squad")
    };
    assert_eq!(task.room, ROOM);
    assert_eq!(task.name, "product");
}

#[test]
fn threshold_preview_uses_held_over_marks_and_unknown_never_becomes_zero() {
    let mut app = captures::case("idle-editor-form", crate::look::Look::default());
    let _commands = crate::runner::forbid_commands();
    let c = app.idle_alert_editor.as_ref().unwrap();
    assert!(
        c.impact(&app)
            .join(" ")
            .contains("2 alerted now: *lead (35m), worker-3 (32m)")
    );
    app.idle_alert_editor.as_mut().unwrap().after = Editor::from("45m");
    assert!(
        app.idle_alert_editor
            .as_ref()
            .unwrap()
            .impact(&app)
            .join(" ")
            .contains("*lead (35m), worker-3 (32m) no longer count as over")
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
        .member_states
        .values_mut()
        .for_each(|state| state.idle_over = false);
    let c = app.idle_alert_editor.as_ref().unwrap();
    assert!(!c.impact(&app).join(" ").contains("worker-3"));
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads[0]
        .observation = crate::board::workspace::OpsObservation::Unavailable;
    let text = app
        .idle_alert_editor
        .as_ref()
        .unwrap()
        .impact(&app)
        .join(" ");
    assert!(text.contains("Current activity unavailable"));
    assert!(!text.contains("no members alerted"));
}

#[test]
fn loading_can_be_cancelled_and_late_read_cannot_reopen_it() {
    let mut app = board();
    let Effect::IdleAlert(task) = app.open_idle_alert(None) else {
        panic!()
    };
    key(&mut app, KeyCode::Esc);
    assert!(app.idle_alert_editor.is_none());
    app.finished_idle_alert(Completed {
        task,
        result: Ok(Box::new(config())),
        recovered: None,
    });
    assert!(app.idle_alert_editor.is_none());
}

#[test]
fn enter_saves_from_after_and_pointer_places_the_shared_caret() {
    let mut app = form();
    let _commands = crate::runner::forbid_commands();
    paint(&app, 160, 36);
    let area = app
        .idle_alert_editor
        .as_ref()
        .unwrap()
        .hits
        .borrow()
        .iter()
        .find_map(|(r, c)| matches!(c, Control::After).then_some(*r))
        .unwrap();
    let event = Event::Mouse(ratatui::crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x + 1,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    });
    app.overlay_event(&event);
    key(&mut app, KeyCode::Char('1'));
    assert_eq!(
        app.idle_alert_editor.as_ref().unwrap().after.as_str(),
        "130m"
    );
    paint(&app, 160, 36);
    let Effect::IdleAlert(task) = key(&mut app, KeyCode::Enter) else {
        panic!("Enter saves from the After row")
    };
    assert_eq!(task.save.as_ref().unwrap().2, "130m");
}
