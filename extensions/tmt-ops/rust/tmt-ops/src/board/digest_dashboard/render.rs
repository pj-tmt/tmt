use super::model;
use crate::{
    board::{
        app::{App, Hit, HitTarget},
        usage_report,
        view::fit,
        workspace::OpsTarget,
    },
    config::Pane,
};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

/// Rows the member list keeps before the Usage report takes the rest.
const LIST_FLOOR: u16 = 6;

fn usage(app: &App, width: u16) -> (Vec<Line<'static>>, std::ops::Range<usize>) {
    let look = app.look();
    let mut lines = vec![Line::styled(
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
        lines.push(Line::styled(text, look.role(Role::Text)));
    } else {
        lines.push(Line::styled("Usage unavailable", look.role(Role::Muted)));
    }
    let mut savings_lines = 0..0;
    if let Some(snapshot) = app.workspace_snapshot() {
        let savings = app.digest_savings(snapshot.roster.members.keys().map(String::as_str));
        let caption = app.savings_caption(&savings, usize::from(width));
        for (index, text) in [
            Some(caption),
            app.digest_savings_start(&savings, usize::from(width)),
        ]
        .into_iter()
        .flatten()
        .enumerate()
        {
            let start = lines.len();
            lines.extend(
                crate::board::notes::wrap(&text, usize::from(width))
                    .into_iter()
                    .map(|line| Line::styled(line, look.role(Role::Dim))),
            );
            if index == 0 {
                savings_lines = start..lines.len();
            }
        }
    }
    (lines, savings_lines)
}

fn status(app: &App) -> Line<'static> {
    if app.loading() {
        Line::styled("Loading workspace…", app.look().role(Role::Muted))
    } else {
        Line::styled(super::summary(app), app.look().role(Role::Text))
    }
}

pub(super) fn status_in_body(app: &App) -> bool {
    // Workspace shell chrome holds navigation, not a dashboard's Status panel.
    app.workspace_snapshot().is_some()
}

pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    use crate::config::DigestPanel as Id;
    let look = app.look();
    let policy = app.digest_panels();
    let (usage, savings_lines) = usage(app, area.width);
    let quota = crate::board::home::limits_of(app, area.width, look);
    let status_in_body = status_in_body(app);
    let required_status =
        status_in_body && (app.loading() || app.labels.digest().is_none_or(|(_, stale)| stale));
    // Hiding ordinary status does not conceal an unavailable/stale/loading read.
    let status_rows =
        u16::from(status_in_body && (policy.shown(Id::Status) || required_status)).min(area.height);
    let context_budget = area.height.saturating_sub(5);
    let usage_rows = if policy.shown(Id::Usage) {
        usage
            .len()
            .min(usize::from(context_budget.saturating_sub(status_rows))) as u16
    } else {
        0
    };
    let quota_rows = if policy.shown(Id::Subscriptions) && !quota.is_empty() {
        quota.len().saturating_add(1).min(usize::from(
            context_budget.saturating_sub(status_rows + usage_rows),
        )) as u16
    } else {
        0
    };
    let remaining = area
        .height
        .saturating_sub(status_rows + usage_rows + quota_rows);
    // The report takes what it needs, except the rows the member list keeps.
    let report = if policy.shown(Id::Report) {
        let state = &app.usage_report;
        usage_report::rows(&state.view(), state.since, area.width, usage_report::zone())
    } else {
        Vec::new()
    };
    let report_rows = report
        .len()
        .min(usize::from(remaining.saturating_sub(LIST_FLOOR))) as u16;
    let chart_and_target = remaining - report_rows;
    let chart_now = crate::status::now_ms();
    let chart_rows = if policy.shown(Id::Chart) {
        crate::board::overview::chart::host::demand(app, area.width, chart_and_target, chart_now)
    } else {
        0
    };
    let target_rows = chart_and_target.saturating_sub(chart_rows);
    let mut y = area.y;
    let mut target = Rect::new(area.x, area.y, area.width, 0);
    let mut painted_chart = false;
    // A hidden Status warning is internal safety chrome, above ordered content.
    if required_status && !policy.shown(Id::Status) && status_rows > 0 {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(area.x, y, area.width, 1),
            status(app),
        );
        y += 1;
    }
    for panel in policy.order() {
        let height = match panel {
            Id::Status if policy.shown(Id::Status) => status_rows,
            Id::Status => 0,
            Id::Usage => usage_rows,
            Id::Subscriptions => quota_rows,
            Id::Chart => chart_rows,
            Id::List => target_rows,
            Id::Report => report_rows,
        };
        if height == 0 {
            continue;
        }
        let rect = Rect::new(area.x, y, area.width, height).intersection(area);
        y += rect.height;
        match panel {
            Id::Status => strip::paint_left(frame.buffer_mut(), rect, status(app)),
            Id::Usage => {
                for (offset, line) in usage.iter().take(usize::from(rect.height)).enumerate() {
                    if savings_lines.contains(&offset) {
                        app.savings_hit(rect.x, rect.y + offset as u16, line.width(), rect.width);
                    }
                    strip::paint_left(
                        frame.buffer_mut(),
                        Rect::new(rect.x, rect.y + offset as u16, rect.width, 1),
                        line.clone(),
                    );
                }
            }
            Id::Subscriptions => {
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect { height: 1, ..rect },
                    Line::styled("Subscriptions · reported windows", look.role(Role::Muted)),
                );
                let rows = quota.len().min(usize::from(rect.height.saturating_sub(1)));
                if rows > 0 && !app.modal_open() {
                    app.frame_clocks
                        .borrow_mut()
                        .push(crate::board::view::ClockSurface::Limits {
                            width: rect.width,
                            rows,
                        });
                }
                for (offset, line) in quota.iter().take(rows).enumerate() {
                    let row = Rect::new(rect.x, rect.y + 1 + offset as u16, rect.width, 1);
                    app.hits.borrow_mut().push(Hit {
                        x: row.x,
                        y: row.y,
                        width: row.width,
                        target: HitTarget::Quota,
                    });
                    strip::paint_left(frame.buffer_mut(), row, line.clone());
                }
            }
            Id::Chart => {
                crate::board::overview::chart::host::render_panel(frame, app, rect, chart_now);
                painted_chart = true;
            }
            Id::List => target = rect,
            Id::Report => usage_report::paint(frame.buffer_mut(), app, rect, &report),
        }
    }
    if !painted_chart {
        crate::board::overview::chart::host::render_panel(
            frame,
            app,
            Rect { height: 0, ..area },
            chart_now,
        );
    }
    app.conversation.borrow_mut().area = target;
    use crate::action::ResizeBoundary as Boundary;
    let placement = app.digest_placement(target);
    app.pane_resize.record_panes(
        placement,
        Boundary::DigestColumns,
        Boundary::DigestRows,
        frame.area(),
    );
    if let (Some(list), Some(detail)) = (placement.list, placement.conversation)
        && list.y == detail.y
    {
        crate::board::divider::paint(
            frame.buffer_mut(),
            Rect::new(list.right(), list.y, 1, list.height),
            false,
            look,
        );
    }
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
