//! A single-line frame with a styled title in its top edge. It keeps
//! what is already drawn inside it: unlike `Modal`, it does not clear or fill,
//! so it frames panes of a base layer rather than an overlay.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};
use tmt_cli_style::table::escape;

/// Every frame style reserves each edge; flat framing changes paint alone.
#[derive(Clone, Copy)]
pub(super) enum Frame {
    Square,
    Flat,
    FlatHeavyTop,
}

pub(super) fn frame_block(frame: Frame) -> Block<'static> {
    let block = Block::new().borders(Borders::ALL);
    let top = if matches!(frame, Frame::FlatHeavyTop) {
        "━"
    } else {
        "─"
    };
    match frame {
        Frame::Square => block,
        Frame::Flat | Frame::FlatHeavyTop => block.border_set(border::Set {
            top_left: top,
            top_right: top,
            horizontal_top: top,
            bottom_left: "─",
            bottom_right: "─",
            vertical_left: " ",
            vertical_right: " ",
            ..border::PLAIN
        }),
    }
}

#[derive(Debug, Clone)]
pub struct Outline<'a> {
    /// Spans keep their own style over `title_style`; an empty title draws none.
    pub title: Line<'a>,
    pub border: Style,
    pub title_style: Style,
}

impl Outline<'_> {
    /// The area inside the border.
    pub fn inner(&self, area: Rect) -> Rect {
        Block::bordered().inner(area)
    }

    pub fn paint(&self, area: Rect, buffer: &mut Buffer) {
        self.paint_frame(area, buffer, Frame::Square);
    }

    /// Horizontal rules and blank side slots, with the same inner and title areas.
    pub fn paint_flat(&self, area: Rect, buffer: &mut Buffer) {
        self.paint_frame(area, buffer, Frame::Flat);
    }

    /// Flat framing with a heavy top rule; inner geometry and other edges stay unchanged.
    pub fn paint_flat_heavy_top(&self, area: Rect, buffer: &mut Buffer) {
        self.paint_frame(area, buffer, Frame::FlatHeavyTop);
    }

    fn paint_frame(&self, area: Rect, buffer: &mut Buffer, frame: Frame) {
        let mut block = frame_block(frame)
            .border_style(self.border)
            .title_style(self.title_style);
        if !self.title.spans.is_empty() {
            // Titles carry user data (names): shown escaped, never interpreted.
            block = block.title(Line::from(
                self.title
                    .spans
                    .iter()
                    .map(|span| Span::styled(escape(&span.content), span.style))
                    .collect::<Vec<_>>(),
            ));
        }
        block.render(area.intersection(buffer.area), buffer);
    }
}

#[cfg(test)]
mod tests;
