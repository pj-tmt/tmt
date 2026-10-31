//! The Terminal tab as the board shows it: when the tab exists, how `j` and the chips swap views, what each state
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
use crate::look::Look;
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    style::Modifier,
};
use std::{sync::Arc, time::Duration};
use tmt_pty_protocol::{ClientMessage, Eviction, Exit, Refusal};
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

/// Swaps the views as a click on the other chip does.
fn press_t(app: &mut App) {
    app.show_terminal(!app.terminal_showing());
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
fn a_member_with_a_stream_gets_chips_and_a_click_swaps_chat_and_terminal() {
    let mut app = reader();
    with_stream(&mut app);
    let chat = text(&app);
    assert!(chat.contains("Chat") && chat.contains("Terminal"), "{chat}");
    assert!(chat.contains("Conversation"));
    press_t(&mut app);
    assert!(app.terminal_showing());
    assert_eq!(app.selected_read(), None, "the chat is not read behind it");
    let terminal = text(&app);
    assert!(!terminal.contains("Conversation"));
    assert!(terminal.contains("Terminal"));
    press_t(&mut app);
    assert!(!app.terminal_showing());
    assert!(text(&app).contains("Conversation"));
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
    // Pinned, never `app.look()`: that reads NO_COLOR and COLORTERM of the process.
    let look = Look::default();
    let mut screen = Screen::new(12, 1);
    screen.feed(b"\x1b[31mred\x1b[0m \x1b[1mbold\x1b[0m");
    let frame = screen.frame();
    let mut live = buffer(12, 1);
    paint::cells(&mut live, Rect::new(0, 0, 12, 1), &frame, false, look);
    assert_eq!(live[(0, 0)].fg, ratatui::style::Color::Indexed(1));
    assert!(live[(4, 0)].modifier.contains(Modifier::BOLD));
    assert!(!live[(0, 0)].modifier.contains(Modifier::DIM));
    let mut dim = buffer(12, 1);
    paint::cells(&mut dim, Rect::new(0, 0, 12, 1), &frame, true, look);
    assert_eq!(dim[(0, 0)].fg, ratatui::style::Color::Indexed(1));
    assert!(dim[(0, 0)].modifier.contains(Modifier::DIM));
    assert!(dim[(4, 0)].modifier.contains(Modifier::BOLD));
}

#[test]
fn no_colour_leaves_the_program_colours_unpainted() {
    let look = Look {
        depth: tmt_cli_style::Depth::None,
        ..Look::default()
    };
    let mut screen = Screen::new(12, 1);
    screen.feed(b"\x1b[31mred\x1b[0m \x1b[1mbold\x1b[0m");
    let mut drawn = buffer(12, 1);
    paint::cells(
        &mut drawn,
        Rect::new(0, 0, 12, 1),
        &screen.frame(),
        false,
        look,
    );
    assert_eq!(drawn[(0, 0)].fg, ratatui::style::Color::Reset);
    assert!(drawn[(4, 0)].modifier.contains(Modifier::BOLD));
}

#[test]
fn j_on_the_list_opens_a_built_in_members_terminal_and_leads_back_and_t_does_not() {
    let mut app = reader();
    with_stream(&mut app);
    app.focus = 0;
    let chips = |app: &App| {
        text(app)
            .lines()
            .find(|line| line.contains("Chat"))
            .expect("the chips row")
            .to_owned()
    };
    assert!(
        chips(&app).contains("Chat   j  Terminal"),
        "{}",
        chips(&app)
    );
    app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    assert!(!app.terminal_showing(), "t has no default meaning here");
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert!(app.terminal_showing());
    // From the terminal, j sits on the Chat chip and leads back with the keyboard alone.
    let shown = chips(&app);
    assert!(
        shown.contains(" j  Chat") && !shown.contains(" j  Terminal"),
        "{shown}"
    );
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert!(!app.terminal_showing());
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
        paint::render(
            &mut drawn,
            &app,
            Rect::new(0, 0, 12, 4),
            Some(&view),
            "worker",
        );
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
        assert!(
            reversed("ab", condition.clone()).is_empty(),
            "{condition:?}"
        );
    }
    // A cell the program reversed itself shows as the plain cell under the cursor.
    assert!(reversed("\x1b[7ma\x1b[0m\x1b[1D", Condition::Live).is_empty());
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn draw_at(app: &App, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
}

/// The screen's size in cells, as the program is asked to take it.
fn window(app: &App) -> tmt_pty_protocol::Size {
    let area = app.terminal.area().expect("the screen was drawn");
    super::stream::tests::size(area.width, area.height)
}

/// A shown terminal attached to a fake supervisor, with the first screen applied.
struct Attached {
    app: App,
    peer: super::stream::tests::Peer,
    events: std::sync::mpsc::Receiver<(u64, StreamEvent)>,
    _fake: super::stream::tests::Fake,
}

impl Attached {
    fn new(snapshot: &[u8]) -> Self {
        Self::opened(snapshot, |app, address| {
            app.apply_terminal_located(MEMBER, Located::Stream(address));
            press_t(app);
        })
    }

    /// Attached once `open` has asked for the terminal of the stream at its address.
    fn opened(snapshot: &[u8], open: impl FnOnce(&mut App, super::discovery::Address)) -> Self {
        let fake = super::stream::tests::Fake::new();
        let (sender, events) = std::sync::mpsc::channel();
        let mut tab = super::tab::Tab::default();
        tab.set_connect(Box::new(move |epoch, address| {
            let sender = sender.clone();
            super::stream::Stream::open_with(address, super::stream::tests::quick(), move |event| {
                let _ = sender.send((epoch, event));
            })
        }));
        let mut app = reader();
        app.terminal = tab;
        open(&mut app, fake.address.clone());
        assert!(app.terminal_sync());
        let mut peer = fake.accept();
        peer.attach(super::stream::tests::size(40, 6), snapshot);
        let mut attached = Self {
            app,
            peer,
            events,
            _fake: fake,
        };
        attached.screen();
        attached
    }

    /// Applies events until the next screen.
    fn screen(&mut self) {
        loop {
            let (epoch, event) = self
                .events
                .recv_timeout(Duration::from_secs(5))
                .expect("an event from the stream");
            let screen = matches!(event, StreamEvent::Screen(_));
            self.app.apply_terminal(epoch, event);
            if screen {
                return;
            }
        }
    }

    /// `⏎`, a frame to learn the area, and the pass that asks for the window.
    fn start_typing(&mut self) {
        self.app.key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(self.app.terminal.typing());
        text(&self.app);
        self.app.terminal_sync();
        assert_eq!(
            self.peer.expect(),
            ClientMessage::Resize(window(&self.app)),
            "the window is asked for when typing starts"
        );
    }

    fn typed(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
        self.app.key(key(code, modifiers));
        match self.peer.expect() {
            ClientMessage::Input(bytes) => bytes,
            other => panic!("expected input, got {other:?}"),
        }
    }
}

#[test]
fn typing_sends_every_key_to_the_program_and_ctrl_backslash_leaves() {
    let mut attached = Attached::new(b"hi");
    attached.start_typing();
    assert_eq!(attached.typed(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
    assert_eq!(attached.typed(KeyCode::Esc, KeyModifiers::NONE), b"\x1b");
    assert_eq!(
        attached.typed(KeyCode::PageUp, KeyModifiers::NONE),
        b"\x1b[5~"
    );
    assert_eq!(attached.typed(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
    assert_eq!(
        attached.typed(KeyCode::Char('c'), KeyModifiers::CONTROL),
        b"\x03",
        "Ctrl-C is the program's, not the board's quit"
    );
    assert!(text(&attached.app).contains("Typing into worker's terminal"));
    // Ctrl-\ leaves and is not sent; the board has its keys again.
    assert!(matches!(
        attached
            .app
            .key(key(KeyCode::Char('\\'), KeyModifiers::CONTROL)),
        crate::board::app::Effect::None
    ));
    assert!(!attached.app.terminal.typing());
    assert!(attached.peer.quiet(), "Ctrl-\\ never reaches the program");
    attached
        .app
        .key(key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(attached.peer.quiet(), "after leaving, keys are the board's");
}

#[test]
fn the_window_is_asked_for_only_when_typing_starts_and_when_its_area_changes() {
    let mut attached = Attached::new(b"hi");
    text(&attached.app);
    attached.app.terminal_sync();
    assert!(
        attached.peer.quiet(),
        "a view that only watches never resizes"
    );
    attached.start_typing();
    attached.app.terminal_sync();
    assert!(
        attached.peer.quiet(),
        "the same area asks for nothing again"
    );
    draw_at(&attached.app, 140, 30);
    attached.app.terminal_sync();
    assert_eq!(
        attached.peer.expect(),
        ClientMessage::Resize(window(&attached.app))
    );
    attached
        .app
        .key(key(KeyCode::Char('\\'), KeyModifiers::CONTROL));
    draw_at(&attached.app, 120, 28);
    attached.app.terminal_sync();
    assert!(
        attached.peer.quiet(),
        "leaving typing resizes nothing back and a later area change asks for nothing"
    );
}

#[test]
fn a_paste_and_the_wheel_go_to_the_program_while_typing() {
    let mut attached = Attached::new(b"\x1b[?1049h");
    attached.start_typing();
    attached.app.paste("ls\n");
    assert_eq!(
        attached.peer.expect(),
        ClientMessage::Input(b"ls\r".to_vec())
    );
    let area = attached.app.terminal.area().unwrap();
    let wheel = |column, row| ratatui::crossterm::event::MouseEvent {
        kind: ratatui::crossterm::event::MouseEventKind::ScrollUp,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    attached
        .app
        .mouse(wheel(area.x + 1, area.y + 1), std::time::Instant::now());
    assert_eq!(
        attached.peer.expect(),
        ClientMessage::Input(b"\x1b[A\x1b[A\x1b[A".to_vec()),
        "the alternate screen scrolls with arrows"
    );
    attached.app.mouse(wheel(0, 0), std::time::Instant::now());
    assert!(attached.peer.quiet(), "a wheel elsewhere is the board's");
    assert!(
        attached.app.terminal.typing(),
        "and a wheel does not end typing"
    );
    attached.app.mouse(
        ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::Down(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(
        !attached.app.terminal.typing(),
        "a press elsewhere ends typing"
    );
    assert!(attached.peer.quiet(), "and the press itself sends no bytes");
}

fn mouse_at(
    kind: ratatui::crossterm::event::MouseEventKind,
    column: u16,
    row: u16,
) -> ratatui::crossterm::event::MouseEvent {
    ratatui::crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn a_press_just_outside_the_screen_ends_typing_on_every_side_and_a_press_inside_does_not() {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind::Down};
    let now = std::time::Instant::now();
    let mut probe = Attached::new(b"hi");
    probe.start_typing();
    let area = probe.app.terminal.area().unwrap();
    assert!(area.x > 0 && area.y > 0, "room on the left and above");
    let sides = [
        ("left", area.x - 1, area.y + 1),
        ("right", area.right(), area.y + 1),
        ("above", area.x + 1, area.y - 1),
        ("below", area.x + 1, area.bottom()),
    ];
    for (side, column, row) in sides {
        let mut attached = Attached::new(b"hi");
        attached.start_typing();
        attached
            .app
            .mouse(mouse_at(Down(MouseButton::Left), column, row), now);
        assert!(
            !attached.app.terminal.typing(),
            "a press {side} of it ends typing"
        );
        assert!(
            attached.peer.quiet(),
            "the press {side} sends the program nothing"
        );
    }
    let mut attached = Attached::new(b"hi");
    attached.start_typing();
    let inside = attached.app.terminal.area().unwrap();
    for button in [MouseButton::Left, MouseButton::Right] {
        attached
            .app
            .mouse(mouse_at(Down(button), inside.x + 1, inside.y + 1), now);
    }
    assert!(
        attached.app.terminal.typing(),
        "a press on the screen keeps typing"
    );
    assert!(attached.peer.quiet(), "and sends nothing");
}

#[test]
fn only_a_press_outside_ends_typing_and_it_lands_on_what_it_hit() {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    let now = std::time::Instant::now();
    let mut attached = Attached::new(b"hi");
    attached.start_typing();
    for kind in [
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Moved,
    ] {
        attached.app.mouse(mouse_at(kind, 0, 0), now);
        assert!(
            attached.app.terminal.typing(),
            "{kind:?} elsewhere keeps typing"
        );
    }
    let hits = attached.app.workspace_strip_hits.borrow().clone();
    let chat = hits
        .iter()
        .find(|(_, choice)| matches!(choice, Choice::TerminalView(false)))
        .expect("the Chat chip")
        .0;
    assert!(attached.app.terminal.shown());
    attached.app.mouse(
        mouse_at(MouseEventKind::Down(MouseButton::Left), chat.x, chat.y),
        now,
    );
    assert!(!attached.app.terminal.typing(), "the press ends typing");
    assert!(
        !attached.app.terminal.shown(),
        "and lands on the Chat chip as it would have without typing"
    );
}

#[test]
fn a_paste_with_ctrl_backslash_bytes_is_text_for_the_program_and_does_not_leave() {
    let mut attached = Attached::new(b"hi");
    attached.start_typing();
    attached.app.paste("a\u{1c}b");
    assert_eq!(
        attached.peer.expect(),
        ClientMessage::Input(b"a\x1cb".to_vec()),
        "pasted bytes are sent as pasted"
    );
    assert!(attached.app.terminal.typing(), "only a key press leaves");
}

#[test]
fn typing_ends_when_the_program_does() {
    let mut attached = Attached::new(b"hi");
    attached.start_typing();
    attached
        .app
        .apply_terminal(1, StreamEvent::Exited(Exit::Code(0)));
    assert!(!attached.app.terminal.typing());
    assert!(text(&attached.app).contains("⏎ attaches again"));
}

#[test]
fn enter_after_the_stream_ended_attaches_again() {
    let (tab, opened) = tab();
    let mut app = reader();
    app.terminal = tab;
    shown(&mut app);
    app.apply_terminal(1, StreamEvent::Exited(Exit::Code(1)));
    app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        !app.terminal.typing(),
        "an ended terminal is attached, not typed into"
    );
    assert!(app.terminal_sync(), "the stream is opened afresh");
    assert_eq!(opened.lock().unwrap().len(), 2);
    assert_eq!(
        app.terminal_locate().as_deref(),
        Some(MEMBER),
        "and core is asked again"
    );
}

#[test]
fn z_zooms_only_a_shown_terminal_and_escape_or_z_leaves() {
    let mut app = reader();
    with_stream(&mut app);
    assert!(
        !app.bindings().contains_key("z"),
        "z is offered only where it acts"
    );
    app.key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(!app.terminal_zoomed(), "the chat has nothing to zoom");
    assert!(!text(&app).contains("Zoom"), "no chip on the chat");
    press_t(&mut app);
    assert!(app.bindings().contains_key("z"));
    let shown = text(&app);
    assert!(
        shown.contains("Zoom") && shown.contains("SQUAD / AGENT"),
        "{shown}"
    );
    app.key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(app.terminal_zoomed());
    let zoomed = text(&app);
    assert!(
        zoomed.contains("Zoom") && !zoomed.contains("SQUAD / AGENT"),
        "{zoomed}"
    );
    app.key(key(KeyCode::Char('j'), KeyModifiers::NONE));
    assert!(
        app.terminal_zoomed(),
        "board keys do nothing behind a zoomed terminal"
    );
    app.key(key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.terminal_zoomed());
    app.key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    app.key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(!app.terminal_zoomed(), "z leaves it too");
    // The chip does the same, in both views.
    let has_chip = |app: &App| {
        app.workspace_strip_hits
            .borrow()
            .iter()
            .any(|(_, choice)| matches!(choice, Choice::TerminalZoom))
    };
    text(&app);
    assert!(has_chip(&app));
    app.choose(Choice::TerminalZoom);
    assert!(app.terminal_zoomed());
    text(&app);
    assert!(has_chip(&app), "the zoomed view keeps the chip to leave by");
    app.choose(Choice::TerminalZoom);
    assert!(!app.terminal_zoomed());
}

#[test]
fn an_authored_zoom_key_works_while_typing_and_never_reaches_the_program() {
    let mut attached = Attached::new(b"hi");
    let path = std::env::temp_dir().join(format!("ops-terminal-zoom-{}.toml", std::process::id()));
    std::fs::write(&path, "[bind]\nctrl-o = 'terminal-zoom'\n").unwrap();
    let config = crate::config::Config::read(path.clone()).unwrap();
    attached
        .app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .configure(&config)
        .unwrap();
    std::fs::remove_file(path).unwrap();
    attached.start_typing();
    attached
        .app
        .key(key(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(attached.app.terminal_zoomed());
    assert!(
        attached.app.terminal.typing(),
        "zooming does not end typing"
    );
    attached
        .app
        .key(key(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(!attached.app.terminal_zoomed());
    assert!(attached.peer.quiet(), "the bound key is never sent");
    // The default z is not a binding the user wrote: it is the program's.
    assert_eq!(attached.typed(KeyCode::Char('z'), KeyModifiers::NONE), b"z");
    assert!(!attached.app.terminal_zoomed());
}

#[test]
fn a_zoomed_terminal_starts_typing_with_enter_and_asks_for_the_whole_room() {
    let mut attached = Attached::new(b"hi");
    attached
        .app
        .key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(attached.app.terminal_zoomed());
    text(&attached.app);
    let zoomed = window(&attached.app);
    attached.app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(attached.app.terminal.typing());
    attached.app.terminal_sync();
    assert_eq!(attached.peer.expect(), ClientMessage::Resize(zoomed));
    assert!(
        zoomed.cols >= 150,
        "the zoomed screen has the width of the board: {zoomed:?}"
    );
}

#[test]
fn each_key_is_a_keycap_right_before_its_word_and_its_hit_covers_both() {
    let mut app = reader();
    with_stream(&mut app);
    app.focus = 0;
    let drawn = text(&app);
    let line = drawn
        .lines()
        .find(|line| line.contains("Chat"))
        .expect("the chips row");
    assert!(line.contains("Chat   j  Terminal"), "{line}");
    let chat = line.chars().position(|c| c == 'C').unwrap();
    let hits = app.workspace_strip_hits.borrow().clone();
    let (area, _) = hits
        .iter()
        .find(|(_, choice)| matches!(choice, Choice::TerminalView(true)))
        .expect("the Terminal chip");
    assert_eq!(usize::from(area.x), chat + 6, "from the keycap");
    assert_eq!(
        usize::from(area.right()),
        chat + 19,
        "to the end of the label"
    );
    let (chat_area, _) = hits
        .iter()
        .find(|(_, choice)| matches!(choice, Choice::TerminalView(false)))
        .expect("the Chat chip");
    assert!(chat_area.right() <= area.x, "Chat has no keycap of its own");
}

#[test]
fn the_terminal_action_has_no_default_key() {
    let mut app = reader();
    with_stream(&mut app);
    assert!(!text(&app).contains("t terminal"));
    assert!(!crate::board::app::workspace::tests::help_lists(&app, "t"));
}

#[test]
fn the_zoom_glyph_is_one_cell_wide_in_narrow_and_cjk_terminals() {
    use unicode_width::UnicodeWidthChar;
    assert_eq!('⤢'.width(), Some(1));
    assert_eq!('⤢'.width_cjk(), Some(1), "not an ambiguous-width character");
}

#[test]
fn the_lines_under_the_screen_name_the_way_back() {
    let mut app = reader();
    shown(&mut app);
    app.apply_terminal(1, StreamEvent::Screen(frame("hi")));
    assert!(text(&app).contains("⏎ type into it"));
    app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(text(&app).contains("Ctrl-\\ back to the list"));
    app.key(key(KeyCode::Char('\\'), KeyModifiers::CONTROL));
    app.key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(text(&app).contains("⏎ type into it · Esc back"));
    app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(text(&app).contains("Typing into worker's terminal · Ctrl-\\ back"));
}

#[test]
fn while_typing_the_footer_names_only_the_way_out() {
    let mut app = reader();
    shown(&mut app);
    app.apply_terminal(1, StreamEvent::Screen(frame("hi")));
    assert!(
        text(&app).contains("p pin"),
        "the list's hints before typing"
    );
    app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    let typing = text(&app);
    let footer = typing.lines().last().unwrap().trim().to_owned();
    assert_eq!(footer, "Ctrl-\\ back", "{typing}");
}

#[test]
fn the_terminal_view_never_offers_the_composer_whichever_pane_has_focus() {
    let mut app = reader();
    shown(&mut app);
    app.apply_terminal(1, StreamEvent::Screen(frame("hi")));
    let no_composer = |app: &App, when: &str| {
        let drawn = text(app);
        for hint in ["to message worker", "⏎ send", "new line", "@ mention"] {
            assert!(!drawn.contains(hint), "{when}: {hint}\n{drawn}");
        }
    };
    no_composer(&app, "reader focus");
    app.focus = 0;
    no_composer(&app, "list focus");
    app.focus = 1;
    app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.terminal.typing());
    no_composer(&app, "typing");
    app.key(key(KeyCode::Char('\\'), KeyModifiers::CONTROL));
    no_composer(&app, "after leaving typing");
    press_t(&mut app);
    assert!(
        text(&app).contains("to message worker"),
        "the chat view offers it again"
    );
}

#[test]
fn a_created_member_terminal_takes_the_keys_as_soon_as_its_stream_opens() {
    // A clean built-in create asks before core has named the stream.
    let mut attached = Attached::opened(b"hi", |app, address| {
        app.terminal.show_typing(MEMBER);
        assert!(!app.terminal_sync(), "nothing to open yet");
        assert!(!app.terminal.typing());
        app.apply_terminal_located(MEMBER, Located::Stream(address));
    });
    assert!(attached.app.terminal.typing(), "typing as if clicked");
    let typing = text(&attached.app);
    for hint in ["to message worker", "⏎ send", "@ mention"] {
        assert!(!typing.contains(hint), "no composer: {hint}\n{typing}");
    }
    attached.app.terminal_sync();
    assert_eq!(
        attached.peer.expect(),
        ClientMessage::Resize(window(&attached.app)),
        "the window is asked for as with a click"
    );
    assert_eq!(
        attached.typed(KeyCode::Char('a'), KeyModifiers::NONE),
        b"a",
        "keys reach the member, not the composer"
    );
}

#[test]
fn a_create_whose_stream_never_opens_leaves_no_typing_behind() {
    let mut app = reader();
    app.terminal.show_typing(MEMBER);
    app.terminal_sync();
    // The member exited before its stream was found.
    app.apply_terminal_located(MEMBER, Located::NoStream);
    app.terminal_sync();
    assert!(!app.terminal.typing());
    // Later the member is started again and picked by hand.
    app.apply_terminal_located(MEMBER, Located::Stream(address("a")));
    assert!(app.terminal_sync(), "the shown terminal opens");
    assert!(!app.terminal.typing(), "the old create does not type");
    press_t(&mut app);
    assert!(
        text(&app).contains("to message worker"),
        "the chat view offers the composer"
    );
}

/// Draws as the board loop does and reports the real cursor: shown, and where.
fn real_cursor(app: &App) -> (bool, ratatui::layout::Position) {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    app.park_hidden_caret(&mut terminal).unwrap();
    let backend = terminal.backend();
    (backend.cursor_visible(), backend.cursor_position())
}

fn at(app: &App, column: u16, row: u16) -> ratatui::layout::Position {
    let area = app.terminal.area().unwrap();
    ratatui::layout::Position::new(area.x + column, area.y + row)
}

#[test]
fn while_typing_the_real_cursor_is_on_the_programs_cursor() {
    let mut attached = Attached::new(b"hi");
    attached.start_typing();
    let (shown, position) = real_cursor(&attached.app);
    assert!(shown, "the program shows its cursor");
    assert_eq!(position, at(&attached.app, 2, 0));
    let cell = buffer_at(&attached.app, position);
    assert!(
        !cell.modifier.contains(Modifier::REVERSED),
        "the real cursor stands in for the reversed cell"
    );
}

#[test]
fn a_hidden_program_cursor_still_moves_the_hidden_real_cursor() {
    // Claude Code hides its cursor and parks it where it takes text.
    let mut attached = Attached::new(b"hi\x1b[?25l\x1b[2;4H");
    attached.start_typing();
    let (shown, position) = real_cursor(&attached.app);
    assert!(!shown, "hidden as the program has it");
    assert_eq!(
        position,
        at(&attached.app, 3, 1),
        "where an input method draws"
    );
}

#[test]
fn leaving_typing_gives_the_caret_back_to_the_board() {
    let mut attached = Attached::new(b"hi\x1b[?25l\x1b[2;4H");
    let before = real_cursor(&attached.app);
    attached.start_typing();
    assert!(attached.app.terminal.caret().is_some());
    attached
        .app
        .key(key(KeyCode::Char('\\'), KeyModifiers::CONTROL));
    assert!(!attached.app.terminal.typing());
    let after = real_cursor(&attached.app);
    assert_eq!(
        attached.app.terminal.caret(),
        None,
        "no terminal caret is kept"
    );
    assert_eq!(after.0, before.0);
    if after.0 {
        assert_eq!(after.1, before.1, "the board's caret is back");
    }
    // Switching to the chat while the stream runs leaves no caret either.
    press_t(&mut attached.app);
    real_cursor(&attached.app);
    assert_eq!(attached.app.terminal.caret(), None);
}

#[test]
fn a_caret_outside_the_drawn_screen_places_nothing() {
    let mut screen = Screen::new(20, 6);
    screen.feed(b"\x1b[?25l\x1b[6;16H");
    let frame = screen.frame();
    let area = Rect::new(4, 2, 12, 4);
    assert_eq!(paint::caret(area, &frame), None, "a cut screen hides it");
    let caret = paint::caret(Rect::new(4, 2, 20, 6), &frame).unwrap();
    assert_eq!(caret.at, ratatui::layout::Position::new(19, 7));
    assert!(!caret.shown);
}

fn buffer_at(app: &App, position: ratatui::layout::Position) -> ratatui::buffer::Cell {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal.backend().buffer()[position].clone()
}

#[test]
fn a_zoomed_watcher_gets_the_whole_room_and_unzoom_gives_the_normal_area_back() {
    let mut attached = Attached::new(b"hi");
    text(&attached.app);
    let normal = window(&attached.app);
    attached.app.terminal_sync();
    assert!(attached.peer.quiet(), "a plain watcher never resizes");
    attached
        .app
        .key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    // The frame that draws the zoomed area asks for it at once, without another wake.
    text(&attached.app);
    attached.app.terminal.fit_window();
    let zoomed = window(&attached.app);
    assert!(zoomed.cols > normal.cols, "{zoomed:?} vs {normal:?}");
    assert_eq!(attached.peer.expect(), ClientMessage::Resize(zoomed));
    assert!(
        !attached.app.terminal.typing(),
        "zoom does not start typing"
    );
    // A loop pass before the next frame still knows only the zoomed area: nothing new.
    attached.app.key(key(KeyCode::Esc, KeyModifiers::NONE));
    attached.app.terminal_sync();
    assert!(attached.peer.quiet());
    text(&attached.app);
    attached.app.terminal.fit_window();
    assert_eq!(
        attached.peer.expect(),
        ClientMessage::Resize(normal),
        "unzoom shrinks the program back"
    );
    // Back to watching: later area changes ask for nothing.
    draw_at(&attached.app, 120, 28);
    attached.app.terminal.fit_window();
    attached.app.terminal_sync();
    assert!(attached.peer.quiet());
}

#[test]
fn unzoom_while_typing_leaves_no_refit_for_after_typing_ends() {
    let mut attached = Attached::new(b"hi");
    attached
        .app
        .key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    text(&attached.app);
    attached.app.terminal.fit_window();
    let zoomed = window(&attached.app);
    assert_eq!(attached.peer.expect(), ClientMessage::Resize(zoomed));
    attached.app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(attached.app.terminal.typing());
    attached.app.toggle_terminal_zoom();
    text(&attached.app);
    attached.app.terminal_sync();
    assert_eq!(
        attached.peer.expect(),
        ClientMessage::Resize(window(&attached.app)),
        "typing fits the program to the unzoomed area"
    );
    attached
        .app
        .key(key(KeyCode::Char('\\'), KeyModifiers::CONTROL));
    assert!(!attached.app.terminal.typing());
    text(&attached.app);
    attached.app.terminal_sync();
    assert!(attached.peer.quiet(), "a watcher asks for nothing more");
}

#[test]
fn a_board_resize_while_zoomed_resizes_the_program_both_ways() {
    let mut attached = Attached::new(b"hi");
    attached
        .app
        .key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    text(&attached.app);
    attached.app.terminal.fit_window();
    let big = window(&attached.app);
    assert_eq!(attached.peer.expect(), ClientMessage::Resize(big));
    draw_at(&attached.app, 100, 24);
    attached.app.terminal.fit_window();
    let small = window(&attached.app);
    assert!(small.cols < big.cols);
    assert_eq!(attached.peer.expect(), ClientMessage::Resize(small));
}

#[test]
fn a_member_without_a_built_in_stream_has_nothing_to_zoom_or_resize() {
    // A tmux member: core names no stream, so there is no terminal and no window to ask for.
    let mut app = reader();
    app.apply_terminal_located(MEMBER, Located::NoStream);
    press_t(&mut app);
    app.key(key(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(!app.terminal_zoomed());
    text(&app);
    app.terminal.fit_window();
    assert!(!app.terminal_sync(), "no stream is opened");
}

/// A second member, `codex-worker`, in the same room as `worker`.
fn with_second_member(app: &mut App) {
    use crate::board::app::workspace::tests::{CODEX_MEMBER, ROOM};
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut member = workspace.roster.members[MEMBER].clone();
    member.id = CODEX_MEMBER.into();
    member.name = "codex-worker".into();
    workspace.roster.members.insert(CODEX_MEMBER.into(), member);
    workspace.roster.squads[0].members.push(CODEX_MEMBER.into());
    workspace.rows.insert(
        crate::board::workspace::OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: CODEX_MEMBER.into(),
        },
        serde_json::json!({"id":CODEX_MEMBER,"name":"codex-worker","state":"working"}),
    );
    app.apply_terminal_located(CODEX_MEMBER, Located::Stream(address("b")));
}

fn select(app: &mut App, identity: &str) {
    app.workspace.selected = Some(crate::board::workspace::OpsTarget::Member {
        room_id: Some(crate::board::app::workspace::tests::ROOM.into()),
        identity_id: identity.into(),
    });
    app.terminal_sync();
}

#[test]
fn a_terminal_asked_for_one_member_leaves_every_other_member_on_chat_and_enter_composes() {
    use crate::board::app::workspace::tests::CODEX_MEMBER;
    let mut app = reader();
    with_second_member(&mut app);
    shown(&mut app);
    assert!(app.terminal_showing());
    select(&mut app, CODEX_MEMBER);
    assert!(!app.terminal_showing(), "another member shows its chat");
    assert!(text(&app).contains("to message codex-worker"));
    app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.input.is_some(), "Enter composes, as for a tmux member");
    assert!(!app.terminal.typing());
    app.input = None;
    // worker keeps its own choice.
    select(&mut app, MEMBER);
    assert!(app.terminal_showing());
}

#[test]
fn after_a_create_types_into_its_member_another_member_still_composes() {
    use crate::board::app::workspace::tests::CODEX_MEMBER;
    let mut app = reader();
    with_second_member(&mut app);
    app.terminal.show_typing(MEMBER);
    with_stream(&mut app);
    assert!(app.terminal_sync());
    assert!(app.terminal.typing(), "the created member takes the keys");
    select(&mut app, CODEX_MEMBER);
    assert!(!app.terminal.typing(), "typing ended with that stream");
    assert!(!app.terminal_showing());
    app.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.input.is_some(), "Enter composes");
}

fn drag(attached: &mut Attached, from: (u16, u16), to: (u16, u16)) -> crate::board::app::Effect {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    let now = std::time::Instant::now();
    let area = attached.app.terminal.area().unwrap();
    let at = |(column, row): (u16, u16)| (area.x + column, area.y + row);
    let (x, y) = at(from);
    attached
        .app
        .mouse(mouse_at(MouseEventKind::Down(MouseButton::Left), x, y), now);
    let (x, y) = at(to);
    attached
        .app
        .mouse(mouse_at(MouseEventKind::Drag(MouseButton::Left), x, y), now);
    attached
        .app
        .mouse(mouse_at(MouseEventKind::Up(MouseButton::Left), x, y), now)
}

fn copied(effect: crate::board::app::Effect) -> Option<String> {
    match effect {
        crate::board::app::Effect::Act(crate::board::app::Request::Copy { text, .. }) => Some(text),
        _ => None,
    }
}

#[test]
fn a_drag_over_the_screen_copies_its_text_while_watching_and_while_typing() {
    let mut attached = Attached::new(b"hello world\r\nsecond line");
    text(&attached.app);
    assert_eq!(
        copied(drag(&mut attached, (6, 0), (5, 1))).as_deref(),
        Some("world\nsecond"),
        "watching: reading order, rows joined"
    );
    let shown = buffer_at(&attached.app, at(&attached.app, 7, 0));
    assert!(
        shown.modifier.contains(Modifier::REVERSED),
        "the selection shows"
    );
    attached.start_typing();
    assert_eq!(
        copied(drag(&mut attached, (0, 0), (4, 0))).as_deref(),
        Some("hello")
    );
    assert!(
        attached.peer.quiet(),
        "the drag sends nothing to the program"
    );
    assert!(attached.app.terminal.typing(), "and typing goes on");
    // A press that does not move copies nothing.
    assert_eq!(copied(drag(&mut attached, (2, 0), (2, 0))), None);
}

#[test]
fn a_click_on_the_watched_screen_still_lands_on_the_board() {
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    let mut attached = Attached::new(b"hello world");
    text(&attached.app);
    let screen = Some(crate::config::Pane::Replies);
    attached.app.focus = 0;
    assert_ne!(attached.app.focused_pane(), screen);
    let area = attached.app.terminal.area().unwrap();
    let (x, y) = (area.x + 2, area.y);
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        assert!(
            attached.app.terminal_mouse(mouse_at(kind, x, y)).is_none(),
            "{kind:?} without a move is the board's"
        );
    }
    attached.app.mouse(
        mouse_at(MouseEventKind::Down(MouseButton::Left), x, y),
        std::time::Instant::now(),
    );
    assert_eq!(
        attached.app.focused_pane(),
        screen,
        "the press focuses the pane"
    );
    assert_eq!(copied(drag(&mut attached, (2, 0), (2, 0))), None);
}

#[test]
fn a_program_that_asked_for_the_mouse_keeps_the_drag_and_the_line_names_the_bypass() {
    let mut attached = Attached::new(b"hello\x1b[?1002h");
    attached.start_typing();
    assert_eq!(copied(drag(&mut attached, (0, 0), (4, 0))), None);
    assert!(text(&attached.app).contains("bypass-drag selects"));
}
