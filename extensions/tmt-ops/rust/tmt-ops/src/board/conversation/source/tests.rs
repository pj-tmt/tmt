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
    let selected = state
        .sessions
        .get(state.active.as_ref().unwrap())
        .unwrap()
        .exact_source
        .as_ref()
        .unwrap()
        .selection
        .quote()
        .unwrap();
    let Origin::Response(key) = selected.key else {
        panic!("response fixture source");
    };
    Quote {
        key,
        range: selected.range,
        text: selected.text,
    }
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
        .quote_button
        .as_ref()
        .map(|(area, _)| *area)
}

#[test]
fn styled_drag_requires_a_new_exact_paint_and_chat_quote_keeps_unicode_text() {
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
    let draft = app.input.as_ref().unwrap();
    assert!(
        draft.response_quotes.is_empty(),
        "chat insertion is not a row-note attachment"
    );
    assert_eq!(draft.text.as_str(), "> \"漢字 e\u{301}\"\n");
    assert_eq!(draft.text.cursor(), draft.text.len());
    assert!(draft.text.selection().is_empty());
    assert_eq!(
        app.input.as_ref().unwrap().draft_key.as_ref().unwrap().1,
        MEMBER
    );
    let Effect::Act(crate::board::app::Request::Talk { text, to, .. }) =
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("only explicit Send delivers the chat quote");
    };
    assert_eq!(to, "worker");
    assert_eq!(text, "[product · worker] > \"漢字 e\u{301}\"\n");
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
    assert!(app.input.as_ref().unwrap().response_quotes.is_empty());
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("> \"{}\"\n", quote.text)
    );
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, crate::board::app::Compose::Talk { to } if to == "renamed")
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.input.is_none());
    assert_eq!(app.focused(), Pane::Rows);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.input.as_ref().unwrap().response_quotes.is_empty());
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("> \"{}\"\n", quote.text)
    );

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
    assert!(app.input.as_ref().unwrap().response_quotes.is_empty());
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("> \"{}\"\n", selected.text)
    );
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
        matches!(app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), Effect::Act(crate::board::app::Request::DirectTalk { recipient, text, .. }) if recipient == MEMBER && text == "[Common pool · worker] > \"literal\"\n")
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
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("alpha draft\n> \"{}\"\n", selected.text)
    );
    assert!(app.input.as_ref().unwrap().response_quotes.is_empty());
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

#[test]
fn committed_exact_selection_copies_the_raw_text_without_sending() {
    let mut app = fixture("漢字 e\u{301} **literal**");
    enter(&mut app);
    draw(&mut app, 100, 36);
    select(&mut app, 6);
    let selected = quote(&app);
    let area = area(&app);
    let effect = mouse(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        area.x + 6,
        area.y,
    );
    let Effect::Act(crate::board::app::Request::Copy {
        text,
        feedback: crate::board::app::CopyFeedback::Selection,
        ..
    }) = effect
    else {
        panic!("committing a nonempty exact-source selection must copy, never send: {effect:?}");
    };
    assert_eq!(text, selected.text);
    assert_eq!(text, "漢字 e\u{301}");
    assert_eq!(text.chars().count(), 5);
    assert!(
        app.input.is_none(),
        "selection alone never creates a draft or sends"
    );
    assert_eq!(
        quote(&app),
        selected,
        "copy leaves the exact selection available"
    );
}

// Make forbidden Core invocation observable, with a positive control proving
// the marker is live before exercising the clipboard-only worker.
fn core_recorder(root: &std::path::Path) -> (crate::core::Core, std::path::PathBuf) {
    let program = root.join("core-recorder");
    let called = program.with_extension("invoked");
    crate::test_support::write_ready_executable(
        &program,
        "#!/bin/sh\nprintf called > \"$0.invoked\"\nexit 9\n",
    );
    assert_eq!(
        std::process::Command::new(&program)
            .status()
            .unwrap()
            .code(),
        Some(9)
    );
    assert_eq!(std::fs::read_to_string(&called).unwrap(), "called");
    std::fs::remove_file(&called).unwrap();
    (crate::core::Core::at(program), called)
}

