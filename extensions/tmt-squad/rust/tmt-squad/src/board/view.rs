//! Drawing only: the board's layout from `App`, with display-width-aware
//! cells so wide characters never misalign columns.

use super::{
    app::{App, Hit, Item, Notes},
    markdown,
    notes::wrap,
};
use crate::config::{BoardMode, Column, Direction, NotesRender, Pane};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

const KEYS: &[&str] = &[
    "↑ ↓ / j k   select a row (↑ ↓ scroll a focused notes pane)",
    "← →         switch squad",
    "/           search; Esc clears",
    "?           this help",
    "q, Esc      close the board",
    "",
];

/// The footer names what the most used keys do for the selected row.
fn hints(app: &App) -> String {
    let bindings = app.bindings();
    let mut hints: Vec<String> = [("enter", "⏎"), ("o", "o"), ("y", "y"), ("tab", "tab")]
        .into_iter()
        .filter_map(|(event, label)| {
            let action = bindings.get(event)?;
            Some(format!("{label} {}", action.verb.name()))
        })
        .collect();
    hints.extend(["/ search", "←→ squad", "? more", "q quit"].map(str::to_owned));
    hints.join("  ")
}

/// Fixed keys, then every binding for the selected row.
fn help_lines(app: &App) -> Vec<String> {
    let mut lines: Vec<String> = KEYS.iter().map(|line| (*line).to_owned()).collect();
    for (event, action) in app.bindings() {
        lines.push(format!("{event:<11} {}", action.text));
    }
    lines
}

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
    app.hits.borrow_mut().clear();
    let [top, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(Paragraph::new(header_line(app)), top);
    render_body(frame, app, body);
    let footer_line = if app.searching {
        Line::from(format!("/{}▏", app.search))
    } else if let Some(notice) = &app.notice {
        Line::from(Span::styled(notice.as_str(), color("amber")))
    } else if let Some(error) = &app.error {
        Line::from(Span::styled(error.as_str(), color("red")))
    } else {
        Line::from(Span::styled(hints(app), color("dim")))
    };
    frame.render_widget(Paragraph::new(footer_line), footer);
    if app.help {
        let lines = help_lines(app);
        let height = (lines.len() as u16).min(body.height);
        let area = Rect { height, ..body };
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>()),
            area,
        );
    }
    if let Some(menu) = &app.menu {
        let height = (menu.entries.len() as u16 + 2).min(body.height);
        let width = body.width.min(48);
        let area = Rect {
            x: body.x + (body.width - width) / 2,
            y: body.y + (body.height - height) / 2,
            width,
            height,
        };
        let lines: Vec<Line> = menu
            .entries
            .iter()
            .enumerate()
            .map(|(index, (event, action))| {
                let line = Line::from(fit(
                    &format!(" {event:<9} {}", action.text),
                    usize::from(width.saturating_sub(2)),
                ));
                if index == menu.selected {
                    line.style(Style::new().add_modifier(Modifier::REVERSED))
                } else {
                    line
                }
            })
            .collect();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(format!(" {} · Enter runs, Esc closes ", menu.title)),
            ),
            area,
        );
    }
}

/// Split mode tiles the configured panes; tabs mode shows the focused pane
/// under a tab bar. The focused pane's border is highlighted.
fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    let Some(view) = &app.view else {
        render_rows(frame, app, area);
        return;
    };
    let board = &view.board;
    let focused = app.focused();
    let pane_block = |pane: Pane| {
        let title = match pane {
            Pane::Notes => format!(
                " notes · {} ",
                view.document["squad"]["lead"]["name"]
                    .as_str()
                    .unwrap_or("no lead")
            ),
            other => format!(" {} ", other.title()),
        };
        let style = if pane == focused && board.panes.len() > 1 {
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            color("dim")
        };
        Block::new()
            .borders(Borders::ALL)
            .border_style(style)
            .title(title)
    };
    match board.mode {
        BoardMode::Split if board.panes.len() == 1 => render_pane(frame, app, board.panes[0], area),
        BoardMode::Split => {
            let constraints = board.sizes.iter().map(|size| Constraint::Percentage(*size));
            let areas = match board.direction {
                Direction::LeftRight => Layout::horizontal(constraints).split(area),
                Direction::TopBottom => Layout::vertical(constraints).split(area),
            };
            for (pane, rect) in board.panes.iter().zip(areas.iter()) {
                let block = pane_block(*pane);
                let inner = block.inner(*rect);
                frame.render_widget(block, *rect);
                render_pane(frame, app, *pane, inner);
            }
        }
        BoardMode::Tabs => {
            let [bar, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
            let mut spans = Vec::new();
            for pane in &board.panes {
                if *pane == focused {
                    spans.push(Span::styled(
                        format!("[{}]", pane.title()),
                        Style::new().add_modifier(Modifier::BOLD),
                    ));
                } else {
                    spans.push(Span::styled(pane.title(), color("dim")));
                }
                spans.push(Span::raw("  "));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), bar);
            let block = pane_block(focused);
            let inner = block.inner(rest);
            frame.render_widget(block, rest);
            render_pane(frame, app, focused, inner);
        }
    }
}

