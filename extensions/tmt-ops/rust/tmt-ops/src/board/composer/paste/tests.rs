use super::*;

use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn draw(app: &mut App, width: u16, height: u16) -> bool {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().cursor_visible()
}

fn click(area: Rect) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn keyboard_preview_follows_the_current_fold_without_an_intermediate_paint() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste("before ");
    let raw = (0..100)
        .map(|line| format!("source line {line}\r\n"))
        .collect::<String>();
    app.paste(&raw);
    app.paste(" between ");
    app.paste(&"a".repeat(501));
    app.paste(" after");
    draw(&mut app, 80, 36);
    let draft = app.input.as_ref().unwrap().text.clone();
    app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
    for _ in 0..8 {
        key(&mut app, KeyCode::Right);
    }
    assert!(app.composer.paste_hits.borrow().is_empty());
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(
        app.paste_preview_shown(),
        "Ctrl-P follows editor state before paint"
    );
    assert!(!draw(&mut app, 80, 36));
    assert_eq!(app.current_paste_preview().unwrap().text.as_str(), raw);
    key(&mut app, KeyCode::PageDown);
    draw(&mut app, 80, 36);
    assert!(
        app.current_paste_preview()
            .unwrap()
            .scroll
            .borrow()
            .offset()
            > 0
    );
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), draft.as_str());
}

#[test]
fn flat_list_preview_keeps_its_anchor_and_scrolls_without_changing_the_draft() {
    let mut app = crate::board::app::workspace::tests::board();
    crate::board::app::workspace::tests::observe_waiting_member(&mut app);
    app.shell.tab = crate::board::shell::OpsTab::Agents;
    app.agents_filter(crate::board::workspace::AgentsFilter::Agents);
    draw(&mut app, 160, 36);
    app.perform(&crate::action::Action::parse("talk").unwrap());
    assert_eq!(app.shell.tab, crate::board::shell::OpsTab::Agents);
    let raw = (0..100)
        .map(|line| format!("line {line} 漢字\r\n"))
        .collect::<String>();
    app.paste(&raw);
    draw(&mut app, 160, 36);
    let hit = app.composer.paste_hits.borrow()[0].clone();
    app.mouse(click(hit.area), std::time::Instant::now());
    assert!(app.paste_preview_shown());
    assert!(!draw(&mut app, 160, 36));
    assert!(
        app.paste_preview_shown(),
        "painting keeps the flat list anchor"
    );
    key(&mut app, KeyCode::PageDown);
    draw(&mut app, 160, 36);
    assert!(
        app.current_paste_preview()
            .unwrap()
            .scroll
            .borrow()
            .offset()
            > 0
    );
    key(&mut app, KeyCode::Esc);
    assert!(!app.paste_preview_shown());
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), raw);
    assert!(draw(&mut app, 160, 36));
}

#[test]
fn preview_is_read_only_scrollable_and_outside_close_can_remove_the_chip() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    let raw = (0..100)
        .map(|line| format!("line {line} 漢字\r\n"))
        .collect::<String>();
    app.paste(&raw);
    draw(&mut app, 100, 36);
    let hit = app.composer.paste_hits.borrow()[0].clone();
    app.mouse(click(hit.area), std::time::Instant::now());
    assert!(app.paste_preview_shown());
    assert!(
        !draw(&mut app, 100, 36),
        "a read-only preview hides the underlying editor caret"
    );
    let before = app.input.as_ref().unwrap().text.clone();
    key(&mut app, KeyCode::Char('z'));
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.input.as_ref().unwrap().text, before);
    key(&mut app, KeyCode::PageDown);
    assert!(
        app.current_paste_preview()
            .unwrap()
            .scroll
            .borrow()
            .offset()
            > 0
    );
    key(&mut app, KeyCode::Esc);
    assert!(!app.paste_preview_shown());
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), raw);
    assert!(
        draw(&mut app, 100, 36),
        "closing the preview restores the editor caret"
    );
    app.mouse(click(hit.area), std::time::Instant::now());
    draw(&mut app, 100, 36);
    app.mouse(click(hit.remove), std::time::Instant::now());
    assert!(!app.paste_preview_shown());
    assert!(
        app.input.as_ref().unwrap().text.is_empty(),
        "outside press also reaches the chip remove control"
    );
}

