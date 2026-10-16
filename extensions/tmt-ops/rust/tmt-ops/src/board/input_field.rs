//! M11 InputField: a three-row neutral filled block; the host retains the editor.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) const ROWS: u16 = 3;
pub(in crate::board) fn style(look: Look, focused: bool) -> Style {
    let fill = look.role(if focused {
        Role::Selection
    } else {
        Role::Track
    });
    if let Some(bg) = fill.bg {
        look.role(Role::Text).bg(bg)
    } else {
        look.role(Role::Text).add_modifier(Modifier::REVERSED)
    }
}
pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    recipient: &str,
    start: &str,
    focused: bool,
    replying: bool,
    look: Look,
) -> Rect {
    let base = style(look, focused);
    for y in area.y..area.bottom() {
        strip::paint_left(
            buffer,
            Rect::new(area.x, y, area.width, 1),
            Line::styled(" ".repeat(usize::from(area.width)), base),
        );
    }
    if area.is_empty() {
        return area;
    }
    let y = area.y + u16::from(area.height > 1);
    let inner = Rect::new(area.x, y, area.width, 1);
    if !focused {
        let words = format!(
            " to {} {}",
            if replying { "reply to" } else { "message" },
            super::notes::single_line(recipient)
        );
        strip::paint_left(
            buffer,
            inner,
            Line::from(vec![
                Span::styled(start.to_owned(), base),
                Span::styled(words, base.patch(look.role(Role::Muted))),
            ]),
        );
        return Rect::new(inner.x, inner.y, inner.width, 1);
    }
    let hint = "⏎ send · Esc cancel";
    let header = Rect::new(area.x, area.y, area.width, 1);
    let hint_width = if hint.width() + 5 <= usize::from(area.width) {
        hint.width() as u16
    } else {
        0
    };
    let left = Rect::new(
        header.x,
        header.y,
        header
            .width
            .saturating_sub(hint_width + u16::from(hint_width > 0)),
        1,
    );
    strip::paint_left(
        buffer,
        left,
        Line::styled(
            format!("› {} as you…", super::notes::single_line(recipient)),
            base.patch(look.role(Role::Muted)),
        ),
    );
    if hint_width > 0 {
        strip::paint_right(
            buffer,
            header,
            Line::styled(hint, base.patch(look.role(Role::Muted))),
        );
    }
    Rect::new(
        area.x + area.width.min(1),
        y,
        area.width.saturating_sub(2),
        1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_and_active_fields_have_no_rule_or_frame_and_keep_right_hint_only_active() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 59, 3));
        let area = buffer.area;
        paint(
            &mut buffer,
            area,
            "night-2",
            "⏎",
            false,
            true,
            Look::default(),
        );
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("⏎ to reply to night-2"));
        assert!(!text.contains("send"));
        assert!(!text.contains('─'));
        let editor = paint(
            &mut buffer,
            area,
            "night-2",
            "⏎",
            true,
            true,
            Look::default(),
        );
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("⏎ send · Esc cancel"));
        assert!(editor.width > 0);
        assert!(editor.right() < 59);
    }
}
