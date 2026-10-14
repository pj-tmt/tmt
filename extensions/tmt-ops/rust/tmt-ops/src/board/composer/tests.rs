use super::*;
use crate::board::app::tests::crew;
use serde_json::json;

fn app() -> App {
    let mut app = crew(Default::default());
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().me_id = Some("ben-id".into());
    app
}
fn open(app: &mut App, name: &str) {
    let index = app
        .rows()
        .iter()
        .position(|(_, row)| row["name"] == name)
        .unwrap();
    let target = app.row_target(index).unwrap();
    let squad = app.target_squad(&target).unwrap();
    let send = app.send_target(target, true).unwrap();
    app.compose_row(send, crate::action::Verb::Talk, squad);
}
fn close(app: &mut App) {
    app.remember_draft();
    app.input = None;
}

#[test]
fn switching_recipients_retains_independent_session_drafts() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "first\nmessage".into();
    open(&mut app, "docs");
    assert!(app.input.as_ref().unwrap().text.is_empty());
    app.input.as_mut().unwrap().text = "second".into();
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text, "first\nmessage");
    open(&mut app, "docs");
    assert_eq!(app.input.as_ref().unwrap().text, "second");
}

#[test]
fn refresh_cannot_move_a_draft_to_a_new_sender_or_reused_name() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "private draft".into();
    app.view.as_mut().unwrap().me = Some("Other".into());
    app.view.as_mut().unwrap().me_id = Some("other-id".into());
    close(&mut app);
    open(&mut app, "auth-fix");
    assert!(app.input.as_ref().unwrap().text.is_empty());
    close(&mut app);
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().me_id = Some("ben-id".into());
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] = json!("replacement");
    open(&mut app, "auth-fix");
    assert!(app.input.as_ref().unwrap().text.is_empty());
    close(&mut app);
    let row = &mut app.view.as_mut().unwrap().document["sections"][0]["rows"][0];
    row["id"] = json!("auth-fix");
    row["name"] = json!("renamed");
    open(&mut app, "renamed");
    assert_eq!(app.input.as_ref().unwrap().text, "private draft");
}

#[test]
fn failure_retains_and_success_clears_only_the_submitted_revision() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "submitted".into();
    app.remember_draft();
    let input = app.input.take().unwrap();
    app.sending_draft(&input);
    app.finish_draft(false);
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text, "submitted");
    app.sending_draft(&input);
    app.input.as_mut().unwrap().text = "new revision".into();
    close(&mut app);
    app.finish_draft(true);
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text, "new revision");
    app.remember_draft();
    let input = app.input.take().unwrap();
    app.sending_draft(&input);
    app.finish_draft(true);
    open(&mut app, "auth-fix");
    assert!(app.input.as_ref().unwrap().text.is_empty());
}

#[test]
fn bracketed_paste_is_exact_folded_and_never_submits() {
    use crate::board::app::{Effect, Request};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = app();
    open(&mut app, "auth-fix");
    let text = format!("  \r\n{}\n\x1b[31m\n  ", "x".repeat(100 * 1024));
    assert_eq!(app.paste(&text), Effect::None);
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), text);
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
    assert!(app.input.as_ref().unwrap().text.layout(80).lines.len() < 3);
    close(&mut app);
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(Request::Talk { text: sent, to, .. }) if sent == text && to == "auth-fix")
    );
}

#[test]
fn closed_nonempty_drafts_hold_automatic_reload_until_explicitly_cleared() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "unfinished".into();
    close(&mut app);
    assert!(app.reload_blocked());
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text.clear();
    close(&mut app);
    assert!(!app.composer.has_drafts());
}

#[test]
fn unrelated_action_feedback_does_not_clear_a_pending_message() {
    let mut app = app();
    open(&mut app, "auth-fix");
    app.input.as_mut().unwrap().text = "unfinished".into();
    app.remember_draft();
    let input = app.input.take().unwrap();
    app.sending_draft(&input);
    app.finished(Ok("Configuration saved".into()));
    assert!(app.composer.pending.is_some());
    assert!(app.composer.has_drafts());
    app.finish_draft(false);
    open(&mut app, "auth-fix");
    assert_eq!(app.input.as_ref().unwrap().text, "unfinished");
}

