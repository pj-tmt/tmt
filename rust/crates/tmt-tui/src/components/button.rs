//! Padded button chips. Painting returns rectangles, not effects: the application
//! keeps hit registration, pointer hover and activation authority.
use super::strip;
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use tmt_cli_style::{
    Depth, Theme,
    grid::Align,
    theme::screen::{self, ChipKind, ChipState},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Button<'a> {
    pub label: &'a str,
    focused: bool,
    selected: bool,
    enabled: bool,
    idle: Option<Style>,
}

impl<'a> Button<'a> {
    pub fn secondary(label: &'a str) -> Self {
        Self {
            label,
            focused: false,
            selected: false,
            enabled: true,
            idle: None,
        }
    }

    /// The button Enter chooses when the focus is not on the button row.
    pub fn primary(label: &'a str) -> Self {
        Self::secondary(label).focused(true)
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn idle(mut self, style: Style) -> Self {
        self.idle = Some(style);
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }
}

/// Where the buttons are painted and how: `active` is false while another layer
/// owns input, so focus and hover show only on the layer the user is in.
#[derive(Debug, Clone, Copy)]
pub struct ButtonRow<'t> {
    pub theme: &'t Theme,
    pub depth: Depth,
    pub align: Align,
    pub active: bool,
    /// The rectangle under the pointer, from the application's hover state.
    pub hovered: Option<Rect>,
}

impl ButtonRow<'_> {
    fn style(&self, button: &Button<'_>, hovered: bool) -> Style {
        let state = if !button.enabled {
            ChipState::Disabled
        } else if button.selected || self.active && (button.focused || hovered) {
            ChipState::Hot
        } else {
            ChipState::Idle
        };
        if state == ChipState::Idle
            && let Some(style) = button.idle
        {
            return style;
        }
        screen::chip(self.theme, ChipKind::Secondary, state, self.depth)
    }

    /// Fit a priority-ordered prefix of whole buttons; a clipped one is `None`.
    /// Hover never moves a button: the hovered one is repainted in place.
    pub fn paint(
        &self,
        buffer: &mut Buffer,
        area: Rect,
        buttons: &[Button<'_>],
    ) -> Vec<Option<Rect>> {
        let labels: Vec<_> = buttons
            .iter()
            .map(|button| (button.label, self.style(button, false)))
            .collect();
        let painted = strip::paint_chips(buffer, area, &labels, self.align);
        for (button, rect) in buttons.iter().zip(&painted) {
            if let Some(rect) = rect
                && self.active
                && button.enabled
                && self.hovered == Some(*rect)
            {
                strip::paint_chip(
                    buffer,
                    *rect,
                    button.label,
                    self.style(button, true),
                    Align::Left,
                );
            }
        }
        painted
    }
}
