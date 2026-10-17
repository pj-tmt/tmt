use super::*;
use crate::board::app::workspace::tests::MEMBER;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::json;

fn fixture(source: &str) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().replies =
        vec![json!({"recipientId":MEMBER,"requestId":"r1","response":source})];
    app
}
fn draw(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}
fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) -> Effect {
    app.mouse(
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    )
}
fn enter(app: &mut App) {
    draw(app, 100, 36);
    let hit = app
        .conversation
        .borrow()
        .reply_hits
        .first()
        .cloned()
        .unwrap();
    assert_eq!(
        mouse(
            app,
            MouseEventKind::Down(MouseButton::Left),
            hit.area.x,
            hit.area.y
        ),
        Effect::None
    );
    assert_eq!(
        mouse(
            app,
            MouseEventKind::Drag(MouseButton::Left),
            hit.area.x + 2,
            hit.area.y
        ),
        Effect::None
    );
    let state = app.conversation.borrow();
    let source = state
        .sessions
        .get(state.active.as_ref().unwrap())
        .unwrap()
        .exact_source
        .as_ref()
        .unwrap();
    assert!(
        source.selection.quote().is_none(),
        "styled drag cannot guess bytes"
    );
    assert!(
        source.painted.is_none(),
        "only a new raw-source paint grants geometry"
    );
}
fn area(app: &App) -> Rect {
    let state = app.conversation.borrow();
    state
        .sessions
        .get(state.active.as_ref().unwrap())
        .unwrap()
        .exact_source
        .as_ref()
        .unwrap()
        .painted
        .unwrap()
        .0
}
fn quote(app: &App) -> Quote {
    let state = app.conversation.borrow();
    state
        .sessions
        .get(state.active.as_ref().unwrap())
        .unwrap()
        .exact_source
        .as_ref()
        .unwrap()
        .selection
        .quote()
        .unwrap()
}
fn select(app: &mut App, column: u16) {
    let area = area(app);
    mouse(app, MouseEventKind::Down(MouseButton::Left), area.x, area.y);
    mouse(
        app,
        MouseEventKind::Drag(MouseButton::Left),
        area.x + column,
        area.y,
    );
}
fn button(app: &App) -> Option<Rect> {
    let state = app.conversation.borrow();
    state
        .sessions
        .get(state.active.as_ref().unwrap())
        .unwrap()
        .exact_source
        .as_ref()
        .unwrap()
        .annotate
        .as_ref()
        .map(|(area, _)| *area)
}

#[test]
fn styled_drag_requires_a_new_exact_paint_and_annotation_keeps_unicode_raw_bytes() {
    let raw = "漢字 e\u{301} **literal**\n\x1b[31m controls";
    let mut app = fixture(raw);
    enter(&mut app);
    let shown = draw(&mut app, 100, 36);
    assert!(
        shown
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .contains("Exact response source")
    );
    assert!(button(&app).is_none());
    select(&mut app, 6);
    let quote = quote(&app);
    assert_eq!(quote.text, "漢字 e\u{301}");
    assert_eq!(raw.get(quote.range.clone()), Some(quote.text.as_str()));
    assert_eq!(quote.key.request, "r1");
    assert_eq!(quote.key.lead, MEMBER);
    draw(&mut app, 160, 36);
    let hit = button(&app).unwrap();
    assert_eq!(
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            hit.x,
            hit.y
        ),
        Effect::None
    );
    assert_eq!(
        app.input.as_ref().unwrap().response_quotes.as_slice(),
        std::slice::from_ref(&quote)
    );
    assert_eq!(
        app.input.as_ref().unwrap().draft_key.as_ref().unwrap().1,
        MEMBER
    );
    let Effect::Act(crate::board::app::Request::Annotate { text, to, .. }) =
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("explicit annotation effect");
    };
    assert_eq!(to, "worker");
    assert_eq!(text, "[product · worker]\n> 漢字 e\u{301}");
}

