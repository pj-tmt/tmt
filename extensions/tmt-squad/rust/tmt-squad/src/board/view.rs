//! Drawing only: the board's layout from `App`, with display-width-aware
//! cells so wide characters never misalign columns.

use super::{
    app::{App, Hit, Item, Notes, Switcher, TabHit},
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
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use serde_json::Value;
use std::collections::BTreeMap;
use tmt_cli_style::{
    Role,
    grid::{self, Align, Truncate},
};
use unicode_width::UnicodeWidthStr;

const KEYS: &[&str] = &[
    "↑ ↓ / j k   select a row; in a focused notes, detail or replies pane, scroll it",
    "PgUp PgDn   page the focused pane; Home End go to its top and bottom",
    "wheel       scroll the pane under the pointer",
    "Shift-drag  select text to copy (Option-drag in some terminals)",
    "← →         switch tab; Shift+← → or drag a tab to move it",
    "/           search; Esc clears",
    "s           switch to any tab, hidden ones too (type to filter)",
    "?           this help",
    "q, Esc      close the board",
    "",
];

/// The footer names what the most used keys do for the selected row.
fn hints(app: &App) -> String {
    let bindings = app.bindings();
    let mut hints: Vec<String> = [
        ("enter", "⏎"),
        ("o", "o"),
        ("y", "y"),
        ("tab", "tab"),
        ("f5", "F5"),
    ]
    .into_iter()
    .filter_map(|(event, label)| {
        let action = bindings.get(event)?;
        Some(format!("{label} {}", action.verb.name()))
    })
    .collect();
    hints.extend(["/ search", "←→ tab"].map(str::to_owned));
    if !bindings.contains_key("s") {
        hints.push("s switch".into());
    }
    hints.extend(["? more", "q quit"].map(str::to_owned));
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
    look: crate::look::Look,
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
        let failed = cell.field.as_deref().is_some_and(|field| {
            row["failed"]
                .as_array()
                .is_some_and(|failed| failed.iter().any(|name| name == field))
        });
        // A cell's own color: `state` by its state, any other field by the
        // token `status::document` resolved for it (a threshold or a
        // provider's). Decoration only: the text says the same without it.
        let token = cell
            .field
            .as_deref()
            .and_then(|field| row["colors"][field].as_str());
        let style = if value.is_none_or(str::is_empty) {
            look.role(Role::Dim)
        } else if cell.field.as_deref() == Some("state") {
            look.named(colors.get(text).map_or("default", String::as_str))
        } else if let Some(token) = token {
            look.named(token)
        } else if failed {
            // A field provider's run failed: its `?` stays quiet.
            look.named("dim")
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

/// Puts a row's age at the right edge of its first line when it fits after
/// the cells; a narrow board drops it before any cell.
fn age_mark(spans: &mut Vec<Span<'static>>, age: &str, width: usize, look: crate::look::Look) {
    let used: usize = spans.iter().map(Span::width).sum();
    let mark = age.width();
    if used + GAP + mark <= width {
        spans.push(Span::raw(" ".repeat(width - used - mark)));
        spans.push(Span::styled(age.to_owned(), look.role(Role::Dim)));
    }
}

/// Shown only when a switch takes long enough to notice.
pub(super) const SPINNER_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const SPINNER_TICK: std::time::Duration = std::time::Duration::from_millis(80);
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(super) fn spinner_frame(app: &App, now: std::time::Instant) -> Option<usize> {
    let elapsed = now.checked_duration_since(app.loading_since?)?;
    (elapsed >= SPINNER_DELAY).then(|| {
        ((elapsed - SPINNER_DELAY).as_millis() / SPINNER_TICK.as_millis()) as usize % SPINNER.len()
    })
}

pub(super) fn spinner_wait(app: &App, now: std::time::Instant) -> Option<std::time::Duration> {
    let elapsed = now.checked_duration_since(app.loading_since?)?;
    Some(if elapsed < SPINNER_DELAY {
        SPINNER_DELAY - elapsed
    } else {
        SPINNER_TICK
            - std::time::Duration::from_millis(
                ((elapsed - SPINNER_DELAY).as_millis() % SPINNER_TICK.as_millis()) as u64,
            )
    })
}

/// Reply age is the board's only clock-derived text. Hidden replies do not
/// invalidate a frame, and minute/hour marks redraw only when their text changes.
pub(super) fn time_marks(app: &App, now: u64) -> Vec<String> {
    let Some(view) = &app.view else {
        return Vec::new();
    };
    let visible = match view.board.mode {
        BoardMode::Split => view.board.panes.contains(&Pane::Replies),
        BoardMode::Tabs => app.focused() == Pane::Replies,
    };
    if !visible {
        return Vec::new();
    }
    view.replies
        .iter()
        .filter_map(|reply| reply["submittedAtMs"].as_u64().map(|at| age(now, at)))
        .collect()
}

/// One pane tab (tabs mode), the same width selected or not: the selected
/// one is bracketed, the others padded.
fn pane_tab(look: crate::look::Look, name: &str, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(
            format!("[{name}]"),
            look.role(Role::Accent).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {name} "), look.role(Role::Muted))
    }
}

/// One tab. Selection uses bold focus color, never extra characters,
/// so switching never moves the tabs beside it (#504). The tab's attention
/// colors it and, so color never carries meaning alone, also adds counts:
/// `◆2` members waiting on you, `!1` blocked.
fn tab(
    look: crate::look::Look,
    name: &str,
    selected: bool,
    attention: Attention,
    colors: &TabColors,
) -> Span<'static> {
    let mut text = format!(" {name}");
    if attention.waiting > 0 {
        text.push_str(&format!(" ◆{}", attention.waiting));
    }
    if attention.blocked > 0 {
        text.push_str(&format!(" !{}", attention.blocked));
    }
    text.push(' ');
    let style = match attention.state() {
        "waiting" => look.named(&colors.waiting),
        "blocked" => look.named(&colors.blocked),
        _ if selected => look.role(Role::Accent),
        _ => look.role(Role::Muted),
    };
    let style = if selected {
        let style = style.add_modifier(Modifier::BOLD);
        if look.depth == tmt_cli_style::Depth::None || look.theme.base == tmt_cli_style::Base::Mono
        {
            style.add_modifier(Modifier::REVERSED)
        } else {
            style
        }
    } else {
        style
    };
    Span::styled(text, style)
}

/// The first header line: only the tabs. Each
/// tab's place is recorded for clicks and drags. When the tabs do not fit,
/// the line scrolls to keep the current tab in view, as little as possible
/// from the last frame, and counts the tabs off each end (`‹ 3`, `5 ›`),
/// colored by the most pressing state among them.
fn tab_line(app: &App, area: Rect) -> Line<'_> {
    let look = app.look();
    let default = TabColors::default();
    let colors = app.view.as_ref().map_or(&default, |view| &view.tab_colors);
    let attention = |key: &String| app.attention.get(key).copied().unwrap_or_default();
    let spans: Vec<Span> = app
        .tabs
        .iter()
        .map(|key| {
            let selected = Some(key) == app.current.as_ref();
            tab(
                look,
                super::tabs::label(key),
                selected,
                attention(key),
                colors,
            )
        })
        .collect();
    let widths: Vec<u16> = spans
        .iter()
        .map(|span| span.content.width() as u16 + 1)
        .collect();
    // Pinned tabs always show; the rest scroll in the room they leave.
    let pinned = app.pinned.min(app.tabs.len());
    let room = area
        .width
        .saturating_sub(widths[..pinned].iter().sum::<u16>());
    let position = app
        .current
        .as_ref()
        .and_then(|current| app.tabs.iter().position(|key| key == current));
    // A hidden squad opened by name or from the switcher is not on the line;
    // it leads it, selected and marked, so the board says what it shows.
    let shown_hidden = match (&app.current, position) {
        (Some(key), None) => {
            let label = format!("{} (hidden)", super::tabs::label(key));
            Some(tab(look, &label, true, attention(key), colors))
        }
        _ => None,
    };
    let reserved = shown_hidden
        .as_ref()
        .map_or(0, |span| span.content.width() as u16 + 1);
    let room = room.saturating_sub(reserved);
    let current = position.and_then(|index| index.checked_sub(pinned));
    let (start, end) = tab_window(&widths[pinned..], current, app.tab_start.get(), room);
    app.tab_start.set(start);
    let (start, end) = (start + pinned, end + pinned);
    let off = |keys: &[String]| {
        let sum = keys
            .iter()
            .map(attention)
            .fold(Attention::default(), |sum, one| Attention {
                waiting: sum.waiting + one.waiting,
                blocked: sum.blocked + one.blocked,
            });
        match sum.state() {
            "waiting" => look.named(&colors.waiting),
            "blocked" => look.named(&colors.blocked),
            _ => look.named("dim"),
        }
    };
    let mut line = Vec::new();
    let mut x = area.x;
    if let Some(span) = shown_hidden {
        x = x.saturating_add(reserved);
        line.push(span);
        line.push(Span::raw(" "));
    }
    let mut spans: Vec<Option<Span>> = spans.into_iter().map(Some).collect();
    let mut draw = |index: usize, line: &mut Vec<Span<'static>>, x: &mut u16| {
        app.tab_hits.borrow_mut().push(TabHit {
            y: area.y,
            x: *x,
            width: widths[index] - 1,
            tab: index,
        });
        *x = x.saturating_add(widths[index]);
        line.push(spans[index].take().expect("each tab is drawn once"));
        line.push(Span::raw(" "));
    };
    for index in 0..pinned {
        draw(index, &mut line, &mut x);
    }
    if start > pinned {
        let text = format!("‹ {} ", start - pinned);
        x = x.saturating_add(text.width() as u16);
        line.push(Span::styled(text, off(&app.tabs[pinned..start])));
    }
    for index in start..end {
        draw(index, &mut line, &mut x);
    }
    if end < app.tabs.len() {
        line.push(Span::styled(
            format!("{} › ", app.tabs.len() - end),
            off(&app.tabs[end..]),
        ));
    }
    if let Some(started) = app.loading_since
        && started.elapsed() >= SPINNER_DELAY
    {
        let frame = (started.elapsed().as_millis() / 100) as usize % SPINNER.len();
        line.push(Span::styled(
            format!("{} loading", SPINNER[frame]),
            look.named("dim"),
        ));
    }
    Line::from(line)
}