#[test]
fn terminal_breaks_do_not_add_content_lines_or_change_pasted_bytes() {
    for separator in ["\n", "\r\n", "\r"] {
        for lines in [10, 11] {
            for terminal in [false, true] {
                let mut raw = vec!["漢"; lines].join(separator);
                if terminal {
                    raw.push_str(separator);
                }
                let mut app = crate::board::app::workspace::tests::board();
                key(&mut app, KeyCode::Enter);
                app.paste(&raw);
                let text = &app.input.as_ref().unwrap().text;
                assert_eq!(text.as_str(), raw);
                assert_eq!(
                    text.folds().len(),
                    usize::from(lines > 10),
                    "{separator:?}, {lines} content lines, terminal={terminal}"
                );
                assert_eq!(counts(&raw), (raw.chars().count(), lines));
            }
        }
        for (raw, lines) in [
            (String::new(), 0),
            (separator.to_owned(), 1),
            (separator.repeat(2), 2),
            (format!("first{separator}{separator}last{separator}"), 3),
        ] {
            assert_eq!(counts(&raw), (raw.chars().count(), lines), "{raw:?}");
        }
    }
}

#[test]
fn paste_closes_preview_and_inserts_once_at_the_kept_caret() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste("before ");
    let raw = "x".repeat(501);
    app.paste(&raw);
    app.paste(" after");
    draw(&mut app, 100, 36);
    for _ in 0..6 {
        key(&mut app, KeyCode::Left);
    }
    let caret = app.input.as_ref().unwrap().text.cursor();
    assert_eq!(caret, 7 + raw.len());
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(app.paste_preview_shown());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(&mut app, frame.area());
            crate::board::view::render(frame, &app);
        })
        .unwrap();
    let painted: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(painted.contains("Pasted · 501 chars · 1 line"));
    assert!(!painted.contains("501 chars · 1 lines"));
    let incoming = "new 漢字\r\n";
    assert_eq!(app.paste(incoming), Effect::None);
    assert!(!app.paste_preview_shown());
    let text = &app.input.as_ref().unwrap().text;
    assert_eq!(text.as_str(), format!("before {raw}{incoming} after"));
    assert_eq!(text.cursor(), caret + incoming.len());
    assert_eq!(text.folds().len(), 1);
}

#[test]
fn draftless_messages_use_the_same_content_line_threshold() {
    for lines in [10, 11] {
        let raw = vec!["line"; lines].join("\r\n") + "\r\n";
        let mut app = crate::board::app::workspace::tests::board();
        app.ask(
            "message form".into(),
            crate::board::app::Compose::Talk {
                to: "worker".into(),
            },
            "product".into(),
        );
        assert!(app.input.as_ref().unwrap().draft_key.is_none());
        app.paste(&raw);
        let text = &app.input.as_ref().unwrap().text;
        assert_eq!(text.as_str(), raw);
        assert_eq!(text.folds().len(), usize::from(lines > 10));
    }
}

#[test]
fn rejected_preview_paste_closes_without_mutating_the_exact_editor() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste(&"漢".repeat(501));
    draw(&mut app, 100, 36);
    app.preview_paste_at_cursor();
    assert!(app.paste_preview_shown());
    let before = app.input.as_ref().unwrap().clone();
    assert_eq!(
        app.paste(&"x".repeat(crate::board::app::MESSAGE_LIMIT)),
        Effect::None
    );
    assert!(!app.paste_preview_shown());
    let after = app.input.as_ref().unwrap();
    assert_eq!(after.text, before.text);
    assert_eq!(after.draft_key, before.draft_key);
    assert!(app.notice.as_deref().unwrap().contains("Paste exceeds"));
    assert!(draw(&mut app, 100, 36), "the editor caret returns");
}

