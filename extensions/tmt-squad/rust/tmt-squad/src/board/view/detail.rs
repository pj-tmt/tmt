//! Selected member fields and notebook presentation.

use crate::board::app::App;
use crate::board::notes::wrap;
use crate::config::Pane;
use ratatui::widgets::Paragraph;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use tmt_cli_style::Role;

/// Fields already shown by detail's header, body or links line.
pub(super) fn detail_represents(field: &str) -> bool {
    matches!(
        field,
        "member"
            | "state"
            | "task"
            | "pending"
            | "note"
            | "activity"
            | "presence"
            | "target"
            | "cwd"
            | "link"
    ) || field.ends_with("_link")
}

/// The selected row: where it is, what it is doing and what it waits on.
pub(super) fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(row) = app.selected_row() else {
        frame.render_widget(
            Paragraph::new(Span::styled("(no row selected)", look.role(Role::Dim))),
            area,
        );
        return;
    };
    let text = |value: &Value| value.as_str().unwrap_or("–").to_owned();
    let mut lines = vec![Line::from(Span::styled(
        text(&row["name"]),
        Style::new().add_modifier(Modifier::BOLD),
    ))];
    if let Some(pending) = row["pending"].as_str() {
        lines.push(Line::styled(
            format!("waiting on you: {pending}"),
            look.role(Role::Waiting),
        ));
    }
    let place = [&row["pane"]["target"], &row["pane"]["cwd"]]
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join(" · ");
    lines.push(Line::from(format!(
        "{} · {}{}",
        text(&row["presence"]),
        text(&row["state"]),
        if place.is_empty() {
            String::new()
        } else {
            format!(" · {place}")
        }
    )));
    for (label, value) in [
        ("task", &row["fields"]["task"]),
        ("activity", &row["activity"]["activity"]),
    ] {
        if let Some(value) = value.as_str() {
            lines.push(Line::from(format!("{label}: {value}")));
        }
    }
    let links: Vec<String> = row["fields"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| key.as_str() == "link" || key.ends_with("_link"))
        .filter_map(|(key, value)| Some(format!("{key} {}", value.as_str()?)))
        .collect();
    if !links.is_empty() {
        lines.push(Line::from(format!("links: {}", links.join("  "))));
    }
    if let Some(view) = &app.view {
        for column in &view.rows.columns {
            let field = &column.field;
            if detail_represents(field) {
                continue;
            }
            let failed = row["failed"]
                .as_array()
                .is_some_and(|failed| failed.iter().any(|name| name == field));
            let value = if failed {
                "?"
            } else {
                row["fields"][field]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .unwrap_or("–")
            };
            lines.push(Line::from(format!(
                "{field}: {}",
                tmt_cli_style::table::escape(value)
            )));
        }
    }
    let width = usize::from(area.width);
    let mut lines: Vec<Line> = lines
        .into_iter()
        .flat_map(|line| {
            let style = line.style;
            let text: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            wrap(&text, width)
                .into_iter()
                .map(move |part| Line::styled(part, style))
        })
        .collect();
    lines.push(Line::default());
    let label = "─ notebook ";
    let divider = format!(
        "{label}{}",
        "─".repeat(width.saturating_sub(label.chars().count()))
    );
    lines.push(Line::styled(
        divider.chars().take(width).collect::<String>(),
        look.role(Role::Dim),
    ));
    if row["lifetime"] != "saved" {
        lines.extend(
            wrap("(temporary identity: no notebook)", width)
                .into_iter()
                .map(|line| Line::styled(line, look.role(Role::Dim))),
        );
    } else if let Some(identity) = row["id"].as_str() {
        let render = app.view.as_ref().unwrap().render;
        lines.extend(
            app.notebooks
                .borrow_mut()
                .lines(identity, width, look, render),
        );
    }
    app.scrolls
        .show(frame, Pane::Detail, area, lines, look.role(Role::Dim));
}