/// The tabs `[start, end)` that fit in `room` columns with the overflow
/// counts, keeping `current` in view and starting as near `previous` as it
/// allows. A tab wider than the whole line still shows, cut at the edge.
fn tab_window(
    widths: &[u16],
    current: Option<usize>,
    previous: usize,
    room: u16,
) -> (usize, usize) {
    let count = widths.len();
    if widths
        .iter()
        .map(|width| usize::from(*width))
        .sum::<usize>()
        <= usize::from(room)
    {
        return (0, count);
    }
    // Room for one count, whichever end it is on: "‹ N " or "N › ".
    let counter = count.to_string().len() + 3;
    let current = current.unwrap_or(0).min(count.saturating_sub(1));
    let mut start = previous.min(current);
    loop {
        let mut used = if start > 0 { counter } else { 0 };
        let mut end = start;
        while end < count {
            let right = if end + 1 < count { counter } else { 0 };
            if used + usize::from(widths[end]) + right > usize::from(room) && end > start {
                break;
            }
            used += usize::from(widths[end]);
            end += 1;
        }
        if current < end || start >= current {
            return (start, end.max(start + 1));
        }
        start += 1;
    }
}

/// The second header line: the shown squad's summary or delayed loading indicator.
fn summary_line(app: &App) -> Line<'_> {
    let look = app.look();
    if let Some(frame) = spinner_frame(app, std::time::Instant::now()) {
        return Line::from(Span::styled(
            format!("{} loading", SPINNER[frame]),
            look.role(Role::Accent).add_modifier(Modifier::BOLD),
        ));
    }
    let Some(view) = app.view.as_ref().filter(|_| !app.loading()) else {
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
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    // The built-in tabs have one row per squad: a person leading two squads
    // is two rows on the leads tab.
    let rows = || {
        view.document["sections"][0]["rows"]
            .as_array()
            .map_or(0, Vec::len)
    };
    let summary = if app.current.as_deref() == Some(super::LEADS) {
        plural(rows(), "squad lead", "squad leads")
    } else if app.current.as_deref() == Some(super::ALL) {
        plural(rows(), "squad", "squads")
    } else {
        format!("{lead} · {}", plural(count, "member", "members"))
    };
    let mut spans = vec![Span::styled(summary, look.role(Role::Muted))];
    let waiting = app
        .current
        .as_ref()
        .and_then(|squad| app.attention.get(squad))
        .map_or(0, |attention| attention.waiting);
    if waiting > 0 {
        spans.push(Span::styled(
            format!(" · {waiting} waiting on you"),
            look.named(&view.tab_colors.waiting),
        ));
    }
    if view.document["olderRequestsNotShown"] == true {
        spans.push(Span::styled(
            " · older requests not shown",
            look.named("dim"),
        ));
    }
    if let Some(notice) = &view.theme_notice {
        spans.push(Span::styled(
            format!(" · {notice}"),
            look.role(Role::Waiting),
        ));
    }
    Line::from(spans)
}