#[test]
fn preview_paste_replaces_selection_and_survives_cancel_before_exact_send() {
    use crate::board::app::{Compose, Request};
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste(&"x".repeat(501));
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    assert_eq!(app.input.as_ref().unwrap().text.selection(), 0..501);
    draw(&mut app, 100, 36);
    app.preview_paste_at_cursor();
    assert!(app.paste_preview_shown());
    let incoming = "replacement\r\n漢字 e\u{301}";
    app.paste(incoming);
    assert!(!app.paste_preview_shown());
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), incoming);
    assert!(app.input.as_ref().unwrap().text.folds().is_empty());
    let draft_key = app.input.as_ref().unwrap().draft_key.clone();
    key(&mut app, KeyCode::Esc);
    assert!(app.input.is_none());
    app.focus_composer();
    let input = app.input.as_ref().unwrap();
    assert_eq!(input.draft_key, draft_key);
    assert_eq!(input.text.as_str(), incoming);
    let Compose::Talk { to } = &input.compose else {
        panic!("chat")
    };
    let expected = format!("[{} · {to}] {incoming}", input.squad);
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::Act(Request::Talk { text, .. }) if text == expected)
    );
}

#[test]
fn draftless_fold_label_and_keyboard_actions_keep_the_editor_owner() {
    use crate::board::app::Compose;
    for compose in [
        Compose::Talk {
            to: "worker".into(),
        },
        Compose::Reply {
            request: "request".into(),
            from: "worker".into(),
        },
        Compose::Annotate {
            to: "lead".into(),
            row: "worker".into(),
        },
    ] {
        let mut app = crate::board::app::workspace::tests::board();
        app.ask("message form".into(), compose, "product".into());
        let raw = "漢".repeat(501);
        app.paste(&raw);
        let input = app.input.as_ref().unwrap();
        assert!(input.draft_key.is_none());
        assert_eq!(input.text.folds()[0].label, "⧉ Pasted · 501 chars");
        assert_eq!(input.text.as_str(), raw);
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.input.as_ref().unwrap().text.folds()[0].expanded);
        assert!(!app.paste_preview_shown());
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(!app.input.as_ref().unwrap().text.folds()[0].expanded);
        key(&mut app, KeyCode::Backspace);
        assert!(app.input.as_ref().unwrap().text.is_empty());
        assert!(app.input.as_ref().unwrap().text.folds().is_empty());
    }
}

