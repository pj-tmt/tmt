//! The Terminal tab as the board shows it: when the tab exists, what `t` does, what each state
//! of the stream looks like, and how cells are laid onto a pane of another size.
use super::{
    discovery::Located,
    paint,
    stream::StreamEvent,
    tab::{
        Condition, View,
        tests::{address, tab},
    },
};
use crate::board::{
    app::{
        App, Choice,
        workspace::tests::{MEMBER, board},
    },
    refresh::SelectedRead,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    style::Modifier,
};
use std::{sync::Arc, time::Duration};
use tmt_pty_protocol::{Eviction, Exit, Refusal};
use tmt_tui::components::terminal::Screen;
use unicode_width::UnicodeWidthStr;

fn reader() -> App {
    let mut app = board();
    app.focus = 1;
    app.terminal = tab().0;
    // The pane has to be drawn before it counts as on screen.
    text(&app);
    app
}

fn text(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..40)
        .map(|y| {
            (0..160)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press_t(app: &mut App) {
    app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
}

fn with_stream(app: &mut App) {
    app.apply_terminal_located(MEMBER, Located::Stream(address("a")));
}

/// The tab shown and its stream open: epoch 1.
fn shown(app: &mut App) {
    with_stream(app);
    press_t(app);
    assert!(app.terminal_sync());
}

fn frame(bytes: &str) -> Arc<tmt_tui::components::terminal::Frame> {
    let mut screen = Screen::new(40, 6);
    screen.feed(bytes.as_bytes());
    Arc::new(screen.frame())
}

#[test]
fn a_member_with_no_stream_has_no_tab_and_t_does_nothing() {
    let mut app = reader();
    assert_eq!(
        app.terminal_locate().as_deref(),
        Some(MEMBER),
        "core has not been asked"
    );
    // The chat's own reads come first; the question follows once they are idle.
    assert!(!matches!(
        app.selected_read(),
        Some(SelectedRead::TerminalStream(_))
    ));
    app.apply_terminal_located(MEMBER, Located::NoStream);
    assert_eq!(app.terminal_locate(), None, "and not again at once");
    assert!(!app.terminal_available());
    let before = text(&app);
    press_t(&mut app);
    assert!(!app.terminal_showing());
    assert_eq!(text(&app), before, "no notice and no change");
    assert!(!before.contains("Terminal"), "no chip");
    assert!(
        !app.bindings().contains_key("t"),
        "t is offered only where it acts"
    );
}

#[test]
fn a_failed_answer_shows_no_tab_and_is_asked_again_later() {
    let mut app = reader();
    crate::status::with_now_ms(1_000, || {
        app.apply_terminal_located(MEMBER, Located::Failed("busy".into()));
        assert!(!app.terminal_available());
        assert_eq!(app.terminal_locate(), None);
        assert_eq!(app.terminal_wait(), Some(Duration::from_secs(10)));
    });
    crate::status::with_now_ms(11_000, || {
        assert_eq!(app.terminal_locate().as_deref(), Some(MEMBER));
    });
}

#[test]
fn a_member_with_a_stream_gets_chips_and_t_swaps_chat_and_terminal() {
    let mut app = reader();
    with_stream(&mut app);
    let chat = text(&app);
    assert!(chat.contains("Chat") && chat.contains("Terminal"), "{chat}");
    assert!(chat.contains("Exchanges with you"));
    assert!(app.bindings().contains_key("t"));
    press_t(&mut app);
    assert!(app.terminal_showing());
    assert_eq!(app.selected_read(), None, "the chat is not read behind it");
    let terminal = text(&app);
    assert!(!terminal.contains("Exchanges with you"));
    assert!(terminal.contains("Terminal"));
    press_t(&mut app);
    assert!(!app.terminal_showing());
    assert!(text(&app).contains("Exchanges with you"));
}

#[test]
fn the_chips_click_to_the_same_two_views() {
    let mut app = reader();
    with_stream(&mut app);
    text(&app);
    let hits = app.workspace_strip_hits.borrow().clone();
    assert!(
        hits.iter()
            .any(|(_, choice)| matches!(choice, Choice::TerminalView(true)))
            && hits
                .iter()
                .any(|(_, choice)| matches!(choice, Choice::TerminalView(false)))
    );
    app.show_terminal(true);
    assert!(app.terminal_showing());
    app.show_terminal(true);
    assert!(
        app.terminal_showing(),
        "the same chip twice changes nothing"
    );
    app.show_terminal(false);
    assert!(!app.terminal_showing());
}

#[test]
fn the_stream_is_open_only_while_the_terminal_is_shown() {
    let (tab, opened) = tab();
    let mut app = reader();
    app.terminal = tab;
    with_stream(&mut app);
    assert!(!app.terminal_sync(), "the chat needs no stream");
    press_t(&mut app);
    assert!(app.terminal_sync());
    assert!(!app.terminal_sync());
    press_t(&mut app);
    assert!(app.terminal_sync(), "leaving closes it");
    assert_eq!(opened.lock().unwrap().len(), 1);
}

#[test]
fn each_state_of_the_stream_has_its_own_words() {
    let mut app = reader();
    shown(&mut app);
    let connecting = text(&app);
    assert!(
        connecting.contains("Connecting to worker's terminal"),
        "{connecting}"
    );

    app.apply_terminal(
        1,
        StreamEvent::Screen(frame("$ cargo test\r\nrunning 3 tests")),
    );
    let live = text(&app);
    assert!(live.contains("running 3 tests"), "{live}");
    assert!(live.contains("Live view of worker's terminal"));
    assert!(live.contains("Terminal 40×6"));

    app.apply_terminal(1, StreamEvent::Connecting { attempt: 2 });
    let again = text(&app);
    assert!(
        again.contains("Reconnecting to worker's terminal"),
        "{again}"
    );
    assert!(again.contains("running 3 tests"), "the last screen stays");

    for (event, words) in [
        (
            StreamEvent::Exited(Exit::Code(3)),
            "The program ended (exit 3)",
        ),
        (
            StreamEvent::Exited(Exit::Signal(9)),
            "The program ended (signal 9)",
        ),
        (
            StreamEvent::Evicted(Eviction::TakenOver),
            "Another viewer took over this terminal",
        ),
        (
            StreamEvent::Evicted(Eviction::TooSlow),
            "This view fell behind",
        ),
        (
            StreamEvent::Refused(Refusal::Full),
            "would not attach this view (too many viewers)",
        ),
        (
            StreamEvent::Lost("the socket is gone".into()),
            "could not be reached: the socket is gone",
        ),
    ] {
        app.apply_terminal(1, event);
        let screen = text(&app);
        assert!(screen.contains(words), "{words}\n{screen}");
        assert!(screen.contains("running 3 tests"), "{words}: screen kept");
    }
}

#[test]
fn an_event_of_a_replaced_stream_changes_nothing() {
    let mut app = reader();
    shown(&mut app);
    assert!(!app.apply_terminal(7, StreamEvent::Lost("old".into())));
    assert!(!text(&app).contains("could not be reached"));
}

#[test]
fn a_member_without_a_stream_next_to_one_keeps_its_chat() {
    let mut app = reader();
    shown(&mut app);
    app.apply_terminal_located(MEMBER, Located::NoStream);
    assert!(!app.terminal_showing(), "no stream, no terminal");
    assert!(app.terminal_sync(), "and the open stream is dropped");
}

fn buffer(width: u16, height: u16) -> Buffer {
    Buffer::empty(Rect::new(0, 0, width, height))
}

fn row(buffer: &Buffer, y: u16) -> String {
    let mut text = String::new();
    let mut x = 0;
    while x < buffer.area.width {
        let symbol = buffer[(x, y)].symbol();
        x += symbol.width().max(1) as u16;
        text.push_str(symbol);
    }
    text.trim_end().to_owned()
}

#[test]
fn a_screen_is_cut_to_a_smaller_pane_and_leaves_a_bigger_one_blank() {
    let app = reader();
    let mut screen = Screen::new(10, 3);
    screen.feed(b"0123456789\r\nabcdefghij\r\nXYZ");
    let frame = screen.frame();
    // Smaller than the screen in both directions.
    let mut small = buffer(6, 2);
    paint::cells(&mut small, Rect::new(0, 0, 6, 2), &frame, false, app.look());
    assert_eq!(
        (row(&small, 0), row(&small, 1)),
        ("012345".into(), "abcdef".into())
    );
    // Equal.
    let mut equal = buffer(10, 3);
    paint::cells(
        &mut equal,
        Rect::new(0, 0, 10, 3),
        &frame,
        false,
        app.look(),
    );
    assert_eq!(row(&equal, 2), "XYZ");
    assert_eq!(row(&equal, 0), "0123456789");
    // Larger: the rest of the pane is untouched.
    let mut large = buffer(14, 5);
    paint::cells(
        &mut large,
        Rect::new(2, 1, 12, 4),
        &frame,
        false,
        app.look(),
    );
    assert_eq!(row(&large, 1), "  0123456789");
    assert_eq!(row(&large, 4), "", "below the screen nothing is drawn");
}

#[test]
fn a_wide_character_is_drawn_whole_or_not_at_all() {
    let app = reader();
    let mut screen = Screen::new(6, 1);
    screen.feed("ab你好".as_bytes());
    let frame = screen.frame();
    let mut whole = buffer(6, 1);
    paint::cells(&mut whole, Rect::new(0, 0, 6, 1), &frame, false, app.look());
    assert_eq!(row(&whole, 0), "ab你好");
    // The pane ends between the two columns of 好: it is left out, not cut.
    let mut cut = buffer(5, 1);
    paint::cells(&mut cut, Rect::new(0, 0, 5, 1), &frame, false, app.look());
    assert_eq!(row(&cut, 0), "ab你");
    let mut narrow = buffer(3, 1);
    paint::cells(
        &mut narrow,
        Rect::new(0, 0, 3, 1),
        &frame,
        false,
        app.look(),
    );
    assert_eq!(row(&narrow, 0), "ab");
}

#[test]
fn the_program_sets_the_colours_and_a_lost_stream_dims_them() {
    let app = reader();
    let mut screen = Screen::new(12, 1);
    screen.feed(b"\x1b[31mred\x1b[0m \x1b[1mbold\x1b[0m");
    let frame = screen.frame();
    let mut live = buffer(12, 1);
    paint::cells(&mut live, Rect::new(0, 0, 12, 1), &frame, false, app.look());
    assert_eq!(live[(0, 0)].fg, ratatui::style::Color::Indexed(1));
    assert!(live[(4, 0)].modifier.contains(Modifier::BOLD));
    assert!(!live[(0, 0)].modifier.contains(Modifier::DIM));
    let mut dim = buffer(12, 1);
    paint::cells(&mut dim, Rect::new(0, 0, 12, 1), &frame, true, app.look());
    assert_eq!(dim[(0, 0)].fg, ratatui::style::Color::Indexed(1));
    assert!(dim[(0, 0)].modifier.contains(Modifier::DIM));
    assert!(dim[(4, 0)].modifier.contains(Modifier::BOLD));
}

#[test]
fn the_legend_names_the_other_view() {
    let mut app = reader();
    shown(&mut app);
    assert!(text(&app).contains("chat view"), "t leads back to the chat");
    press_t(&mut app);
    assert!(text(&app).contains("terminal"));
}

#[test]
fn the_cursor_is_one_reversed_cell_in_the_live_state_only() {
    let app = reader();
    let reversed = |bytes: &str, condition: Condition| {
        let mut screen = Screen::new(10, 2);
        screen.feed(bytes.as_bytes());
        let view = View {
            condition,
            screen: Some(Arc::new(screen.frame())),
        };
        let mut drawn = buffer(12, 4);
        paint::render(&mut drawn, &app, Rect::new(0, 0, 12, 4), Some(&view), "worker");
        (0..4)
            .flat_map(|y| (0..12).map(move |x| (x, y)))
            .filter(|position| drawn[*position].modifier.contains(Modifier::REVERSED))
            .collect::<Vec<_>>()
    };
    assert_eq!(reversed("ab", Condition::Live), [(2, 0)], "after the text");
    assert_eq!(reversed("ab\r\ncd", Condition::Live), [(2, 1)]);
    assert!(
        reversed("ab\x1b[?25l", Condition::Live).is_empty(),
        "hidden when the program hides it"
    );
    for condition in [
        Condition::Connecting { attempt: 2 },
        Condition::Exited(Exit::Code(0)),
        Condition::Lost("gone".into()),
    ] {
        assert!(reversed("ab", condition.clone()).is_empty(), "{condition:?}");
    }
    // A cell the program reversed itself shows as the plain cell under the cursor.
    assert!(reversed("\x1b[7ma\x1b[0m\x1b[1D", Condition::Live).is_empty());
}
