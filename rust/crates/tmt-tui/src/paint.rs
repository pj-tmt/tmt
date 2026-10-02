//! Buffer-only painting; the caller owns selection styling and input dispatch.
use crate::{
    geometry::{Cell, Rect},
    style::TextFlow,
    text,
};
use ratatui::{buffer::Buffer, style::Style};
use tmt_cli_style::{Depth, Role, Theme, theme::screen};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, PartialEq, Eq)]
pub struct Hit<'a> {
    pub rect: Rect,
    pub id: Option<&'a [String]>,
    pub row_id: Option<&'a str>,
}
/// Later paint wins: descendants precede ancestors when picking overlaps.
pub fn hit_at<'a>(hits: &'a [Hit<'_>], x: i32, y: i32) -> Option<&'a Hit<'a>> {
    hits.iter().rev().find(|hit| {
        x >= hit.rect.x
            && y >= hit.rect.y
            && i64::from(x) < i64::from(hit.rect.x) + i64::from(hit.rect.width)
            && i64::from(y) < i64::from(hit.rect.y) + i64::from(hit.rect.height)
    })
}
/// Consume geometry preorder. `selected_style` returns the complete selected
/// style for a Role; its owner supplies background/reverse and emphasis policy.
/// Unselected roles use the injected resolved Theme/Depth directly.
pub fn paint<'a>(
    cells: &[Cell<'a>],
    buffer: &mut Buffer,
    theme: &Theme,
    depth: Depth,
    mut selected_style: impl FnMut(Role) -> Style,
) -> Vec<Hit<'a>> {
    let area = buffer.area;
    let bounds = Rect {
        x: i32::from(area.x),
        y: i32::from(area.y),
        width: u32::from(area.width),
        height: u32::from(area.height),
    };
    let mut inherited = Vec::with_capacity(cells.len());
    let mut hits = Vec::new();
    for cell in cells {
        let (role, selected, id, row, cut) = cell
            .parent
            .map_or((Role::Text, false, None, None, false), |index| {
                inherited[index]
            });
        let role = cell.node.style.token.unwrap_or(role);
        let selected = selected || cell.node.selected;
        let id = cell.node.id.as_deref().or(id);
        let row = cell.node.row_id.as_deref().or(row);
        let cut = cut || cell.cut;
        inherited.push((role, selected, id, row, cut));
        let visible = cell.clip.intersect(bounds);
        if visible.width == 0 || visible.height == 0 {
            continue;
        }
        let style = if selected {
            selected_style(role)
        } else {
            screen::style(theme, role, depth)
        };
        for y in visible.y..visible.y + visible.height as i32 {
            for x in visible.x..visible.x + visible.width as i32 {
                let target = &mut buffer[(x as u16, y as u16)];
                target.reset();
                target.set_style(style);
            }
        }
        if id.is_some() || row.is_some() {
            hits.push(Hit {
                rect: visible,
                id,
                row_id: row,
            });
        }
        let Some(value) = cell.node.text.as_deref() else {
            continue;
        };
        let clip = cell.content.intersect(visible);
        let width = if cut {
            cell.text_width
                .min(clip.width.min(u32::from(u16::MAX)) as u16)
        } else {
            cell.text_width
        };
        for (line, logical) in text::lines(value, cell.text_width, cell.node.style.text_flow)
            .iter()
            .enumerate()
        {
            let y = i64::from(cell.content.y) + line as i64;
            if y < i64::from(clip.y) || y >= i64::from(clip.y) + i64::from(clip.height) {
                continue;
            }
            let visual = if cut {
                text::fit(
                    logical,
                    usize::from(width),
                    cell.node.style.text_flow == TextFlow::Middle,
                    true,
                )
            } else {
                logical.clone()
            };
            let mut x = i64::from(cell.content.x);
            for grapheme in visual.graphemes(true) {
                let size = grapheme.width() as i64;
                if size > 0
                    && x >= i64::from(clip.x)
                    && x + size <= i64::from(clip.x) + i64::from(clip.width)
                {
                    buffer[(x as u16, y as u16)].set_symbol(grapheme);
                }
                x += size;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests;
