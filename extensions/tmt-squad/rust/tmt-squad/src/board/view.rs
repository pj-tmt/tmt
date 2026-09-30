//! Drawing only: the board's layout from `App`, with display-width-aware
//! cells so wide characters never misalign columns.

use super::{
    app::{App, Hit, Item, Notes},
    markdown,
    notes::{sanitize, wrap},
};
use crate::{
    attention::Attention,
    config::{BoardMode, Direction, NotesRender, Pane, TabColors},
    requests::{BODIES, age},
    rows::{Cell as RowCell, Rows},
    split::Split,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use serde_json::Value;
use std::collections::BTreeMap;
use tmt_cli_style::{
    AnsiColor, Effects, Token,
    grid::{self, Align, Truncate},
};
use unicode_width::UnicodeWidthStr;

const KEYS: &[&str] = &[
    "↑ ↓ / j k   select a row; in a focused notes, detail or replies pane, scroll it",
    "PgUp PgDn   page the focused pane; Home End go to its top and bottom",
    "wheel       scroll the pane under the pointer",
    "Shift-drag  select text to copy (Option-drag in some terminals)",
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
    if app.view.as_ref().is_some_and(|view| view.me.is_none()) {
        hints.push(crate::status::UNKNOWN_YOU.to_owned());
    }
    hints.join("  ")
}

/// Fixed keys, then every binding for the selected row.
fn help_lines(app: &App) -> Vec<String> {
    let mut lines: Vec<String> = KEYS.iter().map(|line| (*line).to_owned()).collect();
    if let Some(view) = &app.view {
        lines.insert(
            KEYS.len() - 1,
            match view.refresh {
                Some(every) => format!("reload      automatically every {}s", every.as_secs()),
                None => "reload      automatic reload is off".to_owned(),
            },
        );
    }
    for (event, action) in app.bindings() {
        lines.push(format!("{event:<11} {}", action.text));
    }
    lines
}

/// A palette token as a board style: the same colors and effects as line
/// output, so the board and `status` agree.
fn token(token: Token) -> Style {
    let mut style = Style::new();
    if let Some(color) = token.color() {
        style = style.fg(ansi(color));
    }
    let effects = token.effects();
    if effects.contains(Effects::DIMMED) {
        style = style.add_modifier(Modifier::DIM);
    }
    if effects.contains(Effects::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

fn ansi(color: AnsiColor) -> Color {
    match color {
        AnsiColor::Black => Color::Black,
        AnsiColor::Red => Color::Red,
        AnsiColor::Green => Color::Green,
        AnsiColor::Yellow => Color::Yellow,
        AnsiColor::Blue => Color::Blue,
        AnsiColor::Magenta => Color::Magenta,
        AnsiColor::Cyan => Color::Cyan,
        AnsiColor::White => Color::Gray,
        AnsiColor::BrightBlack => Color::DarkGray,
        AnsiColor::BrightRed => Color::LightRed,
        AnsiColor::BrightGreen => Color::LightGreen,
        AnsiColor::BrightYellow => Color::LightYellow,
        AnsiColor::BrightBlue => Color::LightBlue,
        AnsiColor::BrightMagenta => Color::LightMagenta,
        AnsiColor::BrightCyan => Color::LightCyan,
        AnsiColor::BrightWhite => Color::White,
    }
}

/// A color named in the layout defaults or a user's `squad.toml`. The names
/// with a palette token take it; the rest keep their terminal color.
fn color(name: &str) -> Style {
    match name {
        "dim" => token(Token::Dim),
        "red" => token(Token::Error),
        "amber" => token(Token::Warn),
        "green" => token(Token::Ok),
        "blue" => token(Token::Accent),
        "cyan" => Style::new().fg(Color::Cyan),
        "magenta" => Style::new().fg(Color::Magenta),
        _ => Style::new(),
    }
}

/// Exactly `width` display cells: truncated with an ellipsis, or padded.
pub fn fit(text: &str, width: usize) -> String {
    grid::fit(text, width, Align::Left, Truncate::End)
}

/// A row's value for a field: `member` is its name, `pending` what it waits
/// on you for; anything else comes from its fields.
fn cell_text<'a>(row: &'a Value, field: &str) -> Option<&'a str> {
    match field {
        "member" => row["name"].as_str(),
        "pending" => row["pending"].as_str(),
        field => row["fields"][field].as_str(),
    }
}

/// Space between grid columns.
const GAP: usize = 1;

/// One line of a row on the solved grid: each cell across its spanned
/// columns, fitted by the first column's alignment and truncation. The first
/// line shows `–` for a missing value; a later line with nothing to show is
/// left out.
fn grid_line(
    rows: &Rows,
    widths: &[Option<usize>],
    cells: &[RowCell],
    row: &Value,
    first: bool,
    colors: &BTreeMap<String, String>,
) -> Option<Vec<Span<'static>>> {
    let mut spans = Vec::new();
    let mut position = 0;
    let mut shown_any = false;
    for cell in cells {
        let range = position..position + cell.span;
        position += cell.span;
        if widths[range.clone()].iter().all(Option::is_none) {
            continue;
        }
        let width = grid::span(widths, range.clone(), GAP);
        let value = cell
            .field
            .as_deref()
            .and_then(|field| cell_text(row, field));
        shown_any |= value.is_some_and(|value| !value.is_empty());
        let text = match (value, &cell.field, first) {
            (Some(value), _, _) => value,
            (None, Some(_), true) => "–",
            _ => "",
        };
        let column = &rows.columns[range.start];
        let style = if cell.field.as_deref() == Some("state") {
            color(colors.get(text).map_or("default", String::as_str))
        } else {
            Style::new()
        };
        if !spans.is_empty() {
            spans.push(Span::raw(" ".repeat(GAP)));
        }
        spans.push(Span::styled(
            grid::fit(text, width, column.align, column.truncate),
            style,
        ));
    }
    (first || shown_any).then_some(spans)
}

