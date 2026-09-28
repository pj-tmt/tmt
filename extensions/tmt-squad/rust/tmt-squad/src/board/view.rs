//! Drawing only: the board's layout from `App`, with display-width-aware
//! cells so wide characters never misalign columns.

use super::app::{App, Item};
use crate::config::Column;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

const HINTS: &str = "⏎ jump  t talk  r reply  o open  y copy  / search  ←→ squad  ? more  q quit";
const HELP: &[&str] = &[
    "↑ ↓ / j k   select a row",
    "← →         switch squad",
    "/           search; Esc clears",
    "?           this help",
    "q, Esc      close the board",
    "",
    "Enter, t, r, a, o, y, n and Tab act on rows in a later version.",
];

fn color(name: &str) -> Style {
    let style = Style::new();
    match name {
        "dim" => style.fg(Color::DarkGray),
        "red" => style.fg(Color::Red),
        "amber" => style.fg(Color::Yellow),
        "green" => style.fg(Color::Green),
        "cyan" => style.fg(Color::Cyan),
        "blue" => style.fg(Color::Blue),
        "magenta" => style.fg(Color::Magenta),
        _ => style,
    }
}

/// Exactly `width` display cells: truncated with an ellipsis, or padded.
pub fn fit(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    let total: usize = text.chars().map(|c| c.width().unwrap_or(0)).sum();
    let limit = if total > width {
        width.saturating_sub(1)
    } else {
        width
    };
    for character in text.chars() {
        let cells = character.width().unwrap_or(0);
        if used + cells > limit {
            break;
        }
        out.push(character);
        used += cells;
    }
    if total > width && width > 0 {
        out.push('…');
        used += 1;
    }
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

fn cell_text<'a>(row: &'a Value, field: &str) -> &'a str {
    let value = if field == "member" {
        &row["name"]
    } else {
        &row["fields"][field]
    };
    value.as_str().unwrap_or("–")
}

/// Fixed widths first; columns without a width share what remains.
fn widths(columns: &[Column], total: usize) -> Vec<usize> {
    let gaps = columns.len().saturating_sub(1);
    let fixed: usize = columns
        .iter()
        .filter_map(|c| c.width.map(usize::from))
        .sum();
    let flexible = columns.iter().filter(|c| c.width.is_none()).count().max(1);
    let share = total.saturating_sub(2 + gaps + fixed) / flexible;
    columns
        .iter()
        .map(|column| column.width.map_or(share.max(4), usize::from))
        .collect()
}

