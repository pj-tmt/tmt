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
    let quote = &app.input.as_ref().unwrap().exact_quotes().next().unwrap();
    assert_eq!(quote.key.request, "r1");
    assert_eq!(quote.key.lead, crate::board::app::workspace::tests::MEMBER);
    assert_eq!(quote.range, 0..source.len());
    assert_eq!(quote.text, source);
    assert_eq!(app.view.as_ref().unwrap().document, before);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.input
            .as_ref()
            .unwrap()
            .exact_quotes()
            .next()
            .unwrap()
            .text,
        source
    );
    app.paste("two words");
    app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        format!("{source}\ntwo ")
    );
    assert_eq!(
        app.input
            .as_ref()
            .unwrap()
            .exact_quotes()
            .next()
            .unwrap()
            .text,
        source
    );
    draw(&app, 160, 44);
    let hit = app.composer.paste_hits.borrow().last().cloned().unwrap();
    app.mouse(
        ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::Down(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            column: hit.remove.x,
            row: hit.remove.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(app.input.as_ref().unwrap().exact_quotes().next().is_none());
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "\ntwo ");
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
    assert_eq!(
        app.input.as_ref().unwrap().exact_quotes().next().unwrap(),
        quote
    );
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
    let quote = &input.exact_quotes().next().unwrap();
    assert_eq!(quote.key.lead, MEMBER);
    assert_eq!(quote.range, 0..source.len());
    assert_eq!(quote.text, source);
    assert_eq!(app.view.as_ref().unwrap().document, before);
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Effect::Act(app::Request::Chat { recipient, text, .. })
            if recipient == MEMBER && text == "> A **literal** response\n> 漢字 e\u{301}\n"
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

const ACTOR: &str = "33333333-3333-4333-8333-333333333333";

fn final_item(request: &str, recipient: &str, at: u64) -> Value {
    json!({"requestId":request,"kind":"request","sender":{"identityId":ACTOR},"recipientId":recipient,
        "final":{"status":"retained","submittedAtMs":at},"responsePreview":"preview",
        "preparedAtMs":at - 10,"recipientAcknowledged":true})
}

/// Read every body the way the board does: one selected read at a time.
fn read_all(app: &mut App, body: impl Fn(&str) -> String) -> Vec<String> {
    let mut order = Vec::new();
    for _ in 0..32 {
        draw(app, 148, 51);
        let Some(key) = app.detail_read() else { break };
        order.push(key.request.clone());
        let text = body(&key.request);
        app.apply_message(&key, Ok(text));
    }
    order
}

#[test]
fn finals_are_this_agents_settled_replies_by_request_oldest_first_with_exact_text_and_time() {
    use crate::board::app::workspace::tests::{MEMBER, board};
    let mut app = board();
    let now = crate::status::now_ms();
    let other = "99999999-9999-4999-8999-999999999999";
    app.home_leads.test_results(
        ACTOR,
        json!({"items":[
            final_item("c", MEMBER, now - 60_000),
            final_item("a", MEMBER, now - 3_000_000),
            final_item("elsewhere", other, now - 30_000),
            final_item("b", MEMBER, now - 1_800_000),
            {"requestId":"waiting","kind":"request","sender":{"identityId":ACTOR},"recipientId":MEMBER,
             "final":{"status":"not_submitted"},"preparedAtMs":now - 10_000},
        ],"nextBefore":null}),
    );
    let order = read_all(&mut app, |request| {
        format!("body of {request}\nsecond line")
    });
    assert_eq!(order, ["c", "b", "a"], "the newest is read first");
    let detail = app
        .row_detail(&app.conversation_target().unwrap(), &[])
        .unwrap();
    let finals = detail
        .finals
        .iter()
        .map(|reply| {
            (
                reply.key.as_ref().unwrap().request.as_str(),
                reply.from.as_str(),
                reply.at_ms,
                reply.body.as_str(),
                reply.acquired,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        finals,
        [
            (
                "a",
                "worker",
                Some(now - 3_000_000),
                "body of a\nsecond line",
                true
            ),
            (
                "b",
                "worker",
                Some(now - 1_800_000),
                "body of b\nsecond line",
                true
            ),
            (
                "c",
                "worker",
                Some(now - 60_000),
                "body of c\nsecond line",
                true
            ),
        ],
        "oldest first, exact text, another agent's and unsettled requests left out"
    );
    assert_eq!(
        detail.reply.unwrap().key.unwrap().request,
        "c",
        "the single reply stays the newest final"
    );
}

#[test]
fn only_the_newest_finals_are_kept_and_each_is_read_once() {
    use crate::board::app::workspace::tests::{MEMBER, board};
    let mut app = board();
    let now = crate::status::now_ms();
    let items = (0..FINALS + 3)
        .map(|index| {
            final_item(
                &format!("r{index:02}"),
                MEMBER,
                now - 1_000_000 + index as u64 * 1_000,
            )
        })
        .collect::<Vec<_>>();
    app.home_leads
        .test_results(ACTOR, json!({"items":items,"nextBefore":null}));
    let order = read_all(&mut app, |request| request.to_owned());
    assert_eq!(
        order.len(),
        FINALS,
        "no body outside the kept finals is read"
    );
    assert_eq!(order[0], format!("r{:02}", FINALS + 2));
    let detail = app
        .row_detail(&app.conversation_target().unwrap(), &[])
        .unwrap();
    assert_eq!(detail.finals.len(), FINALS);
    assert_eq!(detail.finals[0].key.as_ref().unwrap().request, "r03");
}

#[test]
fn a_held_reply_shows_its_true_age_instead_of_the_one_it_was_built_with() {
    use crate::board::app::workspace::tests::{MEMBER, board};
    let mut app = board();
    let now = crate::status::now_ms();
    app.home_leads.test_results(
        ACTOR,
        json!({"items":[final_item("old", MEMBER, now - 1_500_000)],"nextBefore":null}),
    );
    read_all(&mut app, |_| "OLD body".into());
    // The reader scrolled away, then a newer final arrives and is held back.
    crate::board::conversation::tests::set_follow(&app, false);
    app.home_leads.test_results(
        ACTOR,
        json!({"items":[final_item("new", MEMBER, now - 60_000), final_item("old", MEMBER, now - 1_500_000)],"nextBefore":null}),
    );
    app.overview_rows.borrow_mut().clear();
    draw(&app, 148, 51);
    // The string the held reply was built with is stale (it once read "1m").
    crate::board::conversation::tests::set_built_age(&app, "1m");
    let buffer = draw(&app, 148, 51);
    let screen = text(&buffer);
    assert!(screen.contains("OLD body") && screen.contains("new output"));
    assert!(screen.contains("25m ago"), "the true age of the held text");
    let rows: Vec<_> = buffer
        .content
        .chunks(148)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect();
    let old = rows
        .iter()
        .position(|row| row.contains("OLD body"))
        .unwrap();
    assert!(
        rows[old - 1].contains("25m ago") && !rows[old - 1].contains("1m ago"),
        "each reply owns its own age: {rows:#?}"
    );
}

/// The acquisition path with a Core that answers only the two storage reads.
/// The originator (`ben`) has no live pane, so no hint ever reaches it; the
/// final reaches the conversation anyway, from list and show by request ID.
#[test]
fn finals_arrive_from_storage_alone_when_the_originator_has_no_live_pane() {
    use crate::board::app::workspace::tests::{MEMBER, board};
    let now = crate::status::now_ms();
    let directory =
        std::env::temp_dir().join(format!("ops-finals-core-{}", crate::id::new_v4().unwrap()));
    std::fs::create_dir(&directory).unwrap();
    let list = json!({"items":[final_item("answer", MEMBER, now - 120_000)],"nextBefore":null});
    let show = json!({"requestId":"answer","sender":{"identityId":ACTOR},"recipientId":MEMBER,
        "kind":"request","final":{"status":"retained","submittedAtMs":now - 120_000,
        "response":"First line\nSecond line"}});
    let log = directory.join("operations");
    crate::test_support::write_ready_executable(
        &directory.join("tmt"),
        &format!(
            "#!/bin/sh\ninput=$(cat)\ncase \"$input\" in\n*requests.list*) echo requests.list >> '{log}'; printf '%s\\n' '{list}' ;;\n*requests.show*) echo requests.show >> '{log}'; printf '%s\\n' '{show}' ;;\n*) echo \"$*\" >> '{log}'; exit 99 ;;\nesac\n",
            log = log.display(),
        ),
    );
    let core = crate::core::Core::at(directory.join("tmt"));
    let mut app = board();
    let read = crate::board::home_leads::Fetch::results(Some(crate::me::Me {
        id: ACTOR.into(),
        name: "ben".into(),
    }))
    .complete(&core);
    app.home_leads.set_sender(Some(ACTOR));
    app.home_leads.replace(read);
    let mut seen = 0;
    for _ in 0..4 {
        draw(&app, 148, 51);
        let Some(key) = app.detail_read() else { break };
        seen += 1;
        let body = key.read(&core).map_err(|error| error.to_string());
        app.apply_message(&key, body);
    }
    assert_eq!(seen, 1);
    let detail = app
        .row_detail(&app.conversation_target().unwrap(), &[])
        .unwrap();
    let [only] = detail.finals.as_slice() else {
        panic!("one final");
    };
    assert_eq!(only.from, "worker");
    assert_eq!(only.at_ms, Some(now - 120_000));
    assert_eq!(only.body, "First line\nSecond line");
    // The same storage-backed result is a separate rendered message. Drawing
    // cannot fall back to notifications or another storage acquisition.
    let _no_reads = crate::runner::forbid_commands();
    let buffer = draw(&app, 148, 51);
    let rows: Vec<_> = buffer
        .content
        .chunks(148)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect();
    let body = rows
        .iter()
        .position(|row| row.contains("First line"))
        .expect("stored final painted");
    assert!(rows[body + 1].contains("Second line"), "{rows:#?}");
    assert!(
        rows[body - 1].contains("worker") && rows[body - 1].contains("2m ago"),
        "{rows:#?}"
    );
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["requests.list", "requests.show"],
        "storage reads only: no notification, inbox or pane call"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn own_final(request: &str, recipient: &str, at: u64) -> Value {
    json!({"requestId":request,"recipientId":recipient,"submittedAtMs":at,"status":"retained"})
}

/// Finals the shared windows scrolled past still reach the conversation: the
/// selected agent's own read supplies them once, then their bodies are read
/// like any other and nothing is shown twice.
#[test]
fn an_agents_older_finals_come_from_its_own_read_once_and_join_the_windows() {
    use crate::board::app::workspace::tests::{MEMBER, board};
    let mut app = board();
    let now = crate::status::now_ms();
    app.home_leads.test_results(
        ACTOR,
        json!({"items":[final_item("new", MEMBER, now - 60_000)],"nextBefore":null}),
    );
    draw(&app, 148, 51);
    let Some(key) = app.finals_read() else {
        panic!("the agent's own finals are read before any body");
    };
    assert_eq!((key.sender.as_str(), key.agent.as_str()), (ACTOR, MEMBER));
    app.apply_finals(
        key,
        Ok(vec![
            own_final("new", MEMBER, now - 60_000),
            own_final("old2", MEMBER, now - 2_000_000),
            own_final("old1", MEMBER, now - 3_000_000),
        ]),
    );
    answer_sent(&mut app);
    assert!(app.finals_read().is_none(), "read once per activation");
    assert!(matches!(
        app.selected_read(),
        Some(crate::board::refresh::SelectedRead::Conversation(_))
    ));
    let order = read_all(&mut app, |request| format!("body {request}"));
    assert_eq!(order, ["new", "old2", "old1"], "newest first, each once");
    let detail = app
        .row_detail(&app.conversation_target().unwrap(), &[])
        .unwrap();
    let shown = detail
        .finals
        .iter()
        .map(|reply| {
            (
                reply.key.as_ref().unwrap().request.as_str(),
                reply.at_ms,
                reply.body.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        [
            ("old1", Some(now - 3_000_000), "body old1"),
            ("old2", Some(now - 2_000_000), "body old2"),
            ("new", Some(now - 60_000), "body new"),
        ]
    );
    assert_eq!(detail.reply.unwrap().key.unwrap().request, "new");
}

#[test]
fn a_failed_or_stale_finals_read_is_not_retried_and_never_hides_the_windows() {
    use crate::board::app::workspace::tests::{MEMBER, board};
    use crate::board::home_leads::FinalsKey;
    let mut app = board();
    let now = crate::status::now_ms();
    app.home_leads.test_results(
        ACTOR,
        json!({"items":[final_item("new", MEMBER, now - 60_000)],"nextBefore":null}),
    );
    draw(&app, 148, 51);
    let Some(key) = app.finals_read() else {
        panic!("finals read first");
    };
    // An answer for an agent that is not on screen is dropped.
    app.apply_finals(
        FinalsKey {
            sender: ACTOR.into(),
            agent: "99999999-9999-4999-8999-999999999999".into(),
        },
        Ok(vec![own_final("elsewhere", MEMBER, now - 1_000)]),
    );
    assert!(app.row_details.agent_finals.is_empty());
    // An older core refuses the recipient form: no retry loop, windows intact.
    app.apply_finals(key, Err("API_INPUT_INVALID".into()));
    answer_sent(&mut app);
    assert!(app.finals_read().is_none());
    assert!(matches!(
        app.selected_read(),
        Some(crate::board::refresh::SelectedRead::Conversation(_))
    ));
    read_all(&mut app, |request| request.to_owned());
    let detail = app
        .row_detail(&app.conversation_target().unwrap(), &[])
        .unwrap();
    let [only] = detail.finals.as_slice() else {
        panic!("the window's final alone");
    };
    assert_eq!(only.key.as_ref().unwrap().request, "new");
}

#[test]
fn the_agents_own_read_asks_core_for_that_originators_finals_to_that_recipient_only() {
    use crate::board::app::workspace::tests::MEMBER;
    use crate::board::home_leads::FinalsKey;
    let now = crate::status::now_ms();
    let directory =
        std::env::temp_dir().join(format!("ops-agent-finals-{}", crate::id::new_v4().unwrap()));
    std::fs::create_dir(&directory).unwrap();
    let foreign = json!({"requestId":"theirs","kind":"request","sender":{"identityId":"99999999-9999-4999-8999-999999999999"},
        "recipientId":MEMBER,"final":{"status":"retained","submittedAtMs":now - 5}});
    let list = json!({"items":[
        final_item("mine", MEMBER, now - 10),
        final_item("misrouted", "88888888-8888-4888-8888-888888888888", now - 20),
        foreign,
    ],"nextBefore":null});
    let log = directory.join("input");
    crate::test_support::write_ready_executable(
        &directory.join("tmt"),
        &format!(
            "#!/bin/sh\ncat > '{log}'\nprintf '%s\\n' '{list}'\n",
            log = log.display(),
        ),
    );
    let core = crate::core::Core::at(directory.join("tmt"));
    let key = FinalsKey {
        sender: ACTOR.into(),
        agent: MEMBER.into(),
    };
    let rows = key.read(&core).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&std::fs::read_to_string(&log).unwrap()).unwrap(),
        json!({"version":1,"operation":"requests.list","input":{
            "originatorId":ACTOR,"recipientId":MEMBER,"view":"results","limit":FINALS}})
    );
    assert_eq!(rows, [own_final("mine", MEMBER, now - 10)]);
    // A refusal (an older core) is an error the board turns into "no more finals".
    crate::test_support::write_ready_executable(
        &directory.join("tmt"),
        "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"error\":{\"code\":\"API_INPUT_INVALID\",\"message\":\"Invalid operation, fields or bounded input.\"}}'\nexit 1\n",
    );
    assert!(key.read(&core).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

/// The agent's own sends, found empty: the lane after finals and before bodies.
fn answer_sent(app: &mut App) {
    draw(app, 148, 51);
    if let Some(key) = app.sent_read() {
        app.apply_sent(key, Ok(vec![]));
    }
    draw(app, 148, 51);
}

fn own_send(request: &str, at: u64, text: &str, truncated: bool) -> Value {
    json!({"requestId":request,"preparedAtMs":at,"text":text,"truncated":truncated})
}

/// After a restart the session has no sends of its own: the agent's own read
/// restores them from storage once its finals are in, ahead of any body read,
/// as "you" blocks interleaved with the finals by time.
#[test]
fn a_restart_restores_the_users_own_sends_between_the_agents_finals() {
    use crate::board::app::workspace::tests::{MEMBER, board};
    let mut app = board();
    let now = crate::status::now_ms();
    app.home_leads.test_results(
        ACTOR,
        json!({"items":[final_item("answer", MEMBER, now - 60_000)],"nextBefore":null}),
    );
    draw(&app, 148, 51);
    let Some(finals) = app.finals_read() else {
        panic!("finals are read first");
    };
    assert!(app.sent_read().is_none(), "sends wait for the finals");
    app.apply_finals(finals, Ok(vec![]));
    draw(&app, 148, 51);
    let Some(key) = app.sent_read() else {
        panic!("the user's own sends are read before any body");
    };
    assert_eq!((key.sender.as_str(), key.agent.as_str()), (ACTOR, MEMBER));
    app.apply_sent(
        key,
        Ok(vec![
            own_send("first", now - 300_000, "Please check the retry loop", false),
            own_send("second", now - 120_000, "Also the backoff curve", false),
        ]),
    );
    assert!(app.sent_read().is_none(), "read once per activation");
    assert!(matches!(
        app.selected_read(),
        Some(crate::board::refresh::SelectedRead::Conversation(_))
    ));
    read_all(&mut app, |request| format!("body {request}"));
    let screen = text(&draw(&app, 148, 51));
    let at = |needle: &str| {
        screen
            .find(needle)
            .unwrap_or_else(|| panic!("{needle}: {screen}"))
    };
    assert!(
        at("Please check the retry loop") < at("Also the backoff curve")
            && at("Also the backoff curve") < at("body answer"),
        "time order: {screen}"
    );
    assert!(!screen.contains("truncated"));
}

#[test]
fn a_restored_send_whose_record_is_gone_says_it_is_truncated_and_how_to_open_it() {
    use crate::board::app::workspace::tests::board;
    let mut app = board();
    draw(&app, 148, 51);
    if let Some(finals) = app.finals_read() {
        app.apply_finals(finals, Ok(vec![]));
        draw(&app, 148, 51);
    }
    let Some(key) = app.sent_read() else {
        panic!("sends are read");
    };
    let now = crate::status::now_ms();
    app.apply_sent(
        key,
        Ok(vec![own_send(
            "req_gone",
            now - 5_000,
            "A long message cut to its",
            true,
        )]),
    );
    let screen = text(&draw(&app, 148, 51));
    assert!(screen.contains("A long message cut to its"), "{screen}");
    assert!(
        screen.contains("truncated · tmt x show req_gone"),
        "{screen}"
    );
}

#[test]
fn a_failed_or_stale_sent_read_is_not_retried_and_hides_nothing() {
    use crate::board::app::workspace::tests::board;
    use crate::board::home_leads::SentKey;
    let mut app = board();
    draw(&app, 148, 51);
    if let Some(finals) = app.finals_read() {
        app.apply_finals(finals, Ok(vec![]));
        draw(&app, 148, 51);
    }
    let Some(key) = app.sent_read() else {
        panic!("sends are read");
    };
    // An answer for an agent that is not on screen is dropped.
    app.apply_sent(
        SentKey {
            sender: ACTOR.into(),
            agent: "99999999-9999-4999-8999-999999999999".into(),
        },
        Ok(vec![own_send("other", 1, "elsewhere", false)]),
    );
    assert!(matches!(
        app.selected_read(),
        Some(crate::board::refresh::SelectedRead::Conversation(_))
    ));
    app.apply_sent(key, Err("storage unavailable".into()));
    assert!(app.sent_read().is_none(), "a failed read is kept as empty");
    assert!(!text(&draw(&app, 148, 51)).contains("elsewhere"));
}

#[test]
fn capture_states_are_core_owned_and_never_regular_replies() {
    for state in ["pending", "bound", "answered", "no_answer", "future"] {
        let send = json!({"mode":"passthrough", "passthrough":{"state":state,
            "answer":{"text":"Full **answer**\n", "truncated":true,"provider":"codex","endedMs":100}}});
        let capture = Capture::read(&send);
        match state {
            "pending" | "bound" => assert_eq!(capture.unwrap().markdown(), "⠹ working…"),
            "answered" => assert_eq!(
                capture.unwrap().markdown(),
                "turn output · truncated\n\nFull **answer**"
            ),
            "no_answer" => assert_eq!(
                capture.unwrap().markdown(),
                "No turn output was captured for this message."
            ),
            _ => assert!(capture.is_none()),
        }
        let mut request = send;
        request["mode"] = json!("request");
        assert!(Capture::read(&request).is_none());
    }
}
// Public history records drive these scenes; no pane text or local timer decides capture.
fn captured_history_app(state: &str) -> App {
    use crate::board::app::workspace::tests::board;
    let mut app = board();
    draw(&app, 148, 51);
    if let Some(key) = app.finals_read() {
        app.apply_finals(key, Ok(vec![]));
        draw(&app, 148, 51);
    }
    let Some(key) = app.sent_read() else {
        panic!("history read");
    };
    let now = crate::status::now_ms();
    let mut sent = own_send(
        "captured-message",
        now - 86_400_000,
        "Check the scheduler",
        false,
    );
    sent["mode"] = json!("passthrough");
    sent["passthrough"] = json!({"state":state,"boundAtMs":now - 86_400_000,
        "answer":{"text":"The scheduler is **ready**.","truncated":true,"provider":"codex","endedMs":now}});
    app.apply_sent(key, Ok(vec![sent]));
    app
}
fn captured_history_screen(state: &str) -> String {
    text(&draw(&captured_history_app(state), 148, 51))
}

#[test]
fn pending_capture_is_working_from_the_core_record() {
    let screen = captured_history_screen("pending");
    assert!(screen.contains("working…"), "{screen}");
}
#[test]
fn bound_capture_remains_working_beyond_ten_minutes() {
    let screen = captured_history_screen("bound");
    assert!(screen.contains("working…"), "{screen}");
    assert!(!screen.contains("No turn output was captured"));
}
#[test]
fn answered_capture_survives_restart_as_turn_output_with_truncation() {
    let screen = captured_history_screen("answered");
    let prompt = screen.find("Check the scheduler").expect("own prompt");
    let label = screen
        .find("turn output · truncated")
        .unwrap_or_else(|| panic!("{screen}"));
    let answer = screen
        .find("The scheduler is ready.")
        .unwrap_or_else(|| panic!("{screen}"));
    assert!(prompt < label && label < answer);
}
#[test]
fn no_answer_is_not_a_reply_or_a_reason_to_resend() {
    let screen = captured_history_screen("no_answer");
    assert!(
        screen.contains("No turn output was captured for this message."),
        "{screen}"
    );
    assert!(!screen.contains("working…"));
}

#[test]
fn a_failed_capture_refresh_keeps_the_acquired_answer_without_retrying_in_a_loop() {
    let mut app = captured_history_app("answered");
    assert!(text(&draw(&app, 148, 51)).contains("The scheduler is ready."));
    app.refresh_sent();
    let key = app
        .sent_read()
        .expect("next acquired refresh admits one history read");
    app.apply_sent(key, Err("storage unavailable".into()));
    assert!(app.sent_read().is_none());
    assert!(text(&draw(&app, 148, 51)).contains("The scheduler is ready."));
    app.refresh_sent();
    assert!(app.sent_read().is_some());
}

#[test]
fn only_the_exact_captured_observation_is_suppressed_from_recent_output() {
    use crate::board::recent_responses::{Item, Observation};
    let now = 1_791_600_000_160;
    crate::status::with_now_ms(now, || {
        let app = captured_history_app("answered");
        draw(&app, 148, 51);
        let key = app.pane_output_read().expect("visible selected agent");
        app.apply_pane_output(
            key,
            Observation::Observed(vec![
                Item {
                    text: "The scheduler is **ready**.".into(),
                    provider: "codex".into(),
                    ended_ms: now,
                    truncated: true,
                },
                Item {
                    text: "Unrelated final remains visible".into(),
                    provider: "codex".into(),
                    ended_ms: now + 1,
                    truncated: false,
                },
            ]),
        );
        let screen = text(&draw(&app, 148, 51));
        assert_eq!(
            screen.matches("The scheduler is ready.").count(),
            1,
            "{screen}"
        );
        assert!(
            screen.contains("Unrelated final remains visible"),
            "{screen}"
        );
    });
}

#[test]
fn malformed_or_oversized_capture_never_invents_an_answer() {
    for answer in [
        json!({}),
        json!({"text":"x".repeat(8193),"truncated":true,"provider":"codex","endedMs":1}),
    ] {
        assert!(
            Capture::read(
                &json!({"mode":"passthrough","passthrough":{"state":"answered","answer":answer}})
            )
            .is_none()
        );
    }
}