/// Shown only when a switch takes long enough to notice.
const SPINNER_DELAY: std::time::Duration = std::time::Duration::from_millis(250);
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// One pane tab (tabs mode), the same width selected or not: the selected
/// one is bracketed, the others padded.
fn pane_tab(name: &str, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(
            format!("[{name}]"),
            Style::new().add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {name} "), color("dim"))
    }
}

/// One tab. Selection is shown by reversing it, never by extra characters,
/// so switching never moves the tabs beside it (#504). The tab's attention
/// colors it and, so color never carries meaning alone, also adds counts:
/// `◆2` members waiting on you, `!1` blocked.
fn tab(name: &str, selected: bool, attention: Attention, colors: &TabColors) -> Span<'static> {
    let mut text = format!(" {name}");
    if attention.waiting > 0 {
        text.push_str(&format!(" ◆{}", attention.waiting));
    }
    if attention.blocked > 0 {
        text.push_str(&format!(" !{}", attention.blocked));
    }
    text.push(' ');
    let style = match attention.state() {
        "waiting" => color(&colors.waiting),
        "blocked" => color(&colors.blocked),
        _ if selected => Style::new(),
        _ => color("dim"),
    };
    let style = if selected {
        style.add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        style
    };
    Span::styled(text, style)
}

/// The first header line: only the tabs, and the loading spinner.
fn tab_line(app: &App) -> Line<'_> {
    let default = TabColors::default();
    let colors = app.view.as_ref().map_or(&default, |view| &view.tab_colors);
    let mut spans = Vec::new();
    for squad in &app.squads {
        let selected = Some(squad) == app.current.as_ref();
        let attention = app.attention.get(squad).copied().unwrap_or_default();
        spans.push(tab(squad, selected, attention, colors));
        spans.push(Span::raw(" "));
    }
    if let Some(started) = app.loading_since
        && started.elapsed() >= SPINNER_DELAY
    {
        let frame = (started.elapsed().as_millis() / 100) as usize % SPINNER.len();
        spans.push(Span::styled(
            format!("{} loading", SPINNER[frame]),
            color("dim"),
        ));
    }
    Line::from(spans)
}