fn render_pane(frame: &mut Frame, app: &App, pane: Pane, area: Rect) {
    match pane {
        Pane::Rows => render_rows(frame, app, area),
        Pane::Notes => render_notes(frame, app, area),
        Pane::Detail => render_detail(frame, app, area),
        Pane::Replies => frame.render_widget(
            Paragraph::new(Span::styled(
                "Replies to your requests appear here in a later version.",
                color("dim"),
            )),
            area,
        ),
    }
}

fn render_notes(frame: &mut Frame, app: &App, area: Rect) {
    let Some(view) = &app.view else { return };
    let text = match &view.notes {
        Notes::Text(text) => text.as_str(),
        Notes::Missing => "(no notes yet)",
        Notes::NoLead => "(the squad has no lead)",
        Notes::NotShown => "",
        Notes::Failed(error) => error.as_str(),
    };
    let width = usize::from(area.width);
    let lines: Vec<Line> = match (&view.notes, view.render) {
        (Notes::Text(text), NotesRender::Markdown) => markdown::render(text, width),
        (Notes::Text(text), NotesRender::Plain) => {
            wrap(text, width).into_iter().map(Line::from).collect()
        }
        _ => wrap(text, width)
            .into_iter()
            .map(|line| Line::styled(line, color("dim")))
            .collect(),
    };
    let limit = lines.len().saturating_sub(usize::from(area.height));
    let scroll = usize::from(app.notes_scroll).min(limit) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

/// The selected row: where it is, what it is doing and what it waits on.
fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let Some(row) = app.selected_row() else {
        frame.render_widget(
            Paragraph::new(Span::styled("(no row selected)", color("dim"))),
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
            color("amber"),
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
        ("note", &row["note"]),
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
    let width = usize::from(area.width);
    let lines: Vec<Line> = lines
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
    frame.render_widget(Paragraph::new(lines), area);
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
                row_lines.push((lines.len(), row_index));
                lines.push(Line::from(spans).style(style));
                if let Some(note) = row["note"].as_str().filter(|_| !note_column) {
                    lines.push(Line::from(Span::styled(
                        fit(&format!("    note {note}"), usize::from(area.width)),
                        color("dim"),
                    )));
                    row_lines.push((lines.len() - 1, row_index));
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
    app.hits.borrow_mut().extend(
        row_lines
            .into_iter()
            .filter(|(line, _)| (offset..offset + height).contains(line))
            .map(|(line, row)| Hit {
                y: area.y + (line - offset) as u16,
                x: area.x,
                width: area.width,
                row,
            }),
    );
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::app::{Notes, Snapshot, View};
    use crate::config::{BoardMode, Direction, Pane};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
                board: crate::config::Board {
                mode: crate::config::BoardMode::Split,
                direction: crate::config::Direction::LeftRight,
                panes: vec![crate::config::Pane::Rows],
                sizes: vec![100],
            },
            notes: crate::board::app::Notes::NotShown,
                render: crate::config::NotesRender::Markdown,
                bindings: crate::action::preset(true),
                section_bindings: Vec::new(),
                opener: None,
                clipboard: None,
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
    fn drawn_rows_are_clickable_and_the_menu_and_help_show_bindings() {
        let mut app = board(json!([
            {"title": "Needs me", "rows": [row("auth-fix", "blocked", "rotate", json!({"note": "needs a call"}))]},
            {"title": "Everyone", "rows": [row("docs", "working", "guide", json!({}))]}
        ]));
        draw(&app, 48, 9);
        let lines: Vec<(u16, usize)> = app.hits.borrow().iter().map(|h| (h.y, h.row)).collect();
        assert_eq!(
            lines,
            [(3, 0), (4, 0), (6, 1)],
            "a row's note line clicks the row"
        );
        // Scrolled: only visible lines are clickable, at their screen rows.
        app.selected = 1;
        draw(&app, 48, 5);
        let lines: Vec<(u16, usize)> = app.hits.borrow().iter().map(|h| (h.y, h.row)).collect();
        assert_eq!(lines, [(1, 0), (3, 1)]);

        app.view.as_mut().unwrap().bindings = crate::action::preset(false);
        app.help = true;
        let help = draw(&app, 60, 24);
        assert!(
            help.iter().any(|line| line == "y           copy"),
            "{help:#?}"
        );
        app.help = false;
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let screen = draw(&app, 60, 16);
        assert!(
            screen
                .iter()
                .any(|line| line.contains("docs · Enter runs, Esc closes"))
        );
        assert!(screen.iter().any(|line| line.contains("backspace back")));
        assert!(draw(&App::new(None), 60, 3)[2].starts_with("/ search"));
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

    fn paned(board: crate::config::Board, notes: Notes) -> App {
        let mut app = App::new(Some("product".into()));
        app.apply(Snapshot {
            squads: vec!["product".into()],
            squad: Some("product".into()),
            view: Ok(View {
                document: json!({"squad": {"name": "product", "lead": {"name": "sol"}}, "sections": [
                    {"title": null, "rows": [row("auth-fix", "blocked", "rotate tokens", json!({
                        "pending": "approve the plan", "note": "needs a call", "presence": "active", "state": "blocked",
                        "pane": {"target": "crew:2.0", "cwd": "/w/app-3"}
                    }))]}
                ]}),
                columns: columns(),
                colors: BTreeMap::new(),
                board,
                notes,
                render: NotesRender::Markdown,
                bindings: crate::action::preset(true),
                section_bindings: Vec::new(),
                opener: None,
                clipboard: None,
            }),
        });
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["fields"]["pr_link"] =
            json!("https://example.com/pull/412");
        app
    }

    fn split(direction: Direction, panes: Vec<Pane>, sizes: Vec<u16>) -> crate::config::Board {
        crate::config::Board {
            mode: BoardMode::Split,
            direction,
            panes,
            sizes,
        }
    }

    #[test]
    fn split_panes_follow_direction_and_sizes() {
        let app = paned(
            split(
                Direction::LeftRight,
                vec![Pane::Rows, Pane::Notes],
                vec![60, 40],
            ),
            Notes::Text("## Now\n- tokens: waiting on Ben".into()),
        );
        let screen = draw(&app, 100, 10);
        // 60% of 100 columns: the notes block starts at column 60.
        let notes_at = screen[1].find("┌ notes · sol").expect("notes block title");
        assert_eq!(screen[1][..notes_at].chars().count(), 60, "{screen:#?}");
        assert!(screen[1].starts_with("┌ rows"));
        assert!(
            screen.iter().any(|line| line.contains("│Now")),
            "markdown heading"
        );
        assert!(screen.iter().any(|line| line.contains("◆ auth-fix")));

        let app = paned(
            split(
                Direction::TopBottom,
                vec![Pane::Rows, Pane::Detail],
                vec![50, 50],
            ),
            Notes::NotShown,
        );
        let screen = draw(&app, 70, 22);
        let detail_row = screen
            .iter()
            .position(|line| line.starts_with("┌ detail"))
            .unwrap();
        assert_eq!(
            detail_row, 11,
            "detail starts halfway down the 20-line body"
        );
        let detail = screen[detail_row..].join("\n");
        for expected in [
            "waiting on you: approve the plan",
            "active · blocked · crew:2.0 · /w/app-3",
            "task: rotate tokens",
            "note: needs a call",
            "links: pr_link https://example.com/pull/412",
        ] {
            assert!(detail.contains(expected), "{expected}\n{detail}");
        }
    }

    #[test]
    fn tabs_show_one_pane_and_tab_moves_focus() {
        let tabs = crate::config::Board {
            mode: BoardMode::Tabs,
            direction: Direction::LeftRight,
            panes: vec![Pane::Rows, Pane::Replies, Pane::Notes],
            sizes: Vec::new(),
        };
        let mut app = paned(tabs, Notes::Missing);
        let screen = draw(&app, 70, 10);
        assert!(
            screen[1].starts_with("[rows]  replies  notes"),
            "{screen:#?}"
        );
        assert!(screen.iter().any(|line| line.contains("auth-fix")));
        let tab = |app: &mut App| {
            app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        };
        tab(&mut app);
        let screen = draw(&app, 70, 10);
        assert!(screen[1].starts_with("rows  [replies]  notes"));
        assert!(
            screen
                .iter()
                .any(|line| line.contains("appear here in a later version"))
        );
        tab(&mut app);
        assert!(
            draw(&app, 70, 10)
                .iter()
                .any(|line| line.contains("(no notes yet)"))
        );
        tab(&mut app);
        assert_eq!(app.focused(), Pane::Rows, "focus wraps around");
    }

    #[test]
    fn focused_notes_scroll_while_rows_keep_their_selection() {
        let text = (1..=30)
            .map(|n| format!("line {n:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut app = paned(
            split(
                Direction::LeftRight,
                vec![Pane::Rows, Pane::Notes],
                vec![50, 50],
            ),
            Notes::Text(text),
        );
        // Plain rendering keeps one note line per display line to scroll by.
        app.view.as_mut().unwrap().render = NotesRender::Plain;
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        for _ in 0..5 {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        let screen = draw(&app, 60, 10);
        assert!(
            screen.iter().any(|line| line.contains("line 06")),
            "{screen:#?}"
        );
        assert!(!screen.iter().any(|line| line.contains("line 01")));
        assert_eq!(app.selected, 0);
    }
}