fn cron_message_form(raw: &str) -> (App, String) {
    use crate::board::cronboard::{detail_id, test_cron, test_view};
    let mut app = crate::board::app::workspace::tests::board();
    let mut job = test_view("worker", raw, None);
    job.job.squad = "product".into();
    job.job.room_id = crate::board::app::workspace::tests::ROOM.into();
    let id = detail_id(&job);
    app.cron.replace(Ok(test_cron(
        vec![job],
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    (app, id)
}

#[test]
fn cron_prefill_and_edit_share_counts_labels_and_exact_message_admission() {
    use crate::board::{
        app::Request,
        cronboard::{CronRequest, Op},
    };
    for lines in [10, 11] {
        let raw = vec!["漢"; lines].join("\r\n") + "\r\n";
        let (mut app, id) = cron_message_form(&raw);
        assert_eq!(app.cron_key('E', Some(&id)), Effect::None);
        let input = app.input.as_ref().unwrap();
        assert!(input.draft_key.is_none());
        assert_eq!(input.text.as_str(), raw);
        assert_eq!(input.text.folds().len(), usize::from(lines > 10));
        if lines > 10 {
            assert_eq!(input.text.folds()[0].label, "⧉ Pasted · 11 lines");
            app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
            assert!(app.input.as_ref().unwrap().text.folds()[0].expanded);
            assert_eq!(app.input.as_ref().unwrap().text.as_str(), raw);
        }
        app.paste(" tail");
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Effect::None
        );
        assert!(!app.message_field(), "the schedule is a separate form step");
        let expected = raw + " tail";
        let Effect::Act(Request::CronPreview(task)) =
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("the form stages a review before writing");
        };
        assert!(matches!(task.request,
            CronRequest::Existing { op: Op::Edit { message: Some(message), .. }, .. } if message == expected));
        assert!(app.cron_review.is_some());
    }
}

#[test]
fn cron_create_paste_and_refused_reentry_preserve_form_owned_bytes() {
    use crate::board::{app::Request, cronboard::CronRequest};
    let raw = "漢字\r\n".repeat(11);
    let (mut app, id) = cron_message_form("stored");
    app.cron_key('n', Some(&id));
    app.input.as_mut().unwrap().text = "worker".into();
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert!(app.message_field());
    app.paste(&raw);
    assert_eq!(
        app.input.as_ref().unwrap().text.folds()[0].label,
        "⧉ Pasted · 11 lines"
    );
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    app.paste("every 30m");
    let Effect::Act(Request::CronPreview(task)) =
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("the form stages a review before writing");
    };
    assert!(matches!(task.request, CronRequest::Add { message, .. } if message == raw));
    assert!(app.cron_review.is_some());

    let blank = "\r\n".repeat(11);
    let (mut app, id) = cron_message_form("stored");
    app.cron_key('n', Some(&id));
    app.input.as_mut().unwrap().text = "worker".into();
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    app.paste(&blank);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    let input = app.input.as_ref().unwrap();
    assert_eq!(input.text.as_str(), blank);
    assert_eq!(input.text.folds()[0].label, "⧉ Pasted · 11 lines");
    assert!(input.hint.as_ref().unwrap().error);
    key(&mut app, KeyCode::Esc);
    assert!(app.input.is_none() && app.cron_draft.is_none());
}

#[test]
fn chips_between_typed_text_are_atomic_and_old_geometry_cannot_edit_a_new_draft() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste("before ");
    let first = "a".repeat(501);
    let second = "b".repeat(501);
    app.paste(&first);
    app.paste(" between ");
    app.paste(&second);
    app.paste(" after");
    draw(&mut app, 100, 36);
    let stale = app.composer.paste_hits.borrow()[0].clone();
    for _ in 0..6 {
        key(&mut app, KeyCode::Left);
    }
    key(&mut app, KeyCode::Left);
    assert_eq!(
        app.input.as_ref().unwrap().text.cursor(),
        "before ".len() + first.len() + " between ".len()
    );
    key(&mut app, KeyCode::Delete);
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("before {first} between  after")
    );
    app.input.as_mut().unwrap().draft_key.as_mut().unwrap().1 = "replacement".into();
    assert!(!app.paste_hit_is_current(&stale));
    app.open_paste_preview(stale);
    assert!(!app.paste_preview_shown());
}

#[test]
fn selected_chip_replacement_and_failed_admission_preserve_surrounding_bytes() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste("left ");
    let raw = "漢字\0\r\n".repeat(100);
    app.paste(&raw);
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    assert_eq!(
        app.input.as_ref().unwrap().text.selection(),
        5..5 + raw.len()
    );
    app.paste("replacement");
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        "left replacement"
    );
    assert!(app.input.as_ref().unwrap().text.folds().is_empty());
    let before = app.input.as_ref().unwrap().text.clone();
    app.paste(&"x".repeat(crate::board::app::MESSAGE_LIMIT));
    assert_eq!(
        app.input.as_ref().unwrap().text,
        before,
        "oversized paste is atomic"
    );
    app.paste(&raw);
    draw(&mut app, 100, 36);
    app.preview_paste_at_cursor();
    assert!(app.paste_preview_shown());
    app.remember_draft();
    app.input = None;
    assert!(!app.paste_preview_shown());
    app.focus_composer();
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("left replacement{raw}")
    );
    assert!(
        !app.paste_preview_shown(),
        "reopening the draft never resurrects a preview"
    );
}