pub fn render(frame: &mut Frame, app: &App) {
    let look = app.look();
    frame.render_widget(Block::new().style(look.role(Role::Text)), frame.area());
    app.hits.borrow_mut().clear();
    app.tab_hits.borrow_mut().clear();
    app.scrolls.begin_frame();
    let [tabs, summary, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(Paragraph::new(tab_line(app, tabs)), tabs);
    frame.render_widget(Paragraph::new(summary_line(app)), summary);
    render_body(frame, app, body);
    let footer_line = if let Some(input) = &app.input {
        Line::from(format!("{} › {}▏", input.prompt, input.text))
    } else if app.searching {
        Line::from(format!("/{}▏", app.search))
    } else if let Some(notice) = &app.notice {
        Line::from(Span::styled(notice.as_str(), look.named("amber")))
    } else if let Some(error) = &app.error {
        Line::from(Span::styled(error.as_str(), look.named("red")))
    } else {
        Line::from(Span::styled(hints(app), look.role(Role::Muted)))
    };
    frame.render_widget(Paragraph::new(footer_line), footer);
    if app.help {
        let lines = help_lines(app);
        let height = (lines.len() as u16).min(body.height);
        let area = Rect { height, ..body };
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
                .style(look.role(Role::Text)),
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
            Paragraph::new(lines).style(look.role(Role::Text)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(format!(" {} · Enter runs, Esc closes ", menu.title)),
            ),
            area,
        );
    }
    if let Some(switcher) = &app.switcher {
        render_switcher(frame, app, switcher, body);
    }
}

/// The quick switcher: the query, then the matching tabs with their counts
/// and state colors; hidden ones are marked.
fn render_switcher(frame: &mut Frame, app: &App, switcher: &Switcher, body: Rect) {
    let look = app.look();
    let keys = app.switchable();
    let found = super::tabs::matching(&keys, &switcher.query);
    let default = TabColors::default();
    let colors = app.view.as_ref().map_or(&default, |view| &view.tab_colors);
    let width = body.width.min(48);
    let height = (found.len() as u16 + 3)
        .clamp(4, body.height.max(4))
        .min(body.height);
    let area = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + (body.height - height) / 2,
        width,
        height,
    };
    let inner = usize::from(width.saturating_sub(2));
    let shown = usize::from(height.saturating_sub(3));
    let first = switcher.selected.saturating_sub(shown.saturating_sub(1));
    let mut lines = vec![Line::from(format!(" › {}▏", switcher.query))];
    if found.is_empty() {
        lines.push(Line::from(Span::styled(
            " (no matching tab)",
            look.named("dim"),
        )));
    }
    for (index, key) in found.iter().enumerate().skip(first).take(shown) {
        let attention = app.attention.get(*key).copied().unwrap_or_default();
        let mut text = format!(" {}", super::tabs::label(key));
        if attention.waiting > 0 {
            text.push_str(&format!(" ◆{}", attention.waiting));
        }
        if attention.blocked > 0 {
            text.push_str(&format!(" !{}", attention.blocked));
        }
        if app.hidden.contains(*key) {
            text.push_str(" (hidden)");
        }
        let style = match attention.state() {
            "waiting" => look.named(&colors.waiting),
            "blocked" => look.named(&colors.blocked),
            _ => Style::new(),
        };
        let style = if index == switcher.selected {
            style.add_modifier(Modifier::REVERSED)
        } else {
            style
        };
        lines.push(Line::from(Span::styled(fit(&text, inner), style)));
    }
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).style(look.role(Role::Text)).block(
            Block::new()
                .borders(Borders::ALL)
                .title(" switch · Enter opens, Esc closes "),
        ),
        area,
    );
}

