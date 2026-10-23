//! Oracle scenes for the Selection menu: a real drag selection at 100x36, the menu opened
//! in the middle of the pane, near its right edge and on its last row.
use super::*;
use crate::board::app::workspace::tests::MEMBER;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

pub(in crate::board) const SCENES: [&str; 3] = [
    "selection-menu-middle",
    "selection-menu-right-edge",
    "selection-menu-bottom",
];

const WIDTH: u16 = 100;
const HEIGHT: u16 = 36;

fn short_lines(count: usize) -> String {
    (0..count).map(|n| format!("passage {n:03}  \n")).collect()
}

/// Paragraphs that wrap to the full width of the pane.
fn wide_lines() -> String {
    (0..8)
        .map(|n| {
            format!(
                "{}\n\n",
                (0..40)
                    .map(|word| format!("note{n}word{word}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        })
        .collect()
}

fn draw(app: &mut App) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    app.mouse(
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
}

/// Drag over everything painted on row `y` of the transcript pane.
fn select_row(app: &mut App, y: u16) {
    let buffer = draw(app);
    let area = app.conversation.borrow().response_area.expect("pane");
    let text: Vec<_> = (area.x..area.right())
        .map(|x| buffer[(x, y)].symbol().to_owned())
        .collect();
    let first = text.iter().position(|s| s != " ").expect("a painted row") as u16;
    let last = text.iter().rposition(|s| s != " ").unwrap() as u16 + 1;
    mouse(
        app,
        MouseEventKind::Down(MouseButton::Left),
        area.x + first,
        y,
    );
    mouse(
        app,
        MouseEventKind::Drag(MouseButton::Left),
        area.x + last,
        y,
    );
    mouse(app, MouseEventKind::Up(MouseButton::Left), area.x + last, y);
}

pub(in crate::board) fn capture_case(scene: &str, look: crate::look::Look) -> App {
    assert!(SCENES.contains(&scene));
    let mut app = crate::board::app::workspace::tests::board();
    let view = app.view.as_mut().unwrap();
    // The bottom scene is the sender row plus seven lines: it fills the pane exactly, so the
    // last pane row is text and no "earlier" hint sits below it.
    let source = match scene {
        "selection-menu-right-edge" => wide_lines(),
        "selection-menu-bottom" => short_lines(7),
        _ => short_lines(60),
    };
    view.replies = vec![json!({"recipientId":MEMBER,"requestId":"r1","response":source})];
    view.look = look;
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    draw(&mut app);
    let area = app.conversation.borrow().response_area.expect("pane");
    let row = match scene {
        "selection-menu-bottom" => area.bottom() - 1,
        _ => area.y + area.height / 2,
    };
    select_row(&mut app, row);
    app
}