#[test]
fn exclusive_thresholds_count_unicode_and_logical_breaks_without_normalizing_source() {
    for chars in [499, 500, 501] {
        let source = "漢".repeat(chars);
        assert_eq!(label(&source).is_some(), chars > 500);
        assert_eq!(counts(&source), (chars, 1));
    }
    for separator in ["\n", "\r", "\r\n"] {
        for lines in [9, 10, 11] {
            let source = vec!["x"; lines].join(separator);
            assert_eq!(counts(&source).1, lines);
            assert_eq!(label(&source).is_some(), lines > 10);
        }
    }
    assert_eq!(counts("x\r\n"), (3, 1));
    assert!(label(&"x".repeat(1123)).unwrap().contains("1,123 chars"));
    assert!(label(&"x\n".repeat(10)).is_none());
    assert!(label(&"x\n".repeat(11)).unwrap().contains("11 lines"));
}

#[test]
fn ascii_and_clipped_chips_keep_the_admitted_editor_width_and_remove_target() {
    let label = format!("{} ×", label(&"x".repeat(501)).unwrap());
    assert_eq!(crate::board::glyph_guard::glyph_error(&label), None);
    for ascii in [false, true] {
        for width in 1..=label.width() {
            let (painted, remove) = display(&label, width, ascii);
            assert_eq!(painted.width(), width);
            assert_eq!(
                painted.chars().nth(remove),
                Some(if ascii { 'x' } else { '×' })
            );
            if ascii {
                assert!(painted.is_ascii());
            }
        }
    }
    let (painted, _) = display(&label, label.width(), true);
    assert!(painted.starts_with("[Pasted ") && painted.trim_end().ends_with(" x]"));
}

#[test]
fn preview_anchors_above_then_below_and_stays_inside_the_conversation() {
    let bounds = Rect::new(40, 5, 80, 30);
    let above = preview_area(bounds, Rect::new(50, 30, 25, 1), 100).unwrap();
    assert_eq!(above.x, 50);
    assert_eq!(above.bottom(), 30);
    assert_eq!(above.height, bounds.height / 2);
    let below = preview_area(bounds, Rect::new(50, 6, 25, 1), 100).unwrap();
    assert_eq!(below.y, 7);
    assert_eq!(below.height, bounds.height / 2);
    assert_eq!(below.intersection(bounds), below);
    assert!(preview_area(bounds, Rect::new(119, 20, 1, 1), 100).is_none());
    assert!(preview_area(Rect::new(0, 0, 80, 3), Rect::new(0, 2, 20, 1), 10).is_none());
}

#[test]
fn paste_admission_uses_event_size_and_strict_line_threshold_in_the_actual_composer() {
    for separator in ["\n", "\r", "\r\n"] {
        for lines in [10, 11] {
            let mut app = crate::board::app::workspace::tests::board();
            key(&mut app, KeyCode::Enter);
            let raw = vec!["漢字"; lines].join(separator);
            app.paste(&raw);
            let editor = &app.input.as_ref().unwrap().text;
            assert_eq!(editor.as_str(), raw);
            assert_eq!(editor.folds().len(), usize::from(lines > 10));
        }
    }
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste(&"漢".repeat(500));
    app.paste(&"字".repeat(500));
    assert!(
        app.input.as_ref().unwrap().text.folds().is_empty(),
        "each ordinary paste stays text even when the complete draft exceeds the threshold"
    );
    app.paste(&"a".repeat(501));
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
}

pub(in crate::board) const SCENES: &[&str] = &[
    "paste-single",
    "paste-mixed",
    "paste-selected",
    "paste-preview",
    "paste-scrolled",
    "paste-removed",
    "paste-retained",
    "paste-ascii",
    "paste-tiny",
    "paste-terminal-ten",
    "paste-terminal-eleven",
    "paste-preview-insert",
    "paste-preview-refused",
    "paste-preview-insert-tiny",
    "paste-form-folded",
    "paste-form-expanded",
    "paste-form-tiny",
    "paste-cron-prefill",
    "paste-cron-retry",
];