#[test]
fn selection_clipboard_feedback_counts_scalars_and_failure_keeps_quote_available() {
    let root = std::env::temp_dir().join(format!("ops-selection-clipboard-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let program = root.join("clipboard");
    crate::test_support::write_ready_executable(&program, "#!/bin/sh\ncat > \"$1\"\nexit \"$2\"\n");
    let (core, core_called) = core_recorder(&root);
    for status in [0, 7] {
        let copied = root.join(format!("observed-{status}"));
        let mut app = fixture("漢字 e\u{301} **literal**");
        app.view.as_mut().unwrap().clipboard = Some(vec![
            program.display().to_string(),
            copied.display().to_string(),
            status.to_string(),
        ]);
        enter(&mut app);
        draw(&mut app, 100, 36);
        select(&mut app, 6);
        let selected = quote(&app);
        let area = area(&app);
        let Effect::Act(request) = mouse(
            &mut app,
            MouseEventKind::Up(MouseButton::Left),
            area.x + 6,
            area.y,
        ) else {
            panic!("selection commits through the existing worker effect");
        };
        let outcome = crate::board::execute(&core, request).map(|outcome| match outcome {
            crate::board::ActionOutcome::Message(message) => message,
            _ => panic!("clipboard cannot produce a message-delivery outcome"),
        });
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), selected.text);
        if status == 0 {
            assert_eq!(outcome.as_deref(), Ok("⧉ Copied 5 characters"));
            assert_eq!(
                crate::board::glyph_guard::glyph_error("⧉ Copied 5 characters"),
                None
            );
        } else {
            assert!(outcome.as_ref().unwrap_err().starts_with("Could not copy:"));
        }
        app.finished(outcome);
        assert_eq!(app.notice_clock.failed, status != 0);
        assert!(
            app.input.is_none(),
            "copy success or failure never sends or opens a draft"
        );
        assert_eq!(quote(&app), selected);
        draw(&mut app, 100, 36);
        assert!(
            button(&app).is_some(),
            "clipboard failure leaves Quote available"
        );
    }
    assert!(
        !core_called.exists(),
        "the clipboard worker must not invoke Core"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn keyboard_selection_copies_and_enter_appends_to_the_retained_draft_without_sending() {
    let mut app = fixture("first\nsecond");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.paste("existing draft");
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    enter(&mut app);
    draw(&mut app, 100, 36);
    assert!(
        matches!(app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        Effect::Act(crate::board::app::Request::Copy { text, feedback: crate::board::app::CopyFeedback::Selection, .. })
            if text == "first\nsecond")
    );
    draw(&mut app, 100, 36);
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    let input = app.input.as_ref().unwrap();
    assert_eq!(input.text.as_str(), "existing draft\n> \"first second\"\n");
    assert_eq!(input.text.cursor(), input.text.len());
    assert!(input.text.selection().is_empty());
    assert!(input.response_quotes.is_empty());
    assert_eq!(input.draft_key.as_ref().unwrap().1, MEMBER);
    assert_eq!(app.focused(), Pane::Replies);
    let repeat = KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        ratatui::crossterm::event::KeyEventKind::Repeat,
    );
    assert_eq!(
        app.key(repeat),
        Effect::None,
        "opening Quote never replays Enter as Send"
    );
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        "existing draft\n> \"first second\"\n"
    );
}

#[test]
fn quote_chip_follows_the_selected_end_and_clipping_never_publishes_a_hit() {
    let mut app = fixture(&"x".repeat(55));
    enter(&mut app);
    draw(&mut app, 100, 36);
    select(&mut app, 50);
    let selected_area = area(&app);
    let painted = draw(&mut app, 100, 36);
    let hit = button(&app).unwrap();
    assert_eq!((hit.x, hit.y), (selected_area.x + 50, selected_area.y));
    assert_eq!(
        painted[(selected_area.x, selected_area.y)].bg,
        app.look()
            .selection()
            .bg
            .unwrap_or(ratatui::style::Color::Reset)
    );
    draw(&mut app, 60, 36);
    assert!(
        button(&app).is_none(),
        "a chip that cannot fit whole has no hit"
    );
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("> \"{}\"\n", "x".repeat(50))
    );
}

#[test]
fn an_outside_press_clears_the_selection_and_chip_without_inserting_a_quote() {
    let mut app = fixture("selected source");
    enter(&mut app);
    draw(&mut app, 100, 36);
    select(&mut app, 4);
    draw(&mut app, 100, 36);
    assert!(button(&app).is_some());
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 0, 35);
    assert!(app.input.is_none());
    assert!(
        app.conversation
            .borrow()
            .sessions
            .values()
            .all(|session| session.exact_source.is_none())
    );
}

