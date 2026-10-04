//! Projected row cells, selection, age labels and continuation hits.

use super::fit;
use crate::board::app::{App, Hit, Item};
use crate::{config::Pane, rows::Rows};
use ratatui::widgets::Paragraph;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use tmt_cli_style::{Role, grid::Truncate};
use unicode_width::UnicodeWidthStr;

/// Space between grid columns.
pub(super) const GAP: usize = 1;

/// One line of a row on the solved grid: each cell across its spanned
/// columns, fitted by the first column's alignment and truncation. The first
/// line shows `–` for a missing value; a later line with nothing to show is
/// left out.
pub(super) fn grid_line(
    look: crate::look::Look,
    rows: &Rows,
    layout: &crate::markup::Grid,
    admitted: &tmt_tui::binding::Node,
    row: &Value,
    line: usize,
    selected: bool,
) -> Option<Vec<Vec<Span<'static>>>> {
    let first = line == 0;
    let mut fitted = Vec::new();
    let mut position = 0;
    let mut shown_any = false;
    for (cell, admitted) in rows.lines[line]
        .iter()
        .zip(&admitted.children[line].children)
    {
        let range = position..position + cell.span;
        position += cell.span;
        let Some(box_width) = layout.span(range.clone()) else {
            continue;
        };
        let width = box_width.visible;
        let value = admitted.text.as_deref();
        shown_any |= value.is_some_and(|value| !value.is_empty());
        let text = match (value, &cell.field, first) {
            (Some(value), _, _) => value,
            (None, Some(_), true) => "–",
            _ => "",
        };
        let column = &rows.columns[range.start];
        let failed = cell.field.as_deref().is_some_and(|field| {
            row["failed"]
                .as_array()
                .is_some_and(|failed| failed.iter().any(|name| name == field))
        });
        // Admitted cell tokens override projected decoration; absent and
        // failed providers without projected colors stay quiet. Look owns selection.
        let token = cell
            .field
            .as_deref()
            .and_then(|field| row["colors"][field].as_str());
        let role = admitted
            .style
            .token
            .filter(|_| !failed || token.is_some())
            .or_else(|| token.and_then(crate::look::role));
        let style = if value.is_none_or(str::is_empty) || (failed && token.is_none()) {
            look.role(Role::Dim)
        } else if let Some(role) = role {
            look.role(role)
        } else {
            Style::new()
        };
        let emphasize = value.is_some_and(|value| !value.is_empty())
            && (matches!(cell.field.as_deref(), Some("state" | "pending"))
                || role.is_some_and(|role| matches!(role, Role::Waiting | Role::Blocked)));
        let style = look.row_span(selected, style, emphasize);
        fitted.push((
            crate::markup::fitted(text, box_width, admitted.style.text_flow, column.align),
            style,
            width,
        ));
    }
    if !first && !shown_any {
        return None;
    }
    let height = fitted
        .iter()
        .map(|(lines, _, _)| lines.len())
        .max()
        .unwrap_or(1);
    Some(
        (0..height)
            .map(|line| {
                let mut spans = Vec::new();
                for (values, style, width) in &fitted {
                    if !spans.is_empty() {
                        spans.push(Span::raw(" ".repeat(GAP)));
                    }
                    spans.push(Span::styled(
                        values
                            .get(line)
                            .cloned()
                            .unwrap_or_else(|| " ".repeat(*width)),
                        *style,
                    ));
                }
                spans
            })
            .collect(),
    )
}

