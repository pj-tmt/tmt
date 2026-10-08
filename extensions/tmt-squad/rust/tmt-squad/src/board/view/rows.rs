//! The rows pane: prepares the cached scene and hands it to `Scrolls` and the
//! shared painter. Cells, ages and hits are built by `row_paint`.

use super::row_paint::{Extra, GAP, RowPaint, row_end};
use crate::board::{app::App, derived};
use crate::display_rows::{Item, RowOrigin};
use crate::{config::Pane, rows::Rows};
use ratatui::{Frame, layout::Rect, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

/// Layout, admitted cells and the paint scene for one width, search and set of
/// clock labels. A failure leaves the previous cache untouched.
fn prepare(
    app: &App,
    rows: &Rows,
    tab: &str,
    area: Rect,
    extras: Vec<Extra>,
) -> Result<derived::Grid, String> {
    let available = usize::from(area.width).saturating_sub(2);
    // Unsized columns start from their widest value on the board.
    let natural = |index: usize| {
        let field = &rows.columns[index].field;
        app.items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Row(_, row) => crate::markup::value(row, field),
                Item::Section(_) | Item::Rule(_) => None,
            })
            // Content demand is unwrapped; measured width is a capped upper bound.
            .map(|value| tmt_cli_style::table::escape(value).width())
            .chain([rows.columns[index].title.width()])
            .max()
            .unwrap_or(0)
    };
    // Two cells for the row mark.
    let layout = crate::markup::Grid::compile(rows, natural, available)
        .map_err(|error| format!("Row layout: {error}"))?;
    // The row-end label room is reserved only when no column would be hidden:
    // first for the age mark plus `cron next`, then for the age mark alone.
    let ages: Vec<_> = app
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::Row(_, row) => Some(crate::staleness::label(&row["staleness"])),
            Item::Section(_) | Item::Rule(_) => None,
        })
        .collect();
    let widest = |label: &dyn Fn(&Option<String>, &Extra) -> Option<String>| {
        ages.iter()
            .zip(&extras)
            .filter_map(|(age, extra)| label(age, extra))
            .map(|label| label.width() + GAP)
            .max()
    };
    let with_next = |age: &Option<String>, extra: &Extra| {
        row_end(age.clone(), extra.next.clone()).into_iter().next()
    };
    let age_only = |age: &Option<String>, _: &Extra| age.clone();
    let shown = layout.columns.iter().flatten().count();
    let layout = [widest(&with_next), widest(&age_only)]
        .into_iter()
        .flatten()
        .find_map(|reserve| {
            crate::markup::Grid::compile(rows, natural, available.saturating_sub(reserve))
                .ok()
                .filter(|reserved| reserved.columns.iter().flatten().count() == shown)
        })
        .unwrap_or(layout);
    let cells = crate::markup::row_values(rows, tab, app.rows())
        .map_err(|error| format!("Row values: {error}"))?;
    // Visibility is per occurrence: a surviving track may still cut this row's
    // text. Mirror the painter's request-age reservation and bounded wrapping.
    let mut painted_extras = extras.clone();
    for (index, (extra, (_, row))) in painted_extras.iter_mut().zip(app.rows()).enumerate() {
        let mut visible = Vec::new();
        for (line, configured) in rows.lines.iter().enumerate() {
            let mut position = 0;
            for (cell, node) in configured.iter().zip(&cells[index].children[line].children) {
                let range = position..position + cell.span;
                position += cell.span;
                let Some(field) = cell.field.as_deref() else {
                    continue;
                };
                let Some(budget) = layout.span(range.clone()) else {
                    continue;
                };
                let pending = field == "pending";
                let text = if pending {
                    super::waiting::text(row)
                } else {
                    node.text.as_deref()
                };
                let (width, flow) = if line == 0 && extra.focus.is_some() {
                    let heading = super::row_paint::heading_labels(
                        ages[index].clone(),
                        extra.next.clone(),
                        extra.focus.as_deref(),
                        usize::from(area.width),
                    );
                    let end = usize::from(area.width)
                        .saturating_sub(heading.first().map_or(0, |label| label.width() + GAP));
                    let start = 2
                        + tmt_cli_style::grid::span(&layout.columns, 0..range.start, GAP)
                        + usize::from(layout.columns[..range.start].iter().any(Option::is_some));
                    (
                        budget.visible.min(end.saturating_sub(start)),
                        tmt_tui::style::TextFlow::Truncate,
                    )
                } else if pending && line > 0 {
                    (
                        budget.visible.saturating_sub(
                            extra
                                .request_age
                                .as_deref()
                                .map_or(0, |age| age.width() + GAP),
                        ),
                        tmt_tui::style::TextFlow::Truncate,
                    )
                } else {
                    (usize::from(budget.text), node.style.text_flow)
                };
                if text.is_some_and(|text| {
                    crate::board::row_detail::uncut(text, width, flow)
                        && (pending && line > 0
                            || !budget.cut
                            || tmt_tui::text::fit_lines(
                                text,
                                budget.text,
                                flow,
                                rows.columns[position - cell.span].align,
                            )
                            .iter()
                            .all(|line| {
                                crate::board::row_detail::uncut(
                                    line.trim_end(),
                                    budget.visible,
                                    tmt_tui::style::TextFlow::Truncate,
                                )
                            }))
                }) {
                    visible.push(field);
                }
            }
        }
        if !rows
            .lines
            .iter()
            .flatten()
            .any(|cell| cell.field.as_deref() == Some("pending"))
        {
            let room = usize::from(area.width).saturating_sub(
                4 + extra
                    .request_age
                    .as_deref()
                    .map_or(0, |age| age.width() + GAP),
            );
            if super::waiting::text(row).is_some_and(|text| {
                crate::board::row_detail::uncut(text, room, tmt_tui::style::TextFlow::Truncate)
            }) {
                visible.push("pending");
            }
        }
        if extra
            .focus
            .as_ref()
            .is_some_and(|text| text.width() <= available / 2)
        {
            visible.push("focus");
        }
        extra.detail = app.detail_value(index, &visible);
    }
    let scene = RowPaint::build(
        rows,
        &layout,
        &cells,
        &app.items(),
        &painted_extras,
        usize::from(area.width),
        if app.search.is_empty() {
            "  (no members)"
        } else {
            "  (no matching members)"
        },
    );
    Ok(derived::Grid {
        width: available,
        search: app.search.clone(),
        extras,
        scene,
        #[cfg(test)]
        layout,
        #[cfg(test)]
        cells,
    })
}

