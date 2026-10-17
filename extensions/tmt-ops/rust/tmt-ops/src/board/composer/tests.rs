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
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(Request::Talk { text: sent, to, .. }) if sent == format!("[product · auth-fix] {text}") && to == "auth-fix")
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
        let send = app.composer.send_hit.borrow().clone().unwrap();
        let label: String = (send.area.x..send.area.right())
            .map(|x| buffer[(x, send.area.y)].symbol())
            .collect();
        assert_eq!(label, " Send ");
        assert_eq!(send.area.width, 6);
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
fn eight_row_composer_keeps_arrow_selection_and_home_restores_the_list() {
    use crate::board::{app::Effect, workspace::OpsTarget};
    use crate::config::{OpsLayout, Pane};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tmt_cli_style::{Depth, Theme, theme::Base};
    for (base, depth) in [
        (Base::Tmt, Depth::TrueColor),
        (Base::TmtLight, Depth::TrueColor),
        (Base::Terminal, Depth::Ansi16),
        (Base::Tmt, Depth::None),
    ] {
        for preference in [OpsLayout::Auto, OpsLayout::Side, OpsLayout::Stack] {
            for width in [60, 80, 100, 160] {
                let mut app = crate::board::app::workspace::tests::board();
                let view = app.view.as_mut().unwrap();
                view.look = crate::look::Look {
                    theme: Theme::new(base),
                    depth,
                };
                view.home.as_mut().unwrap().workspace.preferences.layout = preference;
                let member = app.workspace.selected.clone();
                draw(&mut app, width, 8);
                assert_eq!(
                    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                    Effect::None
                );
                app.paste("eight-row draft");
                let key = app.input.as_ref().unwrap().draft_key.clone();
                assert_eq!(
                    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                    Effect::None
                );
                let buffer = draw(&mut app, width, 8);
                let painted: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
                assert!(app.input.is_none() && app.focused() == Pane::Rows);
                assert_eq!(app.input_band.get().unwrap().height, 4);
                assert!(app.hits.borrow().iter().all(|hit| hit.row().is_none()));
                assert!(
                    painted.contains("Message → worker") && painted.contains("eight-row draft")
                );
                assert_eq!(app.workspace.selected, member);

                assert_eq!(
                    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
                    Effect::None
                );
                let buffer = draw(&mut app, width, 8);
                assert_eq!(
                    app.workspace.selected,
                    Some(OpsTarget::Squad(
                        crate::board::app::workspace::tests::ROOM.into()
                    ))
                );
                assert!(app.input_band.get().is_none());
                assert!(app.hits.borrow().iter().any(|hit| hit.row().is_some()));
                assert!(
                    buffer
                        .content
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>()
                        .contains("product")
                );
                assert_eq!(
                    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                    Effect::None
                );
                draw(&mut app, width, 8);
                assert_eq!(app.workspace.selected, member);
                assert_eq!(
                    app.composer_projection().unwrap().text.unwrap().as_str(),
                    "eight-row draft"
                );

                assert_eq!(
                    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
                    Effect::None
                );
                let buffer = draw(&mut app, width, 8);
                assert_eq!(app.workspace.selected, Some(OpsTarget::Pool));
                assert!(app.input_band.get().is_none());
                assert!(app.hits.borrow().iter().any(|hit| hit.row().is_some()));
                assert!(
                    buffer
                        .content
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>()
                        .contains("Common pool")
                );
                for _ in 0..2 {
                    assert_eq!(
                        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                        Effect::None
                    );
                }
                draw(&mut app, width, 8);
                assert_eq!(app.workspace.selected, member);
                assert_eq!(
                    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                    Effect::None
                );
                assert_eq!(app.input.as_ref().unwrap().draft_key, key);
                assert_eq!(app.input.as_ref().unwrap().text.as_str(), "eight-row draft");
            }
        }
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
                if text == "[product · worker] a real outgoing message" && to == "worker")
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
            matches!(app.mouse(click, std::time::Instant::now()), crate::board::app::Effect::Act(crate::board::app::Request::Talk { text, .. }) if text == "[product · worker] explicit button text")
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
    for width in [1, 2, 3, 10, 14, 15, 16, 20] {
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
                assert_eq!(hit.area.width, 6);
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
            matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(Request::Talk { to, text, .. }) if to == "worker" && text == "[product · worker] retained 漢字 e\u{301}")
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
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(Request::Talk { text, .. }) if text == "[product · worker] t\n漢字 e\u{301}\ncommitted IME")
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
        OpsKeyScope::OverviewReader,
    ] {
        let keys = crate::config::workspace_defaults(scope);
        assert_eq!(
            keys["enter"].verb,
            if matches!(scope, OpsKeyScope::Overview | OpsKeyScope::OverviewReader) {
                crate::action::Verb::Reply
            } else {
                crate::action::Verb::Talk
            }
        );
        assert!(!keys.contains_key("t"));
        assert_eq!(
            keys.get("double-click").map(|action| action.text.as_str()),
            if scope.is_reader() {
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
    assert_eq!(app.composer_focus_hint(), "t write");
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

fn attached_quote(identity: &str, text: &str) -> crate::board::response::Quote {
    crate::board::response::Quote {
        key: crate::board::home_leads::MessageKey {
            sender: "owner".into(),
            lead: identity.into(),
            squad: "team".into(),
            request: "req-private-quote-source".into(),
            kind: crate::board::home_leads::Kind::Reply,
        },
        range: 0..text.len(),
        text: text.into(),
    }
}

#[test]
fn quote_provenance_requires_both_pinned_recipient_and_source_uuid() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::board::app::workspace::tests::board();
    let id = crate::board::app::workspace::tests::MEMBER;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let quote = attached_quote(id, "exact quoted response");
    app.input
        .as_mut()
        .unwrap()
        .response_quotes
        .push(quote.clone());
    let expected = app.quote_provenance(id);
    assert_eq!(expected.len(), 1);
    assert!(
        expected[0].contains("req-private-quote-source")
            && expected[0].contains(id)
            && expected[0].contains("bytes 0..21")
            && expected[0].contains("exact quoted response")
    );
    assert!(app.quote_provenance("same-name-replacement").is_empty());
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .members
        .get_mut(id)
        .unwrap()
        .name = "renamed".into();
    assert_eq!(
        app.quote_provenance(id),
        expected,
        "rename cannot change provenance identity"
    );
    assert!(app.quote_caption(&quote, 100).contains("renamed"));
    app.input.as_mut().unwrap().response_quotes[0].key.lead = "same-name-replacement".into();
    assert!(
        app.quote_provenance(id).is_empty(),
        "a mismatched source is not shown"
    );
    app.input.as_mut().unwrap().response_quotes[0] = quote;
    app.input.as_mut().unwrap().draft_key.as_mut().unwrap().1 = "same-name-replacement".into();
    assert!(
        app.quote_provenance(id).is_empty(),
        "a draft for another UUID is not shown"
    );
}

#[test]
fn quote_excerpts_and_details_are_bounded_and_inert_with_many_unicode_quotes() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::board::app::workspace::tests::board();
    let id = crate::board::app::workspace::tests::MEMBER;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let raw = format!("\x1b[31m\n{}", "漢字 e\u{301} ".repeat(20_000));
    let quote = attached_quote(id, &raw);
    app.input.as_mut().unwrap().response_quotes = vec![quote.clone(); 20];
    let lines = app.quote_provenance(id);
    assert_eq!(lines.len(), 9);
    assert_eq!(lines[8], "12 more attached quotes");
    assert!(
        lines[..8]
            .iter()
            .all(|line| line.len() < 1024 && !line.contains('\x1b') && !line.contains('\n'))
    );
    assert!(lines[0].ends_with('…'));
    assert!(lines[0].contains("漢字") && lines[0].contains("e\u{301}"));
    use unicode_width::UnicodeWidthStr;
    for width in [0, 1, 8, 12, 16, 30, 60] {
        let caption = app.quote_caption(&quote, width);
        assert!(caption.width() <= width && !caption.contains('\x1b'));
        if width >= 16 {
            assert!(
                caption.starts_with("Quote · ") && caption.ends_with('"'),
                "{width}: {caption}"
            );
        }
    }
    assert_eq!(
        app.input.as_ref().unwrap().response_quotes[0].text,
        raw,
        "display bounds never alter stored quote bytes"
    );
}

#[test]
fn quote_remove_is_a_current_whole_control_and_retains_text_and_other_quotes() {
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    for width in [160, 100, 60] {
        let mut app = crate::board::app::workspace::tests::board();
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.paste("kept message");
        let first = attached_quote(crate::board::app::workspace::tests::MEMBER, "first excerpt");
        let mut second = first.clone();
        second.key.request = "req-second-quote".into();
        second.text = "second excerpt".into();
        second.range = 0..second.text.len();
        app.input.as_mut().unwrap().response_quotes = vec![first.clone(), second.clone()];
        let painted: String = draw(&mut app, width, 18)
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(painted.contains("Quote · worker:") && painted.contains("second excerpt"));
        assert!(!painted.contains("req-second-quote") && !painted.contains("bytes 0.."));
        let hit = app.composer.quote_hit.borrow().clone().unwrap();
        assert_eq!(hit.area.width, 3);
        assert_eq!(
            hit.area.intersection(app.input_band.get().unwrap()),
            hit.area
        );
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.area.x,
            row: hit.area.y,
            modifiers: KeyModifiers::NONE,
        };
        app.input
            .as_mut()
            .unwrap()
            .response_quotes
            .last_mut()
            .unwrap()
            .key
            .request = "changed-quote".into();
        app.mouse(click, std::time::Instant::now());
        assert_eq!(
            app.input.as_ref().unwrap().response_quotes.len(),
            2,
            "stale quote identity has no removal authority"
        );
        app.input.as_mut().unwrap().response_quotes[1] = second;
        draw(&mut app, width, 18);
        assert_eq!(
            app.mouse(click, std::time::Instant::now()),
            crate::board::app::Effect::None
        );
        assert_eq!(app.input.as_ref().unwrap().response_quotes, vec![first]);
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "kept message");
        assert!(app.composer.quote_hit.borrow().is_none());
        assert_eq!(
            app.mouse(click, std::time::Instant::now()),
            crate::board::app::Effect::None
        );
        let projection = app.composer_projection().unwrap();
        assert_eq!(projection.quotes.len(), 1);
        assert_eq!(projection.text.unwrap().as_str(), "kept message");
        draw(&mut app, width, 18);
        app.invalidate_frame_geometry();
        assert!(app.composer.quote_hit.borrow().is_none());
        assert_eq!(
            app.mouse(click, std::time::Instant::now()),
            crate::board::app::Effect::None
        );
        let projection = app.composer_projection().unwrap();
        assert_eq!(projection.quotes.len(), 1);
        assert_eq!(projection.text.unwrap().as_str(), "kept message");
    }
}

