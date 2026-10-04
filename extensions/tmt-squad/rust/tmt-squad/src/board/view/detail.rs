//! Selected member fields and notebook presentation.

use crate::board::app::App;
use crate::board::notes::wrap;
use crate::config::Pane;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use tmt_cli_style::Role;
use tmt_tui::components::strip;

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
        strip::paint_left(
            frame.buffer_mut(),
            area,
            Line::from(Span::styled("(no row selected)", look.role(Role::Dim))),
        );
        return;
    };
    if app.selected_is_lead() {
        render_lead(frame, app, area, row);
        return;
    }
    let text = |value: &Value| value.as_str().unwrap_or("–").to_owned();
    let mut lines = vec![Line::from(Span::styled(
        text(&row["name"]),
        Style::new().add_modifier(Modifier::BOLD),
    ))];
    if let Some(pending) = super::waiting::text(row) {
        lines.push(Line::styled(
            format!("waiting on you: {pending}"),
            look.role(Role::Waiting),
        ));
    }
    if crate::attention::waits_on_you(row) {
        let bindings = app.bindings();
        let key = |verb| {
            bindings
                .get("enter")
                .filter(|action| action.verb == verb && action.args.is_empty())
                .map(|_| "⏎")
                .or_else(|| {
                    bindings
                        .iter()
                        .find(|(key, action)| {
                            !matches!(key.as_str(), "click" | "double-click")
                                && action.verb == verb
                                && action.args.is_empty()
                        })
                        .map(|(key, _)| key.as_str())
                })
        };
        let hints = [
            (crate::action::Verb::Reply, "reply"),
            (crate::action::Verb::Jump, "jump"),
        ]
        .into_iter()
        .filter_map(|(verb, label)| key(verb).map(|key| format!("{key} {label}")))
        .collect::<Vec<_>>()
        .join(" · ");
        if !hints.is_empty() {
            lines.push(Line::styled(hints, look.role(Role::Muted)));
        }
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
    if let Some(next) = row["id"]
        .as_str()
        .and_then(|id| app.cron.member_detail(id, app.cron.now_ms()))
    {
        lines.push(Line::from(format!("cron: {next}")));
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
    app.scrolls.show(frame, Pane::Detail, area, lines, look);
}

/// Lead context stays in its row; its notebook and finals have their own panes.
fn render_lead(frame: &mut Frame, app: &App, area: Rect, row: &Value) {
    let look = app.look();
    let text = |value: &Value| {
        value
            .as_str()
            .filter(|value| !value.is_empty())
            .map(tmt_cli_style::table::escape)
    };
    let name = text(&row["name"]).unwrap_or_default();
    let heading = Line::from(vec![
        Span::styled(name.clone(), Style::new().add_modifier(Modifier::BOLD)),
        Span::styled("  lead", look.role(Role::Dim)),
    ]);
    let mut lines = Vec::new();
    let status: Vec<_> = [
        &row["state"],
        &row["fields"]["model"],
        &row["fields"]["cap"],
    ]
    .into_iter()
    .filter_map(text)
    .collect();
    if !status.is_empty() {
        lines.push(Line::from(status.join(" · ")));
    }
    if let Some(task) = text(&row["fields"]["task"]) {
        lines.push(Line::from(format!("task: {task}")));
    }
    if let Some(pending) = text(&row["pending"]) {
        lines.push(Line::styled(
            format!("waits on you: {pending}"),
            look.role(Role::Waiting),
        ));
    }
    let links: Vec<_> = row["fields"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| key.as_str() == "link" || key.ends_with("_link"))
        .filter_map(|(key, value)| text(value).map(|value| format!("{key} {value}")))
        .collect();
    if !links.is_empty() {
        lines.push(Line::from(format!("links: {}", links.join("  "))));
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            format!("no row fields set · tmt sq set {name} task=…"),
            look.role(Role::Dim),
        ));
    }
    lines.push(Line::styled(
        "notes below · replies at right",
        look.role(Role::Dim),
    ));
    let mut wrapped = vec![heading];
    wrapped.extend(lines.into_iter().flat_map(|line| {
        let style = line.style;
        wrap(&line.to_string(), usize::from(area.width))
            .into_iter()
            .map(move |part| Line::styled(part, style))
    }));
    app.scrolls.show(frame, Pane::Detail, area, wrapped, look);
}