pub(super) fn render_rows(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(view) = &app.view else {
        return;
    };
    if app.effective_board().is_some_and(|board| board.members) {
        super::member_list::render_squad(frame, app, area);
        return;
    }
    let Some(tab) = app.shown_tab() else { return };
    let rows = &view.rows;
    let mut derived = view.derived.borrow_mut();
    let available = usize::from(area.width).saturating_sub(2);
    let now = app.cron.now_ms();
    let request_now = crate::status::now_ms();
    let extras: Vec<Extra> = app
        .items()
        .into_iter()
        .filter(|item| matches!(item, Item::Row(..)))
        .enumerate()
        .map(|(index, item)| {
            let Item::Row(origin, row) = item else {
                unreachable!()
            };
            Extra {
                lead: origin == RowOrigin::Lead,
                focus: crate::focus::label(row, request_now),
                detail: app.detail_value(index, &[]),
                next: row["id"]
                    .as_str()
                    .and_then(|id| app.cron.member_label(id, now)),
                request_age: super::waiting::age(row, request_now),
                sent: app.sent.as_ref().is_some_and(|feedback| {
                    feedback.sent && app.row_target(index).as_ref() == Some(&feedback.target)
                }),
                reserve: super::waiting::reserved_lines(app, index, area).unwrap_or(0),
            }
        })
        .collect();
    if derived.grid.as_ref().is_none_or(|grid| {
        grid.width != available || grid.search != app.search || grid.extras != extras
    }) {
        match prepare(app, rows, tab, area, extras) {
            Ok(grid) => derived.grid = Some(grid),
            Err(message) => {
                strip::paint_left(
                    frame.buffer_mut(),
                    area,
                    Line::styled(message, look.role(Role::Muted)),
                );
                return;
            }
        }
    }
    let scene = &derived.grid.as_ref().expect("prepared grid").scene;
    app.row_starts.borrow_mut().extend(&scene.starts);
    // The selection stays on screen until the wheel moves the rows away from
    // it; the column header scrolls with the list.
    if app.follow {
        let selected = scene
            .starts
            .get(app.selected)
            .zip(scene.ends.get(app.selected))
            .map_or(0..0, |(start, end)| *start..*end);
        app.scrolls
            .reveal_range(Pane::Rows, selected, area, scene.height);
    }
    let mut hits = Vec::new();
    let (offset, viewport) = app.scrolls.show_paint(
        frame,
        Pane::Rows,
        area,
        scene.height,
        look,
        |frame, body, offset| {
            hits = scene.paint(frame.buffer_mut(), body, offset, app.selected, look);
        },
    );
    super::waiting::place_input(app, scene.input.clone(), area, offset, viewport);
    app.hits.borrow_mut().extend(hits);
    for (row, start, data) in &scene.details {
        crate::board::row_detail::more_hit(
            app, *row, data, area.width, 2, *start, area, offset, viewport,
        );
    }
}
