//! Disposable button presentation; each existing surface retains activation authority.
use super::app::{App, Overlay};
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::Style,
};
use std::cell::{Cell, RefCell};
use tmt_cli_style::{
    grid::Align,
    theme::screen::{self, ChipKind, ChipState},
};
use tmt_tui::components::strip;

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

#[derive(Clone, Copy)]
pub(super) struct Button<'a> {
    label: &'a str,
    focused: bool,
    enabled: bool,
    idle: Option<Style>,
}

impl<'a> Button<'a> {
    pub(super) fn secondary(label: &'a str) -> Self {
        Self {
            label,
            focused: false,
            enabled: true,
            idle: None,
        }
    }

    pub(super) fn primary(label: &'a str) -> Self {
        Self::secondary(label).focused(true)
    }

    pub(super) fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub(super) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(super) fn idle(mut self, style: Style) -> Self {
        self.idle = Some(style);
        self
    }
}

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
    let style = |button: &Button<'_>, hovered: bool| {
        let state = if !button.enabled {
            ChipState::Disabled
        } else if active && (button.focused || hovered) {
            ChipState::Hot
        } else {
            ChipState::Idle
        };
        if state == ChipState::Idle
            && let Some(style) = button.idle
        {
            return style;
        }
        screen::chip(&look.theme, ChipKind::Secondary, state, look.depth)
    };
    let labels: Vec<_> = buttons
        .iter()
        .map(|button| (button.label, style(button, false)))
        .collect();
    let painted = strip::paint_chips(buffer, area, &labels, align);
    for (button, rect) in buttons.iter().zip(&painted) {
        let Some(rect) = rect else { continue };
        if active && button.enabled {
            app.buttons.regions.borrow_mut().push((*rect, owner));
            if app.buttons.hovered.get() == Some((*rect, owner)) {
                strip::paint_chip(
                    buffer,
                    *rect,
                    button.label,
                    style(button, true),
                    Align::Left,
                );
            }
        }
    }
    painted
}

#[cfg(test)]
mod tests;
