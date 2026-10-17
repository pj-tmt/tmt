//! Compact member statistics; callers retain data and action admission.
use crate::look::Look;
use ratatui::{buffer::Buffer, layout::Rect, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

pub(in crate::board) const ROWS: u16 = 3;
pub(in crate::board) struct Tile<'a> {
    pub label: &'a str,
    pub value: Line<'static>,
    pub detail: Line<'static>,
}

/// One track owner for whole-label fitting, paint and clipped action hits.
pub(in crate::board) fn tracks(area: Rect, count: usize) -> Vec<Rect> {
    if area.is_empty() || count == 0 {
        return vec![];
    }
    let count = count.min(4).min(usize::from(area.width.div_ceil(3)));
    let usable = area.width.saturating_sub(count.saturating_sub(1) as u16);
    let width = usable / count as u16;
    let mut x = area.x;
    (0..count)
        .map(|index| {
            let width = if index + 1 == count {
                area.right().saturating_sub(x)
            } else {
                width
            };
            let rect = Rect::new(x, area.y, width, area.height.min(ROWS));
            if index + 1 < count {
                x = rect.right().saturating_add(1);
            }
            rect
        })
        .collect()
}

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    tiles: &[Tile<'_>],
    look: Look,
) -> Vec<Rect> {
    let drawn = tracks(area, tiles.len());
    for (index, (tile, rect)) in tiles.iter().zip(&drawn).enumerate() {
        let text = Rect::new(
            rect.x + rect.width.min(1),
            rect.y,
            rect.width.saturating_sub(2),
            rect.height,
        );
        let lines = [
            Line::styled(tile.label.to_owned(), look.role(Role::Muted)),
            tile.value.clone(),
            tile.detail.clone(),
        ];
        for (row, mut line) in lines.into_iter().take(usize::from(rect.height)).enumerate() {
            if line.width() > usize::from(text.width) {
                let prefix = line
                    .spans
                    .iter()
                    .take(line.spans.len().saturating_sub(1))
                    .map(ratatui::text::Span::width)
                    .sum::<usize>();
                if let Some(last) = line.spans.last_mut() {
                    last.content = crate::board::view::fit(
                        &last.content,
                        usize::from(text.width).saturating_sub(prefix),
                    )
                    .into();
                }
            }
            strip::paint_left(
                buffer,
                Rect::new(text.x, text.y + row as u16, text.width, 1),
                line,
            );
        }
        if index + 1 < drawn.len() {
            for y in rect.y..rect.bottom() {
                strip::paint_left(
                    buffer,
                    Rect::new(rect.right(), y, 1, 1),
                    Line::styled("│", look.role(Role::Dim)),
                );
            }
        }
    }
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_four_stats_share_the_mock_tracks_and_dividers_keep_the_full_height() {
        let look = Look::default();
        let tiles = ["TOK 1h", "STATE", "DIGEST", "TIMERS"].map(|label| Tile {
            label,
            value: Line::styled("0", look.role(Role::Text)),
            detail: Line::styled("—", look.role(Role::Dim)),
        });
        let area = Rect::new(71, 9, 77, 3);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 148, 51));
        let tracks = paint(&mut buffer, area, &tiles, look);
        assert_eq!(
            tracks.iter().map(|r| r.x + 1).collect::<Vec<_>>(),
            [72, 91, 110, 129]
        );
        for x in [89, 108, 127] {
            for y in 9..12 {
                assert_eq!(buffer[(x, y)].symbol(), "│");
            }
        }
        for width in 0..40 {
            let area = Rect::new(1, 1, width, 2);
            let mut clipped = Buffer::empty(Rect::new(0, 0, 42, 4));
            for rect in paint(&mut clipped, area, &tiles, look) {
                assert_eq!(rect.intersection(area), rect);
            }
            assert_eq!(clipped[(0, 0)].symbol(), " ");
        }
    }
}
