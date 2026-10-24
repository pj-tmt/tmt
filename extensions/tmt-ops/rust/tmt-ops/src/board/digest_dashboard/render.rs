use super::{metrics, model, table};
use crate::board::{app::App, exchange_matrix, usage_report::View};
use crate::config::Pane;
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
    super::band::paint(frame, app, area);
    let band_height = super::band::HEIGHT.min(area.height);
    let now = crate::status::now_ms();
    let chart_height = if app.usage_chart.borrow().focused()
        && app.digest_panels().shown(crate::config::DigestPanel::Chart)
    {
        crate::board::overview::chart::host::demand(
            app,
            area.width,
            area.height.saturating_sub(band_height),
            now,
        )
    } else {
        0
    };
    crate::board::overview::chart::host::render_panel(
        frame,
        app,
        Rect::new(
            area.x,
            area.y.saturating_add(band_height),
            area.width,
            chart_height,
        ),
        now,
    );
    let allowance_height = super::allowance::height(
        app,
        area.width,
        area.height.saturating_sub(band_height + chart_height),
    );
    super::allowance::paint(
        frame,
        app,
        Rect::new(
            area.x,
            area.y.saturating_add(band_height + chart_height),
            area.width,
            allowance_height,
        ),
    );
    let top = band_height + chart_height + allowance_height;
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
    let ranked = super::exchange_participants(app);
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
    let report = metrics::report(app);
    let empty = if ranked.is_empty() {
        Some("No agents observed")
    } else if report
        .and_then(|report| report.exchange.as_ref())
        .is_some_and(|traffic| traffic.maximum == 0)
    {
        Some("No exchanges in this range")
    } else {
        None
    };
    if let Some(text) = empty {
        strip::paint_left(
            frame.buffer_mut(),
            Rect {
                height: 1,
                ..inside
            },
            Line::styled(text, app.look().role(Role::Muted)),
        );
    } else {
        let _cells = exchange_matrix::paint(
            frame.buffer_mut(),
            inside,
            &ranked,
            selected,
            report.and_then(|report| report.exchange.as_ref()),
            app.look(),
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
pub(super) fn detail_lines(
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
    if app.labels.digest().is_none() {
        text.push("Digest not available".into());
        text.push("Install: tmt extension install digest".into());
    }
    text.push(format!("Impact · {}", app.digest_window().label()));
    let summary = metrics::savings(app, &ids);
    text.extend(
        super::band::fields(&summary, app.digest_window())
            .map(|line| line.replacen("List-price equiv.", "List-price equivalent:", 1)),
    );
    if let Some(points) = summary.percent.value {
        text.push(format!(
            "Estimated input reduction: {} ({}/{} agents)",
            model::detail_percent(points),
            summary.percent.included,
            summary.total
        ));
    } else if summary.percent.conflict {
        text.push("Estimated input reduction: — · intervals differ".into());
    }
    let price_column_shown = table::price_column_shown(app);
    if let Some(cost) = metrics::input_price(app, report, &ids) {
        text.push(format!(
            "Observed input quote: {}",
            crate::board::savings::usd_text(cost)
        ));
    } else if !price_column_shown {
        text.push("Observed input quote: —".into());
    }
    if !price_column_shown {
        text.push("Input tokens at list price; not a bill.".into());
    }
    text.push(super::band::DEFINITION.into());
    if let Some(start) = crate::board::savings::savings_start(
        &summary,
        app.digest_window(),
        app.local_zone.as_ref().unwrap_or(&jiff::tz::TimeZone::UTC),
        usize::from(width),
    ) {
        text.push(start);
    }
    text.push("Estimate basis".into());
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

/// Open the already-held Details evidence at its wrapped line, not a guessed offset.
pub(in crate::board) fn open_basis(app: &mut App) -> crate::board::app::Effect {
    let width = app
        .conversation
        .borrow()
        .pane
        .map(crate::board::pane_chrome::content_area)
        .map_or(1, |area| area.width);
    let lines = detail_lines(app, width, crate::status::now_ms(), app.selected_row());
    let offset = lines
        .iter()
        .position(|line| line.to_string() == "Estimate basis")
        .unwrap_or(0);
    app.focus = 0;
    app.scrolls.restore([(Pane::Replies, offset)]);
    app.invalidate_frame_geometry();
    if app.digest_panels().side_shown() {
        crate::board::app::Effect::None
    } else {
        // The existing Details toggle owns revealing a saved hidden pane, including CAS refusal.
        app.perform(
            &crate::action::Action::parse("monitor-details").expect("existing Details action"),
        )
    }
}