#[test]
fn a_filtered_out_recipient_cannot_receive_a_keyboard_or_direct_quote() {
    let mut app = fixture("selected source");
    enter(&mut app);
    draw(&mut app, 100, 36);
    select(&mut app, 4);
    let selected = quote(&app);
    let target = Target::Row(RowTarget::Workspace(
        app.workspace.selected.clone().unwrap(),
    ));
    app.search = "does-not-match-any-agent".into();
    app.workspace_reconcile();
    assert!(!app.workspace_selection_visible());
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    assert!(app.input.is_none());
    let result = app.insert_response_quote(target, selected);
    assert!(matches!(result, Attachment::Refused(_)));
    assert_eq!(result.finish(&mut app), Effect::None);
    assert!(app.input.is_none());
}

#[test]
fn an_outside_tab_press_clears_selection_and_still_opens_its_control() {
    let mut app = fixture("selected source");
    enter(&mut app);
    draw(&mut app, 160, 36);
    select(&mut app, 4);
    draw(&mut app, 160, 36);
    let tab = app
        .tab_hits
        .borrow()
        .iter()
        .find(|hit| hit.tab == 0)
        .copied()
        .unwrap();
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        tab.x,
        tab.y,
    );
    assert!(
        app.shell.tab == crate::board::shell::OpsTab::Overview,
        "clearing selection must not steal the tab's valid hit"
    );
    assert!(
        app.conversation
            .borrow()
            .sessions
            .values()
            .all(|session| session.exact_source.is_none())
    );
    assert!(app.input.is_none());
}

fn task_fixture() -> App {
    use crate::board::workspace::OpsTarget;
    let mut app = fixture("unrelated response");
    app.view.as_mut().unwrap().replies.clear();
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
        .unwrap()["fields"] = json!({"task":"Review **literal**\n漢字", "next":"Ship the fix"});
    app.workspace.selected = Some(OpsTarget::Squad(
        crate::board::app::workspace::tests::ROOM.into(),
    ));
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads[0]
        .lead = Some(MEMBER.into());
    app.workspace_reconcile();
    app
}

#[test]
fn task_fields_open_the_same_exact_selector_and_insert_compact_chat_text_without_request_authority()
{
    let mut app = task_fixture();
    draw(&mut app, 160, 44);
    let field = app.conversation.borrow().task_hits.first().unwrap().area;
    assert_eq!(
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            field.x,
            field.y
        ),
        Effect::None
    );
    assert!(app.input.is_none());
    assert_eq!(
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            field.x + 2,
            field.y
        ),
        Effect::None
    );
    let shown = draw(&mut app, 160, 44);
    assert!(
        shown
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .contains("Exact task field")
    );
    assert!(app.input.is_none());
    let Effect::Act(crate::board::app::Request::Copy { text, .. }) =
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
    else {
        panic!("exact task selection copies through the same worker");
    };
    assert_eq!(text, "Review **literal**\n漢字");
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Effect::None
    );
    let input = app.input.as_ref().unwrap();
    assert_eq!(input.text.as_str(), "> \"Review **literal** 漢字\"\n");
    assert_eq!(input.draft_key.as_ref().unwrap().1, MEMBER);
    assert!(
        input.response_quotes.is_empty(),
        "a task field never becomes an exchange source attachment"
    );
    assert!(matches!(&input.compose, crate::board::app::Compose::Talk {to} if to == "worker"));
    assert!(
        app.view.as_ref().unwrap().replies.is_empty(),
        "selection acquires or acknowledges no request"
    );
}

#[test]
fn task_selection_refuses_a_changed_field_sender_or_frame_before_opening() {
    for change in ["field", "sender", "frame"] {
        let mut app = task_fixture();
        draw(&mut app, 160, 44);
        let field = app.conversation.borrow().task_hits.first().unwrap().area;
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            field.x,
            field.y,
        );
        match change {
            "field" => {
                let target = crate::board::workspace::OpsTarget::Member {
                    room_id: Some(crate::board::app::workspace::tests::ROOM.into()),
                    identity_id: MEMBER.into(),
                };
                app.view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .rows
                    .get_mut(&target)
                    .unwrap()["fields"]["task"] = json!("changed");
            }
            "sender" => {
                app.view.as_mut().unwrap().me_id =
                    Some("44444444-4444-4444-8444-444444444444".into())
            }
            _ => app.invalidate_frame_geometry(),
        }
        assert_eq!(
            mouse(
                &mut app,
                MouseEventKind::Drag(MouseButton::Left),
                field.x + 2,
                field.y
            ),
            Effect::None
        );
        assert!(app.input.is_none(), "{change}");
        assert!(
            app.conversation
                .borrow()
                .sessions
                .values()
                .all(|session| session.exact_source.is_none()),
            "{change}: no stale field source"
        );
    }
}

