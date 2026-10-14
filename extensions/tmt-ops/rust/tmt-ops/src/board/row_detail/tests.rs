use super::*;
use crate::board::app;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

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

fn response_workspace() -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app
}
fn open_response(app: &mut App) {
    assert_eq!(
        app.perform(&crate::action::Action::parse("view-reply").unwrap()),
        Effect::None
    );
    assert!(app.row_details.reader.is_some());
}

#[test]
fn pending_workspace_reader_rejects_late_bodies_after_request_or_membership_changes() {
    use crate::board::app::workspace::tests::{MEMBER, ROOM};
    for change in ["request", "reply", "membership"] {
        let mut app = response_workspace();
        app.view.as_mut().unwrap().me_id = Some("33333333-3333-4333-8333-333333333333".into());
        app.view.as_mut().unwrap().replies = vec![json!({
            "recipientId":MEMBER,"requestId":"r1"
        })];
        let selected = app.workspace.selected.clone();
        open_response(&mut app);
        let key = app.detail_read().unwrap();
        assert!(app.row_details.reader.as_ref().unwrap().frozen.is_none());
        match change {
            "request" => app.view.as_mut().unwrap().replies[0]["requestId"] = json!("r2"),
            "reply" => app.view.as_mut().unwrap().replies.clear(),
            _ => {
                let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
                snapshot
                    .roster
                    .squads
                    .iter_mut()
                    .find(|squad| squad.room_id == ROOM)
                    .unwrap()
                    .members
                    .clear();
            }
        }
        app.reconcile_row_details();
        app.apply_message(&key, Ok("late old body must never appear".into()));
        assert!(app.row_details.bodies.is_empty(), "{change}");
        assert_eq!(app.workspace.selected, selected, "{change}");
        if change == "request" {
            let current = app.detail_read().unwrap();
            assert_eq!(current.request, "r2");
            app.apply_message(&current, Ok("current exact response".into()));
            assert!(text(&draw(&app, 100, 24)).contains("current exact response"));
        } else {
            assert!(app.row_details.reader.is_none(), "{change}");
            assert!(app.detail_read().is_none(), "{change}");
        }
        assert!(!text(&draw(&app, 100, 24)).contains("late old body"));
    }
}

#[test]
fn exact_response_selection_attaches_to_a_retained_draft_without_acknowledging() {
    let mut app = response_workspace();
    let source = "line one\n漢字 e\u{301} \x1b[31m\nline three";
    let view = app.view.as_mut().unwrap();
    view.me = Some("Ben".into());
    view.replies = vec![
        json!({"recipientId":crate::board::app::workspace::tests::MEMBER,"requestId":"r1","response":source}),
    ];
    let before = view.document.clone();
    open_response(&mut app);
    press(&mut app, KeyCode::Char('s'));
    draw(&app, 60, 18);
    let area = app
        .row_details
        .reader
        .as_ref()
        .unwrap()
        .source_area
        .get()
        .unwrap();
    for (kind, column) in [
        (
            ratatui::crossterm::event::MouseEventKind::Down(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            area.x,
        ),
        (
            ratatui::crossterm::event::MouseEventKind::Drag(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            area.x + 4,
        ),
    ] {
        assert_eq!(
            app.mouse(
                ratatui::crossterm::event::MouseEvent {
                    kind,
                    column,
                    row: area.y,
                    modifiers: KeyModifiers::NONE
                },
                std::time::Instant::now()
            ),
            Effect::None
        );
    }
    assert_eq!(
        app.row_details
            .reader
            .as_ref()
            .unwrap()
            .selection
            .as_ref()
            .unwrap()
            .quote()
            .unwrap()
            .text,
        "line"
    );
    app.invalidate_frame_geometry();
    assert!(
        app.row_details
            .reader
            .as_ref()
            .unwrap()
            .source_area
            .get()
            .is_none()
    );
    draw(&app, 100, 24);
    assert_eq!(
        app.row_details
            .reader
            .as_ref()
            .unwrap()
            .selection
            .as_ref()
            .unwrap()
            .quote()
            .unwrap()
            .range,
        0..4
    );
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        Effect::None
    );
    assert!(
        matches!(press(&mut app,KeyCode::Char('y')),Effect::Act(app::Request::Copy { text,.. }) if text==source)
    );
    assert_eq!(press(&mut app, KeyCode::Char('a')), Effect::None);
    assert!(app.row_details.reader.is_none());
    let quote = &app.input.as_ref().unwrap().response_quotes[0];
    assert_eq!(quote.key.request, "r1");
    assert_eq!(quote.key.lead, crate::board::app::workspace::tests::MEMBER);
    assert_eq!(quote.range, 0..source.len());
    assert_eq!(quote.text, source);
    assert_eq!(app.view.as_ref().unwrap().document, before);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.input.as_ref().unwrap().response_quotes[0].text, source);
    app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
    assert!(app.input.as_ref().unwrap().response_quotes.is_empty());
}

