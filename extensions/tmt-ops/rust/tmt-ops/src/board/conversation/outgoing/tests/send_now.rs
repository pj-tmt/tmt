//! The Send now chip on the transcript footer: offered only while Core says the newest own
//! message can be sent now, one request per press, and Core's own words as the result.
use super::*;
use crate::board::{app::Request as AppRequest, row_detail::Sent};
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

/// The workspace board with the conversation focused, where the composer and footer row exist.
fn board() -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().me_id = Some("ben-id".into());
    app.focus = 1;
    app
}

const HELD: &str = "◌ batched to save usage, sends when the agent finishes this turn · 2m";
/// The state line wraps in a narrow pane; its first words are enough to find it.
const HELD_START: &str = "◌ batched to save usage";

fn held(app: &mut App, can_send_now: bool) {
    let fake = Fake::new(
        json!({"requestId":"req_held","status":"queued","notification":"held"}),
        false,
    );
    let mut completed = send(app, &fake, "run the picker tests");
    if let Ok(accepted) = &mut completed.outcome {
        accepted.message_state_label = Some(HELD.into());
        accepted.can_send_now = can_send_now;
    }
    finish(app, completed);
}

/// The whole board, so the conversation has its composer and its footer row.
fn frame(app: &mut App, width: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn text(buffer: &ratatui::buffer::Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

fn ctrl_s() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
}

fn sent(request: &str, at_ms: u64, can_send_now: bool) -> Sent {
    Sent {
        request: request.into(),
        at_ms,
        text: "a message".into(),
        truncated: false,
        capture: None,
        message_state_label: Some(HELD.into()),
        can_send_now,
        working: false,
    }
}

#[test]
fn the_chip_shows_while_core_says_the_message_can_be_sent_now_and_hides_when_it_says_not() {
    let mut app = board();
    held(&mut app, true);
    let painted = text(&frame(&mut app, 120));
    assert!(painted.contains(HELD_START), "{painted}");
    assert!(
        painted.contains("Send now · worker"),
        "the chip names the agent it acts on: {painted}"
    );

    let mut app = board();
    held(&mut app, false);
    let painted = text(&frame(&mut app, 120));
    assert!(
        painted.contains(HELD_START),
        "the state line stays: {painted}"
    );
    assert!(!painted.contains("Send now"), "{painted}");
    assert_eq!(app.key(ctrl_s()), Effect::None, "no chip, no action");
}

#[test]
fn a_press_asks_core_for_exactly_that_request_once_and_the_chip_waits_for_the_answer() {
    let mut app = board();
    held(&mut app, true);
    frame(&mut app, 120);
    let Effect::Act(AppRequest::SendNow { me, request, to }) = app.key(ctrl_s()) else {
        panic!("Ctrl-S presses the painted chip");
    };
    assert_eq!(
        (me.as_str(), request.as_str(), to.as_str()),
        ("Ben", "req_held", "worker")
    );

    let painted = text(&frame(&mut app, 120));
    assert!(painted.contains("Sending now…"), "{painted}");
    assert!(!painted.contains("Send now ·"), "{painted}");
    assert_eq!(app.key(ctrl_s()), Effect::None, "one call per press");
    assert!(app.conversation.borrow().send_now_hit.is_none());
}

#[test]
fn a_click_on_the_chip_presses_it_and_a_click_beside_it_does_not() {
    let mut app = board();
    held(&mut app, true);
    frame(&mut app, 120);
    let area = app.conversation.borrow().send_now_hit.clone().unwrap().area;
    let click = |app: &mut App, column: u16, row: u16| {
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        )
    };
    assert!(
        !matches!(
            click(&mut app, area.x.saturating_sub(2), area.y),
            Effect::Act(AppRequest::SendNow { .. })
        ),
        "beside the chip"
    );
    assert!(matches!(
        click(&mut app, area.x + 1, area.y),
        Effect::Act(AppRequest::SendNow { request, .. }) if request == "req_held"
    ));
}

#[test]
fn core_s_answer_is_the_notice_and_a_refusal_frees_the_chip_again() {
    for outcome in [
        Ok("worker: ✓ delivered".to_owned()),
        Err("Core's sentence for why it could not go now.".to_owned()),
    ] {
        let mut app = board();
        held(&mut app, true);
        frame(&mut app, 120);
        let Effect::Act(AppRequest::SendNow { request, .. }) = app.key(ctrl_s()) else {
            panic!("press");
        };
        app.finished_send_now(crate::board::conversation::SendNowDone {
            request,
            outcome: outcome.clone(),
        });
        let words = match &outcome {
            Ok(words) | Err(words) => words.as_str(),
        };
        assert_eq!(app.notice.as_deref(), Some(words), "Core's own words");
        assert_eq!(app.conversation.borrow().send_now_pending, None);
        // The reload re-reads Core's state; until then the chip is back for an unchanged state.
        frame(&mut app, 120);
        assert!(app.conversation.borrow().send_now_hit.is_some());
    }
}

#[test]
fn nothing_is_offered_while_a_newer_send_is_still_running() {
    let mut app = board();
    held(&mut app, true);
    let (_ticket, _request) = begin(&mut app, "a newer message");
    let painted = text(&frame(&mut app, 120));
    assert!(!painted.contains("Send now"), "{painted}");
    assert_eq!(app.key(ctrl_s()), Effect::None);
}

