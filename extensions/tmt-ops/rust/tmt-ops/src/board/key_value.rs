//! KeyValue: neutral labels and right-aligned observed values.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::{components::strip, style::TextFlow};
use unicode_width::UnicodeWidthStr;

pub(in crate::board) enum Value<'a> {
    Observed(&'a str),
    Unreported(&'a str),
}
impl Value<'_> {
    fn text(&self) -> &str {
        match self {
            Self::Observed(value) | Self::Unreported(value) => value,
        }
    }
}
pub(in crate::board) struct Row<'a> {
    pub key: &'a str,
    pub value: Value<'a>,
}

pub(in crate::board) fn value_area(area: Rect, value: &str) -> Rect {
    let width = value_width(usize::from(area.width), value) as u16;
    Rect::new(
        area.right().saturating_sub(1 + width),
        area.y,
        width,
        area.height.min(1),
    )
}

fn value_width(width: usize, value: &str) -> usize {
    crate::board::notes::sanitize(value)
        .width()
        .min(width.saturating_sub(4))
}

pub(in crate::board) fn line(row: &Row<'_>, width: usize, look: Look) -> Line<'static> {
    if width < 3 {
        return Line::raw(" ".repeat(width));
    }
    let key = crate::board::notes::sanitize(row.key);
    let value = crate::board::notes::sanitize(row.value.text());
    let inner = width - 2;
    let value_width = value_width(width, &value);
    let key_width = inner.saturating_sub(value_width + 1);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            tmt_tui::text::fit_line(
                &key,
                key_width as u16,
                TextFlow::Truncate,
                tmt_cli_style::grid::Align::Left,
            ),
            look.role(Role::Muted),
        ),
        Span::raw(" "),
        Span::styled(
            tmt_tui::text::fit_line(
                &value,
                value_width as u16,
                TextFlow::Truncate,
                tmt_cli_style::grid::Align::Right,
            ),
            look.role(if matches!(row.value, Value::Observed(_)) {
                Role::Text
            } else {
                Role::Dim
            }),
        ),
        Span::raw(" "),
    ])
}

pub(in crate::board) fn paint(buffer: &mut Buffer, area: Rect, row: &Row<'_>, look: Look) {
    strip::paint_left(
        buffer,
        Rect {
            height: area.height.min(1),
            ..area
        },
        line(row, usize::from(area.width), look),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_and_values_keep_their_edges_and_missing_is_not_zero() {
        let look = Look::default();
        for width in [0, 1, 2, 12, 32, 40, 62] {
            let known = line(
                &Row {
                    key: "held",
                    value: Value::Observed("0"),
                },
                width,
                look,
            );
            let missing = line(
                &Row {
                    key: "held",
                    value: Value::Unreported("—"),
                },
                width,
                look,
            );
            assert_eq!(known.width(), width);
            assert_eq!(missing.width(), width);
            if width >= 12 {
                assert!(known.to_string().starts_with(" held"));
                assert!(known.to_string().ends_with("0 "));
                assert!(missing.to_string().ends_with("— "));
                assert_eq!(known.spans[3].style, look.role(Role::Text));
                assert_eq!(missing.spans[3].style, look.role(Role::Dim));
            }
        }
        let escaped = line(
            &Row {
                key: "price\nunsafe",
                value: Value::Observed("界\t42"),
            },
            32,
            look,
        )
        .to_string();
        assert!(!escaped.chars().any(char::is_control));
    }
}