#[test]
fn degraded_layout_explains_the_painted_fallback_without_losing_the_editor_row() {
    use crate::config::OpsLayout;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    for preference in [OpsLayout::Auto, OpsLayout::Side, OpsLayout::Stack] {
        for width in [60, 80, 100, 140, 160] {
            for height in [8, 18, 24, 36, 44] {
                for focused in [false, true] {
                    let mut app = crate::board::app::workspace::tests::board();
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .preferences
                        .layout = preference;
                    let selected = app.workspace.selected.clone();
                    if focused {
                        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        app.paste("kept draft");
                    }
                    let buffer = draw(&mut app, width, height);
                    let painted: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
                    let band = app.input_band.get().unwrap();
                    let editor = app.input_viewport.get().unwrap().0;
                    assert!(
                        !editor.is_empty() && editor.intersection(band) == editor,
                        "{preference:?} {width}×{height} {focused}"
                    );
                    assert_eq!(app.workspace.selected, selected);
                    if let Some(reason) = app.composer_layout_reason() {
                        assert!(
                            painted.contains(reason),
                            "{preference:?} {width}×{height} {focused}: {painted}"
                        );
                    }
                    assert!(painted.contains("Message → worker"));
                    if focused {
                        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "kept draft");
                    } else {
                        assert!(
                            painted.contains("⏎ write"),
                            "{preference:?} {width}×{height}: {painted}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn paste_preview_remove_and_refusal_preserve_exact_payload_and_other_uuid_drafts() {
    use crate::board::app::{Effect, Request};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    for kib in [1, 10, 100] {
        let mut app = app();
        open(&mut app, "auth-fix");
        app.paste("before ");
        let payload = format!(
            "  \r\n{}\n漢字 e\u{301}\n\x1b[31m\n  ",
            "x".repeat(kib * 1024)
        );
        assert_eq!(app.paste(&payload), Effect::None);
        let exact = format!("before {payload}");
        assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
        assert!(app.input.as_ref().unwrap().text.layout(80).lines.len() < 4);
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.input.as_ref().unwrap().text.layout(80).lines.len() > 8);
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), exact);
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.input.as_ref().unwrap().text.as_str(), "before ");
        assert!(app.input.as_ref().unwrap().text.folds().is_empty());
        app.paste(&payload);
        open(&mut app, "docs");
        app.paste("other UUID draft");
        open(&mut app, "auth-fix");
        for width in [160, 100, 80, 60] {
            app.set_body_width(width);
            assert_eq!(app.input.as_ref().unwrap().text.as_str(), exact);
            assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
        }
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] = json!("replacement");
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Effect::None
        );
        assert_eq!(
            app.input.as_ref().unwrap().text.as_str(),
            exact,
            "refusal keeps the exact folded payload"
        );
        assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] = json!("auth-fix");
        assert!(
            matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(Request::Talk { to, text, .. }) if to == "auth-fix" && text == format!("[product · auth-fix] {exact}"))
        );
        open(&mut app, "docs");
        assert_eq!(
            app.input.as_ref().unwrap().text.as_str(),
            "other UUID draft"
        );
    }
}

