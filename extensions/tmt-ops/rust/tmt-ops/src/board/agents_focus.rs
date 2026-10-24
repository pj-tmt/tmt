//! Selection-driven Agents focus over the acquired workspace and existing sessions.
use super::{
    app::{App, RowTarget},
    conversation, conversation_checklist, identity_line, input_field, key_value,
    overview::{cron_counts::CronCountScope, timers},
    section_rule, spark_row,
    stat_tiles::{self, Tile},
    state_cell, timer_rows,
    workspace::{OpsSquad, OpsTarget},
};
use crate::{config::Pane, look::Look};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

// Metadata yields before the one response viewport and admitted input.
fn line(frame: &mut Frame, area: Rect, y: &mut u16, text: Line<'static>) {
    if *y < area.bottom() {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, *y, area.width, 1),
            text,
        );
        *y += 1;
    }
}
fn rule(frame: &mut Frame, app: &App, area: Rect, y: &mut u16, title: &str, note: Line<'static>) {
    if area.bottom().saturating_sub(*y) < 2 {
        *y = area.bottom();
        return;
    }
    if *y < area.bottom() {
        // The pane's top rule carries the Talk/Measure control.
        let line = Rect::new(area.x, *y, area.width, 1);
        let line = if *y == area.y {
            crate::board::talk_measure::paint(frame.buffer_mut(), app, line)
        } else {
            line
        };
        section_rule::paint(frame.buffer_mut(), line, title, note, false, app.look());
        *y += 1;
    }
}
fn blank(area: Rect, y: &mut u16) {
    *y = y.saturating_add(1).min(area.bottom());
}
fn neutral(text: impl Into<String>, look: Look) -> Line<'static> {
    Line::styled(text.into(), look.role(Role::Muted))
}
fn token_label(reading: Option<super::rate::Reading>) -> String {
    reading.map_or_else(
        || "—".into(),
        |reading| {
            format!(
                "{}{}",
                if reading.partial { "~" } else { "" },
                crate::source::tokens(reading.tokens)
            )
        },
    )
}
fn member_state(app: &App, target: &OpsTarget, width: usize) -> Line<'static> {
    let Some(row) = app
        .workspace_snapshot()
        .and_then(|snapshot| super::workspace::member_target_row(&snapshot.roster, target))
    else {
        return neutral("—", app.look());
    };
    let Some(label) = app.activity_label(&row) else {
        return neutral("?", app.look());
    };
    let (primary, task, todo) =
        super::workspace::columns::activity_spans(app, &row, &label, width, false);
    Line::from_iter(primary.into_iter().chain(task).chain(todo))
}