#[test]
fn a_task_selection_cannot_supply_an_exact_row_note_attachment() {
    let mut app = task_fixture();
    draw(&mut app, 160, 44);
    let field = app.conversation.borrow().task_hits.first().unwrap().area;
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        field.x,
        field.y,
    );
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        field.x + 2,
        field.y,
    );
    draw(&mut app, 160, 44);
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    app.perform(&crate::action::Action::parse("annotate member").unwrap());
    let input = app
        .input
        .as_ref()
        .expect("ordinary configured note still opens");
    assert!(input.text.as_str().is_empty());
    assert!(
        input.response_quotes.is_empty(),
        "plain note never autoquotes a task"
    );
}

#[test]
fn a_changed_task_body_actor_or_membership_cannot_copy_or_quote_a_retained_range() {
    for change in ["body", "actor", "membership"] {
        let mut app = task_fixture();
        draw(&mut app, 160, 44);
        let field = app.conversation.borrow().task_hits.first().unwrap().area;
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            field.x,
            field.y,
        );
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            field.x + 2,
            field.y,
        );
        draw(&mut app, 160, 44);
        let target = app.workspace.selected.clone().unwrap();
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        match change {
            "body" => {
                app.view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .rows
                    .get_mut(&target)
                    .unwrap()["fields"]["task"] = json!("replacement")
            }
            "actor" => {
                app.view.as_mut().unwrap().me_id =
                    Some("44444444-4444-4444-8444-444444444444".into())
            }
            _ => app
                .view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .roster
                .squads
                .clear(),
        }
        app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Effect::None,
            "{change}: stale source cannot copy"
        );
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Effect::None
        );
        assert!(app.input.is_none(), "{change}: stale source cannot insert");
    }
}