#[test]
fn resize_scroll_and_changed_source_refuse_old_pointer_authority() {
    for change in ["resize", "scroll", "request", "body", "removed"] {
        let mut app = fixture(&format!("{}\n", "source line ".repeat(30)).repeat(30));
        enter(&mut app);
        draw(&mut app, 100, 36);
        select(&mut app, 6);
        let selected = quote(&app);
        draw(&mut app, 100, 36);
        let old_area = area(&app);
        let old_button = button(&app).unwrap();
        match change {
            "resize" => app.invalidate_frame_geometry(),
            "scroll" => app.scroll_pane(Pane::Replies, crate::board::scroll::Step::Lines(3)),
            "request" => app.view.as_mut().unwrap().replies[0]["requestId"] = json!("r2"),
            "body" => app.view.as_mut().unwrap().replies[0]["response"] = json!("changed"),
            _ => app
                .view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .roster
                .squads[0]
                .members
                .clear(),
        }
        assert_eq!(
            mouse(
                &mut app,
                MouseEventKind::Drag(MouseButton::Left),
                old_area.x + 12,
                old_area.y
            ),
            Effect::None
        );
        assert_eq!(
            mouse(
                &mut app,
                MouseEventKind::Down(MouseButton::Left),
                old_button.x,
                old_button.y
            ),
            Effect::None
        );
        assert!(app.input.is_none(), "{change}: stale control cannot attach");
        if matches!(change, "resize" | "scroll") {
            assert_eq!(
                quote(&app),
                selected,
                "{change}: stale position cannot change bytes"
            );
            draw(&mut app, 60, 36);
        } else if change != "removed" {
            draw(&mut app, 100, 36);
            assert!(
                button(&app).is_none(),
                "{change}: changed source hides Annotate"
            );
        }
    }
}

#[test]
fn inline_quote_tracks_renamed_uuid_and_escape_dismisses_one_owner_per_press() {
    let mut app = fixture("same identity source");
    enter(&mut app);
    draw(&mut app, 100, 36);
    select(&mut app, 4);
    let quote = quote(&app);
    let selected = app.workspace.selected.clone().unwrap();
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.members.get_mut(MEMBER).unwrap().name = "renamed".into();
    home.workspace.rows.get_mut(&selected).unwrap()["name"] = json!("renamed");
    draw(&mut app, 100, 36);
    let hit = button(&app).unwrap();
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        hit.x,
        hit.y,
    );
    assert_eq!(app.input.as_ref().unwrap().response_quotes, [quote]);
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, crate::board::app::Compose::Annotate { to, .. } if to == "renamed")
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.input.is_none());
    assert_eq!(app.focused(), Pane::Rows);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.input.as_ref().unwrap().response_quotes.len(), 1);

    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    enter(&mut app);
    draw(&mut app, 100, 36);
    app.overview_details();
    assert!(app.monitor_details.is_some());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.monitor_details.is_none());
    assert!(
        app.conversation
            .borrow()
            .sessions
            .values()
            .any(|session| session.exact_source.is_some())
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        app.conversation
            .borrow()
            .sessions
            .values()
            .all(|session| session.exact_source.is_none())
    );
    assert_eq!(
        app.focused(),
        Pane::Replies,
        "source Esc stays at conversation level"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.focused(), Pane::Rows);
}

#[test]
fn wrapped_scrolled_control_source_keeps_exact_combining_bytes_after_resize() {
    let prefix = format!("{}\r\n", "0123456789".repeat(12)).repeat(30);
    let raw = format!("{prefix}漢字 e\u{301} \x1b[31m literal");
    let mut app = fixture(&raw);
    enter(&mut app);
    draw(&mut app, 100, 36);
    app.scroll_pane(Pane::Replies, crate::board::scroll::Step::Bottom);
    let shown = draw(&mut app, 60, 36);
    let area = area(&app);
    let row = (area.y..area.bottom())
        .find(|row| shown[(area.x, *row)].symbol() == "漢")
        .unwrap();
    assert!(app.scrolls.offset(Pane::Replies) > 30);
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        area.x,
        row,
    );
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        area.x + 6,
        row,
    );
    let selected = quote(&app);
    assert_eq!(
        selected.range,
        prefix.len()..prefix.len() + "漢字 e\u{301}".len()
    );
    assert_eq!(selected.text, "漢字 e\u{301}");
    draw(&mut app, 160, 36);
    assert_eq!(quote(&app), selected);
    let hit = button(&app).unwrap();
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        hit.x,
        hit.y,
    );
    assert_eq!(app.input.as_ref().unwrap().response_quotes, [selected]);
}

