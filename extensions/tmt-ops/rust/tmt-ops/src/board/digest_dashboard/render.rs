use super::{metrics, table};
use crate::board::{app::App, exchange_matrix};
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
    let band_height = super::summary::height(app, area.width, area.height);
    super::summary::paint(
        frame,
        app,
        Rect {
            height: band_height,
            ..area
        },
    );
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
    let top = band_height + chart_height;
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
    let mut inside = crate::board::pane_chrome::paint(
        frame,
        app,
        area,
        Pane::Replies,
        &super::details::title(app),
    );
    if inside.is_empty() {
        return;
    }
    tabs(
        frame,
        app,
        Rect {
            height: 1,
            ..inside
        },
    );
    inside.y += 1;
    inside.height -= 1;
    // Impact ends with the way to its basis, kept in view below the scrolled amounts.
    if app.digest_details == super::details::Tab::Impact && inside.height > 2 {
        let row = Rect::new(inside.x, inside.bottom() - 1, inside.width, 1);
        if let Some(rect) = crate::board::buttons::paint(
            app,
            frame.buffer_mut(),
            row,
            &[crate::board::buttons::Button::secondary(BASIS)],
            tmt_cli_style::grid::Align::Left,
            None,
        )[0]
        {
            app.workspace_strip_hits
                .borrow_mut()
                .push((rect, crate::board::app::Choice::DigestBasis));
        }
        inside.height -= 1;
    }
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

const BASIS: &str = "Estimate basis…";

/// One chip per Details tab; the shown one is selected and marked `▸`, since
/// some looks tell a selected chip apart only by a nearby background shade.
fn tabs(frame: &mut Frame, app: &App, area: Rect) {
    use crate::board::{app::Choice, buttons::Button, digest_dashboard::details::Tab};
    let labels: Vec<String> = Tab::ALL
        .iter()
        .map(|tab| match *tab == app.digest_details {
            true => format!("▸ {}", tab.label()),
            false => tab.label().to_owned(),
        })
        .collect();
    let buttons: Vec<_> = Tab::ALL
        .iter()
        .zip(&labels)
        .map(|(tab, label)| Button::secondary(label).selected(*tab == app.digest_details))
        .collect();
    let painted = crate::board::buttons::paint(
        app,
        frame.buffer_mut(),
        area,
        &buttons,
        tmt_cli_style::grid::Align::Left,
        None,
    );
    for (tab, rect) in Tab::ALL.into_iter().zip(painted) {
        if let Some(rect) = rect {
            app.workspace_strip_hits
                .borrow_mut()
                .push((rect, Choice::DigestDetails(tab)));
        }
    }
}

pub(super) fn detail_lines(
    app: &App,
    width: u16,
    now_ms: u64,
    row: Option<&serde_json::Value>,
) -> Vec<Line<'static>> {
    let impact = app.digest_details == super::details::Tab::Impact;
    let narrow_activity = !table::activity_shown(app);
    let mut text = super::details::lines(app, now_ms, row);
    if impact && narrow_activity && width < 28 {
        text.push("Widen to 28 columns for 24 hourly cells".into());
    }
    let mut lines: Vec<_> = text
        .into_iter()
        .flat_map(|text| {
            crate::board::notes::wrap(&crate::board::notes::sanitize(&text), usize::from(width))
        })
        .map(|text| Line::styled(text, app.look().role(Role::Text)))
        .collect();
    // A table too narrow for the hourly strip shows it under the usage heading.
    if impact && narrow_activity && width >= 28 {
        let rows = app.workspace_rows();
        let ids = rows
            .iter()
            .find(|row| Some(&row.target) == app.workspace.selected.as_ref())
            .map_or_else(Vec::new, |row| metrics::identities(row, &rows));
        let mut spans = vec![Span::styled("1d ", app.look().role(Role::Muted))];
        spans.extend(table::hourly(
            app,
            &metrics::activity(metrics::report(app), &ids),
            false,
        ));
        lines.insert(1.min(lines.len()), Line::from(spans));
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

/// The status-row and Impact `Estimate basis…` buttons open the Evidence modal over the board.
pub(in crate::board) fn open_basis(app: &mut App) -> crate::board::app::Effect {
    app.open_estimate_basis()
}
