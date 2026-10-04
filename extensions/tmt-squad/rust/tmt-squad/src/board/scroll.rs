//! The one scroll model for every board pane that can overflow: a position
//! per pane, the wheel over the pane under the pointer, keyboard paging, and
//! an overflow indicator. Panes hand their lines to [`Scrolls::show`]; none
//! keeps a scroll position of its own.

use crate::config::Pane;
use ratatui::{Frame, layout::Rect, style::Style, text::Line, widgets::Paragraph};
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

    /// Keeps the selected record visible, or its first line when it is
    /// taller than the viewport.
    pub fn reveal_range(
        &self,
        pane: Pane,
        lines: std::ops::Range<usize>,
        area: Rect,
        content: usize,
    ) {
        let viewport = Self::viewport(area, content).max(1);
        let end = if lines.len() > viewport {
            lines.start.saturating_add(1)
        } else {
            lines.end
        };
        let offset = self.offsets.borrow().get(&pane).copied().unwrap_or(0);
        let offset = if lines.start < offset {
            lines.start
        } else if end > offset.saturating_add(viewport) {
            end.saturating_sub(viewport)
        } else {
            offset
        };
        self.offsets.borrow_mut().insert(pane, offset);
    }

    pub fn visible(&self, pane: Pane) -> bool {
        self.drawn
            .borrow()
            .get(&pane)
            .is_some_and(|drawn| !drawn.area.is_empty() && drawn.viewport > 0)
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
        self.show_with(frame, pane, area, lines.as_ref(), dim, |_, line| {
            line.clone()
        })
    }

    /// Decorates only visible lines; selection never clones the whole notebook.
    pub fn show_with<'a>(
        &self,
        frame: &mut Frame,
        pane: Pane,
        area: Rect,
        lines: &[Line<'a>],
        dim: Style,
        mut decorate: impl FnMut(usize, &Line<'a>) -> Line<'a>,
    ) -> (usize, usize) {
        self.show_paint(
            frame,
            pane,
            area,
            lines.len(),
            dim,
            |frame, body, offset| {
                frame.render_widget(
                    Paragraph::new(
                        lines[offset..(offset + usize::from(body.height)).min(lines.len())]
                            .iter()
                            .enumerate()
                            .map(|(index, line)| decorate(offset + index, line))
                            .collect::<Vec<_>>(),
                    ),
                    body,
                );
            },
        )
    }

    /// The same scroll and indicator owner for a pane that paints its own buffer
    /// cells: `paint` gets the viewport rectangle and the first content line shown.
    pub fn show_paint(
        &self,
        frame: &mut Frame,
        pane: Pane,
        area: Rect,
        content: usize,
        dim: Style,
        paint: impl FnOnce(&mut Frame, Rect, usize),
    ) -> (usize, usize) {
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
        paint(frame, body, offset);
        if viewport < usize::from(area.height) {
            let above = offset;
            let below = content.saturating_sub(offset + viewport);
            let mut parts = Vec::new();
            if above > 0 {
                parts.push(format!("↑ {above}"));
            }
            if below > 0 {
                parts.push(format!("{below} more ↓"));
            }
            let indicator = Rect {
                y: area.y + viewport as u16,
                height: 1,
                ..area
            };
            frame.render_widget(
                Paragraph::new(Line::styled(parts.join("  "), dim)).right_aligned(),
                indicator,
            );
        }
        (offset, viewport)
    }
}

#[cfg(test)]
mod tests;