pub(in crate::board) fn capture_height(scene: &str) -> u16 {
    if scene.ends_with("tiny") { 8 } else { 36 }
}

/// One scene owner for decoded review and the HOME oracle.
pub(in crate::board) fn capture_case(
    scene: &str,
    look: crate::look::Look,
    width: u16,
    height: u16,
) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().look = look;
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    for squad in &workspace.roster.squads {
        workspace.squad_looks.insert(squad.room_id.clone(), look);
    }
    if scene == "paste-ascii" {
        app.cron_glyphs = crate::board::overview::cron_counts::GlyphSet::Ascii;
    }
    if matches!(scene, "paste-terminal-ten" | "paste-terminal-eleven") {
        key(&mut app, KeyCode::Enter);
        let lines = if scene == "paste-terminal-ten" {
            10
        } else {
            11
        };
        app.paste(&(vec!["content line"; lines].join("\r\n") + "\r\n"));
        draw(&mut app, width, height);
        return app;
    }
    if scene.starts_with("paste-form-") {
        app.ask(
            "Message form".into(),
            crate::board::app::Compose::Talk {
                to: "worker".into(),
            },
            "product".into(),
        );
        app.paste(&"form source 漢字\r\n".repeat(11));
        if scene == "paste-form-expanded" {
            app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        }
        draw(&mut app, width, height);
        return app;
    }
    if scene.starts_with("paste-cron-") {
        use crate::board::cronboard::{detail_id, test_cron, test_view};
        let mut job = test_view("worker", &"stored line\r\n".repeat(10), None);
        job.job.squad = "product".into();
        job.job.room_id = crate::board::app::workspace::tests::ROOM.into();
        let id = detail_id(&job);
        app.cron.replace(Ok(test_cron(
            vec![job],
            tmt_ops::cron::ClockStatus::NoClock,
        )));
        if scene == "paste-cron-retry" {
            app.cron_key('n', Some(&id));
            app.input.as_mut().unwrap().text = "worker".into();
            key(&mut app, KeyCode::Enter);
            app.paste(&"\r\n".repeat(11));
            key(&mut app, KeyCode::Enter);
        } else {
            app.cron_key('E', Some(&id));
        }
        draw(&mut app, width, height);
        return app;
    }
    key(&mut app, KeyCode::Enter);
    app.paste("before ");
    let source = if matches!(scene, "paste-preview" | "paste-scrolled") {
        (0..100)
            .map(|index| format!("source line {index:03}"))
            .collect::<Vec<_>>()
            .join("\r\n")
    } else {
        "x".repeat(1123)
    };
    app.paste(&source);
    if matches!(
        scene,
        "paste-mixed" | "paste-removed" | "paste-retained" | "paste-ascii"
    ) {
        app.paste(" between ");
        app.paste(&["second source"; 11].join("\n"));
        app.paste(" after");
    }
    if scene == "paste-selected" {
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    }
    draw(&mut app, width, height);
    if matches!(
        scene,
        "paste-preview"
            | "paste-scrolled"
            | "paste-preview-insert"
            | "paste-preview-refused"
            | "paste-preview-insert-tiny"
    ) {
        app.preview_paste_at_cursor();
        draw(&mut app, width, height);
        if scene == "paste-scrolled" {
            key(&mut app, KeyCode::PageDown);
        } else if scene == "paste-preview-refused" {
            app.paste(&"x".repeat(crate::board::app::MESSAGE_LIMIT));
            draw(&mut app, width, height);
        } else if scene.starts_with("paste-preview-insert") {
            app.paste(" inserted 漢字\r\n");
            draw(&mut app, width, height);
        }
    } else if scene == "paste-removed" {
        let hit = app.composer.paste_hits.borrow().last().cloned().unwrap();
        app.mouse(click(hit.remove), std::time::Instant::now());
    } else if scene == "paste-retained" {
        key(&mut app, KeyCode::Esc);
    }
    app
}