fn header_line(app: &App) -> Line<'_> {
    let mut spans = vec![Span::styled(
        "squad  ",
        Style::new().add_modifier(Modifier::BOLD),
    )];
    for squad in &app.squads {
        if Some(squad) == app.current.as_ref() {
            spans.push(Span::styled(
                format!("[{squad}]"),
                Style::new().add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(squad.clone(), color("dim")));
        }
        spans.push(Span::raw("  "));
    }
    if let Some(view) = &app.view {
        let lead = view.document["squad"]["lead"]["name"]
            .as_str()
            .unwrap_or("no lead");
        // A member can appear in several user sections; count people once.
        let count = view.document["sections"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|section| section["rows"].as_array().into_iter().flatten())
            .filter_map(|row| row["name"].as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        spans.push(Span::styled(
            format!("  {lead} · {count} members"),
            color("dim"),
        ));
    }
    Line::from(spans)
}

pub fn render(frame: &mut Frame, app: &App) {
    let [top, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(Paragraph::new(header_line(app)), top);
    render_rows(frame, app, body);
    let footer_line = if app.searching {
        Line::from(format!("/{}▏", app.search))
    } else if let Some(notice) = app.notice {
        Line::from(Span::styled(notice, color("amber")))
    } else if let Some(error) = &app.error {
        Line::from(Span::styled(error.as_str(), color("red")))
    } else {
        Line::from(Span::styled(HINTS, color("dim")))
    };
    frame.render_widget(Paragraph::new(footer_line), footer);
    if app.help {
        let height = (HELP.len() as u16 + 2).min(body.height);
        let area = Rect { height, ..body };
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(
                HELP.iter()
                    .map(|line| Line::from(*line))
                    .collect::<Vec<_>>(),
            ),
            area,
        );
    }
}

fn render_rows(frame: &mut Frame, app: &App, area: Rect) {
    let Some(view) = &app.view else {
        let message = if app.error.is_some() {
            ""
        } else {
            "Loading…"
        };
        frame.render_widget(Paragraph::new(message), area);
        return;
    };
    let columns = &view.columns;
    let widths = widths(columns, usize::from(area.width));
    let note_column = columns.iter().any(|column| column.field == "note");
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "  {}",
            columns
                .iter()
                .zip(&widths)
                .map(|(column, width)| fit(&column.title, *width))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        color("dim"),
    ))];
    let mut selected_line = 0;
    let mut row_index = 0;
    for item in app.items() {
        match item {
            Item::Header(title) => lines.push(Line::from(Span::styled(
                title.to_uppercase(),
                Style::new().add_modifier(Modifier::BOLD),
            ))),
            Item::Row(row) => {
                let selected = row_index == app.selected;
                if selected {
                    selected_line = lines.len();
                }
                let marker = if row["pending"].is_string() {
                    "◆ "
                } else {
                    "  "
                };
                let mut spans = vec![Span::raw(marker)];
                for (index, (column, width)) in columns.iter().zip(&widths).enumerate() {
                    if index > 0 {
                        spans.push(Span::raw(" "));
                    }
                    let text = cell_text(row, &column.field);
                    let style = if column.field == "state" {
                        color(view.colors.get(text).map_or("default", String::as_str))
                    } else {
                        Style::new()
                    };
                    spans.push(Span::styled(fit(text, *width), style));
                }
                let style = if selected {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    Style::new()
                };
                lines.push(Line::from(spans).style(style));
                if let Some(note) = row["note"].as_str().filter(|_| !note_column) {
                    lines.push(Line::from(Span::styled(
                        fit(&format!("    note {note}"), usize::from(area.width)),
                        color("dim"),
                    )));
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
            color("dim"),
        )));
    }
    // Keep the selected row visible; the column header scrolls with the list.
    let height = usize::from(area.height);
    let offset = selected_line.saturating_sub(height.saturating_sub(1));
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::app::{Snapshot, View};
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn columns() -> Vec<Column> {
        [
            ("member", "MEMBER", Some(10)),
            ("state", "STATE", Some(8)),
            ("task", "TASK", None),
        ]
        .into_iter()
        .map(|(field, title, width)| Column {
            field: field.into(),
            title: title.into(),
            width,
        })
        .collect()
    }

    fn draw(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                let mut line = String::new();
                let mut x = 0;
                while x < width {
                    let symbol = buffer[(x, y)].symbol().to_owned();
                    x += symbol
                        .chars()
                        .map(|c| c.width().unwrap_or(0) as u16)
                        .sum::<u16>()
                        .max(1);
                    line.push_str(&symbol);
                }
                line.trim_end().to_owned()
            })
            .collect()
    }

    fn board(sections: Value) -> App {
        let mut app = App::new(Some("product".into()));
        app.apply(Snapshot {
            squads: vec!["product".into(), "reviews".into()],
            squad: Some("product".into()),
            view: Ok(View {
                document: json!({"squad": {"name": "product", "lead": {"name": "sol"}}, "sections": sections}),
                columns: columns(),
                colors: BTreeMap::from([("blocked".into(), "amber".into())]),
            }),
        });
        app
    }

    fn row(name: &str, state: &str, task: &str, extra: Value) -> Value {
        let mut row = json!({"name": name, "fields": {"state": state, "task": task}, "pending": null, "note": null});
        for (key, value) in extra.as_object().unwrap() {
            row[key] = value.clone();
        }
        row
    }

    #[test]
    fn rows_show_pending_marker_notes_sections_and_aligned_wide_text() {
        let app = board(json!([
            {"title": "Needs me", "rows": [row("auth-fix", "blocked", "rotate session tokens", json!({"pending": "approve", "note": "needs a call"}))]},
            {"title": "Everyone", "rows": [row("文件-sweep", "working", "整理安装指南", json!({}))]}
        ]));
        let screen = draw(&app, 48, 9);
        assert_eq!(screen[0], "squad  [product]  reviews    sol · 2 members");
        assert_eq!(screen[1], "  MEMBER     STATE    TASK");
        assert_eq!(screen[2], "NEEDS ME");
        assert_eq!(screen[3], "◆ auth-fix   blocked  rotate session tokens");
        assert_eq!(screen[4], "    note needs a call");
        assert_eq!(screen[5], "EVERYONE");
        assert_eq!(screen[6], "  文件-sweep working  整理安装指南");
        assert!(screen[8].starts_with("⏎ jump"));
    }

    #[test]
    fn fit_truncates_by_display_width_and_selection_scrolls_into_view() {
        assert_eq!(fit("整理安装指南", 7), "整理安…");
        assert_eq!(fit("abc", 5), "abc  ");
        assert_eq!(fit("abcdef", 4), "abc…");
        let rows: Vec<Value> = (0..20)
            .map(|i| row(&format!("m{i:02}"), "working", "", json!({})))
            .collect();
        let mut app = board(json!([{"title": null, "rows": rows}]));
        app.selected = 15;
        let screen = draw(&app, 40, 8);
        assert!(screen.iter().any(|line| line.contains("m15")), "{screen:?}");
        assert!(!screen.iter().any(|line| line.contains("m00")));
    }

    #[test]
    fn search_help_and_errors_use_the_footer_and_overlay() {
        let mut app = board(json!([{"title": null, "rows": [row("a", "idle", "", json!({}))]}]));
        app.searching = true;
        app.search = "zz".into();
        let screen = draw(&app, 40, 6);
        assert!(
            screen
                .iter()
                .any(|line| line.contains("(no matching members)"))
        );
        assert_eq!(screen[5], "/zz▏");
        app.searching = false;
        app.search.clear();
        app.help = true;
        assert!(
            draw(&app, 60, 12)
                .iter()
                .any(|line| line.contains("switch squad"))
        );
        let mut failed = App::new(Some("product".into()));
        failed.error = Some("tmt did not finish in time".into());
        assert_eq!(draw(&failed, 40, 4)[3], "tmt did not finish in time");
    }
}
