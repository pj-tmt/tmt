//! Final wide right column: one existing response viewport, then inert observations.
use crate::{
    board::{
        app::App, conversation, digest_matrix, m3_section_rule, m14_divider, overview::matrix,
    },
    config::Pane,
};
use ratatui::{Frame, layout::Rect, text::Line};

pub(in crate::board) fn render(frame: &mut Frame, app: &App, left: Option<Rect>, area: Rect) {
    if app.input.is_some()
        && app.composer_projection().is_none()
        && app.conversation_input_area().is_some()
    {
        conversation::render(frame, app, area);
        return;
    }
    let look = app.look();
    if let Some(left) = left.filter(|left| left.right() < area.x) {
        m14_divider::paint(
            frame.buffer_mut(),
            Rect::new(left.right(), area.y, 1, area.height),
            true,
            look,
        );
    }
    let title = app
        .conversation_target()
        .and_then(|target| app.target_row(&target))
        .and_then(|row| row["name"].as_str())
        .unwrap_or("Needs you");
    m3_section_rule::paint(
        frame.buffer_mut(),
        Rect {
            height: area.height.min(1),
            ..area
        },
        title,
        Line::default(),
        app.focused() == Pane::Replies,
        look,
    );
    let remaining = area.height.saturating_sub(1);
    let input = remaining.min(crate::board::input_field::ROWS);
    let gap = u16::from(remaining > input + 2);
    let exchange = remaining
        .saturating_sub(input + gap)
        .min(conversation::exchange::desired_height(app, area.width));
    let conversation_area = Rect::new(area.x, area.y + 1, area.width, exchange);
    conversation::exchange::render(frame, app, conversation_area);
    let input_area = Rect::new(area.x, conversation_area.bottom() + gap, area.width, input);
    if app.composer_projection().is_some_and(|projection| {
        projection.key.is_some() && (projection.focused || app.composer_start().is_some())
    }) {
        app.input_band.set(Some(input_area));
    }
    let mut y = input_area.bottom().saturating_add(1);
    let timers = super::lower::Band::timers(app);
    if area.bottom().saturating_sub(y) >= timers.height as u16 {
        for row in 0..timers.height {
            timers.paint(
                frame,
                app,
                Rect::new(area.x, y + row as u16, area.width, 1),
                row,
            );
        }
        y += timers.height as u16 + 1;
    }
    if app
        .overview_panels()
        .shown(crate::config::OverviewPanel::Digest)
        && area.bottom().saturating_sub(y) >= 8
    {
        let matrix = matrix::Matrix::project(app, area.width);
        m3_section_rule::paint(
            frame.buffer_mut(),
            Rect::new(area.x, y, area.width, 1),
            "Digest",
            Line::styled(matrix.note, look.role(tmt_cli_style::Role::Muted)),
            app.focused() == Pane::Replies,
            look,
        );
        for row in 0..7 {
            digest_matrix::paint(
                frame.buffer_mut(),
                Rect::new(area.x, y + 1 + row as u16, area.width, 1),
                row,
                &matrix.labels,
                &matrix.columns,
                look,
            );
        }
    }
}

#[cfg(test)]
mod tests;