#[test]
#[ignore = "exports task-owned paste review evidence; never records a frozen fixture"]
fn capture_paste_packet() {
    let output = std::env::var("TMT_PASTE_CAPTURE").expect("task-owned evidence path");
    let mut captures = Vec::new();
    for (name, look) in crate::board::home::tests::oracle::looks() {
        for scene in SCENES {
            let height = capture_height(scene);
            for width in [160, 100, 80] {
                let app = capture_case(scene, look, width, height);
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                        .unwrap();
                terminal
                    .draw(|frame| crate::board::view::render(frame, &app))
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let cells = buffer.content.iter().map(|cell| serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                let lines = buffer
                    .content
                    .chunks(usize::from(width))
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>();
                captures.push(serde_json::json!({"scene":scene,"look":name,"width":width,"height":height,"cells":cells,"lines":lines,"inputBand":format!("{:?}",app.input_band.get()),"editor":format!("{:?}",app.input_viewport.get()),"paste":capture_metadata(&app),"draft":editor_metadata(&app)}));
            }
        }
    }
    std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
}

fn editor_metadata(app: &App) -> serde_json::Value {
    let text = app.input.as_ref().map(|input| &input.text).or_else(|| {
        app.composer_projection()
            .and_then(|projection| projection.text)
    });
    text.map_or(serde_json::Value::Null, |text| {
        let selection = text.selection();
        serde_json::json!({
            "source": text.as_str(), "cursor": text.cursor(),
            "selection": [selection.start, selection.end],
            "folds": text.folds().iter().map(|fold| serde_json::json!({
                "id":fold.id,"source":[fold.source.start,fold.source.end],
                "label":fold.label,"expanded":fold.expanded
            })).collect::<Vec<_>>(),
            "owner": app.input.as_ref().and_then(|input| input.draft_key.as_ref()),
            "form": app.input.as_ref().map(|input| format!("{:?}", input.compose)),
            "notice": app.notice,
        })
    })
}

/// The external capture driver owns its terminal; this fixture issues no tmux commands.
#[test]
#[ignore = "requires an explicitly isolated private-terminal capture"]
fn paste_private_terminal_evidence() {
    use std::io::{Read, Write};
    assert!(std::env::var_os("TMUX").is_none());
    assert!(std::env::var_os("TMUX_PANE").is_none());
    let ready = std::env::var("TMT_PASTE_TERMINAL_READY").unwrap();
    assert!(
        ready.starts_with("paste-4151-")
            && ready
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    );
    let width: u16 = std::env::var("TMT_PASTE_TERMINAL_WIDTH")
        .unwrap()
        .parse()
        .unwrap();
    assert!([80, 100, 160].contains(&width));
    let scene = std::env::var("TMT_PASTE_TERMINAL_SCENE").unwrap();
    assert!(SCENES.contains(&scene.as_str()) || scene == "help");
    let height = capture_height(&scene);
    let look = match std::env::var("TMT_PASTE_TERMINAL_LOOK").unwrap().as_str() {
        "tmt" => crate::look::Look::default(),
        "tmt-light" => crate::look::Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::TmtLight),
            depth: Depth::TrueColor,
        },
        "ansi16" => crate::look::Look {
            depth: Depth::Ansi16,
            ..crate::look::Look::default()
        },
        "NO_COLOR" => crate::look::Look {
            depth: Depth::None,
            ..crate::look::Look::default()
        },
        other => panic!("unsupported fixture look: {other}"),
    };
    let metadata =
        std::path::PathBuf::from(std::env::var_os("TMT_PASTE_TERMINAL_METADATA").unwrap());
    assert!(
        metadata.starts_with("/private/tmp")
            && metadata.components().any(|part| part
                .as_os_str()
                .to_string_lossy()
                .starts_with("ux-3-4151-terminal-"))
    );
    let _no_commands = crate::runner::forbid_commands();
    crate::board::terminal::restore_before_panic_reports();
    let mut screen =
        crate::board::terminal::Guard::enter(crate::board::terminal::Crossterm::default()).unwrap();
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout())).unwrap();
    assert_eq!(
        terminal.size().unwrap(),
        ratatui::layout::Size::new(width, height)
    );
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut app = capture_case(
            if scene == "help" {
                "paste-single"
            } else {
                &scene
            },
            look,
            width,
            height,
        );
        if scene == "help" {
            key(&mut app, KeyCode::Esc);
            key(&mut app, KeyCode::Char('?'));
        }
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        std::fs::write(
            metadata,
            serde_json::to_vec_pretty(&editor_metadata(&app)).unwrap(),
        )
        .unwrap();
    });
    // The private-server window-renamed hook observes all preceding frame bytes.
    write!(std::io::stdout(), "\x1bk{ready}\x1b\\").unwrap();
    std::io::stdout().flush().unwrap();
    let mut quit = [0];
    std::io::stdin().read_exact(&mut quit).unwrap();
    assert_eq!(quit, [b'q']);
    screen.restore().unwrap();
}