/// Puts a row's age at the right edge of its first line when it fits after
/// the cells; a narrow board drops it before any cell.
pub(super) fn age_mark(
    spans: &mut Vec<Span<'static>>,
    age: &str,
    width: usize,
    look: crate::look::Look,
    selected: bool,
) {
    let used: usize = spans.iter().map(Span::width).sum();
    let mark = age.width();
    if used + GAP + mark <= width {
        spans.push(Span::raw(" ".repeat(width - used - mark)));
        spans.push(Span::styled(
            age.to_owned(),
            look.row_span(selected, look.role(Role::Dim), false),
        ));
    }
}
pub(super) fn render_rows(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(view) = &app.view else {
        let message = if app.error.is_some() {
            ""
        } else {
            "Loading…"
        };
        frame.render_widget(Paragraph::new(message), area);
        return;
    };
    let Some(tab) = app.shown_tab() else { return };
    let rows = &view.rows;
    let mut derived = view.derived.borrow_mut();
    let available = usize::from(area.width).saturating_sub(2);
    if derived
        .grid
        .as_ref()
        .is_none_or(|grid| grid.width != available || grid.search != app.search)
    {
        // Unsized columns start from their widest value on the board.
        let natural = |index: usize| {
            let field = &rows.columns[index].field;
            app.items()
                .into_iter()
                .filter_map(|item| match item {
                    Item::Row(row) => crate::markup::value(row, field),
                    Item::Header(_) => None,
                })
                // Content demand is unwrapped; measured width is a capped upper bound.
                .map(|value| tmt_cli_style::table::escape(value).width())
                .chain([rows.columns[index].title.width()])
                .max()
                .unwrap_or(0)
        };
        // Two cells for the row mark, and room at the right for the widest age
        // mark when a row has one, unless that would hide a column: then the
        // marks give way.
        let available = usize::from(area.width).saturating_sub(2);
        let layout = match crate::markup::Grid::compile(rows, natural, available) {
            Ok(layout) => layout,
            Err(error) => {
                frame.render_widget(Paragraph::new(format!("Row layout: {error}")), area);
                return;
            }
        };
        let ages = app
            .items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Row(row) => crate::staleness::label(&row["staleness"]),
                Item::Header(_) => None,
            })
            .map(|age| age.width() + GAP)
            .max();
        let layout = match ages {
            Some(age) => {
                match crate::markup::Grid::compile(rows, natural, available.saturating_sub(age)) {
                    Ok(reserved)
                        if reserved.columns.iter().flatten().count()
                            == layout.columns.iter().flatten().count() =>
                    {
                        reserved
                    }
                    _ => layout,
                }
            }
            None => layout,
        };
        let cells = match crate::markup::row_values(rows, tab, app.rows()) {
            Ok(cells) => cells,
            Err(error) => {
                frame.render_widget(Paragraph::new(format!("Row values: {error}")), area);
                return;
            }
        };
        derived.grid = Some(crate::board::derived::Grid {
            width: available,
            search: app.search.clone(),
            layout,
            cells,
        });
    }
    let layout = &derived.grid.as_ref().expect("prepared grid").layout;
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "  {}",
            rows.columns
                .iter()
                .enumerate()
                .filter_map(|(index, column)| {
                    layout.span(index..index + 1).map(|box_width| {
                        crate::markup::fitted(
                            &column.title,
                            box_width,
                            if column.truncate == Truncate::Middle {
                                tmt_tui::style::TextFlow::Middle
                            } else {
                                tmt_tui::style::TextFlow::Truncate
                            },
                            column.align,
                        )
                        .remove(0)
                    })
                })
                .collect::<Vec<_>>()
                .join(&" ".repeat(GAP))
        ),
        look.role(Role::Muted),
    ))];
    let mut selected_lines = 0..0;
    let mut row_index = 0;
    // The screen lines of each row, for mouse events.
    let mut row_lines = Vec::new();
    for item in app.items() {
        match item {
            Item::Header(title) => lines.push(Line::from(Span::styled(
                title.to_uppercase(),
                Style::new().add_modifier(Modifier::BOLD),
            ))),
            Item::Row(row) => {
                let selected = row_index == app.selected;
                let start = lines.len();
                app.row_starts.borrow_mut().push(start);
                let marker = if row["pending"].is_string() {
                    "◆ "
                } else {
                    "  "
                };
                // Its age mark: a row nobody updated for a while is quiet,
                // and says how long. This is the row's content age, not the
                // frame still loading another squad.
                let age = crate::staleness::label(&row["staleness"]);
                let style = if selected {
                    look.selection()
                } else if age.is_some() {
                    look.role(Role::Dim)
                } else {
                    Style::new()
                };
                for index in 0..rows.lines.len() {
                    let first = index == 0;
                    let Some(cells) = grid_line(
                        look,
                        rows,
                        layout,
                        &derived.grid.as_ref().expect("prepared grid").cells[row_index],
                        row,
                        index,
                        selected,
                    ) else {
                        continue;
                    };
                    for (visual, cells) in cells.into_iter().enumerate() {
                        let initial = first && visual == 0;
                        let mut spans = vec![Span::styled(
                            if initial { marker } else { "  " },
                            look.row_span(
                                selected,
                                Style::new(),
                                initial && row["pending"].is_string(),
                            ),
                        )];
                        spans.extend(cells);
                        if let Some(age) = age.as_deref().filter(|_| initial) {
                            age_mark(&mut spans, age, usize::from(area.width), look, selected);
                        }
                        row_lines.push((lines.len(), row_index));
                        lines.push(Line::from(spans).style(style));
                    }
                }
                if let Some(text) = row["annotation"]["text"].as_str() {
                    let to = row["annotation"]["to"].as_str().unwrap_or_default();
                    lines.push(Line::from(Span::styled(
                        fit(
                            &format!("    ✎ sent to {to}: {text}"),
                            usize::from(area.width),
                        ),
                        look.role(Role::Dim),
                    )));
                    row_lines.push((lines.len() - 1, row_index));
                }
                if selected {
                    selected_lines = start..lines.len();
                }
                row_index += 1;
            }
        }
    }
    if row_index == 0 {
        lines.push(Line::from(Span::styled(
            if app.search.is_empty() {
                "  (no members)"
            } else {
                "  (no matching members)"
            },
            look.role(Role::Dim),
        )));
    }
    // The selection stays on screen until the wheel moves the rows away from
    // it; the column header scrolls with the list.
    if app.follow {
        app.scrolls
            .reveal_range(Pane::Rows, selected_lines, area, lines.len());
    }
    let (offset, viewport) = app
        .scrolls
        .show(frame, Pane::Rows, area, lines, look.role(Role::Dim));
    app.hits.borrow_mut().extend(
        row_lines
            .into_iter()
            .filter(|(line, _)| (offset..offset + viewport).contains(line))
            .map(|(line, row)| Hit {
                y: area.y + (line - offset) as u16,
                x: area.x,
                width: area.width,
                row,
            }),
    );
}