fn draw(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn agents_composer_is_visible_idle_and_escape_keeps_the_uuid_draft_at_every_width() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    for width in [160, 100, 60] {
        let mut app = crate::board::app::workspace::tests::board();
        let buffer = draw(&mut app, width, 18);
        let painted: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(
            painted.contains("Message → worker") && painted.contains("Write a message…"),
            "{width}: {painted}"
        );
        assert!(painted.contains("│ › Write a message…"));
        assert!(painted.contains("[ Send ] │"));
        assert!(app.input.is_none());
        assert!(app.input_band.get().unwrap().height >= 1);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let key = app.input.as_ref().unwrap().draft_key.clone().unwrap();
        assert_eq!(key.1, crate::board::app::workspace::tests::MEMBER);
        app.paste("retained exact draft");
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.input.is_none());
        assert_eq!(app.focused(), crate::config::Pane::Rows);
        let buffer = draw(&mut app, width, 18);
        let painted: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(
            painted.contains("retained exact draft"),
            "{width}: {painted}"
        );
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(app.input.is_none(), "idle list keys never edit the draft");
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.input.as_ref().unwrap().draft_key.as_ref(), Some(&key));
        assert_eq!(
            app.input.as_ref().unwrap().text.as_str(),
            "retained exact draft"
        );
    }
}

#[test]
fn clicking_idle_composer_focuses_and_send_keeps_an_empty_editor_ready() {
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    for width in [160, 100, 60] {
        let mut app = crate::board::app::workspace::tests::board();
        draw(&mut app, width, 18);
        let (area, _) = app.input_viewport.get().unwrap();
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now()
            ),
            crate::board::app::Effect::None
        );
        assert!(app.input.is_some());
        app.paste("a real outgoing message");
        assert!(
            matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            crate::board::app::Effect::Act(crate::board::app::Request::Talk { text, to, .. })
                if text == "a real outgoing message" && to == "worker")
        );
        assert!(app.input.as_ref().unwrap().text.is_empty());
        draw(&mut app, width, 18);
        assert!(app.input_viewport.get().is_some());
        app.finish_draft(false);
        assert_eq!(
            app.input.as_ref().unwrap().text.as_str(),
            "a real outgoing message"
        );
    }
}

#[test]
fn boxed_send_is_clickable_only_on_its_current_whole_uuid_fenced_button() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    for width in [160, 100, 60] {
        let mut app = crate::board::app::workspace::tests::board();
        draw(&mut app, width, 18);
        let hit = app.composer.send_hit.borrow().clone().unwrap();
        app.key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Enter,
            KeyModifiers::NONE,
        ));
        app.paste("explicit button text");
        draw(&mut app, width, 18);
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.area.x,
            row: hit.area.y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(
            matches!(app.mouse(click, std::time::Instant::now()), crate::board::app::Effect::Act(crate::board::app::Request::Talk { text, .. }) if text == "explicit button text")
        );
        app.in_flight = 1;
        draw(&mut app, width, 18);
        assert!(
            app.composer.send_hit.borrow().is_none(),
            "in-flight button has no authority"
        );
        app.invalidate_frame_geometry();
        assert!(app.composer.send_hit.borrow().is_none());
    }
}

#[test]
fn low_composer_keeps_a_bordered_editable_row_and_clipped_buttons_have_no_hit() {
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    for width in [1, 2, 3, 10, 14, 20] {
        for height in [1, 2, 3, 4] {
            let app = crate::board::app::workspace::tests::board();
            let band = Rect::new(0, 0, width, height);
            app.input_band.set(Some(band));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::board::view::waiting::inline_prompt(frame, &app, band))
                .unwrap();
            let editor = app.input_viewport.get().unwrap().0;
            assert!(!editor.is_empty());
            assert_eq!(editor.intersection(band), editor);
            if width < 16 {
                assert!(app.composer.send_hit.borrow().is_none());
            }
            if let Some(hit) = app.composer.send_hit.borrow().as_ref() {
                assert_eq!(hit.area.width, 8);
                assert_eq!(hit.area.intersection(band), hit.area);
            }
        }
    }
}