#[test]
fn removing_last_membership_restores_the_same_uuid_draft_in_pool_and_name_reuse_is_empty() {
    use crate::board::{app::Effect, workspace::OpsTarget};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::board::app::workspace::tests::board();
    let id = crate::board::app::workspace::tests::MEMBER;
    let old_target = app.workspace.selected.clone().unwrap();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let payload = "draft\n".repeat(200);
    app.paste(&payload);
    let key = app.input.as_ref().unwrap().draft_key.clone();
    let quote = attached_quote(id, "kept source quote");
    app.input
        .as_mut()
        .unwrap()
        .response_quotes
        .push(quote.clone());
    let pool = OpsTarget::Member {
        room_id: None,
        identity_id: id.into(),
    };
    {
        let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        for squad in &mut workspace.roster.squads {
            squad.members.retain(|member| member != id);
            squad.member_states.remove(id);
            if squad.lead.as_deref() == Some(id) {
                squad.lead = None;
            }
        }
        let row = workspace.rows.remove(&old_target).unwrap();
        workspace.rows.insert(pool.clone(), row);
    }
    app.workspace_reconcile();
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None,
        "the removed occurrence refuses before sending"
    );
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), payload);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.workspace.pool_expanded = true;
    app.workspace.selected = Some(pool.clone());
    app.workspace_reconcile();
    draw(&mut app, 100, 36);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let input = app.input.as_ref().unwrap();
    assert_eq!(input.draft_key, key);
    assert_eq!(input.text.as_str(), payload);
    assert_eq!(input.text.folds().len(), 1);
    assert_eq!(input.response_quotes, vec![quote]);
    assert!(
        input.squad.is_empty(),
        "the old membership grants no squad scope in Common pool"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let replacement = OpsTarget::Member {
        room_id: None,
        identity_id: "88888888-8888-4888-8888-888888888888".into(),
    };
    {
        let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        let mut member = workspace.roster.members.remove(id).unwrap();
        member.id = "88888888-8888-4888-8888-888888888888".into();
        workspace.roster.members.insert(member.id.clone(), member);
        let mut row = workspace.rows.remove(&pool).unwrap();
        row["id"] = json!("88888888-8888-4888-8888-888888888888");
        workspace.rows.insert(replacement.clone(), row);
    }
    app.workspace.selected = Some(replacement);
    app.workspace_reconcile();
    draw(&mut app, 60, 18);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.input.as_ref().unwrap().text.is_empty());
    assert!(app.input.as_ref().unwrap().response_quotes.is_empty());
    assert_eq!(
        app.composer
            .drafts
            .get(key.as_ref().unwrap())
            .unwrap()
            .text
            .as_str(),
        payload
    );
}

