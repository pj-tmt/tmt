//! A press outside a dialog closes it and is not delivered to the board under it. Each overlay's
//! own tests open it and run [`assert_outside_press_closes`], so a missed registration fails there.
use crate::board::app::{App, Overlay};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use std::time::Instant;

fn paint(app: &App) {
    Terminal::new(TestBackend::new(160, 40))
        .unwrap()
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
}

fn click(app: &mut App, column: u16, row: u16) {
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    );
}

/// A painted board row that the sheet does not cover, if there is one.
fn row_outside(app: &App, sheet: Rect) -> Option<(u16, u16)> {
    app.hits.borrow().iter().find_map(|hit| {
        let at = (hit.x + 2, hit.y);
        (hit.row().is_some() && !sheet.contains(at.into())).then_some(at)
    })
}

/// `app` has `overlay` open and on top. Paints it, checks that it registered its sheet, presses
/// outside the sheet and checks that it closed and that the press did not land on the board.
pub(in crate::board) fn assert_outside_press_closes(app: &mut App, overlay: Overlay) {
    assert_eq!(app.overlay(), Some(overlay), "opened");
    paint(app);
    let sheet = app
        .peek_areas
        .dialog(overlay)
        .unwrap_or_else(|| panic!("{overlay:?} registers its painted sheet"));
    let selected = app.selected;
    let (column, row) = row_outside(app, sheet).unwrap_or_else(|| {
        let corner = (159, 39);
        assert!(
            !sheet.contains(corner.into()),
            "{overlay:?} leaves a corner"
        );
        corner
    });
    click(app, column, row);
    assert_ne!(app.overlay(), Some(overlay), "{overlay:?} closes");
    assert_eq!(app.selected, selected, "{overlay:?}: the press is taken");
}
