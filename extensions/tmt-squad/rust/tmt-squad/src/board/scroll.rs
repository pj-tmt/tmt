//! The one scroll model for every board pane that can overflow: a position
//! per pane, the wheel over the pane under the pointer, keyboard paging, and
//! an overflow indicator. Panes hand their lines to [`Scrolls::show`]; none
//! keeps a scroll position of its own.

use crate::config::Pane;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::Paragraph,
};
use std::{cell::RefCell, collections::BTreeMap};

/// Lines one wheel notch moves.
pub const WHEEL_LINES: usize = 3;

/// Where a pane was last drawn and how much it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Drawn {
    area: Rect,
    /// Lines the pane shows at once: its height, less the indicator line
    /// when it overflows.
    viewport: usize,
    content: usize,
}

/// How far a scroll moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Lines(isize),
    /// One screen, less a line of context.
    Pages(isize),
    Top,
    Bottom,
}

#[derive(Default)]
pub struct Scrolls {
    offsets: RefCell<BTreeMap<Pane, usize>>,
    drawn: RefCell<BTreeMap<Pane, Drawn>>,
}

impl Scrolls {
    /// Forgets where panes were drawn; a render records them again.
    pub fn begin_frame(&self) {
        self.drawn.borrow_mut().clear();
    }

    /// The first line `pane` shows, kept within its content.
    pub fn offset(&self, pane: Pane) -> usize {
        let offset = self.offsets.borrow().get(&pane).copied().unwrap_or(0);
        match self.drawn.borrow().get(&pane) {
            Some(drawn) => offset.min(drawn.content.saturating_sub(drawn.viewport)),
            None => offset,
        }
    }

    /// Lines `pane` shows at once in `area` for `content` lines.
    pub fn viewport(area: Rect, content: usize) -> usize {
        let height = usize::from(area.height);
        if content > height && height > 1 {
            height - 1
        } else {
            height
        }
    }

    /// Moves `pane`. Before it was ever drawn, only its position is kept.
    pub fn scroll(&self, pane: Pane, step: Step) {
        let drawn = self.drawn.borrow().get(&pane).copied();
        let limit = drawn.map_or(usize::MAX, |drawn| {
            drawn.content.saturating_sub(drawn.viewport)
        });
        let current = self.offset(pane);
        let next = match step {
            Step::Lines(lines) => current.saturating_add_signed(lines),
            Step::Pages(pages) => {
                current.saturating_add_signed(pages * self.page_lines(pane) as isize)
            }
            Step::Top => 0,
            Step::Bottom => limit,
        };
        self.offsets.borrow_mut().insert(pane, next.min(limit));
    }

    /// Lines one page moves `pane`: its last viewport less a line of context.
    pub fn page_lines(&self, pane: Pane) -> usize {
        self.drawn
            .borrow()
            .get(&pane)
            .map_or(1, |drawn| drawn.viewport.saturating_sub(1).max(1))
    }

    /// Scrolls `pane` just enough that `line` is on screen.
    pub fn reveal(&self, pane: Pane, line: usize, area: Rect, content: usize) {
        let viewport = Self::viewport(area, content).max(1);
        let offset = self.offsets.borrow().get(&pane).copied().unwrap_or(0);
        let offset = if line < offset {
            line
        } else if line >= offset + viewport {
            line + 1 - viewport
        } else {
            offset
        };
        self.offsets.borrow_mut().insert(pane, offset);
    }

    /// The pane drawn under a screen cell, for the wheel.
    pub fn pane_at(&self, column: u16, row: u16) -> Option<Pane> {
        self.drawn
            .borrow()
            .iter()
            .find(|(_, drawn)| {
                (drawn.area.x..drawn.area.x + drawn.area.width).contains(&column)
                    && (drawn.area.y..drawn.area.y + drawn.area.height).contains(&row)
            })
            .map(|(pane, _)| *pane)
    }

    /// Draws `lines` in `area` from the pane's position, with `↑ n  ↓ m`
    /// on the last line when there is more above or below. Returns the first
    /// line shown and how many are shown.
    pub fn show<'a>(
        &self,
        frame: &mut Frame,
        pane: Pane,
        area: Rect,
        lines: impl AsRef<[Line<'a>]>,
        dim: Style,
    ) -> (usize, usize) {
        let lines = lines.as_ref();
        let content = lines.len();
        let viewport = Self::viewport(area, content);
        self.drawn.borrow_mut().insert(
            pane,
            Drawn {
                area,
                viewport,
                content,
            },
        );
        let offset = self.offset(pane);
        self.offsets.borrow_mut().insert(pane, offset);
        let body = Rect {
            height: viewport as u16,
            ..area
        };
        frame.render_widget(
            Paragraph::new(lines[offset..(offset + viewport).min(content)].to_vec()),
            body,
        );
        if viewport < usize::from(area.height) {
            let above = offset;
            let below = content.saturating_sub(offset + viewport);
            let mut parts = Vec::new();
            if above > 0 {
                parts.push(format!("↑ {above}"));
            }
            if below > 0 {
                parts.push(format!("↓ {below}"));
            }
            let indicator = Rect {
                y: area.y + viewport as u16,
                height: 1,
                ..area
            };
            frame.render_widget(
                Paragraph::new(Line::styled(
                    parts.join("  "),
                    dim.add_modifier(Modifier::DIM),
                ))
                .right_aligned(),
                indicator,
            );
        }
        (offset, viewport)
    }
}

#[cfg(test)]
mod tests;
