use super::{metrics, model, table};
use crate::board::{
    app::{App, Choice},
    exchange_matrix,
    usage_report::View,
};
use crate::config::{Pane, TokRange};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

/// Summary and content share the same placement owner as focus and pointer input.
pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect) {
    let _rows = app.workspace_frame_rows();
    summary(frame, app, area);
    let now = crate::status::now_ms();
    let chart_height = if app.usage_chart.borrow().focused()
        && app.digest_panels().shown(crate::config::DigestPanel::Chart)
    {
        crate::board::overview::chart::host::demand(
            app,
            area.width,
            area.height.saturating_sub(1),
            now,
        )
    } else {
        0
    };
    crate::board::overview::chart::host::render_panel(
        frame,
        app,
        Rect::new(area.x, area.y.saturating_add(1), area.width, chart_height),
        now,
    );
    let allowance_height = super::allowance::height(
        app,
        area.width,
        area.height.saturating_sub(1 + chart_height),
    );
    super::allowance::paint(
        frame,
        app,
        Rect::new(
            area.x,
            area.y.saturating_add(1 + chart_height),
            area.width,
            allowance_height,
        ),
    );
    let top = 1 + chart_height + allowance_height;
    let body = Rect::new(
        area.x,
        area.y.saturating_add(top),
        area.width,
        area.height.saturating_sub(top),
    );
    app.conversation.borrow_mut().area = body;
    let placement = app.digest_placement(body);
    use crate::action::ResizeBoundary as Boundary;
    app.pane_resize.record_panes(
        placement,
        Boundary::DigestColumns,
        Boundary::DigestRows,
        94,
        frame.area(),
    );
    if let (Some(list), Some(side)) = (placement.list, placement.conversation)
        && list.y == side.y
        && list.x != side.x
        && !(crate::board::pane_chrome::boxed(list) && crate::board::pane_chrome::boxed(side))
    {
        // Framed panes mark the seam with their own edges; the gap between them stays the drag handle.
        crate::board::divider::paint(
            frame.buffer_mut(),
            Rect::new(list.right(), list.y, 1, list.height),
            false,
            app.look(),
        );
    }
    if let Some(list) = placement.list {
        table::paint(frame, app, list);
    }
    app.conversation.borrow_mut().pane = placement.conversation;
    if let Some(side) = placement.conversation {
        let wide = placement
            .list
            .is_some_and(|list| list.y == side.y && list.x != side.x);
        let details = app.digest_panels().side_shown();
        if wide {
            let matrix_height = 9.min(side.height);
            matrix(
                frame,
                app,
                Rect {
                    height: matrix_height,
                    ..side
                },
                !details,
            );
            if details {
                detail(
                    frame,
                    app,
                    Rect::new(
                        side.x,
                        side.y + matrix_height,
                        side.width,
                        side.height - matrix_height,
                    ),
                );
            }
        } else if app.focused() == Pane::Replies || !details {
            matrix(frame, app, side, true);
        } else {
            detail(frame, app, side);
        }
    }
}
fn summary(frame: &mut Frame, app: &App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let rows = app.workspace_rows();
    let ids: Vec<_> = rows.iter().filter_map(metrics::id).collect();
    let mut parts = vec![
        format!("Range {} ▾", app.tok_range().argument()),
        format!(
            "TOK {}",
            metrics::token_text(metrics::tokens(metrics::report(app), &ids))
        ),
        if app.labels.digest().is_none() {
            "Digest unavailable".into()
        } else {
            super::summary(app)
        },
    ];
    if app.loading() {
        parts.insert(1, "Loading workspace…".into());
    }
    match app.usage_report.view_for(app.tok_range()) {
        View::Loading => parts.insert(1, "Usage loading…".into()),
        View::Failed(_) => parts.insert(1, "Usage unavailable".into()),
        View::Report(_) if app.usage_report.refresh_error(app.tok_range()).is_some() => {
            parts.insert(1, "Usage stale · refresh failed".into())
        }
        _ => {}
    }
    let savings = metrics::savings(app, &ids);
    let savings_available = savings.as_ref().is_some_and(|summary| {
        crate::board::savings::savings_value(
            summary,
            if app.tok_range() == TokRange::Day {
                crate::config::DigestWindow::Day
            } else {
                crate::config::DigestWindow::Week
            },
            app.savings_unit(),
        )
        .0
        .is_some()
    });
    if let Some(savings) = savings {
        let window = if app.tok_range() == TokRange::Day {
            crate::config::DigestWindow::Day
        } else {
            crate::config::DigestWindow::Week
        };
        parts.push(crate::board::savings::savings_caption(
            &savings,
            window,
            app.savings_unit(),
            usize::from(area.width),
        ));
    } else {
        parts.push("Est. avoided unavailable for this range".into());
    }
    // Omit complete trailing metrics, never half an amount.
    let mut text = String::new();
    for part in parts {
        let candidate = if text.is_empty() {
            part.clone()
        } else {
            format!("{text} · {part}")
        };
        if unicode_width::UnicodeWidthStr::width(candidate.as_str()) > usize::from(area.width) {
            break;
        }
        if savings_available && part.starts_with("Est.") {
            let gap = if text.is_empty() { 0 } else { 3 };
            let x = area.x + (unicode_width::UnicodeWidthStr::width(text.as_str()) + gap) as u16;
            app.savings_hit(
                x,
                area.y,
                unicode_width::UnicodeWidthStr::width(part.as_str()),
                area.right().saturating_sub(x),
            );
        }
        text = candidate;
    }
    strip::paint_left(
        frame.buffer_mut(),
        Rect { height: 1, ..area },
        Line::styled(text, app.look().role(Role::Text)),
    );
    let chip = format!("Range {} ▾", app.tok_range().argument());
    let width = unicode_width::UnicodeWidthStr::width(chip.as_str()) as u16;
    if width <= area.width {
        app.workspace_strip_hits.borrow_mut().push((
            Rect::new(area.x, area.y, width, 1),
            Choice::TokRange(app.tok_range().next()),
        ));
    }
}
fn matrix(frame: &mut Frame, app: &App, area: Rect, chrome: bool) {
    let inside = if chrome {
        crate::board::pane_chrome::paint(
            frame,
            app,
            area,
            Pane::Replies,
            "Exchanges · talk + reply",
        )
    } else {
        if !area.is_empty() {
            strip::paint_left(
                frame.buffer_mut(),
                Rect { height: 1, ..area },
                Line::styled("── Exchanges · talk + reply", app.look().role(Role::Muted)),
            );
        }
        Rect::new(
            area.x.saturating_add(u16::from(area.width > 0)),
            area.y.saturating_add(u16::from(area.height > 0)),
            area.width.saturating_sub(2),
            area.height.saturating_sub(1),
        )
    };
    if inside.is_empty() {
        return;
    }
    let rows = app.workspace_rows();
    let mut ranked: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            Some(exchange_matrix::ExchangeParticipant {
                id: metrics::id(row)?.into(),
                name: row.title.clone(),
            })
        })
        .take(5)
        .collect();
    let selected = app
        .workspace
        .selected
        .as_ref()
        .and_then(|target| match target {
            crate::board::workspace::OpsTarget::Member { identity_id, .. } => {
                Some(identity_id.as_str())
            }
            _ => None,
        });
    if let Some(id) = selected
        && !ranked.iter().any(|row| row.id == id)
        && let Some(row) = rows.iter().find(|row| metrics::id(row) == Some(id))
    {
        ranked.push(exchange_matrix::ExchangeParticipant {
            id: id.into(),
            name: row.title.clone(),
        });
    }
    let report = metrics::report(app);
    let _cells = exchange_matrix::paint(
        frame.buffer_mut(),
        inside,
        &ranked,
        selected,
        report.and_then(|report| report.exchange.as_ref()),
        app.look(),
    );
    if ranked.is_empty() {
        strip::paint_left(
            frame.buffer_mut(),
            Rect {
                height: 1,
                ..inside
            },
            Line::styled("No agents observed", app.look().role(Role::Muted)),
        );
    }
}
fn detail(frame: &mut Frame, app: &App, area: Rect) {
    let inside = crate::board::pane_chrome::paint(frame, app, area, Pane::Replies, "Details");
    let lines = detail_lines(
        app,
        inside.width,
        crate::status::now_ms(),
        app.selected_row(),
    );
    app.scrolls
        .show(frame, Pane::Replies, inside, lines, app.look());
    if !inside.is_empty()
        && let Some(target) = app.row_target(app.selected)
    {
        app.frame_clocks
            .borrow_mut()
            .push(crate::board::view::ClockSurface::DigestDetails {
                target,
                width: inside.width,
                height: inside.height,
            });
    }
}
fn detail_lines(
    app: &App,
    width: u16,
    now_ms: u64,
    row: Option<&serde_json::Value>,
) -> Vec<Line<'static>> {
    let rows = app.workspace_rows();
    let selected = rows
        .iter()
        .find(|row| Some(&row.target) == app.workspace.selected.as_ref());
    let ids = selected.map_or_else(Vec::new, |row| metrics::identities(row, &rows));
    let report = metrics::report(app);
    let mut text = vec![
        selected.map_or("Select a visible agent or all".into(), |row| {
            row.title.clone()
        }),
        format!(
            "{} · TOK {}",
            app.tok_range().argument(),
            metrics::token_text(metrics::tokens(report, &ids))
        ),
    ];
    let price_column_shown = table::price_column_shown(app);
    if let Some(cost) = metrics::input_price(app, report, &ids) {
        text.push(format!(
            "Input list-price estimate: {}",
            crate::board::savings::usd_text(cost)
        ));
    } else if !price_column_shown {
        text.push("Input list-price estimate: —".into());
    }
    if !price_column_shown {
        text.push("Input tokens at list price; not a bill.".into());
    }
    if let Some(id) = row.and_then(|row| row["id"].as_str()) {
        text.extend(model::details(
            &app.labels,
            id,
            now_ms,
            app.local_zone.as_ref().unwrap_or(&jiff::tz::TimeZone::UTC),
        ));
        text.push(
            report
                .and_then(|report| metrics::recipient(report, id))
                .map_or("Talks to: unavailable".into(), |(id, count)| {
                    format!(
                        "Talks to: {} · {} messages",
                        metrics::name(app, id),
                        crate::source::tokens(u128::from(count))
                    )
                }),
        );
        if let Some(seat) = report.and_then(|report| report.seat(id)) {
            text.push(format!(
                "Usage coverage: {}% of {}{}",
                seat.covered_ms.saturating_mul(100) / seat.window_ms.max(1),
                app.tok_range().argument(),
                if seat.complete {
                    " · complete"
                } else {
                    " · partial"
                }
            ));
            let zone = app.local_zone.clone().unwrap_or(jiff::tz::TimeZone::UTC);
            let timestamp = |ms| {
                i64::try_from(ms)
                    .ok()
                    .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
                    .map_or("unknown".into(), |at| {
                        at.to_zoned(zone.clone())
                            .strftime("%m-%d %H:%M")
                            .to_string()
                    })
            };
            text.push(format!(
                "Usage window: {}–{}",
                timestamp(seat.from_ms),
                timestamp(seat.to_ms)
            ));
            text.push(format!(
                "Usage observed: {}",
                model::observation_time(Some(seat.observed_at_ms), now_ms, &zone)
            ));
            if seat.series.iter().any(|point| !point.complete) {
                text.push("Hourly activity is partial; fills show observed lower bounds".into());
            }
        } else if let Some(unavailable) = report.and_then(|report| {
            report
                .unavailable
                .iter()
                .find(|row| row.who.id.as_deref() == Some(id))
        }) {
            text.push(format!("Usage unavailable: {}", unavailable.reason));
        }
        if let Some(point) = report.and_then(|report| metrics::busiest(report, id)) {
            let zone = app.local_zone.clone().unwrap_or(jiff::tz::TimeZone::UTC);
            let clock = |ms| {
                i64::try_from(ms)
                    .ok()
                    .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
                    .map_or("unknown".into(), |at| {
                        at.to_zoned(zone.clone()).strftime("%H:%M").to_string()
                    })
            };
            text.push(format!(
                "Busiest hour: {}–{} · {}{} TOK",
                clock(point.from_ms),
                clock(point.to_ms),
                if point.complete { "" } else { "≥" },
                crate::source::tokens(u128::from(point.tokens))
            ));
        }
    }
    if app.labels.digest().is_none() {
        text.push("Digest not available".into());
        text.push("Install: tmt extension install digest".into());
    }
    match app.usage_report.view_for(app.tok_range()) {
        View::Loading => text.push("Usage loading…".into()),
        View::Failed(error) => text.push(format!("Usage unavailable: {error}")),
        View::Report(report) => {
            if row.and_then(|row| row["id"].as_str()).is_none() {
                text.push(format!(
                    "Latest usage observed: {}",
                    model::observation_time(
                        Some(report.observed_at_ms),
                        now_ms,
                        app.local_zone.as_ref().unwrap_or(&jiff::tz::TimeZone::UTC)
                    )
                ));
                text.push("Totals combine matching source windows only".into());
            }
            text.extend(
                report
                    .warnings
                    .iter()
                    .map(|warning| format!("{}: {}", warning.section, warning.message)),
            );
        }
    }
    if let Some(error) = app.usage_report.refresh_error(app.tok_range()) {
        text.push(format!("Stale usage retained · refresh failed: {error}"));
    }
    let narrow_activity = app.conversation.borrow().area.width.saturating_sub(2) < 70;
    if narrow_activity {
        text.push(if width >= 28 {
            "24 cells, 1h each · observed counts · ? unknown · · zero".into()
        } else {
            "Widen to 28 columns for 24 hourly cells".into()
        });
    }
    if let Some((status, _)) = app.labels.digest() {
        text.extend(model::pricing_details(status));
    }
    let mut lines: Vec<_> = text
        .into_iter()
        .flat_map(|text| {
            crate::board::notes::wrap(&crate::board::notes::sanitize(&text), usize::from(width))
        })
        .map(|text| Line::styled(text, app.look().role(Role::Text)))
        .collect();
    if narrow_activity && width >= 28 {
        let mut spans = vec![Span::styled("1d ", app.look().role(Role::Muted))];
        spans.extend(table::hourly(app, &metrics::activity(report, &ids), false));
        lines.insert(1, Line::from(spans));
    }
    lines
}
/// Clock evidence uses the same visible wrapped projection as paint.
pub(in crate::board) fn details_clock_mark(
    app: &App,
    target: &crate::board::app::RowTarget,
    width: u16,
    height: u16,
    now_ms: u64,
) -> String {
    let lines = detail_lines(app, width, now_ms, app.target_row(target));
    let viewport =
        crate::board::scroll::Scrolls::viewport(Rect::new(0, 0, width, height), lines.len());
    let offset = app
        .scrolls
        .offset(Pane::Replies)
        .min(lines.len().saturating_sub(viewport));
    lines
        .into_iter()
        .skip(offset)
        .take(viewport)
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
