//! DigestMatrix: one identity per row with whole, right-aligned metric values.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{components::strip, style::TextFlow};
use unicode_width::UnicodeWidthStr;

pub(in crate::board) const METRICS: usize = 6;
pub(in crate::board) const HEADERS: [&str; 7] =
    ["AGENT", "MODE", "HELD", "TOK 1h", "CTX 24h", "% IN", "≈ $"];
const METRIC_CELLS: [usize; METRICS] = [4, 5, 8, 8, 6, 8];
const GUTTERS_AND_GAPS: usize = 2 + METRICS;
pub(in crate::board) struct Row {
    pub name: String,
    pub values: [Option<String>; METRICS],
}
pub(in crate::board) fn agents(height: u16) -> usize {
    usize::from(height.saturating_sub(2)) // Header and the scoped all row come first.
}

/// Padding gives the approved 59-cell table its column endpoints. Under pressure,
/// shrink padding and identity text before any metric value or unit is lost.
fn widths(width: u16, rows: &[Row]) -> Option<(usize, [usize; METRICS])> {
    let minimum = std::array::from_fn::<_, METRICS, _>(|column| {
        rows.iter()
            .filter_map(|row| row.values[column].as_deref())
            .map(UnicodeWidthStr::width)
            .chain([HEADERS[column + 1].width(), "—".width()])
            .max()
            .unwrap()
    });
    let available = usize::from(width).checked_sub(GUTTERS_AND_GAPS + 1)?;
    if minimum.iter().sum::<usize>() > available {
        return None;
    }
    let mut metrics = std::array::from_fn(|column| METRIC_CELLS[column].max(minimum[column]));
    while metrics.iter().sum::<usize>() + HEADERS[0].width() > available {
        let Some(column) = (0..METRICS)
            .rev()
            .find(|&column| metrics[column] > minimum[column])
        else {
            break;
        };
        metrics[column] -= 1;
    }
    Some((
        usize::from(width) - GUTTERS_AND_GAPS - metrics.iter().sum::<usize>(),
        metrics,
    ))
}

/// Returns false when the full metric columns cannot fit, without clipping values.
pub(in crate::board) fn paint(buffer: &mut Buffer, area: Rect, values: &[Row], look: Look) -> bool {
    let Some((name_cells, metrics)) = widths(area.width, values) else {
        super::empty_state::paint(buffer, area, " Widen pane for Digest values.", None, look);
        return false;
    };
    let fit = |text: &str, width: usize, align| {
        tmt_tui::text::fit_line(
            &super::notes::single_line(text),
            width as u16,
            TextFlow::Truncate,
            align,
        )
    };
    for row in 0..usize::from(area.height).min(values.len() + 1) {
        let value = if row == 0 { None } else { values.get(row - 1) };
        let emphasis = if row == 1 {
            ratatui::style::Modifier::BOLD
        } else {
            ratatui::style::Modifier::empty()
        };
        let mut spans = vec![
            Span::raw(" "),
            Span::styled(
                fit(
                    value.map_or(HEADERS[0], |row| row.name.as_str()),
                    name_cells,
                    Align::Left,
                ),
                look.role(if row == 0 { Role::Muted } else { Role::Text })
                    .add_modifier(emphasis),
            ),
        ];
        for (column, cells) in metrics.into_iter().enumerate() {
            let text = if row == 0 {
                Some(HEADERS[column + 1])
            } else {
                value.and_then(|row| row.values[column].as_deref())
            };
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                fit(text.unwrap_or("—"), cells, Align::Right),
                look.role(if row == 0 {
                    Role::Muted
                } else if text.is_none() {
                    Role::Dim
                } else {
                    Role::Text
                })
                .add_modifier(emphasis),
            ));
        }
        strip::paint_left(
            buffer,
            Rect {
                y: area.y + row as u16,
                height: 1,
                ..area
            },
            Line::from(spans),
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lines(buffer: &Buffer) -> Vec<String> {
        buffer
            .content
            .chunks(usize::from(buffer.area.width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }
    #[test]
    fn transposed_mock_keeps_header_edges_numbers_and_bold_all_row() {
        let rows = vec![
            Row {
                name: "all".into(),
                values: ["Auto", "3", "243.6M", "12.2M", "11%", "$41.10"].map(|v| Some(v.into())),
            },
            Row {
                name: "ux-revamp".into(),
                values: ["Auto", "0", "31.2M", "4.1M", "13%", "$16.40"].map(|v| Some(v.into())),
            },
        ];
        for width in [59, 64] {
            let mut buffer = Buffer::empty(Rect::new(0, 0, width, 4));
            paint(
                &mut buffer,
                Rect::new(0, 0, width, 4),
                &rows,
                Look::default(),
            );
            let text = lines(&buffer);
            let extra = usize::from(width - 59);
            if width == 59 {
                assert_eq!(
                    text[0],
                    " AGENT        MODE  HELD   TOK 1h  CTX 24h   % IN      ≈ $ "
                );
                assert_eq!(
                    text[1],
                    " all          Auto     3   243.6M    12.2M    11%   $41.10 "
                );
            }
            assert_eq!(&text[0][..1], " ");
            assert!(text[0].starts_with(" AGENT"));
            for (end, value) in [
                (17, "Auto"),
                (23, "3"),
                (32, "243.6M"),
                (41, "12.2M"),
                (48, "11%"),
                (57, "$41.10"),
            ] {
                assert_eq!(
                    text[1]
                        .chars()
                        .skip(end + extra + 1 - value.len())
                        .take(value.len())
                        .collect::<String>(),
                    value
                );
            }
            assert!(
                buffer[(1, 1)]
                    .modifier
                    .contains(ratatui::style::Modifier::BOLD)
            );
            assert!(
                !buffer[(1, 2)]
                    .modifier
                    .contains(ratatui::style::Modifier::BOLD)
            );
            assert!(text[3].trim().is_empty());
        }
    }
    #[test]
    fn large_metric_values_shrink_identity_or_refuse_instead_of_truncating_numbers() {
        let rows = vec![Row {
            name: "an-identity-name-that-does-not-fit".into(),
            values: [
                "~mixed",
                "12345678901234567890",
                "~243.6M",
                "~12.2M",
                "~100%",
                "~$123.45M",
            ]
            .map(|v| Some(v.into())),
        }];
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 3));
        paint(&mut buffer, Rect::new(0, 0, 80, 3), &rows, Look::default());
        let text = lines(&buffer);
        for value in rows[0].values.iter().flatten() {
            assert!(text[1].contains(value), "{value}: {}", text[1]);
        }
        let mut narrow = Buffer::empty(Rect::new(0, 0, 36, 3));
        paint(&mut narrow, Rect::new(0, 0, 36, 3), &rows, Look::default());
        let text = lines(&narrow);
        assert!(text[0].contains("Widen pane for Digest values."));
        assert!(text[1].trim().is_empty() && text[2].trim().is_empty());
    }
    #[test]
    fn height_budget_and_missing_metrics_remain_distinct_from_zero() {
        assert_eq!(agents(7), 5);
        assert_eq!(agents(12), 10);
        assert_eq!(agents(1), 0);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 59, 7));
        let values = vec![Row {
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
        paint(
            &mut buffer,
            Rect::new(0, 0, 59, 7),
            &values,
            Look::default(),
        );
        assert_eq!(buffer[(17, 1)].symbol(), "0");
        assert_eq!(buffer[(23, 1)].symbol(), "—");
        assert_eq!(buffer[(32, 1)].symbol(), "M");
        assert_eq!(buffer[(1, 0)].symbol(), "A");
    }
}
