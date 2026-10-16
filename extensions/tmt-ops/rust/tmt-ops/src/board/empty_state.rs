//! EmptyState: a shared status line; callers supply only currently usable keys.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    message: &str,
    action: Option<(&str, &str)>,
    look: Look,
) {
    let mut spans = vec![Span::styled(message.to_owned(), look.role(Role::Muted))];
    if let Some((key, explanation)) = action {
        spans.extend([
            Span::styled(format!(" · {key}"), look.role(Role::Text)),
            Span::styled(format!(" {explanation}"), look.role(Role::Dim)),
        ]);
    }
    strip::paint_left(
        buffer,
        Rect {
            height: area.height.min(1),
            ..area
        },
        Line::from(spans),
    );
}
