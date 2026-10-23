//! Oracle scenes for the Selection menu: a real drag selection with the menu opened below
//! it in the middle of a tall pane, flipped left near the pane's right edge, and above it
//! in a short pane whose last row is selected.
use super::*;
use crate::board::app::workspace::tests::MEMBER;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

pub(in crate::board) const SCENES: [&str; 3] = [
    "selection-menu-middle",
    "selection-menu-right-edge",
    "selection-menu-bottom",
];

/// The frame each scene is captured at: the tall ones leave the pane room for the box below
/// the selection, the short one (the oracle's usual 100x36) only above it.
pub(in crate::board) fn size(scene: &str) -> (u16, u16) {
    match scene {
        "selection-menu-bottom" => (100, 36),
        _ => (100, 56),
    }
}

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

fn draw(app: &mut App, (width, height): (u16, u16)) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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

/// The row nearest the middle of the pane whose text reaches the pane's right edge.
fn full_row_near_middle(buffer: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> u16 {
    let middle = area.y + area.height / 2;
    (0..area.height)
        .map(|offset| {
            // Alternate below and above the middle: 0, +1, -1, +2, -2, ...
            let step = (offset + 1) / 2;
            if offset % 2 == 1 {
                middle + step
            } else {
                middle.saturating_sub(step)
            }
        })
        .find(|y| {
            (area.right().saturating_sub(14)..area.right()).any(|x| buffer[(x, *y)].symbol() != " ")
        })
        .expect("a wrapped row that reaches the right edge")
}

/// Drag over everything painted on row `y` of the transcript pane.
fn select_row(app: &mut App, size: (u16, u16), y: u16) {
    let buffer = draw(app, size);
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
    let size = size(scene);
    draw(&mut app, size);
    let area = app.conversation.borrow().response_area.expect("pane");
    let row = match scene {
        "selection-menu-bottom" => area.bottom() - 1,
        "selection-menu-right-edge" => full_row_near_middle(&draw(&mut app, size), area),
        _ => area.y + area.height / 2,
    };
    select_row(&mut app, size, row);
    app
}