#[test]
#[ignore = "exports exact-selection renderer evidence without changing frozen fixtures"]
fn capture_copy_quote_packet() {
    use tmt_cli_style::{Base, Depth, Theme};
    let path = std::env::var_os("TMT_SELECTION_PACKET").expect("task-owned packet path");
    let root = std::env::temp_dir().join(format!("ops-selection-packet-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let clipboard = root.join("clipboard");
    crate::test_support::write_ready_executable(
        &clipboard,
        "#!/bin/sh\ncat > /dev/null\nexit \"$1\"\n",
    );
    let (core, core_called) = core_recorder(&root);
    let mut frames = Vec::new();
    for (base, depth) in [
        ("tmt", Depth::TrueColor),
        ("tmt-light", Depth::TrueColor),
        ("tmt", Depth::None),
    ] {
        for width in [160, 100, 60] {
            for scene in [
                "response-selected",
                "copy-failed",
                "quoted-draft",
                "task-selected",
                "overview-task-selected",
                "reader-selected",
            ] {
                let task = matches!(scene, "task-selected" | "overview-task-selected");
                let mut app = if task {
                    task_fixture()
                } else {
                    fixture("Review **literal** source\nSecond line 漢字 e\u{301}")
                };
                if scene == "overview-task-selected" {
                    app.shell.tab = crate::board::shell::OpsTab::Overview;
                    app.overview_view = crate::board::overview::monitor::View::Agents;
                    app.workspace.selected = Some(crate::board::workspace::OpsTarget::Member {
                        room_id: Some(crate::board::app::workspace::tests::ROOM.into()),
                        identity_id: MEMBER.into(),
                    });
                    app.workspace_reconcile();
                }
                let look = crate::look::Look {
                    theme: Theme::new(Base::parse(base).unwrap()),
                    depth,
                };
                let view = app.view.as_mut().unwrap();
                view.look = look;
                for scoped in view
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .squad_looks
                    .values_mut()
                {
                    *scoped = look;
                }
                app.view.as_mut().unwrap().clipboard = Some(vec![
                    clipboard.display().to_string(),
                    if scene == "copy-failed" { "7" } else { "0" }.into(),
                ]);
                if scene == "reader-selected" {
                    app.perform(&crate::action::Action::parse("view-reply").unwrap());
                    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
                } else if task {
                    draw(&mut app, 160, 44);
                    let field = app.conversation.borrow().task_hits.first().unwrap().area;
                    mouse(
                        &mut app,
                        MouseEventKind::Down(MouseButton::Left),
                        field.x,
                        field.y,
                    );
                    mouse(
                        &mut app,
                        MouseEventKind::Drag(MouseButton::Left),
                        field.x + 2,
                        field.y,
                    );
                } else {
                    enter(&mut app);
                }
                draw(&mut app, width, 36);
                let Effect::Act(request) =
                    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
                else {
                    panic!("fresh keyboard selection copies");
                };
                let outcome = crate::board::execute(&core, request).map(|outcome| match outcome {
                    crate::board::ActionOutcome::Message(message) => message,
                    _ => panic!("copy cannot send"),
                });
                app.finished(outcome);
                if scene == "quoted-draft" {
                    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    app.paste("My comment stays unsent.");
                }
                let buffer = draw(&mut app, width, 36);
                frames.push(json!({"scene":scene,"width":width,"height":36,"base":base,"depth":format!("{depth:?}"),
                    "lines":buffer.content.chunks(usize::from(width)).map(|row|row.iter().map(|cell|cell.symbol()).collect::<String>()).collect::<Vec<_>>(),
                    "cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>() }));
            }
        }
    }
    std::fs::write(path, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
    assert!(
        !core_called.exists(),
        "the clipboard worker must not invoke Core"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_second_press_outside_cancels_a_pending_task_drag() {
    let mut app = task_fixture();
    draw(&mut app, 160, 44);
    let field = app.conversation.borrow().task_hits.first().unwrap().area;
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        field.x,
        field.y,
    );
    assert!(app.conversation.borrow().task_drag.is_some());
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), 0, 0);
    assert!(app.conversation.borrow().task_drag.is_none());
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        field.x + 2,
        field.y,
    );
    assert!(
        app.conversation
            .borrow()
            .sessions
            .values()
            .all(|session| session.exact_source.is_none())
    );
    assert!(app.input.is_none());
}

#[test]
fn overview_now_grants_only_the_task_it_actually_paints_not_a_question_preview() {
    use crate::board::{shell::OpsTab, workspace::OpsTarget};
    for preview in [false, true] {
        let mut app = task_fixture();
        app.shell.tab = OpsTab::Overview;
        app.overview_view = crate::board::overview::monitor::View::Agents;
        let target = OpsTarget::Member {
            room_id: Some(crate::board::app::workspace::tests::ROOM.into()),
            identity_id: MEMBER.into(),
        };
        app.workspace.selected = Some(target.clone());
        if preview {
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .rows
                .get_mut(&target)
                .unwrap()["waitingOnYou"] =
                json!([{"requestId":"unacquired", "preview":"A question is not a task source"}]);
        }
        app.workspace_reconcile();
        draw(&mut app, 160, 44);
        let field = app
            .conversation
            .borrow()
            .task_hits
            .first()
            .map(|hit| hit.area);
        if preview {
            assert!(
                field.is_none(),
                "Now paints a question preview, so it grants no task selector"
            );
            continue;
        }
        let field = field.expect("visible agent Now task grants field geometry");
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            field.x,
            field.y,
        );
        assert!(
            app.conversation.borrow().task_drag.is_some(),
            "actual Now down registers a task gesture; selected={:?}; notice={:?}",
            app.workspace.selected,
            app.notice
        );
        mouse(
            &mut app,
            MouseEventKind::Drag(MouseButton::Left),
            field.x + 1,
            field.y,
        );
        assert!(
            app.conversation
                .borrow()
                .sessions
                .values()
                .any(|session| session.exact_source.is_some()),
            "Now drag opens raw source; selected={:?}; receiving={:?}; notice={:?}",
            app.workspace.selected,
            app.conversation_target(),
            app.notice
        );
        draw(&mut app, 160, 44);
        let Effect::Act(crate::board::app::Request::Copy { text, .. }) =
            app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
        else {
            panic!(
                "the displayed task uses the shared exact selector; focus={:?}; selected={:?}; receiving={:?}; active={:?}; notice={:?}",
                app.focused(),
                app.workspace.selected,
                app.conversation_target(),
                app.conversation.borrow().active,
                app.notice
            );
        };
        assert_eq!(
            text, "Review **literal**\n漢字",
            "Now omits Next, so it cannot lend those bytes"
        );
        assert!(app.input.is_none());
    }
}
