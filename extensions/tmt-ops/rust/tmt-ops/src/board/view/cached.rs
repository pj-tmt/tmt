//! The first frame from a stored display: inert text, never a selectable view.
//! It names its age through the shared status slot until fresh data replaces it.

use crate::board::{app::App, snapshot_cache::Display};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use serde_json::Value;
use std::time::Duration;
use tmt_cli_style::Role;
use tmt_tui::components::{StatusLabel, StatusSlot, strip};

/// The summary row: spinner while updating, and how old the shown rows are.
pub(super) fn status(frame: &mut Frame, app: &App, display: &Display, area: Rect) {
    let look = app.look();
    let age = crate::status::now_ms().saturating_sub(display.written_at_ms());
    let prefix = format!("{} · cached", crate::tabs::label(display.tab()));
    StatusSlot {
        label: StatusLabel::Age {
            prefix: &prefix,
            age: Duration::from_millis(age),
        },
        frame: super::spinner_frame(app, std::time::Instant::now()),
    }
    .paint(frame.buffer_mut(), area, &look.theme, look.depth);
}

pub(super) fn body(frame: &mut Frame, app: &App, display: &Display, area: Rect) {
    let look = app.look();
    for (line, y) in lines(&display.0["view"], usize::from(area.width))
        .into_iter()
        .zip(area.top()..area.bottom())
    {
        let line = Line::from(
            line.into_iter()
                .map(|(text, role)| Span::styled(text, look.role(role)))
                .collect::<Vec<_>>(),
        );
        strip::paint_left(
            frame.buffer_mut(),
            Rect {
                y,
                height: 1,
                ..area
            },
            line,
        );
    }
}

type Piece = (String, Role);

fn mark(row: &Value) -> Piece {
    let (mark, role) = if row["waiting"] == true {
        ("◆", Role::Waiting)
    } else {
        match row["state"].as_str() {
            Some("blocked") => ("✗", Role::Blocked),
            Some("review") => ("◐", Role::Review),
            Some("working") => ("●", Role::Working),
            _ => ("○", Role::Dim),
        }
    };
    (mark.into(), role)
}

fn text(value: &Value) -> String {
    tmt_cli_style::table::escape(value.as_str().unwrap_or_default())
}

/// Rows and HOME summaries as styled pieces, cut to `width` by the strip painter.
pub(super) fn lines(view: &Value, width: usize) -> Vec<Vec<Piece>> {
    let mut lines = Vec::new();
    if let Some(home) = view["home"].as_object() {
        let squads = home["squads"].as_array().into_iter().flatten();
        let name = squads
            .clone()
            .map(|line| text(&line["squad"]).chars().count())
            .max()
            .unwrap_or(0)
            .min(24);
        for line in squads {
            let counts = &line["counts"];
            let mut pieces = vec![(
                format!(" {}  ", super::fit(&text(&line["squad"]), name)),
                Role::Text,
            )];
            for (mark, key, role) in [
                ("◆", "waiting", Role::Waiting),
                ("✗", "blocked", Role::Blocked),
                ("●", "working", Role::Working),
                ("○", "idle", Role::Dim),
            ] {
                let count = counts[key].as_u64().unwrap_or(0);
                pieces.push((mark.into(), if count == 0 { Role::Dim } else { role }));
                pieces.push((format!(" {count}  "), Role::Text));
            }
            if let Some(lead) = line["lead"].as_str() {
                pieces.push((
                    format!("lead {}", tmt_cli_style::table::escape(lead)),
                    Role::Muted,
                ));
            }
            lines.push(pieces);
        }
        for section in home["sections"].as_array().into_iter().flatten() {
            let rows = section["rows"].as_array().map_or(&[][..], Vec::as_slice);
            if rows.is_empty() {
                continue;
            }
            let (title, mark, role) = match section["key"].as_str() {
                Some("needs-you") => ("Needs you", "◆", Role::Waiting),
                _ => ("Blocked", "✗", Role::Blocked),
            };
            lines.push(Vec::new());
            lines.push(vec![(format!(" {title}"), Role::Muted)]);
            for row in rows {
                lines.push(vec![
                    (format!(" {mark} "), role),
                    (text(&row["name"]), Role::Text),
                    (format!(" · {}", text(&row["squad"])), Role::Muted),
                ]);
            }
        }
        return lines;
    }
    let rows = || {
        view["lead"]
            .as_object()
            .map(|_| &view["lead"])
            .into_iter()
            .chain(
                view["sections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|section| section["rows"].as_array().into_iter().flatten()),
            )
    };
    let name = rows()
        .map(|row| text(&row["name"]).chars().count())
        .max()
        .unwrap_or(0)
        .min(24);
    let state = rows()
        .map(|row| text(&row["state"]).chars().count())
        .max()
        .unwrap_or(0)
        .min(12);
    let row_line = |row: &Value| {
        let (mark, role) = mark(row);
        let mut pieces = vec![
            (format!(" {mark} "), role),
            (super::fit(&text(&row["name"]), name), Role::Text),
        ];
        if state > 0 {
            pieces.push((
                format!("  {}", super::fit(&text(&row["state"]), state)),
                Role::Muted,
            ));
        }
        let task = text(&row["task"]);
        if !task.is_empty() && width > name + state + 7 {
            pieces.push((format!("  {task}"), Role::Text));
        }
        pieces
    };
    if view["lead"].is_object() {
        lines.push(row_line(&view["lead"]));
    }
    for section in view["sections"].as_array().into_iter().flatten() {
        if let Some(title) = section["title"].as_str().filter(|title| !title.is_empty()) {
            if !lines.is_empty() {
                lines.push(Vec::new());
            }
            lines.push(vec![(
                format!(" {}", tmt_cli_style::table::escape(title)),
                Role::Muted,
            )]);
        }
        for row in section["rows"].as_array().into_iter().flatten() {
            lines.push(row_line(row));
        }
    }
    lines
}

#[cfg(test)]
mod tests;
