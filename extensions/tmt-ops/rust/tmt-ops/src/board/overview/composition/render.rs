//! Overview panel painting through existing source and action owners.
use super::Panel;
use ratatui::layout::Rect;
fn text(frame: &mut ratatui::Frame, area: Rect, value: String, app: &crate::board::app::App) {
    tmt_tui::components::strip::paint_left(
        frame.buffer_mut(),
        area,
        ratatui::text::Line::styled(
            crate::board::view::fit(
                &crate::board::notes::sanitize(&value),
                usize::from(area.width),
            ),
            app.look().role(tmt_cli_style::Role::Muted),
        ),
    );
}

fn key(app: &crate::board::app::App, action: crate::action::OpsUiAction) -> String {
    app.workspace_bindings()
        .into_iter()
        .find(|(_, binding)| binding.verb == crate::action::Verb::Ui(action))
        .map_or(String::new(), |(key, _)| format!("{key} "))
}

pub(in crate::board) fn render(
    frame: &mut ratatui::Frame,
    app: &crate::board::app::App,
    area: Rect,
) {
    use crate::board::{
        app::{Hit, HitTarget},
        view::ClockSurface,
    };
    let Some(snapshot) = app.workspace_snapshot() else {
        return;
    };
    let area =
        crate::board::pane_chrome::paint(frame, app, area, crate::config::Pane::Rows, "Overview");
    let scene = super::super::monitor::scene(app, snapshot);
    let limits = if crate::board::home::limits_enabled(app) {
        super::super::quota::lines(
            app.limits.as_deref(),
            crate::status::now_ms(),
            area.width,
            app.look(),
        )
    } else {
        Vec::new()
    };
    let placed = app.overview_layout.borrow_mut().panels(
        area,
        if limits.is_empty() {
            0
        } else {
            limits.len().saturating_add(1).min(usize::from(u16::MAX)) as u16
        },
    );
    // The existing chart host clears its prior-frame visibility/hits even when its panel cannot fit.
    if !placed.iter().any(|(panel, _)| *panel == Panel::Chart) {
        let _ = super::super::chart::host::render(frame, app, Rect { height: 0, ..area });
    }
    let mut chart_rest = None;
    for (panel, mut rect) in placed.iter().copied() {
        match panel {
            Panel::Summary => {
                let timer = if let Some(cron) = app
                    .cron
                    .cron
                    .as_ref()
                    .filter(|_| app.cron.failure.is_none())
                {
                    match &cron.clock {
                        tmt_ops::cron::ClockStatus::Running(_) => "Timers · on",
                        tmt_ops::cron::ClockStatus::NoClock => "Timers · no clock",
                        tmt_ops::cron::ClockStatus::Unknown => "Timers · unknown",
                    }
                } else {
                    "Timers · unknown"
                };
                let timer = format!("[ {timer} ]");
                let width = unicode_width::UnicodeWidthStr::width(timer.as_str()) as u16;
                let summary = if width + 24 <= rect.width {
                    let control = Rect::new(rect.right() - width, rect.y, width, 1);
                    text(frame, control, timer, app);
                    app.hits.borrow_mut().push(Hit {
                        x: control.x,
                        y: control.y,
                        width,
                        target: HitTarget::AllRoomTimers,
                    });
                    Rect {
                        width: rect.width - width - 1,
                        ..rect
                    }
                } else {
                    rect
                };
                tmt_tui::components::strip::paint_left(
                    frame.buffer_mut(),
                    summary,
                    scene.summary.agents_paint(
                        matches!(app.cron_glyphs, super::super::cron_counts::GlyphSet::Ascii),
                        app.look(),
                    ),
                );
            }
            Panel::Digest => {
                let (counts, estimate) = scene.summary.digest_parts(usize::from(rect.width));
                match estimate {
                    None => text(frame, rect, counts, app),
                    // Counts are measured state; the estimate keeps the dim role every
                    // other savings surface gives it. Both fit whole by construction.
                    Some(estimate) => {
                        use unicode_width::UnicodeWidthStr;
                        let look = app.look();
                        // Only the caption answers a click, not the separator before it.
                        let caption = estimate.trim_start_matches(" · ");
                        let offset = counts.width() + estimate.width() - caption.width();
                        if offset < usize::from(rect.width) {
                            app.savings_hit(
                                rect.x + offset as u16,
                                rect.y,
                                caption.width(),
                                rect.width - offset as u16,
                            );
                        }
                        tmt_tui::components::strip::paint_left(
                            frame.buffer_mut(),
                            rect,
                            ratatui::text::Line::from(vec![
                                ratatui::text::Span::styled(
                                    counts,
                                    look.role(tmt_cli_style::Role::Muted),
                                ),
                                ratatui::text::Span::styled(
                                    estimate,
                                    look.role(tmt_cli_style::Role::Dim),
                                ),
                            ]),
                        );
                    }
                }
            }
            Panel::Quota => {
                text(
                    frame,
                    Rect { height: 1, ..rect },
                    crate::board::view::scene::rule("Subscriptions", usize::from(rect.width)),
                    app,
                );
                let painted_rows = limits.len().min(usize::from(rect.height.saturating_sub(1)));
                if painted_rows > 0 {
                    app.frame_clocks
                        .borrow_mut()
                        .push(ClockSurface::OverviewQuota {
                            width: rect.width,
                            rows: painted_rows,
                        });
                }
                for (offset, line) in limits
                    .iter()
                    .take(usize::from(rect.height.saturating_sub(1)))
                    .enumerate()
                {
                    if line.width() <= usize::from(rect.width) {
                        app.hits.borrow_mut().push(Hit {
                            x: rect.x,
                            y: rect.y + 1 + offset as u16,
                            width: rect.width,
                            target: HitTarget::Quota,
                        });
                    }
                    tmt_tui::components::strip::paint_left(
                        frame.buffer_mut(),
                        Rect::new(rect.x, rect.y + 1 + offset as u16, rect.width, 1),
                        line.clone(),
                    );
                }
            }
            Panel::Chart => {
                let table = placed
                    .iter()
                    .find(|(panel, _)| *panel == Panel::Table)
                    .unwrap()
                    .1;
                chart_rest = Some(super::super::chart::host::render(
                    frame,
                    app,
                    Rect {
                        height: table.bottom() - rect.y,
                        ..rect
                    },
                ));
            }
            Panel::Table => {
                if let Some(rest) = chart_rest {
                    rect = rest;
                }
                let sort = snapshot
                    .preferences
                    .overview_sort
                    .map_or("needs you".into(), |sort| {
                        format!(
                            "{} {}",
                            sort.column.title(),
                            if sort.direction == crate::config::OverviewDirection::Ascending {
                                "↑"
                            } else {
                                "↓"
                            }
                        )
                    });
                let controls = format!(
                    "{}{} · Sort: {sort} · {}{}",
                    key(app, crate::action::OpsUiAction::Search),
                    if app.workspace.query.is_empty() {
                        "Search agents…"
                    } else {
                        app.workspace.query.as_str()
                    },
                    key(app, crate::action::OpsUiAction::OverviewFilter),
                    app.overview_filter.label()
                );
                if rect.height >= 4 {
                    text(frame, Rect { height: 1, ..rect }, controls, app);
                    rect.y += 1;
                    rect.height -= 1;
                }
                super::super::view::render(frame, app, rect);
            }
            Panel::Exchanges => {
                text(
                    frame,
                    Rect { height: 1, ..rect },
                    crate::board::view::scene::rule("Recent exchanges", usize::from(rect.width)),
                    app,
                );
                match &scene.recent {
                    super::super::events::Recent::Unavailable => text(
                        frame,
                        Rect::new(rect.x, rect.y + 1, rect.width, 1),
                        "Recent exchanges unavailable".into(),
                        app,
                    ),
                    super::super::events::Recent::Observed(rows) if rows.is_empty() => text(
                        frame,
                        Rect::new(rect.x, rect.y + 1, rect.width, 1),
                        "No observed recent exchanges".into(),
                        app,
                    ),
                    super::super::events::Recent::Observed(rows) => {
                        let now = crate::status::now_ms();
                        for (index, row) in
                            rows.iter().take(usize::from(rect.height - 1)).enumerate()
                        {
                            let age = row.at_ms.and_then(|at| now.checked_sub(at)).map_or(
                                "time not known".into(),
                                tmt_cli_style::value::relative_time,
                            );
                            let kind = match row.kind {
                                crate::board::home_leads::Kind::Question => "Question",
                                crate::board::home_leads::Kind::Asked => "Asked",
                                crate::board::home_leads::Kind::Reply => "Reply",
                            };
                            text(
                                frame,
                                Rect::new(rect.x, rect.y + 1 + index as u16, rect.width, 1),
                                format!("{age} · {} · {kind} · {}", row.agent, row.preview),
                                app,
                            );
                        }
                    }
                }
            }
            Panel::Legend => {
                let hits = app.hits.borrow();
                let indices: Vec<_> = hits.iter().filter_map(Hit::row).collect();
                let first = indices.iter().min().map_or(0, |index| index + 1);
                let last = indices.iter().max().map_or(0, |index| index + 1);
                let total = scene.rows.len();
                text(
                    frame,
                    Rect { height: 1, ..rect },
                    format!(
                        "{first}–{last} / {total}{} rows · {} more ↓",
                        if scene.summary.complete {
                            ""
                        } else {
                            " observed"
                        },
                        total.saturating_sub(last)
                    ),
                    app,
                );
                if let Some([first, _]) =
                    crate::board::view::footer::overview_legend(app, usize::from(rect.width))
                {
                    tmt_tui::components::strip::paint_left(
                        frame.buffer_mut(),
                        Rect::new(rect.x, rect.y + 1, rect.width, 1),
                        crate::board::view::footer::hint_line(&first, app.look()),
                    );
                }
            }
        }
    }
}