fn share(reading: Option<super::rate::Reading>, total: Option<(u128, bool)>) -> String {
    reading
        .zip(total)
        .filter(|(_, (total, _))| *total > 0)
        .map_or_else(
            || "—".into(),
            |(reading, (total, partial))| {
                format!(
                    "{}{:.0}%",
                    if reading.partial || partial { "~" } else { "" },
                    reading.tokens as f64 / total as f64 * 100.0
                )
            },
        )
}

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    if area.is_empty() {
        return;
    }
    // Non-message editors retain their existing modal owner and bottom band.
    let area = if app.input.is_some()
        && app.composer_projection().is_none()
        && app.conversation_input_area().is_some()
    {
        let height =
            super::view::waiting::input_lines(app, area, input_field::padded(frame.area().height))
                .min(usize::from(area.height)) as u16;
        app.input_band.set(Some(Rect::new(
            area.x,
            area.bottom() - height,
            area.width,
            height,
        )));
        Rect {
            height: area.height - height,
            ..area
        }
    } else {
        area
    };
    if area.is_empty() {
        return;
    }
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let retained = app.workspace_retained_search_target();
    if retained.is_none() && !app.workspace_selection_visible() {
        let mut y = area.y;
        rule(frame, app, area, &mut y, "Selection", Line::default());
        line(
            frame,
            area,
            &mut y,
            neutral(
                super::workspace::render::selection_message(app.workspace_selection_state()),
                app.look(),
            ),
        );
        app.conversation.borrow_mut().active = None;
        return;
    }
    let Some(selected) = retained.or(app.workspace.selected.as_ref()) else {
        return;
    };
    let raw = snapshot.rows.get(selected);
    let title = raw
        .and_then(|row| row["name"].as_str())
        .unwrap_or("Selection unavailable");
    let squad = match selected {
        OpsTarget::Squad(room)
        | OpsTarget::Member {
            room_id: Some(room),
            ..
        } => snapshot
            .roster
            .squads
            .iter()
            .find(|squad| &squad.room_id == room),
        _ => None,
    };
    let recipient = if retained.is_some() {
        app.workspace_preview_member()
    } else {
        app.workspace_message_target()
    }
    .map(RowTarget::Workspace);
    let row = recipient.as_ref().and_then(|target| app.target_row(target));
    let note = if matches!(selected, OpsTarget::Squad(_)) {
        squad.map_or_else(
            || neutral("unavailable", app.look()),
            |squad| {
                let members = squad
                    .members
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                let waiting = squad
                    .member_states
                    .values()
                    .filter(|state| state.waiting)
                    .count();
                let partial = if squad.observation == super::workspace::OpsObservation::Complete {
                    ""
                } else {
                    "~"
                };
                let mut spans = vec![Span::styled(
                    format!(
                        "{partial}{members} {}",
                        if members == 1 { "member" } else { "members" }
                    ),
                    app.look().role(Role::Muted),
                )];
                if waiting > 0 {
                    spans.push(Span::raw(" · "));
                    spans.push(state_cell::mark(Some("waiting"), app.look()));
                    spans.push(Span::styled(
                        format!(" {partial}{waiting} waiting"),
                        app.look().role(Role::Muted),
                    ));
                }
                Line::from(spans)
            },
        )
    } else {
        member_state(app, selected, usize::from(area.width.saturating_sub(8)))
    };
    let title_rule = crate::board::talk_measure::paint(
        frame.buffer_mut(),
        app,
        Rect {
            height: area.height.min(1),
            ..area
        },
    );
    section_rule::paint(
        frame.buffer_mut(),
        title_rule,
        title,
        note,
        app.focused() == Pane::Replies,
        app.look(),
    );
    let admitted_input = retained.is_none()
        && app.composer_projection().is_some_and(|projection| {
            projection.key.is_some() && (projection.focused || app.composer_start().is_some())
        });
    let reserved = if admitted_input {
        app.composer_rows(
            area.width.saturating_sub(input_field::GUTTER * 2),
            input_field::padded(frame.area().height),
        ) + 3
    } else {
        2
    };
    let metadata = Rect::new(
        area.x,
        area.y.saturating_add(area.height.min(1)),
        area.width,
        area.height.saturating_sub(1 + reserved),
    );
    let mut y = metadata.y;
    match selected {
        OpsTarget::DigestAll => {}
        OpsTarget::Member { .. } => {
            if let Some(row) = raw {
                member::render(frame, app, metadata, &mut y, row, selected, squad);
            }
        }
        OpsTarget::Squad(_) => squad::render(frame, app, metadata, &mut y, squad, row),
        OpsTarget::Pool => {
            line(
                frame,
                metadata,
                &mut y,
                neutral(
                    " Common pool · select a member to read its exchanges.",
                    app.look(),
                ),
            );
            blank(metadata, &mut y);
        }
    }
    let remaining = area.bottom().saturating_sub(y);
    if remaining == 0 {
        return;
    }
    // TODO #3217: use “Exchanges” after the all-party read contract ships.
    if y == area.y + 1 {
        // At the smallest heights metadata has yielded entirely. The optional
        // exchange rule yields too, keeping one gap below the focus title.
        blank(area, &mut y);
    } else {
        let shown = recipient
            .as_ref()
            .filter(|_| app.screen_mode() && retained.is_none());
        rule(
            frame,
            app,
            area,
            &mut y,
            &shown.map_or_else(
                || "Exchanges with you".to_owned(),
                |target| conversation::screen::title(app, target, usize::from(area.width)),
            ),
            recipient
                .as_ref()
                .and_then(|target| app.conversation_spinner(target))
                .map_or_else(Line::default, |glyph| neutral(glyph, app.look())),
        );
    }
    let Some(target) = recipient else {
        line(
            frame,
            area,
            &mut y,
            neutral(" No current recipient observed.", app.look()),
        );
        app.conversation.borrow_mut().active = None;
        return;
    };
    let surface = conversation::exchange::Surface {
        target,
        interactive: retained.is_none(),
    };
    let available = area.bottom().saturating_sub(y);
    let input = if admitted_input {
        app.composer_rows(
            area.width.saturating_sub(input_field::GUTTER * 2),
            input_field::padded(frame.area().height),
        )
        .min(available)
    } else {
        0
    };
    let gap = u16::from(input > 0 && available > input);
    if app.screen_mode() && retained.is_none() {
        // The live screen takes the exchanges' place; the composer below stays.
        conversation::screen::render(
            frame,
            app,
            Rect::new(area.x, y, area.width, available.saturating_sub(input + gap)),
            &surface.target,
        );
    } else {
        let height = conversation::exchange::desired_height(app, area.width, &surface)
            .min(available.saturating_sub(input + gap));
        conversation::exchange::render(
            frame,
            app,
            Rect::new(area.x, y, area.width, height),
            &surface,
        );
    }
    if admitted_input {
        app.input_band.set(Some(Rect::new(
            area.x + area.width.min(input_field::GUTTER),
            area.bottom() - input,
            area.width.saturating_sub(input_field::GUTTER * 2),
            input,
        )));
    }
}

pub(in crate::board) mod member;
mod squad;

#[cfg(test)]
mod tests;
