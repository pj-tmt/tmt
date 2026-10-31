//! The Terminal tab for the oracle: a built-in-host member whose published screen is injected,
//! so no socket is involved.
use super::{
    discovery::Located,
    stream::StreamEvent,
    tab::tests::{address, tab},
};
use crate::board::app::{
    App,
    workspace::tests::{MEMBER, board},
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::Arc;
use tmt_pty_protocol::Exit;
use tmt_tui::components::terminal::Screen;

pub(in crate::board) const SCENES: &[&str] = &[
    "terminal-chips",
    "terminal-connecting",
    "terminal-live",
    "terminal-reconnecting",
    "terminal-ended",
    "terminal-typing",
    "terminal-zoomed",
    "terminal-list-focus",
    "terminal-back-to-chat",
];

/// A build with colours, a bold word, a wide character and the cursor on the sixth and last row
/// that fits the pane at 100x36.
fn program() -> Arc<tmt_tui::components::terminal::Frame> {
    let mut screen = Screen::new(48, 8);
    screen.feed(
        "\u{1b}[1m$ cargo test\u{1b}[0m\r\n\
         \u{1b}[32m   Compiling\u{1b}[0m tmt-ops v0.1.0\r\n\
         \u{1b}[33mwarning\u{1b}[0m: 你好 \u{1b}[31merror[E0308]\u{1b}[0m\r\n\
         \u{1b}[7m running 3 tests \u{1b}[0m\r\n\
         test a ... \u{1b}[32mok\u{1b}[0m\r\n\
         $ "
        .as_bytes(),
    );
    Arc::new(screen.frame())
}

pub(in crate::board) fn capture_case(scene: &str, look: crate::look::Look) -> App {
    let mut app = board();
    let view = app.view.as_mut().unwrap();
    view.look = look;
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    app.focus = 1;
    app.terminal = tab().0;
    app.apply_terminal_located(MEMBER, Located::Stream(address("oracle")));
    if scene != "terminal-chips" {
        app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(app.terminal_sync());
    }
    let events: &[StreamEvent] = &match scene {
        "terminal-live"
        | "terminal-typing"
        | "terminal-zoomed"
        | "terminal-list-focus"
        | "terminal-back-to-chat" => {
            vec![StreamEvent::Screen(program())]
        }
        "terminal-reconnecting" => vec![
            StreamEvent::Screen(program()),
            StreamEvent::Connecting { attempt: 2 },
        ],
        "terminal-ended" => vec![
            StreamEvent::Screen(program()),
            StreamEvent::Exited(Exit::Code(101)),
        ],
        _ => vec![],
    };
    for event in events {
        app.apply_terminal(1, event.clone());
    }
    match scene {
        "terminal-typing" => assert!(app.terminal.start_typing()),
        "terminal-zoomed" => {
            app.toggle_terminal_zoom();
        }
        // The list has focus: still no composer under the terminal (#5763).
        "terminal-list-focus" => app.focus = 0,
        // A click on the Chat chip: the chat and its composer come back.
        "terminal-back-to-chat" => {
            app.show_terminal(false);
        }
        _ => {}
    }
    app
}
