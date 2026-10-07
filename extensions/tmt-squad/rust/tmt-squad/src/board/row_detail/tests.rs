use super::*;
use crate::board::app;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

fn fixture() -> App {
    let mut app = App::new(Some("product".into()));
    app.apply(app::tests::snapshot("product",json!([{"rows":[
        {"id":"u1","name":"one","fields":{"task":"ship the feature","note":"watch CI","pr_link":"https://example.test/pr"}},
        {"id":"u2","name":"two","fields":{}}
    ]}])));
    app.view.as_mut().unwrap().bindings = crate::action::preset(true, &[]);
    app.view.as_mut().unwrap().board.members = true;
    app
}
fn press(app: &mut App, key: KeyCode) -> Effect {
    app.key(KeyEvent::new(key, KeyModifiers::NONE))
}
fn draw(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| super::super::view::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn text(buffer: &ratatui::buffer::Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn e_toggles_in_place_without_pane_focus_or_a_reply_and_arrows_move_rows() {
    let mut app = fixture();
    let target = Target::Row(app.row_target(0).unwrap());
    assert!(!text(&draw(&app, 80, 24)).contains("no reply yet"));
    app.focus = 1;
    press(&mut app, KeyCode::Char('e'));
    assert!(app.row_details.contains(&target));
    assert!(app.input.is_none());
    let buffer = draw(&app, 80, 24);
    let shown = text(&buffer);
    assert!(
        shown.contains("note")
            && shown.contains("ship the feature")
            && shown.contains("no reply yet")
    );
    assert_eq!(
        shown.matches("ship the feature").count(),
        1,
        "the visible task is not repeated in detail"
    );
    press(&mut app, KeyCode::Down);
    assert_eq!(app.selected, 1);
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(app.row_details.expanded.len(), 2);
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(app.row_details.expanded, vec![target]);
}

#[test]
fn disappearing_rows_and_replaced_requests_drop_expansions_and_late_bodies() {
    let mut app = fixture();
    app.view.as_mut().unwrap().me_id = Some("user".into());
    app.view.as_mut().unwrap().replies = vec![json!({"recipientId":"u1","requestId":"r1"})];
    press(&mut app, KeyCode::Char('e'));
    let key = app.detail_read().unwrap();
    press(&mut app, KeyCode::Char('e'));
    app.apply_message(&key, Ok("late collapsed body".into()));
    assert!(app.row_details.bodies.is_empty());
    press(&mut app, KeyCode::Char('e'));
    app.view.as_mut().unwrap().replies[0]["requestId"] = json!("r2");
    app.apply_message(&key, Ok("stale replaced request".into()));
    assert!(app.row_details.bodies.is_empty());
    let key = app.detail_read().unwrap();
    app.view.as_mut().unwrap().document["sections"][0]["rows"] = json!([]);
    app.reconcile_row_details();
    app.apply_message(&key, Ok("late removed row".into()));
    assert!(app.row_details.expanded.is_empty() && app.row_details.bodies.is_empty());
    for index in 0..100 {
        app.row_details.toggle(Target::Job(format!("gone/{index}")));
        app.reconcile_row_details();
        assert!(app.row_details.expanded.is_empty());
    }
}

#[test]
fn v_reader_is_read_only_and_preserves_the_normal_enter_and_open_actions() {
    let mut app = fixture();
    press(&mut app, KeyCode::Char('v'));
    assert!(app.row_details.reader.is_none());
    app.view.as_mut().unwrap().replies = vec![
        json!({"recipientId":"u1","requestId":"r1","response":"full **reply**\n\n".repeat(30)}),
    ];
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        Effect::Act(app::Request::Jump("one".into()))
    );
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        Effect::Act(app::Request::Jump("one".into()))
    );
    assert!(
        matches!(press(&mut app,KeyCode::Char('o')),Effect::Act(app::Request::Open{link,..}) if link=="https://example.test/pr")
    );
    assert!(app.reply_hint());
    press(&mut app, KeyCode::Char('v'));
    let _ = draw(&app, 80, 24);
    for key in [
        KeyCode::Char('x'),
        KeyCode::Enter,
        KeyCode::Backspace,
        KeyCode::Tab,
    ] {
        assert_eq!(press(&mut app, key), Effect::None);
    }
    assert!(app.input.is_none());
    press(&mut app, KeyCode::PageDown);
    assert!(
        app.row_details
            .reader
            .as_ref()
            .unwrap()
            .scroll
            .borrow()
            .offset()
            > 0
    );
    press(&mut app, KeyCode::Esc);
    assert!(app.row_details.reader.is_none() && app.reply_hint());
}

