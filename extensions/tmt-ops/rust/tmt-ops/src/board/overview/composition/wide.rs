//! Final wide right column: one existing response viewport, then inert observations.
use crate::{
    board::{
        app::App, conversation, digest_matrix, divider, overview::matrix, section_rule, state_cell,
        workspace::OpsTarget,
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
        divider::paint(
            frame.buffer_mut(),
            Rect::new(left.right(), area.y, 1, area.height),
            true,
            look,
        );
    }
    let row = app
        .conversation_target()
        .and_then(|target| app.target_row(&target));
    let squad = app.workspace.selected.as_ref().and_then(|target| {
        let OpsTarget::Squad(room) = target else {
            return None;
        };
        app.workspace_snapshot()?
            .roster
            .squads
            .iter()
            .find(|squad| &squad.room_id == room)
    });
    let title = squad
        .map(|squad| squad.name.as_str())
        .or_else(|| row.and_then(|row| row["name"].as_str()))
        .unwrap_or("Conversation");
    let mut note = Vec::new();
    if squad.is_some()
        && let Some(name) = row.and_then(|row| row["name"].as_str())
    {
        note.push(ratatui::text::Span::styled(
            format!("lead {}", crate::board::notes::single_line(name)),
            look.role(tmt_cli_style::Role::Muted),
        ));
    }
    if let Some(member) = row
        .and_then(|row| row["id"].as_str())
        .and_then(|id| app.workspace_snapshot()?.roster.members.get(id))
        && member.waiting
    {
        if !note.is_empty() {
            note.push(ratatui::text::Span::raw(" · "));
        }
        note.push(state_cell::mark(Some("waiting"), look));
        note.push(ratatui::text::Span::styled(
            " waiting",
            look.role(tmt_cli_style::Role::Muted),
        ));
    }
    section_rule::paint(
        frame.buffer_mut(),
        Rect {
            height: area.height.min(1),
            ..area
        },
        title,
        Line::from(note),
        app.focused() == Pane::Replies,
        look,
    );
    let remaining = area.height.saturating_sub(1);
    let admitted_input = app.composer_projection().is_some_and(|projection| {
        projection.key.is_some() && (projection.focused || app.composer_start().is_some())
    });
    let input = if admitted_input {
        remaining.min(crate::board::input_field::ROWS)
    } else {
        0
    };
    let gap = u16::from(input > 0 && remaining > input + 2);
    let exchange = remaining
        .saturating_sub(input + gap)
        .min(conversation::exchange::desired_height(app, area.width));
    let conversation_area = Rect::new(area.x, area.y + 1, area.width, exchange);
    conversation::exchange::render(frame, app, conversation_area);
    let input_area = Rect::new(area.x, conversation_area.bottom() + gap, area.width, input);
    if admitted_input {
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
        section_rule::paint(
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
