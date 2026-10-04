//! A square single-line border with a styled title in its top edge. It keeps
//! what is already drawn inside it: unlike `Modal`, it does not clear or fill,
//! so it frames panes of a base layer rather than an overlay.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};
use tmt_cli_style::table::escape;

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
        let mut block = Block::new()
            .borders(Borders::ALL)
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