#[test]
fn grapheme_join_fallback_keeps_every_byte_when_pasting_or_typing_a_combining_mark() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    app.paste("e");
    let raw = format!("\u{301}{}", "x".repeat(501));
    app.paste(&raw);
    let text = &app.input.as_ref().unwrap().text;
    assert_eq!(text.as_str(), format!("e{raw}"));
    assert!(
        text.folds().is_empty(),
        "joining the preceding grapheme retains ordinary source text"
    );

    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    let raw = "x".repeat(501);
    app.paste(&raw);
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
    key(&mut app, KeyCode::Char('\u{301}'));
    let text = &app.input.as_ref().unwrap().text;
    assert_eq!(text.as_str(), format!("{raw}\u{301}"));
    assert!(
        text.folds().is_empty(),
        "a joining edit removes only the chip projection, never source bytes"
    );
}

/// Draft/fold provenance and clipped geometry accompany every new scene.
pub(in crate::board) fn capture_metadata(app: &App) -> serde_json::Value {
    let rect = |area: Rect| [area.x, area.y, area.width, area.height];
    let hits = app.composer.paste_hits.borrow().iter().map(|hit| serde_json::json!({
        "area":rect(hit.area),"remove":rect(hit.remove),"draft":[hit.key.0,hit.key.1],"fold":hit.id,"source":[hit.source.start,hit.source.end],"label":hit.label
    })).collect::<Vec<_>>();
    let preview = app.current_paste_preview().map(|preview| {
        let scroll = preview.scroll.borrow();
        serde_json::json!({"fold":preview.id,"viewport":rect(scroll.viewport()),"offset":scroll.offset(),"content":scroll.content()})
    });
    serde_json::json!({"chips":hits,"preview":preview})
}

#[test]
fn too_small_preview_and_shrinking_layout_never_capture_invisible_input() {
    let mut app = crate::board::app::workspace::tests::board();
    key(&mut app, KeyCode::Enter);
    let raw = "x".repeat(1123);
    app.paste(&raw);
    draw(&mut app, 80, 8);
    app.preview_paste_at_cursor();
    assert!(
        !app.paste_preview_shown(),
        "no invisible modal when the pane cannot fit four rows"
    );
    key(&mut app, KeyCode::Char('z'));
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), format!("{raw}z"));
    key(&mut app, KeyCode::Backspace);
    draw(&mut app, 100, 36);
    app.preview_paste_at_cursor();
    assert!(app.paste_preview_shown());
    draw(&mut app, 100, 36);
    draw(&mut app, 80, 8);
    assert!(
        !app.paste_preview_shown(),
        "a preview that can no longer paint closes"
    );
    key(&mut app, KeyCode::Char('z'));
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), format!("{raw}z"));
    draw(&mut app, 100, 36);
    assert!(
        !app.paste_preview_shown(),
        "enlarging the pane never resurrects a dismissed preview"
    );
    assert_eq!(app.input.as_ref().unwrap().text.folds().len(), 1);
}