#[test]
fn scrolled_wrapped_source_drag_preserves_unicode_bytes_across_resize() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = response_workspace();
    let prefix = format!("{}\r\n", "0123456789".repeat(12)).repeat(30);
    let source = format!("{prefix}漢字 e\u{301} Ω");
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":crate::board::app::workspace::tests::MEMBER,
        "requestId":"r1", "response":source
    })];
    open_response(&mut app);
    press(&mut app, KeyCode::Char('s'));
    draw(&app, 60, 18);
    app.key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL));
    let buffer = draw(&app, 60, 18);
    let reader = app.row_details.reader.as_ref().unwrap();
    assert!(
        reader.scroll.borrow().offset() > 30,
        "long source really wraps and scrolls"
    );
    let area = reader.source_area.get().unwrap();
    let row = (area.y..area.bottom())
        .find(|row| buffer[(area.x, *row)].symbol() == "漢")
        .expect("the exact source tail is painted");
    for (kind, column) in [
        (MouseEventKind::Down(MouseButton::Left), area.x),
        (MouseEventKind::Drag(MouseButton::Left), area.x + 6),
    ] {
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now()
            ),
            Effect::None
        );
    }
    app.invalidate_frame_geometry();
    draw(&app, 160, 18);
    let selected = "漢字 e\u{301}";
    let quote = app
        .row_details
        .reader
        .as_ref()
        .unwrap()
        .selection
        .as_ref()
        .unwrap()
        .quote()
        .unwrap();
    assert_eq!(quote.range, prefix.len()..prefix.len() + selected.len());
    assert_eq!(quote.text, selected);
    assert_eq!(source.get(quote.range.clone()), Some(selected));
    assert!(matches!(press(&mut app, KeyCode::Char('y')),
        Effect::Act(app::Request::Copy { text, .. }) if text == selected));
    assert_eq!(press(&mut app, KeyCode::Char('a')), Effect::None);
    assert_eq!(app.input.as_ref().unwrap().response_quotes[0], quote);
}

#[test]
fn common_pool_response_quote_opens_talk_without_a_squad_note_or_acknowledgment() {
    use crate::board::{app::workspace::tests::MEMBER, workspace::OpsTarget};
    let mut app = response_workspace();
    let target = OpsTarget::Member {
        room_id: None,
        identity_id: MEMBER.into(),
    };
    let source = "A **literal** response\n漢字 e\u{301}";
    let view = app.view.as_mut().unwrap();
    let snapshot = &mut view.home.as_mut().unwrap().workspace;
    snapshot.roster.squads[0].members.clear();
    snapshot
        .rows
        .insert(target.clone(), json!({"id":MEMBER,"name":"worker"}));
    view.replies = vec![json!({"recipientId":MEMBER,"requestId":"r1","response":source})];
    let before = view.document.clone();
    app.workspace.pool_expanded = true;
    app.workspace.selected = Some(target);
    app.workspace_reconcile();
    open_response(&mut app);
    press(&mut app, KeyCode::Char('s'));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_eq!(press(&mut app, KeyCode::Char('a')), Effect::None);
    assert!(app.row_details.reader.is_none());
    let input = app.input.as_ref().unwrap();
    assert!(matches!(&input.compose, app::Compose::Talk { to } if to == "worker"));
    assert!(input.squad.is_empty());
    assert!(
        input
            .others
            .iter()
            .all(|mode| !matches!(mode, app::Compose::Annotate { .. }))
    );
    let quote = &input.response_quotes[0];
    assert_eq!(quote.key.lead, MEMBER);
    assert_eq!(quote.range, 0..source.len());
    assert_eq!(quote.text, source);
    let reference = quote.reference();
    assert_eq!(app.view.as_ref().unwrap().document, before);
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Effect::Act(app::Request::DirectTalk { recipient, text, .. })
            if recipient == MEMBER && text == reference
    ));
}

#[test]
fn failed_or_replaced_response_source_cannot_be_attached() {
    let mut app = response_workspace();
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().replies =
        vec![json!({"recipientId":crate::board::app::workspace::tests::MEMBER,"requestId":"r1"})];
    open_response(&mut app);
    let key = app.detail_read().unwrap();
    app.apply_message(&key, Err("Unavailable".into()));
    press(&mut app, KeyCode::Char('s'));
    assert!(app.row_details.reader.as_ref().unwrap().selection.is_none());
    app.apply_message(&key, Ok("exact source".into()));
    press(&mut app, KeyCode::Char('s'));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    app.view.as_mut().unwrap().replies[0]["requestId"] = json!("replacement");
    assert_eq!(press(&mut app, KeyCode::Char('a')), Effect::None);
    assert!(app.input.is_none());
    assert!(app.notice.as_deref().unwrap().contains("source changed"));
    app.view.as_mut().unwrap().replies.clear();
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
    app.reconcile_row_details();
    assert!(text(&draw(&app, 80, 24)).contains("exact source"));
    assert_eq!(press(&mut app, KeyCode::Char('a')), Effect::None);
    assert!(app.input.is_none(), "a vanished agent is never retargeted");
    assert!(matches!(
        press(&mut app, KeyCode::Char('y')),
        Effect::Act(app::Request::Copy { text, .. }) if text == "exact source"
    ));
    press(&mut app, KeyCode::Esc);
    assert!(app.row_details.reader.is_none());
}

#[test]
fn reader_reuses_markdown_during_scroll_and_rebuilds_on_width_or_body_change() {
    let mut app = response_workspace();
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":crate::board::app::workspace::tests::MEMBER, "requestId":"r1", "response":"**reading**  \n".repeat(80)
    })];
    open_response(&mut app);
    draw(&app, 80, 24);
    let cached = app
        .row_details
        .reader
        .as_ref()
        .unwrap()
        .rendered
        .borrow()
        .as_ref()
        .unwrap()
        .lines
        .as_ptr();
    press(&mut app, KeyCode::PageDown);
    draw(&app, 80, 24);
    assert_eq!(
        cached,
        app.row_details
            .reader
            .as_ref()
            .unwrap()
            .rendered
            .borrow()
            .as_ref()
            .unwrap()
            .lines
            .as_ptr()
    );
    draw(&app, 60, 24);
    assert!(
        app.row_details
            .reader
            .as_ref()
            .unwrap()
            .rendered
            .borrow()
            .as_ref()
            .unwrap()
            .width
            < 60
    );
    app.view.as_mut().unwrap().replies[0]["response"] = json!("replacement body");
    assert!(text(&draw(&app, 60, 24)).contains("replacement body"));
}