/// Split mode tiles the configured panes; tabs mode shows the focused pane
/// under a tab bar. The focused pane's border is highlighted.
fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(view) = &app.view else {
        render_rows(frame, app, area);
        return;
    };
    let board = &view.board;
    let focused = app.focused();
    let pane_block = |pane: Pane| {
        let title = match pane {
            // The lead's notes nobody updated for a while say how long.
            Pane::Notes => {
                let lead = view.document["squad"]["lead"]["name"]
                    .as_str()
                    .unwrap_or("no lead");
                let mut title = vec![Span::raw(format!(" notes · {lead} "))];
                if let Some(age) =
                    crate::staleness::label(&view.document["squad"]["notesStaleness"])
                {
                    title.push(Span::styled(format!("· {age} "), look.role(Role::Waiting)));
                }
                Line::from(title)
            }
            other => Line::from(format!(" {} ", other.title())),
        };
        let style = if pane == focused && board.panes.len() > 1 {
            look.role(Role::Accent).add_modifier(Modifier::BOLD)
        } else {
            look.named("dim")
        };
        Block::new()
            .borders(Borders::ALL)
            .border_style(style)
            .title_style(if pane == focused && board.panes.len() > 1 {
                look.role(Role::Accent).add_modifier(Modifier::BOLD)
            } else {
                look.role(Role::Muted)
            })
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
                spans.push(pane_tab(look, pane.title(), *pane == focused));
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
    let look = app.look();
    let Some(view) = &app.view else { return };
    let text = match &view.notes {
        Notes::Text(text) => text.as_str(),
        Notes::Missing => "(no notes yet)",
        Notes::NoLead => "(the squad has no lead)",
        Notes::NotShown => "",
        Notes::Failed(error) => error.as_str(),
    };
    let width = usize::from(area.width);
    let mut derived = view.derived.borrow_mut();
    if derived
        .notes
        .as_ref()
        .is_none_or(|(cached_width, _)| *cached_width != width)
    {
        let lines: Vec<Line> = match (&view.notes, view.render) {
            (Notes::Text(text), NotesRender::Markdown) => markdown::render(text, width, look),
            (Notes::Text(text), NotesRender::Plain) => {
                wrap(text, width).into_iter().map(Line::from).collect()
            }
            _ => wrap(text, width)
                .into_iter()
                .map(|line| Line::styled(line, look.named("dim")))
                .collect(),
        };
        derived.notes = Some((width, lines));
    }
    let lines = &derived.notes.as_ref().expect("prepared notes").1;
    app.scrolls
        .show(frame, Pane::Notes, area, lines, look.named("dim"));
}

/// Lines of one reply body shown before it is cut.
const BODY_LINES: usize = 6;

/// Finals to the user's squad requests, newest first. Bodies are
/// agent-written, so they are sanitized like notes; reading acknowledges
/// nothing.
pub fn reply_lines(
    look: crate::look::Look,
    replies: &[Value],
    width: usize,
    now_ms: u64,
) -> Vec<Line<'static>> {
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
            look.named("dim"),
        ));
        match reply["response"].as_str() {
            Some(response) => {
                let body = wrap(&sanitize(response), width.saturating_sub(2));
                for line in body.iter().take(BODY_LINES) {
                    lines.push(Line::from(format!("  {line}")));
                }
                if body.len() > BODY_LINES {
                    lines.push(Line::styled("  …", look.named("dim")));
                }
            }
            None => {
                let id = reply["requestId"].as_str().unwrap_or_default();
                let hint = if index >= BODIES || reply["status"] == "retained" {
                    format!("  tmt result {id}")
                } else {
                    format!("  (final {})", text("status"))
                };
                lines.push(Line::styled(fit(&hint, width), look.named("dim")));
            }
        }
        lines.push(Line::from(""));
    }
    lines
}

fn render_replies(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(view) = &app.view else { return };
    let lines = if view.replies.is_empty() {
        vec![Line::styled(
            if view.me.is_none() {
                "(tmt squad me <name> shows the replies to your requests)"
            } else {
                "(no replies to your squad requests yet)"
            },
            look.named("dim"),
        )]
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        reply_lines(look, &view.replies, usize::from(area.width), now)
    };
    app.scrolls
        .show(frame, Pane::Replies, area, lines, look.named("dim"));
}