/// The second header line: the shown squad's summary.
fn summary_line(app: &App) -> Line<'_> {
    let Some(view) = app.view.as_ref().filter(|_| !app.stale()) else {
        return Line::default();
    };
    let lead = match view.document["squad"]["lead"]["name"].as_str() {
        Some(lead) => format!("lead {lead}"),
        None => "no lead".into(),
    };
    // A member can appear in several user sections; count people once.
    let count = view.document["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| section["rows"].as_array().into_iter().flatten())
        .filter_map(|row| row["name"].as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let mut spans = vec![Span::styled(
        format!(
            "{lead} · {count} {}",
            if count == 1 { "member" } else { "members" }
        ),
        color("dim"),
    )];
    let waiting = app
        .current
        .as_ref()
        .and_then(|squad| app.attention.get(squad))
        .map_or(0, |attention| attention.waiting);
    if waiting > 0 {
        spans.push(Span::styled(
            format!(" · {waiting} waiting on you"),
            color(&view.tab_colors.waiting),
        ));
    }
    if view.document["olderRequestsNotShown"] == true {
        spans.push(Span::styled(" · older requests not shown", color("dim")));
    }
    Line::from(spans)
}

pub fn render(frame: &mut Frame, app: &App) {
    app.hits.borrow_mut().clear();
    app.scrolls.begin_frame();
    let [tabs, summary, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(Paragraph::new(tab_line(app)), tabs);
    frame.render_widget(Paragraph::new(summary_line(app)), summary);
    render_body(frame, app, body);
    let footer_line = if let Some(input) = &app.input {
        Line::from(format!("{} › {}▏", input.prompt, input.text))
    } else if app.searching {
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
            .map(|(index, entry)| {
                let line = Line::from(fit(
                    &format!(" {:<9} {}", entry.key, entry.label),
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
            token(Token::Accent).add_modifier(Modifier::BOLD)
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
        BoardMode::Split => render_split(frame, app, &board.split, area, &pane_block),
        BoardMode::Tabs => {
            let [bar, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
            let mut spans = Vec::new();
            for pane in &board.panes {
                spans.push(pane_tab(pane.title(), *pane == focused));
                spans.push(Span::raw(" "));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), bar);
            let block = pane_block(focused);
            let inner = block.inner(rest);
            frame.render_widget(block, rest);
            render_pane(frame, app, focused, inner);
        }
    }
}

/// One split, nested splits within it: each child gets its size or grow
/// share of the split, and every pane its own bordered block.
fn render_split(
    frame: &mut Frame,
    app: &App,
    split: &Split,
    area: Rect,
    pane_block: &dyn Fn(Pane) -> Block<'static>,
) {
    match split {
        Split::Pane(pane) => {
            let block = pane_block(*pane);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            render_pane(frame, app, *pane, inner);
        }
        Split::Group {
            direction,
            children,
        } => {
            let constraints = children.iter().map(|(size, _)| size.constraint());
            let areas = match direction {
                Direction::LeftRight => Layout::horizontal(constraints).split(area),
                Direction::TopBottom => Layout::vertical(constraints).split(area),
            };
            for ((_, child), rect) in children.iter().zip(areas.iter()) {
                render_split(frame, app, child, *rect, pane_block);
            }
        }
    }
}

fn render_pane(frame: &mut Frame, app: &App, pane: Pane, area: Rect) {
    match pane {
        Pane::Rows => render_rows(frame, app, area),
        Pane::Notes => render_notes(frame, app, area),
        Pane::Detail => render_detail(frame, app, area),
        Pane::Replies => render_replies(frame, app, area),
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
    app.scrolls
        .show(frame, Pane::Notes, area, lines, color("dim"));
}

/// Lines of one reply body shown before it is cut.
const BODY_LINES: usize = 6;

/// Finals to the user's squad requests, newest first. Bodies are
/// agent-written, so they are sanitized like notes; reading acknowledges
/// nothing.
pub fn reply_lines(replies: &[Value], width: usize, now_ms: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (index, reply) in replies.iter().enumerate() {
        let text = |key: &str| sanitize(reply[key].as_str().unwrap_or_default());
        let when = reply["submittedAtMs"]
            .as_u64()
            .map_or(String::new(), |at| format!(" · {}", age(now_ms, at)));
        lines.push(Line::from(Span::styled(
            fit(&format!("{}{when}", text("to")), width),
            Style::new().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::styled(
            fit(&format!("  › {}", text("prompt")), width),
            color("dim"),
        ));
        match reply["response"].as_str() {
            Some(response) => {
                let body = wrap(&sanitize(response), width.saturating_sub(2));
                for line in body.iter().take(BODY_LINES) {
                    lines.push(Line::from(format!("  {line}")));
                }
                if body.len() > BODY_LINES {
                    lines.push(Line::styled("  …", color("dim")));
                }
            }
            None => {
                let id = reply["requestId"].as_str().unwrap_or_default();
                let hint = if index >= BODIES || reply["status"] == "retained" {
                    format!("  tmt result {id}")
                } else {
                    format!("  (final {})", text("status"))
                };
                lines.push(Line::styled(fit(&hint, width), color("dim")));
            }
        }
        lines.push(Line::from(""));
    }
    lines
}

fn render_replies(frame: &mut Frame, app: &App, area: Rect) {
    let Some(view) = &app.view else { return };
    let lines = if view.replies.is_empty() {
        vec![Line::styled(
            if view.me.is_none() {
                "(tmt squad me <name> shows the replies to your requests)"
            } else {
                "(no replies to your squad requests yet)"
            },
            color("dim"),
        )]
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        reply_lines(&view.replies, usize::from(area.width), now)
    };
    app.scrolls
        .show(frame, Pane::Replies, area, lines, color("dim"));
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
    app.scrolls
        .show(frame, Pane::Detail, area, lines, color("dim"));
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
    let rows = &view.rows;
    // Unsized columns start from their widest value on the board.
    let natural = |index: usize| {
        let field = &rows.columns[index].field;
        app.items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Row(row) => cell_text(row, field),
                Item::Header(_) => None,
            })
            // Measured as drawn: `grid::fit` shows control characters escaped.
            .map(|value| tmt_cli_style::table::escape(value).width())
            .chain([rows.columns[index].title.width()])
            .max()
            .unwrap_or(0)
    };
    let tracks: Vec<_> = (0..rows.columns.len())
        .map(|index| rows.columns[index].track(natural(index)))
        .collect();
    // Two cells for the row mark.
    let widths = grid::solve(
        &tracks,
        Some(usize::from(area.width).saturating_sub(2)),
        GAP,
    );
    let note_column = rows.fields().contains(&"note");
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "  {}",
            rows.columns
                .iter()
                .zip(&widths)
                .filter_map(|(column, width)| width.map(|width| grid::fit(
                    &column.title,
                    width,
                    column.align,
                    column.truncate
                )))
                .collect::<Vec<_>>()
                .join(&" ".repeat(GAP))
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
                let style = if selected {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    Style::new()
                };
                for (index, cells) in rows.lines.iter().enumerate() {
                    let first = index == 0;
                    let Some(cells) = grid_line(rows, &widths, cells, row, first, &view.colors)
                    else {
                        continue;
                    };
                    let mut spans = vec![Span::raw(if first { marker } else { "  " })];
                    spans.extend(cells);
                    row_lines.push((lines.len(), row_index));
                    lines.push(Line::from(spans).style(style));
                }
                if let Some(note) = row["note"].as_str().filter(|_| !note_column) {
                    lines.push(Line::from(Span::styled(
                        fit(&format!("    note {note}"), usize::from(area.width)),
                        color("dim"),
                    )));
                    row_lines.push((lines.len() - 1, row_index));
                }
                if let Some(text) = row["annotation"]["text"].as_str() {
                    let to = row["annotation"]["to"].as_str().unwrap_or_default();
                    lines.push(Line::from(Span::styled(
                        fit(
                            &format!("    ✎ sent to {to}: {text}"),
                            usize::from(area.width),
                        ),
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
    // The selection stays on screen until the wheel moves the rows away from
    // it; the column header scrolls with the list.
    if app.follow {
        app.scrolls
            .reveal(Pane::Rows, selected_line, area, lines.len());
    }
    let (offset, viewport) = app
        .scrolls
        .show(frame, Pane::Rows, area, lines, color("dim"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::app::{Notes, Snapshot, View};
    use crate::config::{BoardMode, Direction, Pane};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    use std::collections::BTreeMap;
    use unicode_width::UnicodeWidthChar;

    /// Rows read from a squad config snippet, as `squad.toml` would give them.
    fn rows_from(text: &str) -> Rows {
        let config: toml_edit::DocumentMut = text.parse().unwrap();
        crate::rows::read(config["p"].as_table_like(), "p").unwrap()
    }

    fn columns() -> Rows {
        rows_from(
            "[p.columns]\nshow = [\"member\", \"state\", \"task\"]\n\
             member = { width = 10 }\nstate = { width = 8 }\n",
        )
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
            attention: Default::default(),
            squad: Some("product".into()),
            view: Ok(View {
                document: json!({"squad": {"name": "product", "lead": {"name": "sol"}}, "sections": sections}),
                rows: columns(),
                colors: BTreeMap::from([("blocked".into(), "amber".into())]),
                refresh: Some(crate::config::DEFAULT_REFRESH),
                board: crate::config::Board::simple(
                    crate::config::BoardMode::Split,
                    crate::config::Direction::LeftRight,
                    vec![crate::config::Pane::Rows],
                    &[100],
                ),
            notes: crate::board::app::Notes::NotShown,
                render: crate::config::NotesRender::Markdown,
                bindings: crate::action::preset(true),
                section_bindings: Vec::new(),
                opener: None,
                clipboard: None,
                tab_colors: Default::default(),
                me: None,
                replies: Vec::new(),
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

    /// The board as every preset draws it: the default columns, read from
    /// an empty config.
    fn preset_board() -> App {
        let path = std::env::temp_dir().join(format!("squad-golden-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let config = crate::config::Config::read(path).unwrap();
        let mut app = board(json!([
            {"title": "Needs me", "rows": [
                row("auth-fix", "blocked", "rotate session tokens without logging everyone out", json!({
                    "pending": "approve", "note": "needs a call",
                    "fields": {"state": "blocked", "task": "rotate session tokens without logging everyone out", "pr_link": "https://github.com/wkh237/tmt/pull/4242"}
                }))
            ]},
            {"title": "Everyone", "rows": [
                row("文件-sweep-long-name", "working", "整理安装指南和常见问题", json!({})),
                row("perf", "", "", json!({"fields": {}, "annotation": {"to": "sol", "text": "check the cache hit rate"}})),
            ]}
        ]));
        app.view.as_mut().unwrap().rows = config.rows("product").unwrap();
        app
    }

    /// Golden: every preset's board as drawn before the rows moved onto the
    /// shared grid solver. The layout engine must keep these byte for byte.
    #[test]
    fn preset_columns_draw_exactly_as_before_at_every_width() {
        let app = preset_board();
        let golden: [(u16, [&str; 8]); 4] = [
            (
                48,
                [
                    "  MEMBER         STATE      TASK    PR",
                    "NEEDS ME",
                    "◆ auth-fix       blocked    rotate… https://git…",
                    "    note needs a call",
                    "EVERYONE",
                    "  文件-sweep-lo… working    整理安… –",
                    "  perf           –          –       –",
                    "    ✎ sent to sol: check the cache hit rate",
                ],
            ),
            (
                60,
                [
                    "  MEMBER         STATE      TASK                PR",
                    "NEEDS ME",
                    "◆ auth-fix       blocked    rotate session tok… https://git…",
                    "    note needs a call",
                    "EVERYONE",
                    "  文件-sweep-lo… working    整理安装指南和常见… –",
                    "  perf           –          –                   –",
                    "    ✎ sent to sol: check the cache hit rate",
                ],
            ),
            (
                80,
                [
                    "  MEMBER         STATE      TASK                                    PR",
                    "NEEDS ME",
                    "◆ auth-fix       blocked    rotate session tokens without logging … https://git…",
                    "    note needs a call",
                    "EVERYONE",
                    "  文件-sweep-lo… working    整理安装指南和常见问题                  –",
                    "  perf           –          –                                       –",
                    "    ✎ sent to sol: check the cache hit rate",
                ],
            ),
            (
                120,
                [
                    "  MEMBER         STATE      TASK                                                                            PR",
                    "NEEDS ME",
                    "◆ auth-fix       blocked    rotate session tokens without logging everyone out                              https://git…",
                    "    note needs a call",
                    "EVERYONE",
                    "  文件-sweep-lo… working    整理安装指南和常见问题                                                          –",
                    "  perf           –          –                                                                               –",
                    "    ✎ sent to sol: check the cache hit rate",
                ],
            ),
        ];
        for (width, lines) in golden {
            assert_eq!(draw(&app, width, 11)[2..10], lines, "at {width} columns");
        }
    }

    #[test]
    fn a_narrow_preset_board_drops_the_link_instead_of_clipping() {
        let screen = draw(&preset_board(), 44, 11);
        assert_eq!(screen[2], "  MEMBER         STATE      TASK");
        assert_eq!(screen[4], "◆ auth-fix       blocked    rotate session …");
        assert!(screen[2..10].iter().all(|line| line.width() <= 44));
    }

    #[test]
    fn handbook_rows_take_a_second_line_span_align_and_step_aside() {
        let mut app = board(json!([{"title": null, "rows": [
            row("auth-fix", "blocked", "rotate session tokens", json!({
                "pending": "approve the rollout plan",
                "fields": {"state": "blocked", "task": "rotate session tokens", "pr": "#4242"},
            })),
            row("docs", "working", "guide", json!({"fields": {"state": "working", "task": "guide", "pr": "#7"}})),
        ]}]));
        app.view.as_mut().unwrap().rows = rows_from(
            r#"[p.rows]
columns = [
  { name = "member", min = 10 },
  { name = "state",  width = 9 },
  { name = "task",   grow = 1, min = 12 },
  { name = "pr",     width = 10, align = "right", priority = 2 },
]
lines = [
  ["member", "state", "task", "pr"],
  ["",       { field = "pending", span = 3 }],
]
"#,
        );
        let wide = draw(&app, 60, 9);
        assert_eq!(
            wide[2],
            "  MEMBER     STATE     TASK                               PR"
        );
        assert_eq!(
            wide[3],
            "◆ auth-fix   blocked   rotate session tokens           #4242"
        );
        assert_eq!(wide[4], "             approve the rollout plan");
        // Nothing to show on the second line: the row keeps one line.
        assert_eq!(
            wide[5],
            "  docs       working   guide                              #7"
        );
        // Narrow: the prioritized column steps aside and the span shrinks.
        let narrow = draw(&app, 36, 9);
        assert_eq!(narrow[2], "  MEMBER     STATE     TASK");
        assert_eq!(narrow[3], "◆ auth-fix   blocked   rotate sessi…");
        assert_eq!(narrow[4], "             approve the rollout pl…");
    }

    #[test]
    fn middle_truncation_keeps_both_ends_of_a_link() {
        let mut app = board(json!([{"title": null, "rows": [
            row("docs", "working", "", json!({"fields": {"link": "https://github.com/wkh237/tmt/pull/4242"}})),
        ]}]));
        app.view.as_mut().unwrap().rows = rows_from(
            "[p.rows]\ncolumns = [{ name = \"member\", width = 6 }, { name = \"link\", width = 20, truncate = \"middle\" }]\n",
        );
        assert_eq!(draw(&app, 40, 4)[2], "  docs   https://gi…pull/4242");
    }

    #[test]
    fn rows_show_pending_marker_notes_sections_and_aligned_wide_text() {
        let app = board(json!([
            {"title": "Needs me", "rows": [row("auth-fix", "blocked", "rotate session tokens", json!({"pending": "approve", "note": "needs a call"}))]},
            {"title": "Everyone", "rows": [row("文件-sweep", "working", "整理安装指南", json!({}))]}
        ]));
        let screen = draw(&app, 48, 10);
        // The tab line holds only the tabs; the summary has its own line.
        assert_eq!(screen[0], " product   reviews");
        assert_eq!(screen[1], "lead sol · 2 members");
        assert_eq!(screen[2], "  MEMBER     STATE    TASK");
        assert_eq!(screen[3], "NEEDS ME");
        assert_eq!(screen[4], "◆ auth-fix   blocked  rotate session tokens");
        assert_eq!(screen[5], "    note needs a call");
        assert_eq!(screen[6], "EVERYONE");
        assert_eq!(screen[7], "  文件-sweep working  整理安装指南");
        assert!(screen[9].starts_with("⏎ jump"));
    }

    #[test]
    fn a_squad_of_one_has_one_member() {
        let app =
            board(json!([{"title": null, "rows": [row("docs", "working", "guide", json!({}))]}]));
        assert_eq!(draw(&app, 48, 5)[1], "lead sol · 1 member");
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
            [(4, 0), (5, 0), (7, 1)],
            "a row's note line clicks the row"
        );
        // Scrolled: only visible lines are clickable, at their screen rows.
        app.selected = 1;
        draw(&app, 48, 6);
        let lines: Vec<(u16, usize)> = app.hits.borrow().iter().map(|h| (h.y, h.row)).collect();
        // An overflowing pane keeps its last line for the indicator, so two of
        // the three lines show rows.
        assert_eq!(lines, [(3, 1)]);

        app.view.as_mut().unwrap().bindings = crate::action::preset(false);
        app.help = true;
        let help = draw(&app, 60, 24);
        assert!(
            help.iter().any(|line| line == "y           copy"),
            "{help:#?}"
        );
        assert!(
            help.iter()
                .any(|line| line == "reload      automatically every 5s"),
            "{help:#?}"
        );
        app.view.as_mut().unwrap().refresh = None;
        let help = draw(&app, 60, 24);
        assert!(
            help.iter()
                .any(|line| line == "reload      automatic reload is off"),
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
        assert!(draw(&App::new(None), 60, 4)[3].starts_with("/ search"));
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
            attention: Default::default(),
            squad: Some("product".into()),
            view: Ok(View {
                document: json!({"squad": {"name": "product", "lead": {"name": "sol"}}, "sections": [
                    {"title": null, "rows": [row("auth-fix", "blocked", "rotate tokens", json!({
                        "pending": "approve the plan", "note": "needs a call", "presence": "active", "state": "blocked",
                        "pane": {"target": "crew:2.0", "cwd": "/w/app-3"}
                    }))]}
                ]}),
                rows: columns(),
                colors: BTreeMap::new(),
                refresh: None,
                board,
                notes,
                render: NotesRender::Markdown,
                bindings: crate::action::preset(true),
                section_bindings: Vec::new(),
                opener: None,
                clipboard: None,
                tab_colors: Default::default(),
                me: None,
                replies: Vec::new(),
            }),
        });
        app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["fields"]["pr_link"] =
            json!("https://example.com/pull/412");
        app
    }

    fn split(direction: Direction, panes: Vec<Pane>, sizes: Vec<u16>) -> crate::config::Board {
        crate::config::Board::simple(BoardMode::Split, direction, panes, &sizes)
    }

    #[test]
    fn nested_splits_draw_rows_beside_detail_over_notes() {
        use crate::split::{Size, Split};
        let board = crate::config::Board {
            mode: BoardMode::Split,
            panes: vec![Pane::Rows, Pane::Detail, Pane::Notes],
            split: Split::Group {
                direction: Direction::LeftRight,
                children: vec![
                    (Size::Percent(60), Split::Pane(Pane::Rows)),
                    (
                        Size::Percent(40),
                        Split::simple(
                            Direction::TopBottom,
                            &[Pane::Detail, Pane::Notes],
                            &[40, 60],
                        ),
                    ),
                ],
            },
        };
        let mut app = paned(board, Notes::Text("## Now\n- tokens".into()));
        let screen = draw(&app, 100, 23);
        // Rows take 60 of 100 columns; detail sits over notes in the rest.
        let right = |line: &str| line.chars().skip(60).collect::<String>();
        assert!(screen[2].starts_with("┌ rows"), "{screen:#?}");
        assert!(right(&screen[2]).starts_with("┌ detail"), "{screen:#?}");
        let notes_top = screen
            .iter()
            .position(|line| right(line).starts_with("┌ notes · sol"))
            .expect("notes block");
        // 40% of the 20 body lines is detail: notes start 8 lines below it.
        assert_eq!(notes_top, 2 + 8, "{screen:#?}");
        assert!(screen.iter().any(|line| right(line).contains("auth-fix")));
        // Tab walks the panes in reading order.
        for expected in [Pane::Detail, Pane::Notes, Pane::Rows] {
            app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
            assert_eq!(app.focused(), expected);
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
        let screen = draw(&app, 100, 11);
        // 60% of 100 columns: the notes block starts at column 60.
        let notes_at = screen[2].find("┌ notes · sol").expect("notes block title");
        assert_eq!(screen[2][..notes_at].chars().count(), 60, "{screen:#?}");
        assert!(screen[2].starts_with("┌ rows"));
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
        let screen = draw(&app, 70, 23);
        let detail_row = screen
            .iter()
            .position(|line| line.starts_with("┌ detail"))
            .unwrap();
        assert_eq!(
            detail_row, 12,
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
    fn replies_show_recipient_age_prompt_and_a_sanitized_bounded_body() {
        assert_eq!(age(100_000, 55_000), "45s");
        assert_eq!(age(3_600_000, 0), "1h");
        assert_eq!(age(200_000_000, 0), "2d");
        assert_eq!(
            age(0, 5_000),
            "0s",
            "a clock behind the final is not negative"
        );
        let body = "line 1\n\u{1b}[31mred\u{1b}[0m\n3\n4\n5\n6\n7\n8";
        let mut replies = vec![
            json!({"requestId": "r1", "to": "sol", "prompt": "[product · auth-fix] split", "status": "retained", "submittedAtMs": 40_000, "response": body}),
            json!({"requestId": "r2", "to": "docs", "prompt": "check", "status": "expired", "submittedAtMs": 30_000, "response": null}),
        ];
        for index in 0..BODIES {
            replies.push(json!({"requestId": format!("old{index}"), "to": "sol", "prompt": "p", "status": "retained", "submittedAtMs": 0, "response": null}));
        }
        let lines: Vec<String> = reply_lines(&replies, 40, 100_000)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect();
        assert_eq!(lines[0], "sol · 1m");
        assert_eq!(lines[1], "  › [product · auth-fix] split");
        assert_eq!(
            lines[2..8],
            ["  line 1", "  red", "  3", "  4", "  5", "  6"],
            "escapes removed"
        );
        assert_eq!(lines[8], "  …", "cut after six lines");
        assert_eq!(lines[12], "  (final expired)");
        assert!(
            lines.contains(&format!("  tmt result old{}", BODIES - 1)),
            "older finals point to tmt result"
        );
        assert!(!lines.iter().any(|line| line.contains('\u{1b}')));
    }

    #[test]
    fn every_row_is_one_line_cut_by_display_width_at_any_width() {
        let long = "rotate session tokens without logging everyone out of every device";
        let mut app = board(json!([{"title": null, "rows": [
            row("ascii-member-with-a-long-name", "blocked", long, json!({
                "fields": {"state": "blocked", "task": long, "pr_link": "https://github.com/wkh237/tmt/pull/4242"}
            })),
            row("文件整理小组成员", "进行中", "整理安装指南和常见问题并补充截图说明", json!({})),
            row("mix-混合-🚀", "review", "修 bug in 登录 flow 🚀 then ship", json!({})),
        ]}]));
        app.view.as_mut().unwrap().rows = crate::rows::Rows::preset();
        for width in [30u16, 44, 60, 100] {
            let screen = draw(&app, width, 9);
            // Tabs, summary, header, three rows, blank, blank, footer: nothing
            // wrapped.
            for (line, name) in screen[3..6].iter().zip(["ascii-", "文件", "mix-"]) {
                assert!(line.contains(name), "at {width}: {screen:#?}");
            }
            assert_eq!(screen[6], "", "at {width}: a row spilled: {screen:#?}");
            for line in &screen[2..6] {
                assert!(line.width() <= usize::from(width), "at {width}: {line:?}");
            }
        }
    }

    #[test]
    fn tabs_carry_attention_by_color_and_count_and_the_summary_has_its_own_line() {
        let mut app = board(json!([{"title": null, "rows": [
            row("auth-fix", "blocked", "rotate", json!({"pending": "approve"})),
        ]}]));
        app.attention = BTreeMap::from([
            (
                "product".into(),
                Attention {
                    waiting: 1,
                    blocked: 1,
                },
            ),
            (
                "reviews".into(),
                Attention {
                    waiting: 0,
                    blocked: 2,
                },
            ),
        ]);
        let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let tabs: String = (0..60)
            .map(|x| buffer[(x, 0)].symbol().to_owned())
            .collect();
        // Counts say what the color says, so no meaning is color-only.
        assert_eq!(tabs.trim_end(), " product ◆1 !1   reviews !2");
        let column = |name: &str| tabs[..tabs.find(name).unwrap()].chars().count() as u16;
        let product = &buffer[(column("product"), 0)];
        // Waiting wins over blocked; the selected tab is reversed, not bracketed.
        assert_eq!(product.fg, color("amber").fg.unwrap_or(Color::Reset));
        assert!(product.modifier.contains(Modifier::REVERSED));
        let reviews = &buffer[(column("reviews"), 0)];
        assert_eq!(reviews.fg, color("red").fg.unwrap_or(Color::Reset));
        assert!(!reviews.modifier.contains(Modifier::REVERSED));

        let screen = draw(&app, 60, 6);
        assert_eq!(screen[1], "lead sol · 1 member · 1 waiting on you");
        // Colors come from [tabs.colors]; without a lead the summary says so.
        let view = app.view.as_mut().unwrap();
        view.tab_colors.blocked = "magenta".into();
        view.document["squad"]["lead"] = Value::Null;
        app.attention.clear();
        app.attention.insert(
            "reviews".into(),
            Attention {
                waiting: 0,
                blocked: 2,
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let tabs: String = (0..60)
            .map(|x| buffer[(x, 0)].symbol().to_owned())
            .collect();
        let reviews = tabs[..tabs.find("reviews").unwrap()].chars().count() as u16;
        assert_eq!(buffer[(reviews, 0)].fg, Color::Magenta);
        assert_eq!(draw(&app, 60, 6)[1], "no lead · 1 member");
    }

    #[test]
    fn switching_squads_never_moves_a_tab_or_blanks_the_frame() {
        let mut app = board(json!([
            {"title": null, "rows": [row("auth-fix", "blocked", "rotate", json!({}))]}
        ]));
        let before = draw(&app, 60, 6);
        let place = |line: &str, name: &str| line.find(name).unwrap();
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.loading_since = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        let during = draw(&app, 60, 6);
        for name in ["product", "reviews"] {
            assert_eq!(
                place(&before[0], name),
                place(&during[0], name),
                "{name} moved: {before:?} / {during:?}"
            );
        }
        assert_eq!(app.current.as_deref(), Some("reviews"));
        // Selection is a style, so the tab text is the same either way.
        assert!(during[0].starts_with(" product   reviews "), "{during:?}");
        assert_eq!(before[0].trim_end(), " product   reviews");
        assert!(
            during[0].contains("loading"),
            "a slow switch shows a spinner"
        );
        assert!(
            during.iter().any(|line| line.contains("auth-fix")),
            "the previous frame stays: {during:?}"
        );
        assert!(!during.iter().any(|line| line.contains("Loading…")));
    }

    #[test]
    fn tabs_show_one_pane_and_tab_moves_focus() {
        let tabs = crate::config::Board::simple(
            BoardMode::Tabs,
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Replies, Pane::Notes],
            &[],
        );
        let mut app = paned(tabs, Notes::Missing);
        let screen = draw(&app, 70, 11);
        assert!(
            screen[2].starts_with("[rows]  replies   notes"),
            "{screen:#?}"
        );
        assert!(screen.iter().any(|line| line.contains("auth-fix")));
        let tab = |app: &mut App| {
            app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        };
        tab(&mut app);
        let screen = draw(&app, 70, 11);
        // Same columns as before the switch: only the brackets move.
        assert!(
            screen[2].starts_with(" rows  [replies]  notes"),
            "{screen:#?}"
        );
        assert!(
            screen
                .iter()
                .any(|line| line.contains("tmt squad me <name> shows the replies"))
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

    fn wheel(app: &mut App, column: u16, row: u16, down: bool) {
        use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
        app.mouse(
            MouseEvent {
                kind: if down {
                    MouseEventKind::ScrollDown
                } else {
                    MouseEventKind::ScrollUp
                },
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
    }

    #[test]
    fn the_wheel_scrolls_the_pane_under_the_pointer_whichever_is_focused() {
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
        app.view.as_mut().unwrap().render = NotesRender::Plain;
        let before = draw(&app, 60, 11);
        assert!(before.iter().any(|line| line.contains("line 01")));
        assert!(
            before.iter().any(|line| line.contains("↓ 25")),
            "an overflowing pane says how much is below: {before:#?}"
        );
        // Rows is focused; the wheel over the notes (right half) moves them.
        wheel(&mut app, 45, 5, true);
        wheel(&mut app, 45, 5, true);
        let after = draw(&app, 60, 11);
        assert!(
            !after.iter().any(|line| line.contains("line 06")),
            "{after:#?}"
        );
        assert!(after.iter().any(|line| line.contains("line 07")));
        assert!(after.iter().any(|line| line.contains("↑ 6  ↓ 19")));
        wheel(&mut app, 45, 5, false);
        assert!(
            draw(&app, 60, 11)
                .iter()
                .any(|line| line.contains("line 04"))
        );
        assert_eq!(app.selected, 0, "the rows' selection never moves");
    }

    #[test]
    fn the_wheel_scrolls_rows_away_from_the_selection_until_a_key_brings_it_back() {
        let rows: Vec<Value> = (0..20)
            .map(|i| row(&format!("m{i:02}"), "working", "", json!({})))
            .collect();
        let mut app = board(json!([{"title": null, "rows": rows}]));
        draw(&app, 40, 8);
        for _ in 0..3 {
            wheel(&mut app, 5, 4, true);
        }
        let screen = draw(&app, 40, 8);
        assert!(
            !screen.iter().any(|line| line.contains("m00")),
            "{screen:#?}"
        );
        assert!(screen.iter().any(|line| line.contains("m09")));
        assert_eq!(app.selected, 0);
        // Only visible rows take clicks, at their screen lines.
        assert!(app.hits.borrow().iter().all(|hit| hit.row >= 8));
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let screen = draw(&app, 40, 8);
        assert!(
            screen.iter().any(|line| line.contains("m01")),
            "{screen:#?}"
        );
        // PgDn and End page the selection when they are not bound.
        app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(app.selected, 19);
        assert!(draw(&app, 40, 8).iter().any(|line| line.contains("m19")));
        app.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_bound_paging_key_runs_its_binding_instead_of_scrolling() {
        let rows: Vec<Value> = (0..20)
            .map(|i| row(&format!("m{i:02}"), "working", "", json!({})))
            .collect();
        let mut app = board(json!([{"title": null, "rows": rows}]));
        app.view.as_mut().unwrap().bindings =
            crate::action::parse_bindings([("pagedown", Some("copy {name}"))].into_iter(), "bind")
                .unwrap();
        draw(&app, 40, 8);
        let effect = app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.selected, 0, "no paging");
        assert!(
            matches!(effect, crate::board::app::Effect::Act(_)),
            "{effect:?}"
        );
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