#[test]
fn a_removed_reply_closes_its_reader_but_keeps_the_rows_other_details() {
    let mut app = fixture();
    app.view.as_mut().unwrap().me_id = Some("user".into());
    app.view.as_mut().unwrap().replies = vec![json!({"recipientId":"u1","requestId":"r1"})];
    press(&mut app, KeyCode::Char('e'));
    press(&mut app, KeyCode::Char('v'));
    let key = app.detail_read().unwrap();
    assert!(app.row_details.reader.is_some());
    app.view.as_mut().unwrap().replies.clear();
    app.reconcile_row_details();
    assert!(app.row_details.reader.is_none());
    assert_eq!(app.row_details.expanded.len(), 1);
    app.apply_message(&key, Ok("late removed reply".into()));
    assert!(app.row_details.bodies.is_empty());
    let shown = text(&draw(&app, 80, 24));
    assert!(shown.contains("watch CI") && shown.contains("no reply yet"));
    assert_eq!(
        press(&mut app, KeyCode::Enter),
        Effect::Act(app::Request::Jump("one".into()))
    );
}

#[test]
fn the_shared_renderer_wraps_limits_and_keeps_only_the_gutter_selected() {
    for width in [40, 80, 160] {
        for look in [
            Look::default(),
            Look::new(tmt_cli_style::Theme::new(tmt_cli_style::Base::TmtLight)),
        ] {
            let value = json!({"fields":[{"label":"pending","text":"◆ a long field ".repeat(100),"role":"waiting"}],"show_reply":true,"reply":{"from":"one","age":"1m","body":"a **styled** reply line\n\n".repeat(30)}});
            let lines = render(&value, width, 3, look, true);
            assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
            assert!(
                lines
                    .iter()
                    .any(|line| line.to_string().contains("more lines · v view"))
            );
            assert!(lines.iter().any(|line| line.to_string().contains('…')));
            assert!(lines.iter().all(|line| line.style.bg.is_none()
                && line.spans.iter().all(|span| span.style.bg.is_none())));
            assert_eq!(lines[0].spans[1].content, "│");
            assert_eq!(lines[0].spans[1].style, look.role(Role::Accent));
        }
    }
}

#[test]
fn the_overflow_line_opens_its_parent_reply_without_running_the_row_click_action() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = fixture();
    app.view.as_mut().unwrap().replies =
        vec![json!({"recipientId":"u1","requestId":"r1","response":"a paragraph\n\n".repeat(30)})];
    press(&mut app, KeyCode::Char('e'));
    let _ = draw(&app, 80, 24);
    let (hit, target) = app
        .detail_more_hits
        .borrow()
        .first()
        .cloned()
        .expect("the visible overflow line owns a hit");
    // Moving from a meter to the reply overflow clears hover without cycling it.
    let window = app.token_window;
    app.meter_hover = Some(None);
    assert_eq!(
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: hit.x + 10,
                row: hit.y,
                modifiers: KeyModifiers::NONE
            },
            std::time::Instant::now()
        ),
        Effect::None
    );
    assert_eq!(app.row_details.reader.as_ref().unwrap().target, target);
    assert_eq!(app.meter_hover, None);
    assert_eq!(app.token_window, window);
    assert!(app.input.is_none() && app.menu.is_none());
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('e'));
    let _ = draw(&app, 80, 24);
    assert!(
        app.detail_more_hits.borrow().is_empty(),
        "collapsed rows leave no old click target"
    );
}