/// The selected row: where it is, what it is doing and what it waits on.
fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(row) = app.selected_row() else {
        frame.render_widget(
            Paragraph::new(Span::styled("(no row selected)", look.named("dim"))),
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
            look.named("amber"),
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
        .show(frame, Pane::Detail, area, lines, look.named("dim"));
}

fn render_rows(frame: &mut Frame, app: &App, area: Rect) {
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
        // Two cells for the row mark, and room at the right for the widest age
        // mark when a row has one, unless that would hide a column: then the
        // marks give way.
        let available = usize::from(area.width).saturating_sub(2);
        let widths = grid::solve(&tracks, Some(available), GAP);
        let ages = app
            .items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Row(row) => crate::staleness::label(&row["staleness"]),
                Item::Header(_) => None,
            })
            .map(|age| age.width() + GAP)
            .max();
        let widths = match ages {
            Some(age) => {
                let reserved = grid::solve(&tracks, Some(available.saturating_sub(age)), GAP);
                let shown = |widths: &[Option<usize>]| widths.iter().flatten().count();
                if shown(&reserved) == shown(&widths) {
                    reserved
                } else {
                    widths
                }
            }
            None => widths,
        };
        derived.grid = Some(super::derived::Grid {
            width: available,
            search: app.search.clone(),
            widths,
        });
    }
    let widths = &derived.grid.as_ref().expect("prepared grid").widths;
    let note_column = rows.fields().contains(&"note");
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "  {}",
            rows.columns
                .iter()
                .zip(widths)
                .filter_map(|(column, width)| width.map(|width| grid::fit(
                    &column.title,
                    width,
                    column.align,
                    column.truncate
                )))
                .collect::<Vec<_>>()
                .join(&" ".repeat(GAP))
        ),
        look.role(Role::Muted),
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
                for (index, cells) in rows.lines.iter().enumerate() {
                    let first = index == 0;
                    let Some(cells) =
                        grid_line(look, rows, widths, cells, row, first, &view.colors)
                    else {
                        continue;
                    };
                    let mut spans = vec![Span::raw(if first { marker } else { "  " })];
                    spans.extend(cells);
                    if let Some(age) = age.as_deref().filter(|_| first) {
                        age_mark(&mut spans, age, usize::from(area.width), look);
                    }
                    row_lines.push((lines.len(), row_index));
                    lines.push(Line::from(spans).style(style));
                }
                if let Some(note) = row["note"].as_str().filter(|_| !note_column) {
                    lines.push(Line::from(Span::styled(
                        fit(&format!("    note {note}"), usize::from(area.width)),
                        look.named("dim"),
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
                        look.named("dim"),
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
            look.named("dim"),
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
        .show(frame, Pane::Rows, area, lines, look.named("dim"));
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
            tabs: vec!["product".into(), "reviews".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("product".into()),
            view: Ok(View {
                derived: Default::default(),
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
                look: Default::default(),
                theme_notice: None,
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
        let help = draw(&app, 60, 26);
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
        let help = draw(&app, 60, 26);
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
                .any(|line| line.contains("switch tab"))
        );
        let mut failed = App::new(Some("product".into()));
        failed.error = Some("tmt did not finish in time".into());
        assert_eq!(draw(&failed, 40, 4)[3], "tmt did not finish in time");
    }

    fn paned(board: crate::config::Board, notes: Notes) -> App {
        let mut app = App::new(Some("product".into()));
        app.apply(Snapshot {
            tabs: vec!["product".into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some("product".into()),
            view: Ok(View {
                derived: Default::default(),
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
                look: Default::default(),
                theme_notice: None,
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
    fn a_cell_shows_its_resolved_color_token_as_decoration() {
        let mut app = board(json!([{"title": null, "rows": [
            row("auth-fix", "working", "rotate", json!({"colors": {"task": "blocked"}})),
            row("docs", "working", "write", json!({
                "colors": {"task": "review"},
                "staleness": {"state": "stale", "ageMs": 3_600_000},
            })),
            row("ci", "working", "fix", json!({})),
        ]}]));
        app.selected = 0;
        let cell = |app: &App, name: &str| {
            let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
            terminal.draw(|frame| render(frame, app)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            let screen = draw(app, 60, 8);
            let y = screen.iter().position(|line| line.contains(name)).unwrap();
            let x = screen[y].find(match name {
                "auth-fix" => "rotate",
                "docs" => "write",
                _ => "fix",
            });
            buffer[(x.unwrap() as u16, y as u16)].clone()
        };
        let role = |app: &App, role: Role| app.look().role(role).fg.unwrap_or_default();
        // The selected row keeps its background and the cell keeps its token.
        let selected = cell(&app, "auth-fix");
        assert_eq!(selected.fg, role(&app, Role::Blocked));
        assert_eq!(Some(selected.bg), app.look().selection().bg);
        assert!(!selected.modifier.contains(Modifier::REVERSED));
        // On a stale (dim) row the cell's own color still shows.
        assert_eq!(cell(&app, "docs").fg, role(&app, Role::Review));
        // No token: no color of its own.
        assert_ne!(cell(&app, "ci").fg, role(&app, Role::Blocked));
        // Without color the text is all there is.
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::default(),
            depth: tmt_cli_style::Depth::None,
        };
        assert_eq!(cell(&app, "auth-fix").fg, ratatui::style::Color::Reset);
    }

    #[test]
    fn light_body_chrome_and_selection_use_the_theme_and_no_color_keeps_focus() {
        let mut app = board(json!([{ "title": null, "rows": [
            row("docs", "working", "write", json!({})),
            row("ci", "working", "fix", json!({})),
            row("empty", "", "", json!({"fields": {}})),
        ] }]));
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            app.view.as_mut().unwrap().look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::TmtLight),
                depth,
            };
            let mut terminal = Terminal::new(TestBackend::new(60, 7)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let fg = |role| app.look().role(role).fg.unwrap_or_default();
            // Readable chrome and explicit body text follow the light theme.
            for (x, y) in [(1, 1), (2, 2), (1, 6), (12, 0)] {
                assert_eq!(buffer[(x, y)].fg, fg(Role::Muted), "chrome {x},{y}");
            }
            assert_eq!(buffer[(2, 4)].fg, fg(Role::Text));
            assert_eq!(buffer[(13, 5)].fg, fg(Role::Dim));
            assert_eq!(buffer[(22, 5)].fg, fg(Role::Dim));
            assert_eq!(buffer[(1, 0)].fg, fg(Role::Accent));
            assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));
            let selected = &buffer[(2, 3)];
            assert_eq!(selected.fg, fg(Role::Text));
            assert_eq!(selected.bg, app.look().selection().bg.unwrap_or_default());
            assert_eq!(
                selected.modifier.contains(Modifier::REVERSED),
                depth != tmt_cli_style::Depth::TrueColor
            );
            assert_eq!(
                buffer[(1, 0)].modifier.contains(Modifier::REVERSED),
                depth == tmt_cli_style::Depth::None
            );
        }
    }

    #[test]
    fn stale_rows_and_notes_are_quiet_and_say_how_old() {
        let age = |state: &str, ms: u64| json!({"state": state, "ageMs": ms});
        let mut app = board(json!([
            {"title": "working", "rows": [
                row("auth-fix", "working", "rotate", json!({"staleness": age("stale", 3 * 3_600_000)})),
                row("docs", "working", "write", json!({"staleness": age("fresh", 60_000)})),
                row("ci", "working", "fix", json!({"staleness": {"state": "unknown"}})),
            ]},
            // The same member twice: every row of it carries the same age.
            {"title": "again", "rows": [
                row("auth-fix", "working", "rotate", json!({"staleness": age("stale", 3 * 3_600_000)})),
            ]},
        ]));
        app.selected = 1;
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen = draw(&app, 60, 12);
        let line_of = |name: &str, from: usize| {
            from + screen[from..]
                .iter()
                .position(|line| line.contains(name))
                .unwrap()
        };
        let stale = line_of("auth-fix", 0);
        assert!(screen[stale].ends_with("stale 3h"), "{screen:#?}");
        let dim = app.look().role(Role::Dim).fg;
        assert_eq!(Some(buffer[(4, stale as u16)].fg), dim, "the row is quiet");
        let again = line_of("auth-fix", stale + 1);
        assert!(screen[again].ends_with("stale 3h"), "repeated rows agree");
        for name in ["docs", "ci"] {
            let line = line_of(name, 0);
            assert!(!screen[line].contains("stale"), "{name}: no mark");
            assert_ne!(Some(buffer[(4, line as u16)].fg), dim);
        }

        // Narrow: the age goes first, the cells stay.
        let narrow = draw(&app, 24, 12);
        let line = narrow.iter().find(|line| line.contains("auth")).unwrap();
        assert!(!line.contains("stale"), "{narrow:#?}");

        // Without color the age is still there in words.
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::default(),
            depth: tmt_cli_style::Depth::None,
        };
        assert!(draw(&app, 60, 12)[stale].ends_with("stale 3h"));
    }

    #[test]
    fn stale_lead_notes_say_so_on_the_pane_title() {
        let mut app = paned(
            split(
                Direction::LeftRight,
                vec![Pane::Rows, Pane::Notes],
                vec![50, 50],
            ),
            Notes::Text("ship it".into()),
        );
        let title = |app: &App| draw(app, 70, 8)[2].clone();
        assert!(!title(&app).contains("stale"), "unknown notes: no mark");
        app.view.as_mut().unwrap().document["squad"]["notesStaleness"] =
            json!({"state": "stale", "ageMs": 2 * 3_600_000});
        let line = title(&app);
        assert!(line.contains("notes · sol · stale 2h"), "{line}");
        let mut terminal = Terminal::new(TestBackend::new(70, 8)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let at = line[..line.find("stale 2h").unwrap()].chars().count() as u16;
        assert_eq!(
            Some(buffer[(at, 2)].fg),
            app.look().role(Role::Waiting).fg,
            "the notes' age asks for attention"
        );
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
        let lines: Vec<String> = reply_lines(crate::look::Look::default(), &replies, 40, 100_000)
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
        // Waiting wins over blocked; selection is bold without moving the tab.
        let fg = |role| app.look().role(role).fg.unwrap_or_default();
        assert_eq!(product.fg, fg(Role::Waiting));
        assert!(product.modifier.contains(Modifier::BOLD));
        assert!(!product.modifier.contains(Modifier::REVERSED));
        let reviews = &buffer[(column("reviews"), 0)];
        assert_eq!(reviews.fg, fg(Role::Blocked));
        assert!(!reviews.modifier.contains(Modifier::REVERSED));

        let screen = draw(&app, 60, 6);
        assert_eq!(screen[1], "lead sol · 1 member · 1 waiting on you");
        // Colors come from [tabs.colors]; without a lead the summary says so.
        let view = app.view.as_mut().unwrap();
        view.tab_colors.blocked = "review".into();
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
        assert_eq!(
            buffer[(reviews, 0)].fg,
            app.look().role(Role::Review).fg.unwrap_or_default()
        );
        assert_eq!(draw(&app, 60, 6)[1], "no lead · 1 member");
    }

    #[test]
    fn the_leads_tab_is_labelled_leads_and_counts_squad_leads() {
        let mut view = board(json!([{"title": null, "rows": [
            row("sol", "working", "plan", json!({"fields": {"squad": "product", "state": "working", "task": "plan"}})),
            row("rin", "blocked", "ci", json!({"fields": {"squad": "infra", "state": "blocked", "task": "ci"}})),
        ]}]))
        .view
        .take()
        .unwrap();
        view.rows = crate::rows::Rows::leads();
        let mut app = App::new(Some(crate::board::LEADS.into()));
        app.apply(Snapshot {
            tabs: vec!["product".into(), crate::board::LEADS.into()],
            hidden: Vec::new(),
            pinned: 0,
            attention: Default::default(),
            squad: Some(crate::board::LEADS.into()),
            view: Ok(view),
        });
        let screen = draw(&app, 60, 6);
        assert_eq!(screen[0], " product   leads");
        assert_eq!(screen[1], "2 squad leads");
        assert_eq!(screen[2], "  SQUAD          LEAD           STATE      TASK");
        assert_eq!(screen[3], "  product        sol            working    plan");
    }

    #[test]
    fn tabs_move_with_shift_arrows_or_a_drag_and_the_order_is_saved() {
        use crate::board::app::{Effect, Request};
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs.push(crate::board::LEADS.into());
        let order = |app: &App| app.tabs.clone();
        let shift = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
        assert_eq!(
            app.key(shift(KeyCode::Right)),
            Effect::Act(Request::Reorder(
                ["reviews", "product", crate::board::LEADS]
                    .map(String::from)
                    .to_vec()
            ))
        );
        assert_eq!(app.current.as_deref(), Some("product"), "still shown");
        assert_eq!(
            app.key(shift(KeyCode::Right)),
            Effect::Act(Request::Reorder(order(&app)))
        );
        assert_eq!(
            app.key(shift(KeyCode::Right)),
            Effect::None,
            "no wrap at the end"
        );

        // Drag: press on the first tab (showing it), release over the last.
        let screen = draw(&app, 60, 6);
        assert_eq!(screen[0], " reviews   leads   product");
        let mouse = |kind, column| MouseEvent {
            kind,
            column,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            app.mouse(
                mouse(MouseEventKind::Down(MouseButton::Left), 2),
                std::time::Instant::now()
            ),
            Effect::Load("reviews".into())
        );
        assert_eq!(
            app.mouse(
                mouse(MouseEventKind::Up(MouseButton::Left), 22),
                std::time::Instant::now()
            ),
            Effect::Act(Request::Reorder(
                ["leads", "product", "reviews"]
                    .map(|name| if name == "leads" {
                        crate::board::LEADS.to_owned()
                    } else {
                        name.to_owned()
                    })
                    .to_vec()
            ))
        );
        // Releasing off the tab line moves nothing.
        app.mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 2),
            std::time::Instant::now(),
        );
        let mut away = mouse(MouseEventKind::Up(MouseButton::Left), 2);
        away.row = 4;
        assert_eq!(app.mouse(away, std::time::Instant::now()), Effect::None);
    }

    #[test]
    fn many_tabs_scroll_to_keep_the_current_one_and_count_the_rest() {
        // Every tab is 8 columns with its gap; 40 columns hold four, or three
        // beside one count.
        let widths = [8u16; 10];
        assert_eq!(tab_window(&[8, 8], Some(1), 0, 40), (0, 2), "all fit");
        assert_eq!(tab_window(&widths, Some(0), 0, 40), (0, 4));
        assert_eq!(tab_window(&widths, Some(3), 0, 40), (0, 4));
        // Moving right scrolls only as far as needed, then left keeps it.
        let (start, end) = tab_window(&widths, Some(4), 0, 40);
        assert!(start > 0 && (start..end).contains(&4), "{start}..{end}");
        assert_eq!(tab_window(&widths, Some(4), start, 40), (start, end));
        assert_eq!(tab_window(&widths, Some(9), start, 40).1, 10);
        assert_eq!(tab_window(&widths, Some(2), 5, 40).0, 2);
        // A tab wider than the line still shows.
        assert_eq!(tab_window(&[80, 8], Some(0), 0, 40), (0, 1));

        let names: Vec<String> = (0..9).map(|n| format!("sq{n}")).collect();
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs = names.clone();
        app.current = Some("sq7".into());
        app.attention.insert(
            "sq1".into(),
            Attention {
                waiting: 0,
                blocked: 1,
            },
        );
        app.attention.insert(
            "sq8".into(),
            Attention {
                waiting: 2,
                blocked: 0,
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(32, 6)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let line: String = (0..32)
            .map(|x| buffer[(x, 0)].symbol().to_owned())
            .collect();
        assert!(line.starts_with("‹ "), "{line:?}");
        assert!(
            line.contains(" sq7 "),
            "the current tab stays in view: {line:?}"
        );
        assert!(line.trim_end().ends_with("1 ›"), "{line:?}");
        // The left count hides a blocked tab, the right one a waiting tab.
        assert_eq!(
            buffer[(0, 0)].fg,
            app.look().role(Role::Blocked).fg.unwrap()
        );
        let right = line.trim_end().chars().count() as u16 - 1;
        assert_eq!(
            buffer[(right, 0)].fg,
            app.look().role(Role::Waiting).fg.unwrap()
        );
        // Only shown tabs can be clicked, at their drawn places.
        let hits = app.tab_hits.borrow().clone();
        assert!(hits.iter().all(|hit| hit.tab >= app.tab_start.get()));
        let seven = hits.iter().find(|hit| hit.tab == 7).unwrap();
        let at = line[..line.find(" sq7").unwrap()].chars().count() as u16;
        assert_eq!(seven.x, at);
    }

    #[test]
    fn a_hidden_squad_being_shown_leads_the_tab_line_selected() {
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs = (0..9).map(|n| format!("sq{n}")).collect();
        app.hidden = vec!["quiet".into()];
        app.current = Some("quiet".into());
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let line: String = (0..40)
            .map(|x| buffer[(x, 0)].symbol().to_owned())
            .collect();
        assert!(line.starts_with(" quiet (hidden) "), "{line:?}");
        assert!(buffer[(1, 0)].modifier.contains(Modifier::BOLD));
        assert!(line.trim_end().ends_with(" ›"), "{line:?}");
        // It is not one of the tabs, so it cannot be clicked or dragged, and
        // the tabs after it are hit where they are drawn.
        let hits = app.tab_hits.borrow().clone();
        let first = hits.iter().find(|hit| hit.tab == 0).unwrap();
        let at = line[..line.find(" sq0").unwrap()].chars().count() as u16;
        assert_eq!(first.x, at, "{line:?}");
        assert!(hits.iter().all(|hit| hit.x >= at));
    }

    #[test]
    fn the_switcher_filters_every_tab_and_opens_the_chosen_one() {
        use crate::board::app::Effect;
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs.push(crate::board::LEADS.into());
        app.hidden = vec!["quiet".into()];
        app.attention.insert(
            "reviews".into(),
            Attention {
                waiting: 1,
                blocked: 0,
            },
        );
        let press = |app: &mut App, code| app.key(KeyEvent::new(code, KeyModifiers::NONE));
        press(&mut app, KeyCode::Char('s'));
        let screen = draw(&app, 60, 12);
        let body = screen.join("\n");
        for expected in [
            "switch · Enter opens",
            " product",
            " reviews ◆1",
            " leads",
            " quiet (hidden)",
        ] {
            assert!(body.contains(expected), "{expected}: {body}");
        }
        for character in "qt".chars() {
            press(&mut app, KeyCode::Char(character));
        }
        let screen = draw(&app, 60, 12);
        let listed: Vec<&str> = screen
            .iter()
            .filter_map(|line| line.split('│').nth(1))
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(listed, ["› qt▏", "quiet (hidden)"], "{screen:#?}");
        // Enter shows the hidden squad; the switcher closes.
        assert_eq!(
            press(&mut app, KeyCode::Enter),
            Effect::Load("quiet".into())
        );
        assert!(app.switcher.is_none());
        // Esc closes without switching; `s` bound by the user runs the binding.
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.switcher.is_none());
        app.view.as_mut().unwrap().bindings =
            crate::action::parse_bindings([("s", Some("refresh"))].into_iter(), "bind").unwrap();
        assert_eq!(press(&mut app, KeyCode::Char('s')), Effect::Refresh);
        assert!(app.switcher.is_none());
    }

    #[test]
    fn pinned_tabs_stay_in_view_and_keep_their_pin_order() {
        use crate::board::app::Effect;
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs = std::iter::once(crate::board::ALL.to_owned())
            .chain((0..9).map(|n| format!("sq{n}")))
            .collect();
        app.pinned = 1;
        app.current = Some("sq8".into());
        let line = draw(&app, 36, 6)[0].clone();
        assert!(
            line.starts_with(" all  ‹ 5 "),
            "the pin stays first: {line:?}"
        );
        assert!(
            line.ends_with(" sq8"),
            "the current tab is in view: {line:?}"
        );
        let hits = app.tab_hits.borrow().clone();
        assert_eq!(hits[0].tab, 0);
        assert_eq!(hits[0].x, 0);
        // A pin neither moves nor is passed; the other tabs move among
        // themselves. (A saved `order` could not reorder the pins.)
        let shift = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
        let refused = Some("Pinned tabs keep the order in [tabs] pin.");
        app.current = Some("sq0".into());
        assert_eq!(app.key(shift(KeyCode::Left)), Effect::None);
        assert_eq!(app.notice.as_deref(), refused);
        app.pinned = 2;
        app.current = Some(crate::board::ALL.into());
        app.notice = None;
        assert_eq!(app.key(shift(KeyCode::Right)), Effect::None);
        assert_eq!(app.notice.as_deref(), refused);
        assert_eq!(app.tabs[..2], [crate::board::ALL, "sq0"]);
        app.pinned = 1;
        app.current = Some("sq0".into());
        assert!(matches!(app.key(shift(KeyCode::Right)), Effect::Act(_)));
        assert_eq!(app.tabs[..3], [crate::board::ALL, "sq1", "sq0"]);
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
            during[1].contains("loading"),
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
    fn a_click_focuses_the_pane_under_it() {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
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
        let click = |app: &mut App, column, row| {
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now(),
            )
        };
        draw(&app, 60, 11);
        assert_eq!(app.focused(), Pane::Rows);
        // A click in the notes focuses them; keys then scroll the notes.
        assert_eq!(click(&mut app, 45, 5), crate::board::Effect::None);
        assert_eq!(app.focused(), Pane::Notes);
        assert_eq!(app.selected, 0);
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let screen = draw(&app, 60, 11);
        assert!(
            !screen.iter().any(|line| line.contains("line 01")),
            "{screen:#?}"
        );
        assert!(screen.iter().any(|line| line.contains("line 02")));
        // The rows border now shows the notes as focused.
        let mut terminal = Terminal::new(TestBackend::new(60, 11)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        assert!(buffer[(31, 2)].modifier.contains(Modifier::BOLD));
        assert!(!buffer[(1, 2)].modifier.contains(Modifier::BOLD));
        // A click on a row focuses the rows again and selects it.
        let hit = app.hits.borrow()[0];
        click(&mut app, hit.x, hit.y);
        assert_eq!(app.focused(), Pane::Rows);
        assert_eq!(app.selected, hit.row);
        // Outside every pane, or under the help, a click changes nothing.
        click(&mut app, 45, 5);
        assert_eq!(app.focused(), Pane::Notes);
        click(&mut app, 5, 0);
        assert_eq!(app.focused(), Pane::Notes, "the header is not a pane");
        app.help = true;
        click(&mut app, 5, 5);
        assert_eq!(app.focused(), Pane::Notes, "the help takes no clicks");
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
    #[test]
    fn loading_is_delayed_animated_and_absent_on_a_cached_switch() {
        let mut app = board(json!([]));
        let started = std::time::Instant::now();
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.loading_since = Some(started);
        assert_eq!(
            spinner_frame(
                &app,
                started + SPINNER_DELAY - std::time::Duration::from_millis(1)
            ),
            None
        );
        assert_eq!(spinner_frame(&app, started + SPINNER_DELAY), Some(0));
        assert_eq!(
            spinner_frame(&app, started + SPINNER_DELAY + SPINNER_TICK),
            Some(1)
        );
        app.apply(crate::board::app::tests::snapshot("reviews", json!([])));
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(!app.loading());
        assert_eq!(spinner_frame(&app, std::time::Instant::now()), None);
        assert!(
            draw(&app, 60, 8)
                .iter()
                .all(|line| !line.contains("loading"))
        );
    }

    #[test]
    fn derivations_survive_selection_and_change_with_width_search_and_snapshot() {
        let mut app = board(
            json!([{ "title": null, "rows": [row("first", "working", "one", json!({})), row("second", "working", "two", json!({}))] }]),
        );
        let view = app.view.as_mut().unwrap();
        view.board = crate::config::Board::simple(
            BoardMode::Tabs,
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            &[],
        );
        view.notes = Notes::Text("# Notes\nA sentence that wraps at a narrow width.".into());
        draw(&app, 60, 12);
        assert_eq!(
            app.view
                .as_ref()
                .unwrap()
                .derived
                .borrow()
                .grid
                .as_ref()
                .unwrap()
                .width,
            56
        );
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(
            draw(&app, 60, 12)
                .iter()
                .any(|line| line.contains("second"))
        );
        app.search = "second".into();
        draw(&app, 35, 12);
        let grid = app.view.as_ref().unwrap().derived.borrow();
        assert_eq!(grid.grid.as_ref().unwrap().search, "second");
        assert_eq!(grid.grid.as_ref().unwrap().width, 31);
        drop(grid);
        app.focus = 1;
        draw(&app, 60, 12);
        let lines = app
            .view
            .as_ref()
            .unwrap()
            .derived
            .borrow()
            .notes
            .as_ref()
            .unwrap()
            .1
            .clone();
        draw(&app, 25, 12);
        assert_ne!(
            app.view
                .as_ref()
                .unwrap()
                .derived
                .borrow()
                .notes
                .as_ref()
                .unwrap()
                .1,
            lines
        );
        app.apply(crate::board::app::tests::snapshot("product", json!([])));
        assert!(app.view.as_ref().unwrap().derived.borrow().notes.is_none());
        assert!(app.view.as_ref().unwrap().derived.borrow().grid.is_none());
    }
    #[test]
    fn only_visible_reply_age_text_invalidates_the_clock() {
        let mut app = board(json!([]));
        let view = app.view.as_mut().unwrap();
        view.board = crate::config::Board::simple(
            BoardMode::Tabs,
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Replies],
            &[],
        );
        view.replies = vec![json!({ "submittedAtMs": 1_000 })];
        assert!(time_marks(&app, 61_000).is_empty());
        app.focus = 1;
        let first = time_marks(&app, 61_000);
        assert!(!first.is_empty());
        assert_eq!(first, time_marks(&app, 61_200));
        assert_ne!(first, time_marks(&app, 121_000));
    }
}