#[test]
fn the_chip_fits_the_footer_at_eighty_columns_beside_its_key() {
    let mut app = board();
    held(&mut app, true);
    for width in [80, 60, 44] {
        let buffer = frame(&mut app, width);
        let painted = text(&buffer);
        assert!(painted.contains("Send now"), "{width}: {painted}");
        let hit = app.conversation.borrow().send_now_hit.clone().unwrap();
        assert!(hit.area.right() <= width, "{width}: {hit:?}");
        let footer: String = (0..width)
            .map(|x| buffer[(x, hit.area.y)].symbol())
            .collect();
        assert!(
            footer.contains("Ctrl-S"),
            "{width}: the key is listed beside it: {footer}"
        );
    }
}

#[test]
fn only_the_newest_own_message_is_offered_and_only_when_core_says_so() {
    let mut session = Session {
        sent: vec![sent("old", 1, true), sent("new", 2, false)],
        ..Session::default()
    };
    assert_eq!(
        session.send_now_request(),
        None,
        "an older held message is not guessed at"
    );
    session.sent = vec![sent("old", 1, false), sent("new", 2, true)];
    assert_eq!(session.send_now_request(), Some("new"));
    session.sent = vec![sent("new", 2, true), sent("old", 1, true)];
    assert_eq!(
        session.send_now_request(),
        Some("new"),
        "by time, not by list order"
    );
    session.sent = Vec::new();
    assert_eq!(session.send_now_request(), None);
}

#[test]
fn in_the_editor_ctrl_s_is_the_chip_and_elsewhere_an_authored_binding_wins() {
    let mut app = crate::board::app::workspace::tests::configured_member(
        "[bind]\nctrl-s = \"copy {name}\"\n",
        json!({"id":crate::board::app::workspace::tests::MEMBER,"name":"worker","fields":{}}),
    );
    app.view.as_mut().unwrap().me = Some("Ben".into());
    app.view.as_mut().unwrap().me_id = Some("ben-id".into());
    app.focus = 1;
    held(&mut app, true);
    frame(&mut app, 120);
    assert!(app.input.is_some(), "the message editor is open");
    assert!(
        matches!(app.key(ctrl_s()), Effect::Act(AppRequest::SendNow { .. })),
        "no binding is active while typing, so the chip's key works"
    );
    app.finished_send_now(crate::board::conversation::SendNowDone {
        request: "req_held".into(),
        outcome: Err("Core's sentence.".into()),
    });
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.input.is_none(), "Esc leaves the editor");
    frame(&mut app, 120);
    assert!(
        app.conversation.borrow().send_now_hit.is_some(),
        "the chip is still offered"
    );
    assert!(
        matches!(app.key(ctrl_s()), Effect::Act(AppRequest::Copy { text, .. }) if text == "worker"),
        "outside the editor the authored action runs, not Send now"
    );
    assert_eq!(app.conversation.borrow().send_now_pending, None);
}

/// The refresh after a send re-reads Core's state for it: the chip follows that, even for a
/// message this session sent itself.
#[test]
fn when_core_moves_the_state_on_the_chip_goes_even_for_a_send_made_here() {
    let mut app = board();
    held(&mut app, true);
    frame(&mut app, 120);
    assert!(app.conversation.borrow().send_now_hit.is_some());

    if let Some(finals) = app.finals_read() {
        app.apply_finals(finals, Ok(vec![]));
        frame(&mut app, 120);
    }
    let Some(key) = app.sent_read() else {
        panic!("sends are read");
    };
    app.apply_sent(
        key,
        Ok(vec![
            json!({"requestId":"req_held","preparedAtMs":crate::status::now_ms(),
            "text":"run the picker tests","truncated":false,
            "messageState":{"state":"delivered","canSendNow":false,"reason":{"text":"delivered"}}}),
        ]),
    );
    let painted = text(&frame(&mut app, 120));
    assert!(painted.contains("✓ delivered"), "{painted}");
    assert!(!painted.contains("Send now"), "{painted}");
    assert!(app.conversation.borrow().send_now_hit.is_none());
}

/// The `working` message-state mark is the shell spinner: it advances with the tick and holds
/// still without motion; other states keep their static marks and Core's words are verbatim.
#[test]
fn the_working_state_mark_spins_with_the_shared_tick_and_other_marks_hold_still() {
    const SENT: u64 = 1_791_600_000_000;
    let mark_before = |screen: &str, words: &str| -> String {
        let at = screen
            .find(words)
            .unwrap_or_else(|| panic!("{words}: {screen}"));
        screen[..at]
            .chars()
            .rev()
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    };
    for (state, text, spins, reduced) in [
        ("working", "the agent has started on it", true, false),
        ("working", "the agent has started on it", false, true),
        ("delivered", "typed into the agent", false, false),
    ] {
        let mut app = crate::board::app::workspace::tests::board();
        let view = app.view.as_mut().unwrap();
        view.look = crate::look::Look::default();
        view.home.as_mut().unwrap().workspace.squad_looks.clear();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .reduced_motion = reduced;
        let fake = Fake::new(
            json!({"requestId":"w1","status":"sent",
                "messageState":{"state":state,"reason":{"text":text}}}),
            false,
        );
        let completed = crate::status::with_now_ms(SENT, || send(&mut app, &fake, "hello"));
        finish(&mut app, completed);
        let marks: std::collections::BTreeSet<String> = (0..10)
            .map(|tick| {
                let screen = crate::status::with_now_ms(SENT + 2_000 + tick * 80, || draw(&app));
                assert!(
                    screen.contains(text),
                    "Core's words stay verbatim: {screen}"
                );
                mark_before(&screen, text)
            })
            .collect();
        assert_eq!(
            marks.len() > 1,
            spins,
            "{state} reduced={reduced}: {marks:?}"
        );
    }
}
