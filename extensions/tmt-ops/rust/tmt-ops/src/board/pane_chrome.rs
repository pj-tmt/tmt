//! Pane presentation projects the App's existing focus, never a second focus store.
use super::{app::App, overview::cron_counts::GlyphSet};
use crate::config::Pane;
use ratatui::{Frame, layout::Rect, style::Modifier, text::Line};
use tmt_cli_style::{Depth, Role};
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

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
    let hint = if focused && pane == Pane::Replies {
        " · focused · Esc back"
    } else if focused {
        " · focused"
    } else {
        ""
    };
    let title = tmt_cli_style::table::escape(&crate::board::notes::sanitize(title));
    let width = usize::from(area.width);
    let title = crate::board::view::fit(
        &title,
        title
            .width()
            .min(width.saturating_sub(4 + marker.width() + hint.width())),
    );
    let rule = if focused { "━" } else { "─" };
    let head = format!("{rule}{rule} {marker}{title}{hint} ");
    let text = format!("{head}{}", rule.repeat(width.saturating_sub(head.width())));
    let style = look.role(if focused { Role::Accent } else { Role::Muted });
    strip::paint_left(
        frame.buffer_mut(),
        Rect { height: 1, ..area },
        Line::styled(
            text,
            if focused {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            },
        ),
    );
    Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 1)
}

#[cfg(test)]
mod tests;