#[test]
fn paused_byte_anchor_and_selection_survive_reflow_without_stale_pointer_authority() {
    let raw = format!(
        "{}TAIL_END",
        format!("{}\r\n", "漢 e\u{301} 0123456789 ".repeat(15)).repeat(80)
    );
    let mut app = fixture(&raw);
    enter(&mut app);
    draw(&mut app, 100, 36);
    let anchor = |app: &App| {
        let state = app.conversation.borrow();
        state.sessions[state.active.as_ref().unwrap()]
            .exact_source
            .as_ref()
            .unwrap()
            .anchor
    };
    app.scroll_pane(Pane::Replies, crate::board::scroll::Step::Lines(7));
    let first = anchor(&app);
    app.scroll_pane(Pane::Replies, crate::board::scroll::Step::Lines(5));
    let kept = anchor(&app);
    assert!(matches!((first, kept), (Anchor::Byte(a), Anchor::Byte(b)) if b > a));
    draw(&mut app, 100, 36);
    select(&mut app, 6);
    let selected = quote(&app);
    assert_eq!(
        raw.get(selected.range.clone()),
        Some(selected.text.as_str())
    );
    for width in [60, 160, 100] {
        let stale = area(&app);
        app.invalidate_frame_geometry();
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            stale.x,
            stale.y,
        );
        app.scroll_pane(Pane::Replies, crate::board::scroll::Step::Lines(0));
        app.scroll_pane(Pane::Replies, crate::board::scroll::Step::Lines(3));
        assert_eq!(anchor(&app), kept);
        assert_eq!(quote(&app), selected);
        draw(&mut app, width, 36);
        assert_eq!(anchor(&app), kept, "paint cannot replace the source anchor");
        assert_eq!(quote(&app), selected);
        let painted = area(&app);
        let state = app.conversation.borrow();
        let source = state.sessions[state.active.as_ref().unwrap()]
            .exact_source
            .as_ref()
            .unwrap();
        let layout = source.selection.source.layout(painted.width);
        let first = &layout.lines[app.scrolls.offset(Pane::Replies)];
        let Anchor::Byte(byte) = kept else {
            panic!("relative scrolling must retain a byte anchor")
        };
        assert!(first.start <= byte && byte <= first.end);
        assert!(raw.is_char_boundary(byte));
    }
    app.scroll_pane(Pane::Replies, crate::board::scroll::Step::Bottom);
    for width in [60, 160, 100] {
        app.invalidate_frame_geometry();
        let shown = draw(&mut app, width, 36);
        assert_eq!(anchor(&app), Anchor::Bottom);
        assert_eq!(quote(&app), selected);
        let text = shown
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("TAIL_END"), "Bottom remains the visible tail");
    }
}

#[test]
fn common_pool_inline_quote_opens_pinned_talk_and_never_a_squad_note() {
    use crate::board::workspace::OpsTarget;
    let mut app = fixture("literal Common pool response");
    let target = OpsTarget::Member {
        room_id: None,
        identity_id: MEMBER.into(),
    };
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.squads[0].members.clear();
    home.workspace.rows.insert(
        target.clone(),
        json!({"id":MEMBER,"name":"worker","state":"working"}),
    );
    app.workspace.pool_expanded = true;
    app.workspace.selected = Some(target);
    app.workspace_reconcile();
    enter(&mut app);
    draw(&mut app, 100, 36);
    select(&mut app, 7);
    assert_eq!(quote(&app).text, "literal");
    draw(&mut app, 100, 36);
    let hit = button(&app).unwrap();
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        hit.x,
        hit.y,
    );
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, crate::board::app::Compose::Talk { to } if to == "worker")
    );
    assert!(app.input.as_ref().unwrap().squad.is_empty());
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(crate::board::app::Request::DirectTalk { recipient, text, .. }) if recipient == MEMBER && text == "[Common pool · worker]\n> literal")
    );
}