#[test]
fn first_enter_focuses_retained_uuid_draft_and_only_a_new_active_press_submits() {
    use crate::board::app::{Effect, Request};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    for width in [160, 100, 60] {
        let mut app = crate::board::app::workspace::tests::board();
        draw(&mut app, width, 18);
        assert!(!app.bindings().contains_key("t"));
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
            Effect::None
        );
        assert!(app.input.is_none());
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Effect::None
        );
        let key = app.input.as_ref().unwrap().draft_key.clone().unwrap();
        assert_eq!(key.1, crate::board::app::workspace::tests::MEMBER);
        assert_eq!(key.0, app.view.as_ref().unwrap().me_id.as_deref().unwrap());
        app.paste("retained 漢字 e\u{301}");
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Effect::None
        );
        assert_eq!(
            app.input.as_ref().unwrap().text.as_str(),
            "retained 漢字 e\u{301}"
        );
        for kind in [
            KeyEventKind::Repeat,
            KeyEventKind::Repeat,
            KeyEventKind::Release,
        ] {
            assert_eq!(
                app.key(KeyEvent {
                    kind,
                    ..KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
                }),
                Effect::None
            );
            assert_eq!(
                app.input.as_ref().unwrap().text.as_str(),
                "retained 漢字 e\u{301}"
            );
        }
        assert!(
            matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(Request::Talk { to, text, .. }) if to == "worker" && text == "retained 漢字 e\u{301}")
        );
        app.in_flight = 1;
        for _ in 0..3 {
            assert_eq!(
                app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                Effect::None
            );
        }
        assert!(app.input.as_ref().unwrap().text.is_empty());
    }
}

#[test]
fn enter_shift_enter_and_paste_keep_open_edit_and_submit_distinct() {
    use crate::board::app::{Effect, Request};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::board::app::workspace::tests::board();
    draw(&mut app, 100, 36);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None,
        "empty active Enter never sends"
    );
    app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
    assert_eq!(app.paste("漢字 e\u{301}\ncommitted IME"), Effect::None);
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        "t\n漢字 e\u{301}\ncommitted IME"
    );
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(Request::Talk { text, .. }) if text == "t\n漢字 e\u{301}\ncommitted IME")
    );
}

#[test]
fn scoped_enter_preserves_group_inspect_and_explicit_keys_in_help_and_hints() {
    use crate::{board::app::Effect, config::OpsKeyScope};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    for scope in [
        OpsKeyScope::Member,
        OpsKeyScope::Overview,
        OpsKeyScope::Reader,
    ] {
        let keys = crate::config::workspace_defaults(scope);
        assert_eq!(keys["enter"].verb, crate::action::Verb::Talk);
        assert!(!keys.contains_key("t"));
        assert_eq!(
            keys.get("double-click").map(|action| action.text.as_str()),
            if scope == OpsKeyScope::Reader {
                None
            } else {
                Some("inspect")
            }
        );
    }
    for scope in [OpsKeyScope::Pool, OpsKeyScope::Squad] {
        assert_eq!(
            crate::config::workspace_defaults(scope)["enter"].text,
            "inspect"
        );
    }
    let mut app = crate::board::app::workspace::tests::board();
    app.workspace.selected = Some(crate::board::workspace::OpsTarget::Squad(
        crate::board::app::workspace::tests::ROOM.into(),
    ));
    app.workspace_reconcile();
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert!(app.input.is_none(), "group Enter never invents a recipient");
    app.workspace.selected = Some(crate::board::workspace::OpsTarget::Member {
        room_id: Some(crate::board::app::workspace::tests::ROOM.into()),
        identity_id: crate::board::app::workspace::tests::MEMBER.into(),
    });
    app.workspace
        .expanded
        .insert(crate::board::app::workspace::tests::ROOM.into());
    app.workspace_reconcile();
    let path = std::env::temp_dir().join(format!(
        "ops-enter-override-{}.toml",
        crate::id::new_v4().unwrap()
    ));
    std::fs::write(&path, "[bind]\nenter = 'menu'\nt = 'talk'\n").unwrap();
    let config = crate::config::Config::read(path.clone()).unwrap();
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
    draw(&mut app, 100, 36);
    assert_eq!(app.composer_focus_hint(), "t write · click to write");
    let help = crate::board::help::model(&app).value().to_string();
    assert!(
        help.contains("binding-enter") && help.contains("binding-t") && help.contains("t write")
    );
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert!(
        app.menu.is_some() && app.input.is_none(),
        "authored Enter keeps its menu effect"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    assert!(app.input.is_some(), "authored Talk key remains valid");
}
