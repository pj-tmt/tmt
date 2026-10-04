//! Overlay box geometry and opaque chrome from Full-screen interaction/Overlays.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Block, Borders, Clear, Widget},
};
use tmt_cli_style::{Depth, Role, Theme, table::escape, theme::screen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Body,
    Center,
    Docked,
}

#[derive(Debug, Clone)]
pub struct Modal {
    pub title: String,
    pub placement: Placement,
}

#[derive(Debug, Clone, Copy)]
pub struct ModalAreas {
    pub outer: Rect,
    pub content: Rect,
    pub footer: Rect,
    pub status: Rect,
    pub position: Rect,
}

impl Modal {
    /// Body is supplied by the application, excluding its base header/footer.
    /// Footer/status space is reserved before content gets a viewport.
    pub fn areas(&self, body: Rect, demand: [u16; 2], footer: bool, status: bool) -> ModalAreas {
        self.areas_with_lines(body, demand, u16::from(footer), u16::from(status))
    }

    /// Fixed text is measured by the surface before reserving its visible lines.
    pub(crate) fn areas_with_lines(
        &self,
        body: Rect,
        demand: [u16; 2],
        footer_lines: u16,
        status_lines: u16,
    ) -> ModalAreas {
        let outer = if self.placement == Placement::Body {
            body
        } else {
            let width = if body.width < 100 {
                body.width
            } else {
                demand[0].min((u32::from(body.width) * 9 / 10) as u16)
            };
            let height = demand[1].min((u32::from(body.height) * 4 / 5) as u16);
            Rect::new(
                body.x + body.width.saturating_sub(width) / 2,
                if self.placement == Placement::Docked {
                    body.y + body.height.saturating_sub(height)
                } else {
                    body.y + body.height.saturating_sub(height) / 2
                },
                width,
                height,
            )
        };
        let inner = outer.inner(ratatui::layout::Margin::new(1, 1));
        let inset = Rect::new(
            inner.x.saturating_add(u16::from(inner.width > 0)),
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        );
        let footer_height = footer_lines.min(inset.height);
        let status_height = status_lines.min(inset.height.saturating_sub(footer_height));
        let position_height = u16::from(inset.height > footer_height + status_height);
        let content = Rect {
            height: inset
                .height
                .saturating_sub(footer_height + status_height + position_height),
            ..inset
        };
        let position = Rect {
            y: inset.y + content.height,
            height: position_height,
            ..inset
        };
        let status = Rect {
            y: position.y + position.height,
            height: status_height,
            ..inset
        };
        let footer = Rect {
            y: status.y + status.height,
            height: footer_height,
            ..inset
        };
        ModalAreas {
            outer,
            content,
            footer,
            status,
            position,
        }
    }

    pub fn paint(&self, areas: ModalAreas, buffer: &mut Buffer, theme: &Theme, depth: Depth) {
        let area = areas.outer.intersection(buffer.area);
        Clear.render(area, buffer);
        // Establish an opaque text style even on blank cells and with NO_COLOR.
        buffer.set_style(area, screen::style(theme, Role::Text, depth));
        let mut border = Block::new()
            .borders(Borders::ALL)
            .border_style(screen::style(theme, Role::Dim, depth));
        if !self.title.is_empty() {
            border = border.title(Line::styled(
                format!(" {} ", escape(&self.title)),
                screen::style(theme, Role::Muted, depth),
            ));
        }
        border.render(area, buffer);
    }
}
