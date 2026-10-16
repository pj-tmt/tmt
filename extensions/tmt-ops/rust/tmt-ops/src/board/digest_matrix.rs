//! DigestMatrix: one neutral metric-by-identity grid, with fixed ten-cell columns.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{components::strip, style::TextFlow};

const LABEL_CELLS: usize = 17;
const END_CELLS: usize = 2;
pub(in crate::board) const COLUMN_CELLS: usize = 10;
pub(in crate::board) const METRICS: usize = 6;
pub(in crate::board) struct Column {
    pub name: String,
    pub values: [Option<String>; METRICS],
}
pub(in crate::board) fn columns(width: u16) -> usize {
    usize::from(width).saturating_sub(LABEL_CELLS + END_CELLS) / COLUMN_CELLS
}
pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    row: usize,
    labels: &[String; METRICS],
    values: &[Column],
    look: Look,
) {
    let key = if row == 0 {
        ""
    } else {
        match labels.get(row - 1) {
            Some(label) => label,
            None => return,
        }
    };
    let fit = |text: &str, width: usize, align| {
        tmt_tui::text::fit_line(
            &super::notes::single_line(text),
            width as u16,
            TextFlow::Truncate,
            align,
        )
    };
    let key_width = LABEL_CELLS.min(usize::from(area.width));
    let mut spans = vec![Span::styled(
        fit(&format!(" {key}"), key_width, Align::Left),
        look.role(Role::Muted),
    )];
    for column in values.iter().take(columns(area.width)) {
        let value = if row == 0 {
            Some(column.name.as_str())
        } else {
            column.values[row - 1].as_deref()
        };
        spans.push(Span::styled(
            fit(value.unwrap_or("—"), COLUMN_CELLS - 1, Align::Right),
            look.role(if row == 0 {
                Role::Muted
            } else if value.is_none() {
                Role::Dim
            } else {
                Role::Text
            }),
        ));
        spans.push(Span::raw(" "));
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_width_fits_all_and_three_agents_and_missing_is_not_zero() {
        assert_eq!(columns(59), 4);
        assert_eq!(columns(69), 5);
        assert_eq!(columns(28), 0);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 59, 7));
        let values = vec![Column {
            name: "all".into(),
            values: [
                Some("0".into()),
                None,
                Some("243.6M".into()),
                None,
                None,
                None,
            ],
        }];
        let labels = [
            "mode",
            "held",
            "TOK 1h",
            "ctx avoided 24h",
            "% of input",
            "≈ $ avoided",
        ]
        .map(str::to_owned);
        for row in 0..7 {
            paint(
                &mut buffer,
                Rect::new(0, row as u16, 59, 1),
                row,
                &labels,
                &values,
                Look::default(),
            );
        }
        assert_eq!(buffer[(25, 1)].symbol(), "0");
        assert_eq!(buffer[(25, 2)].symbol(), "—");
        assert_eq!(buffer[(25, 0)].symbol(), "l");
    }
}
