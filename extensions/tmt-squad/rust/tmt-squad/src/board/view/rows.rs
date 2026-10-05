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
    let scene = RowPaint::build(
        rows,
        &layout,
        &cells,
        &app.items(),
        &extras,
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
}