#[test]
fn colab_link_and_exact_source_context_survive_composer_reader_and_tab_transitions() {
    use crate::board::app::{Effect, Request};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::board::app::workspace::tests::board();
    let id = crate::board::app::workspace::tests::MEMBER;
    let selected = app.workspace.selected.clone();
    let url = "https://example.test/colab/e85910cf";
    let request = "req_66666666-6666-4666-8666-666666666666";
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(selected.as_ref().unwrap())
        .unwrap()["fields"]["link"] = json!(url);
    app.view.as_mut().unwrap().replies =
        vec![json!({"recipientId":id,"requestId":request,"response":url})];
    let _no_acquisition = crate::runner::forbid_commands();
    let painted: String = draw(&mut app, 160, 44)
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(painted.contains(url));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.paste(&format!("Literal Colab reference {url}"));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert!(app.row_details.reader.is_some());
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    draw(&mut app, 160, 44);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        Effect::None
    );
    let input = app.input.as_ref().unwrap();
    assert_eq!(
        input.text.as_str(),
        format!("Literal Colab reference {url}")
    );
    let quote = &input.response_quotes[0];
    assert_eq!(quote.key.request, request);
    assert_eq!(quote.key.lead, id);
    assert_eq!(quote.range, 0..url.len());
    assert_eq!(quote.text, url);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    for tab in ['1', '2'] {
        app.key(KeyEvent::new(KeyCode::Char(tab), KeyModifiers::NONE));
        draw(&mut app, 100, 36);
        assert_eq!(app.workspace.selected, selected);
    }
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
        Effect::Act(Request::Open {
            link: url.into(),
            opener: None
        })
    );
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("Literal Colab reference {url}")
    );
    assert_eq!(app.input.as_ref().unwrap().response_quotes[0].text, url);
}

