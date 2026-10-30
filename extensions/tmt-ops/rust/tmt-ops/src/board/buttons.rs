//! Disposable button presentation; each existing surface retains activation authority.
use super::app::{App, Overlay};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
};
use std::cell::{Cell, RefCell};
use tmt_cli_style::grid::Align;
use tmt_tui::components::ButtonRow;

type Region = (Rect, Option<Overlay>);

#[derive(Default)]
pub(super) struct State {
    regions: RefCell<Vec<Region>>,
    hovered: Cell<Option<Region>>,
}

impl State {
    pub(super) fn begin_frame(&self) {
        self.regions.borrow_mut().clear();
    }

    pub(super) fn clear_hover(&self) {
        self.hovered.set(None);
    }

    pub(super) fn invalidate(&self) {
        self.begin_frame();
        self.clear_hover();
    }

    pub(super) fn move_pointer(&self, point: Position, owner: Option<Overlay>) -> bool {
        let next = self
            .regions
            .borrow()
            .iter()
            .rev()
            .copied()
            .find(|(rect, layer)| *layer == owner && rect.contains(point));
        self.hovered.replace(next) != next
    }
}

pub(super) use tmt_tui::components::Button;

/// Return painted rectangles, not effects; callers publish enabled activation hits themselves.
pub(super) fn paint(
    app: &App,
    buffer: &mut Buffer,
    area: Rect,
    buttons: &[Button<'_>],
    align: Align,
    owner: Option<Overlay>,
) -> Vec<Option<Rect>> {
    let look = app.look();
    let active = app.button_owner_is_active(owner);
    let hovered = app
        .buttons
        .hovered
        .get()
        .filter(|(_, layer)| *layer == owner)
        .map(|(rect, _)| rect);
    let row = ButtonRow {
        theme: &look.theme,
        depth: look.depth,
        align,
        active,
        hovered,
    };
    let painted = row.paint(buffer, area, buttons);
    if active {
        for (button, rect) in buttons.iter().zip(&painted) {
            if let Some(rect) = rect
                && button.is_enabled()
            {
                app.buttons.regions.borrow_mut().push((*rect, owner));
            }
        }
    }
    painted
}

#[cfg(test)]
mod tests;
