//! Pane presentation projects the App's existing focus, never a second focus store.
use super::{app::App, overview::cron_counts::GlyphSet};
use crate::config::Pane;
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

/// Panes shorter than this draw their top rule only.
const BOXED_MIN_HEIGHT: u16 = 6;

/// Panes narrower than this keep their content against the frame.
const INSET_MIN_WIDTH: u16 = 20;

/// Whether a pane of this size draws all four sides of its frame.
pub(super) fn boxed(area: Rect) -> bool {
    area.height >= BOXED_MIN_HEIGHT && area.width >= 3
}

/// The frame's glyphs: heavy lines for the pane that has focus, light lines for the others.
struct Lines {
    horizontal: &'static str,
    vertical: &'static str,
    corners: [&'static str; 4],
}

fn glyphs(focused: bool, ascii: bool) -> Lines {
    match (focused, ascii) {
        (true, false) => Lines {
            horizontal: "━",
            vertical: "┃",
            corners: ["┏", "┓", "┗", "┛"],
        },
        (false, false) => Lines {
            horizontal: "─",
            vertical: "│",
            corners: ["┌", "┐", "└", "┘"],
        },
        (true, true) => Lines {
            horizontal: "=",
            vertical: "#",
            corners: ["+"; 4],
        },
        (false, true) => Lines {
            horizontal: "-",
            vertical: "|",
            corners: ["+"; 4],
        },
    }
}

/// Draws the pane's frame on all four sides and returns the area inside it. The focused pane has a
/// heavy bright frame and a bold `▸` title; the others keep light dim lines and a secondary title.
pub(super) fn paint(frame: &mut Frame, app: &App, area: Rect, pane: Pane, title: &str) -> Rect {
    // Preserve navigation space on terminals that cannot afford a frame.
    if area.height < 3 || area.width < 3 {
        return area;
    }
    let look = app.look();
    let focused = app.focused() == pane;
    let ascii = matches!(app.cron_glyphs, GlyphSet::Ascii);
    let lines = glyphs(focused, ascii);
    let marker = match (focused, ascii) {
        (false, _) => "",
        (true, false) => "▸ ",
        (true, true) => "> ",
    };
    let hint = if focused && pane == Pane::Replies {
        " · Esc back"
    } else {
        ""
    };
    let title = tmt_cli_style::table::escape(&crate::board::notes::sanitize(title));
    let width = usize::from(area.width);
    let title = crate::board::view::fit(
        &title,
        title
            .width()
            .min(width.saturating_sub(5 + marker.width() + hint.width())),
    );
    let line = look.role(if focused { Role::Text } else { Role::Dim });
    let words = if focused {
        look.role(Role::Text).add_modifier(Modifier::BOLD)
    } else {
        look.role(Role::Muted)
    };
    let head = format!(" {marker}{title}{hint} ");
    let [top_left, top_right, bottom_left, bottom_right] = lines.corners;
    // A short pane keeps only its top rule; the corners and sides need room to be worth drawing.
    let boxed = boxed(area);
    let (open, close) = if boxed {
        (top_left, top_right)
    } else {
        (lines.horizontal, lines.horizontal)
    };
    let fill = width.saturating_sub(2 + 1 + head.width());
    strip::paint_left(
        frame.buffer_mut(),
        Rect { height: 1, ..area },
        Line::from(vec![
            Span::styled(format!("{open}{}", lines.horizontal), line),
            Span::styled(head, words),
            Span::styled(format!("{}{close}", lines.horizontal.repeat(fill)), line),
        ]),
    );
    if !boxed {
        return Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 1);
    }
    let bottom = area.bottom() - 1;
    for y in area.y + 1..bottom {
        for x in [area.x, area.right() - 1] {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(x, y, 1, 1),
                Line::styled(lines.vertical, line),
            );
        }
    }
    strip::paint_left(
        frame.buffer_mut(),
        Rect::new(area.x, bottom, area.width, 1),
        Line::styled(
            format!(
                "{bottom_left}{}{bottom_right}",
                lines.horizontal.repeat(width - 2)
            ),
            line,
        ),
    );
    content_area(area)
}

/// The exact content viewport used by frame paint and column admission.
pub(super) fn content_area(area: Rect) -> Rect {
    if area.height < 3 || area.width < 3 {
        return area;
    }
    if !boxed(area) {
        return Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 1);
    }
    // Content is inset by one cell from each side, unless the pane is too narrow to spare them.
    let inset = u16::from(area.width >= INSET_MIN_WIDTH);
    Rect::new(
        area.x + 1 + inset,
        area.y + 1,
        area.width - 2 - 2 * inset,
        area.height - 2,
    )
}

/// The Agents tab has no boxed panes: its two columns are told apart by their control rules. The
/// focused column's rule is heavy and bright and starts with a bold `▸`; the other keeps its light
/// dim line. Chips, search and titles keep their own styling.
pub(super) fn mark_focused_rule(
    buffer: &mut Buffer,
    app: &App,
    list: Option<Rect>,
    detail: Option<Rect>,
) {
    let rect = match (app.focused(), list, detail) {
        (Pane::Rows, Some(rect), _) | (Pane::Replies, _, Some(rect)) => rect,
        _ => return,
    };
    let look = app.look();
    let area = rect.intersection(buffer.area);
    if area.is_empty() || buffer[(area.x, area.y)].symbol() != "─" {
        return;
    }
    let marker = if matches!(app.cron_glyphs, GlyphSet::Ascii) {
        ">"
    } else {
        "▸"
    };
    for x in area.x..area.right() {
        let cell = &mut buffer[(x, area.y)];
        if x == area.x {
            cell.set_symbol(marker)
                .set_style(look.role(Role::Text).add_modifier(Modifier::BOLD));
        } else if cell.symbol() == "─" {
            cell.set_symbol("━").set_style(look.role(Role::Text));
        }
    }
}

#[cfg(test)]
mod tests;
