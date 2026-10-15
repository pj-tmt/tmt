use super::model;
use crate::{
    board::{
        app::{App, Hit, HitTarget},
        conversation::layout::{self, Preference},
        view::fit,
        workspace::OpsTarget,
    },
    config::{OpsLayout, Pane},
};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) fn render(frame: &mut Frame, app: &App, mut area: Rect) {
    let look = app.look();
    let mut summary = vec![Line::styled(
        if app.workspace.visibility.scope.is_empty() {
            "Usage · unique identities"
        } else {
            "Usage · global identities"
        },
        look.role(Role::Muted),
    )];
    if let Some(usage) = app.home_header_usage(std::time::Instant::now()) {
        let text = usage
            .windows
            .iter()
            .zip(usage.totals)
            .filter(|(window, _)| window.milliseconds() <= 3_600_000)
            .map(|(window, reading)| {
                format!(
                    "{} {}",
                    window.label(),
                    reading.map_or_else(
                        || "unavailable".into(),
                        |reading| format!(
                            "{}{} tok",
                            if reading.partial { "≥" } else { "" },
                            crate::source::tokens(reading.tokens as f64)
                        )
                    )
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        summary.push(Line::styled(text, look.role(Role::Text)));
    } else {
        summary.push(Line::styled("Usage unavailable", look.role(Role::Muted)));
    }
    if let Some(snapshot) = app.workspace_snapshot() {
        let savings = app.digest_savings(snapshot.roster.members.keys().map(String::as_str));
        let caption =
            model::savings_caption(&savings, app.digest_window(), usize::from(area.width));
        for text in [Some(caption), app.digest_savings_start(&savings)]
            .into_iter()
            .flatten()
        {
            summary.extend(
                crate::board::notes::wrap(&text, usize::from(area.width))
                    .into_iter()
                    .map(|line| Line::styled(line, look.role(Role::Dim))),
            );
        }
    }
    summary.push(Line::styled(super::summary(app), look.role(Role::Text)));
    let quota = crate::board::home::limits_of(app, area.width, look);
    let quota_start = summary.len() + 1;
    let quota_count = quota.len();
    if !quota.is_empty() {
        summary.push(Line::styled(
            "Subscriptions · reported windows",
            look.role(Role::Muted),
        ));
        summary.extend(quota);
    }
    // Preserve a usable list/detail area at low height; complete data remains in details.
    let height = summary
        .len()
        .min(usize::from(area.height.saturating_sub(5))) as u16;
    let quota_visible = usize::from(height)
        .saturating_sub(quota_start)
        .min(quota_count);
    // Modal input owns the body; its retained background adds no clock work.
    if quota_visible > 0 && !app.modal_open() {
        app.frame_clocks
            .borrow_mut()
            .push(crate::board::view::ClockSurface::Limits {
                width: area.width,
                rows: quota_visible,
            });
    }
    for (offset, line) in summary.into_iter().take(usize::from(height)).enumerate() {
        if offset >= quota_start {
            app.hits.borrow_mut().push(Hit {
                x: area.x,
                y: area.y + offset as u16,
                width: area.width,
                target: HitTarget::Quota,
            });
        }
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, area.y + offset as u16, area.width, 1),
            line,
        );
    }
    area.y += height;
    area.height -= height;
    area = crate::board::overview::chart::host::render(frame, app, area);
    app.conversation.borrow_mut().area = area;
    let preference = app
        .workspace_snapshot()
        .map_or(Preference::Auto, |snapshot| {
            match snapshot.preferences.layout {
                OpsLayout::Auto => Preference::Auto,
                OpsLayout::Side => Preference::Side,
                OpsLayout::Stack => Preference::Stack,
            }
        });
    let placement = layout::agents(area, preference, app.focused() == Pane::Replies);
    if let Some(list) = placement.list {
        members(frame, app, list);
    }
    app.conversation.borrow_mut().pane = placement.conversation;
    if let Some(detail) = placement.conversation {
        details(frame, app, detail);
    }
}

fn members(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let mut inside = crate::board::pane_chrome::paint(frame, app, area, Pane::Rows, "Digest");
    if inside.height == 0 {
        return;
    }
    let name_width = (usize::from(inside.width) / 3)
        .clamp(8, 28)
        .min(usize::from(inside.width));
    strip::paint_left(
        frame.buffer_mut(),
        Rect {
            height: 1,
            ..inside
        },
        Line::styled(
            format!("{}Digest", fit("Agent", name_width)),
            look.role(Role::Muted),
        ),
    );
    inside.y += 1;
    inside.height -= 1;
    let rows = app.workspace_rows();
    *app.row_starts.borrow_mut() = (0..rows.len()).collect();
    if app.follow
        && let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
    {
        app.scrolls
            .reveal_range(Pane::Rows, index..index + 1, inside, rows.len());
    }
    app.scrolls.show_paint(
        frame,
        Pane::Rows,
        inside,
        rows.len().max(1),
        look,
        |frame, viewport, offset| {
            if rows.is_empty() {
                let text = if app.workspace.query.is_empty() {
                    "No agents observed"
                } else {
                    "No matching agents"
                };
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect {
                        height: u16::from(viewport.height > 0),
                        ..viewport
                    },
                    Line::styled(text, look.role(Role::Muted)),
                );
            }
            for (index, row) in rows
                .iter()
                .enumerate()
                .skip(offset)
                .take(usize::from(viewport.height))
            {
                let OpsTarget::Member { identity_id, .. } = &row.target else {
                    continue;
                };
                let area = Rect::new(
                    viewport.x,
                    viewport.y + (index - offset) as u16,
                    viewport.width,
                    1,
                );
                let selected = Some(&row.target) == app.workspace.selected.as_ref();
                let base = if selected {
                    look.selection()
                } else {
                    look.role(Role::Text)
                };
                if selected {
                    frame.buffer_mut().set_style(area, base);
                }
                let mut spans = vec![Span::styled(
                    fit(&crate::board::notes::sanitize(&row.title), name_width),
                    base,
                )];
                let mut x = area.x.saturating_add(name_width as u16).min(area.right());
                let labels = model::labels(&app.labels, identity_id);
                let label_style = |role| if selected { base } else { look.role(role) };
                if labels.is_empty() {
                    spans.push(Span::styled("unavailable", label_style(Role::Muted)));
                }
                for (position, (text, role, choice)) in labels.into_iter().enumerate() {
                    let gap = if position == 0 { "" } else { " · " };
                    let width = (gap.width() + text.width()).min(usize::from(u16::MAX)) as u16;
                    if x.saturating_add(width) > area.right() {
                        break;
                    }
                    spans.push(Span::styled(format!("{gap}{text}"), label_style(role)));
                    if choice {
                        app.chip_hits
                            .borrow_mut()
                            .push((Rect::new(x, area.y, width, 1), index));
                    }
                    x += width;
                }
                strip::paint_left(frame.buffer_mut(), area, Line::from(spans));
                app.hits.borrow_mut().push(Hit {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    target: HitTarget::Row(index),
                });
            }
        },
    );
}

fn details(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let inside = crate::board::pane_chrome::paint(frame, app, area, Pane::Replies, "Details");
    let mut text = Vec::new();
    if let Some(row) = app.selected_row() {
        text.push(crate::board::notes::sanitize(
            row["name"].as_str().unwrap_or("Agent"),
        ));
        if let Some(id) = row["id"].as_str() {
            text.extend(model::details(&app.labels, id));
        }
    } else {
        text.push("Select a visible agent to inspect its Digest status".into());
    }
    let lines: Vec<_> = text
        .into_iter()
        .flat_map(|text| {
            crate::board::notes::wrap(
                &crate::board::notes::sanitize(&text),
                usize::from(inside.width),
            )
        })
        .map(|text| Line::styled(text, look.role(Role::Text)))
        .collect();
    app.scrolls.show(frame, Pane::Replies, inside, lines, look);
}