fn open_blocking_owner(app: &mut crate::board::app::App, owner: &str) {
    use crate::board::app::Effect;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let effect = match owner {
        "review" => app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        "details" => app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)),
        "help" => app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
        "reader" => app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE)),
        "menu" => app.perform(&crate::action::Action::parse("menu").unwrap()),
        _ => unreachable!(),
    };
    assert_eq!(effect, Effect::None, "{owner}");
    assert!(
        app.modal_open(),
        "{owner}: existing blocking owner must open"
    );
}

#[test]
fn blocking_owners_mask_only_the_passive_band_and_restore_the_exact_uuid_draft() {
    use crate::board::app::Effect;
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    crate::status::with_now_ms(1_791_600_000_000, || {
        for (width, height) in [(160, 36), (100, 30), (60, 18)] {
            for owner in ["review", "details", "help", "reader", "menu"] {
                for remove in [false, true] {
                    let mut app = crate::board::app::workspace::tests::board();
                    let id = crate::board::app::workspace::tests::MEMBER;
                    let quote = attached_quote(id, "source");
                    app.view.as_mut().unwrap().replies = vec![
                        json!({"recipientId":id,"requestId":quote.key.request,"response":"source"}),
                    ];
                    let _no_acquisition = crate::runner::forbid_commands();
                    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    app.paste("retained message draft");
                    app.input
                        .as_mut()
                        .unwrap()
                        .response_quotes
                        .push(quote.clone());
                    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                    // Compare the stable base, not Escape's transient notice.
                    app.notice = None;
                    let base = draw(&mut app, width, height);
                    let band = app.input_band.get().unwrap();
                    let key = app.composer_projection().unwrap().key.unwrap();
                    let selected = app.workspace.selected.clone();
                    let shell = app.shell.tab;
                    let focus = app.focus;
                    let expanded = app.workspace.expanded.clone();
                    let scrolls = app.scrolls.offsets();
                    let point = if remove {
                        app.composer.quote_hit.borrow().as_ref().unwrap().area
                    } else {
                        app.composer.send_hit.borrow().as_ref().unwrap().area
                    };
                    open_blocking_owner(&mut app, owner);
                    let covered = draw(&mut app, width, height);
                    assert!(app.composer.send_hit.borrow().is_none(), "{owner}");
                    assert!(app.composer.quote_hit.borrow().is_none(), "{owner}");
                    assert!(app.input_viewport.get().is_none(), "{owner}");
                    let band_text: String = (band.y..band.bottom())
                        .flat_map(|y| (band.x..band.right()).map(move |x| (x, y)))
                        .map(|at| covered[at].symbol())
                        .collect();
                    assert!(
                        !band_text.contains("Message →")
                            && !band_text.contains("[ Send ]")
                            && !band_text.contains("Quote ·"),
                        "{owner}: {band_text}"
                    );
                    assert_eq!(
                        app.mouse(
                            MouseEvent {
                                kind: MouseEventKind::Down(MouseButton::Left),
                                column: point.x,
                                row: point.y,
                                modifiers: KeyModifiers::NONE
                            },
                            std::time::Instant::now()
                        ),
                        Effect::None,
                        "{owner}"
                    );
                    if app.modal_open() {
                        assert_eq!(
                            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                            Effect::None
                        );
                    }
                    assert!(!app.modal_open(), "{owner}: dismissal");
                    assert_eq!(app.workspace.selected, selected, "{owner}");
                    assert_eq!(app.workspace.expanded, expanded, "{owner}");
                    let restored = draw(&mut app, width, height);
                    assert_eq!(app.scrolls.offsets(), scrolls, "{owner}");
                    let changed_rows = (0..height)
                        .filter_map(|y| {
                            let before: String =
                                (0..width).map(|x| base[(x, y)].symbol()).collect();
                            let after: String =
                                (0..width).map(|x| restored[(x, y)].symbol()).collect();
                            (before != after).then_some((y, before, after))
                        })
                        .collect::<Vec<_>>();
                    assert!(
                        restored == base,
                        "{owner} {width}x{height}: no base reflow; shell {shell:?}->{:?}, focus {focus}->{}, changed rows {changed_rows:?}",
                        app.shell.tab,
                        app.focus
                    );
                    let draft = app.composer_projection().unwrap();
                    assert_eq!(draft.key.as_ref(), Some(&key));
                    assert_eq!(draft.text.unwrap().as_str(), "retained message draft");
                    assert_eq!(draft.quotes, &[quote]);
                    assert!(
                        app.composer.send_hit.borrow().is_some()
                            && app.composer.quote_hit.borrow().is_some()
                    );
                }
            }
        }
    });
}

#[test]
fn blocking_owner_preserves_focused_composer_paint() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = crate::board::app::workspace::tests::board();
    let id = crate::board::app::workspace::tests::MEMBER;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.paste("focused draft");
    app.input
        .as_mut()
        .unwrap()
        .response_quotes
        .push(attached_quote(id, "source"));
    app.help = true;
    draw(&mut app, 160, 36);
    assert!(app.composer_projection().unwrap().focused);
    assert!(app.composer.send_hit.borrow().is_some() && app.composer.quote_hit.borrow().is_some());
    assert!(app.input_viewport.get().is_some());
}