#[test]
fn collapsed_fields_are_complete_only_when_their_entire_text_survives() {
    use tmt_tui::style::TextFlow;
    assert!(!uncut("a long field", 5, TextFlow::Truncate));
    assert!(!uncut("a long field", 5, TextFlow::Middle));
    assert!(!uncut("a long field", 5, TextFlow::Clip));
    assert!(uncut("a long field", 12, TextFlow::Truncate));
    assert!(!uncut("one two three", 5, TextFlow::Clamp(2)));
    assert!(uncut("one two three", 5, TextFlow::Clamp(3)));
    assert!(uncut("one two three", 5, TextFlow::Wrap));
    assert!(!uncut("界", 1, TextFlow::Wrap));
    assert!(!uncut("🇯🇵", 1, TextFlow::Wrap));
    assert!(uncut("界", 2, TextFlow::Truncate));
}

#[test]
fn a_boxed_members_truncated_task_and_model_remain_reachable_in_detail() {
    let mut app = fixture();
    let task = "Review the long release checklist and confirm every staging result before shipping";
    let row = &mut app.view.as_mut().unwrap().document["sections"][0]["rows"][0];
    row["fields"]["task"] = json!(task);
    row["fields"]["model"] = json!("a-long-model-name");
    press(&mut app, KeyCode::Char('e'));
    let narrow = text(&draw(&app, 80, 30));
    assert!(narrow.contains("│task"), "{narrow}");
    assert!(narrow.contains("before shipping"), "{narrow}");
    assert!(narrow.contains("│model") && narrow.contains("a-long-model-name"));
    let wide = text(&draw(&app, 160, 30));
    assert!(!wide.contains("│task"), "{wide}");
    assert_eq!(wide.matches(task).count(), 1);
    assert!(
        wide.contains("│model"),
        "fixed model track still truncates at 160"
    );
}

#[test]
fn grid_fields_use_each_rows_actual_budget_including_wrap_and_request_age() {
    let mut app = fixture();
    let task = "Review the long release checklist and confirm every staging result before shipping";
    let question = "Please confirm the final staging results and every outstanding release checklist item before the release ships";
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["fields"]["task"] = json!(task);
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["pending"] = json!(question);
    let config: toml_edit::DocumentMut =
        "[product]\nrows.columns=[{name='member',width=16},{name='task',grow=1}]\n"
            .parse()
            .unwrap();
    app.view.as_mut().unwrap().rows =
        crate::rows::read(config["product"].as_table_like(), "product").unwrap();
    app.view.as_mut().unwrap().board.members = false;
    press(&mut app, KeyCode::Char('e'));
    let narrow = text(&draw(&app, 80, 30));
    assert!(
        narrow.contains("│task") && narrow.contains("before shipping"),
        "{narrow}"
    );
    let wide = text(&draw(&app, 160, 30));
    assert!(!wide.contains("│task"), "{wide}");
    assert_eq!(wide.matches(task).count(), 1);
    assert!(
        !wide.contains("│pending"),
        "the uncut automatic pending line is not repeated"
    );
    assert!(
        narrow.contains("│pending"),
        "the cut automatic request stays readable in detail"
    );
    // Two wrapped lines reveal the whole task; a one-line clamp leaves its tail hidden.
    for (max_lines, hidden) in [(1, true), (2, false)] {
        let view = app.view.as_mut().unwrap();
        view.rows.columns[1].overflow = Some(tmt_cli_style::grid::Overflow::Wrap { max_lines });
        view.derived.borrow_mut().grid = None;
        let shown = text(&draw(&app, 80, 30));
        assert_eq!(shown.contains("│task"), hidden, "{shown}");
        assert!(shown.contains("before shipping"), "{shown}");
    }
    // The same pending value fits without the age badge and is cut with it.
    let question = "q".repeat(74);
    let row = &mut app.view.as_mut().unwrap().document["sections"][0]["rows"][0];
    row["pending"] = json!(question);
    row["waitingOnYou"] = json!([]);
    assert!(!text(&draw(&app, 80, 30)).contains("│pending"));
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["waitingOnYou"] =
        json!([{ "preparedAtMs": crate::status::now_ms() - 600_000, "preview": question }]);
    assert!(text(&draw(&app, 80, 30)).contains("│pending"));
}
