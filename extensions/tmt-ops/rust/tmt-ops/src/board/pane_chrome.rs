//! Pane presentation projects the App's existing focus, never a second focus store.
use super::{app::App, overview::cron_counts::GlyphSet};
use crate::config::Pane;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use tmt_cli_style::{Depth, Role};
use tmt_tui::components::Outline;

pub(super) fn paint(frame: &mut Frame, app: &App, area: Rect, pane: Pane, title: &str) -> Rect {
    // Preserve navigation space on terminals that cannot afford a frame.
    if area.height < 3 || area.width < 3 {
        return area;
    }
    let look = app.look();
    let focused = app.focused() == pane;
    let marker = if focused && look.depth == Depth::None {
        if matches!(app.cron_glyphs, GlyphSet::Ascii) {
            "> "
        } else {
            "▶ "
        }
    } else {
        ""
    };
    let title = format!(
        " {marker}{title}{} ",
        if focused && pane == Pane::Replies {
            " · focused · Esc back"
        } else if focused {
            " · focused"
        } else {
            ""
        }
    );
    let style = look.role(if focused { Role::Accent } else { Role::Muted });
    let outline = Outline {
        title: Line::from(title),
        border: style,
        title_style: if focused {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        },
    };
    if focused {
        outline.paint_flat_heavy_top(area, frame.buffer_mut());
    } else {
        outline.paint_flat(area, frame.buffer_mut());
    }
    outline.inner(area)
}

/// A retained selection is identifiable without impersonating the receiving cursor.
pub(super) fn row_style(app: &App, selected: bool, style: Style) -> Style {
    let look = app.look();
    if !selected {
        style
    } else if app.focused() == Pane::Rows {
        look.selection().patch(look.row_span(true, style, true))
    } else {
        look.role(Role::Muted).add_modifier(Modifier::DIM)
    }
}

#[cfg(test)]
mod tests;