#[test]
fn inline_selection_and_drafts_never_move_between_recipient_uuids() {
    use crate::board::workspace::OpsTarget;
    const OTHER: &str = "44444444-4444-4444-8444-444444444444";
    let mut app = fixture("alpha exact source");
    let target_a = app.workspace.selected.clone().unwrap();
    let target_b = OpsTarget::Member {
        room_id: Some(crate::board::app::workspace::tests::ROOM.into()),
        identity_id: OTHER.into(),
    };
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    let mut member = home.workspace.roster.members[MEMBER].clone();
    member.id = OTHER.into();
    member.name = "other".into();
    home.workspace.roster.members.insert(OTHER.into(), member);
    home.workspace.roster.squads[0].members.push(OTHER.into());
    home.workspace.rows.insert(
        target_b.clone(),
        json!({"id":OTHER,"name":"other","state":"working"}),
    );
    app.view
        .as_mut()
        .unwrap()
        .replies
        .push(json!({"recipientId":OTHER,"requestId":"r2","response":"beta source"}));
    app.workspace_reconcile();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.paste("alpha draft");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    enter(&mut app);
    draw(&mut app, 100, 36);
    select(&mut app, 5);
    let selected = quote(&app);
    app.workspace.selected = Some(target_b);
    app.workspace_reconcile();
    draw(&mut app, 100, 36);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.paste("beta draft");
    assert!(app.input.as_ref().unwrap().response_quotes.is_empty());
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.workspace.selected = Some(target_a);
    app.workspace_reconcile();
    draw(&mut app, 100, 36);
    assert_eq!(quote(&app), selected);
    let hit = button(&app).unwrap();
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        hit.x,
        hit.y,
    );
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "alpha draft");
    assert_eq!(app.input.as_ref().unwrap().response_quotes, [selected]);
    assert_eq!(
        app.input.as_ref().unwrap().draft_key.as_ref().unwrap().1,
        MEMBER
    );
}

#[test]
fn outgoing_text_and_status_have_no_response_selection_authority() {
    let mut app = fixture("baseline exact response");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.paste("outgoing is not response source");
    let Effect::Act(request) = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) else {
        panic!("explicit send");
    };
    let ticket = app.outgoing_ticket(&request).unwrap();
    app.finished_send(crate::board::conversation::outgoing::Completed {
        ticket: Some(ticket),
        outcome: Ok(crate::send::Accepted {
            request: "sent-request".into(),
            disposition: crate::send::Disposition::Sent,
        }),
        what: "Message".into(),
        to: "worker".into(),
    });
    app.view.as_mut().unwrap().replies.insert(
        0,
        json!({"recipientId":MEMBER,"requestId":"sent-request","response":"actual exact reply"}),
    );
    let shown = draw(&mut app, 160, 44);
    let state = app.conversation.borrow();
    let hits = state.reply_hits.clone();
    for hit in &hits {
        let text: String = (hit.area.x..hit.area.right())
            .map(|x| shown[(x, hit.area.y)].symbol())
            .collect();
        assert!(
            text.contains("baseline exact response") || text.contains("actual exact reply"),
            "{text}"
        );
    }
    let hit = hits
        .iter()
        .find(|hit| hit.key.request == "sent-request")
        .cloned()
        .unwrap();
    drop(state);
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        hit.area.x,
        hit.area.y,
    );
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        hit.area.x + 4,
        hit.area.y,
    );
    draw(&mut app, 160, 44);
    select(&mut app, 6);
    assert_eq!(quote(&app).text, "actual");
    assert_eq!(quote(&app).key.request, "sent-request");
}
