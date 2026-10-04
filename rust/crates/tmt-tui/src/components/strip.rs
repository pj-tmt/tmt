//! One-line rich text: spans painted through the crate's text fitting, so width
//! fitting and terminal escaping stay in this crate. Each span keeps the
//! caller's resolved style; the strip owns no role or selection policy.
//!
//! A strip is one text cell per span, so it needs no layout: each span is clipped
//! to the room left in the area, filled with its style, and fitted and escaped by
//! `text::fit_line`, exactly as the geometry and paint pipeline would place it.
use crate::{style::TextFlow, text};
use ratatui::{buffer::Buffer, layout::Rect, style::Style, text::Line};
use tmt_cli_style::{Depth, Theme, grid::Align};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Paint `line` from the left edge of `area`; text beyond its width is clipped.
/// Spans carry resolved styles, so `theme` and `depth` are not consulted; they
/// stay in the signature so every caller keeps one painting contract.
pub fn paint_left(buffer: &mut Buffer, area: Rect, line: Line<'_>, theme: &Theme, depth: Depth) {
    paint_from(buffer, area, area.x, line, theme, depth);
}

/// Paint `line` flush with the right edge of `area`; a line wider than the
/// area keeps its start, as `paint_left` does.
pub fn paint_right(buffer: &mut Buffer, area: Rect, line: Line<'_>, theme: &Theme, depth: Depth) {
    let width = line.width().min(usize::from(area.width)) as u16;
    paint_from(buffer, area, area.right() - width, line, theme, depth);
}

fn paint_from(
    buffer: &mut Buffer,
    area: Rect,
    start: u16,
    line: Line<'_>,
    _theme: &Theme,
    _depth: Depth,
) {
    let mut x = start.max(area.x);
    for span in line.spans {
        let width = span
            .width()
            .min(usize::from(area.right().saturating_sub(x))) as u16;
        if width == 0 {
            continue;
        }
        paint_span(
            buffer,
            Rect::new(x, area.y, width, area.height),
            &span.content,
            line.style.patch(span.style),
        );
        x += width;
    }
}

/// Fill `slot` with `style`, then place the escaped text on its first row. Cells
/// outside the buffer are skipped; a wide grapheme that would cross the slot's
/// right edge is dropped and leaves styled blanks.
fn paint_span(buffer: &mut Buffer, slot: Rect, text: &str, style: Style) {
    let visible = slot.intersection(buffer.area);
    if visible.is_empty() {
        return;
    }
    for y in visible.top()..visible.bottom() {
        for x in visible.left()..visible.right() {
            let cell = &mut buffer[(x, y)];
            cell.reset();
            cell.set_style(style);
        }
    }
    // The text sits on the slot's first row, which may lie above the buffer.
    if slot.y < visible.y {
        return;
    }
    let fitted = text::fit_line(text, slot.width, TextFlow::Clip, Align::Left);
    let mut x = slot.x;
    for grapheme in fitted.graphemes(true) {
        let size = grapheme.width() as u16;
        if size > 0 && x >= visible.left() && x + size <= visible.right() {
            buffer[(x, slot.y)].set_symbol(grapheme);
        }
        x += size;
    }
}

#[cfg(test)]
mod reference;
#[cfg(test)]
mod tests;
