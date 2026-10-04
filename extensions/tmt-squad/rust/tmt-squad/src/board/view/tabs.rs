//! Tab labels, windows and their painted hit geometry.

use super::header::{SPINNER, SPINNER_DELAY};
use crate::board::app::App;
use crate::board::app::TabHit;
use crate::{attention::Attention, config::TabColors};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::{Role, mark::Mark};
use unicode_width::UnicodeWidthStr;

pub(super) fn pane_tab(look: crate::look::Look, name: &str, selected: bool) -> Span<'static> {
    if selected {
        let selection = look.selection();
        Span::styled(
            format!("[{name}]"),
            Style {
                bg: selection.bg,
                ..look
                    .role(Role::Accent)
                    .add_modifier(Modifier::BOLD | selection.add_modifier)
            },
        )
    } else {
        Span::styled(format!(" {name} "), look.role(Role::Muted))
    }
}

/// A tab or switcher entry has one fixed mark slot and one styled label owner.
pub(super) fn tab_label(
    look: crate::look::Look,
    name: &str,
    attention: Attention,
    colors: &TabColors,
    style: Style,
) -> Line<'static> {
    let attention_style = |color: &str| {
        let foreground = look.named(color);
        Style {
            // Explicit default foreground prevents the label's accent/muted
            // foreground leaking into a mark whose configured color is default.
            fg: Some(foreground.fg.unwrap_or_default()),
            bg: style.bg,
            ..foreground
                .add_modifier(Modifier::BOLD | (style.add_modifier & Modifier::REVERSED))
                .remove_modifier(
                    style.add_modifier
                        & !(foreground.add_modifier | Modifier::BOLD | Modifier::REVERSED),
                )
        }
    };
    let (mark, count, color) = if attention.waiting > 0 {
        (Mark::Decision.symbol(), attention.waiting, &colors.waiting)
    } else if attention.blocked > 0 {
        (Mark::Failed.symbol(), attention.blocked, &colors.blocked)
    } else {
        (" ", 0, &colors.waiting)
    };
    let mut spans = vec![
        Span::styled(
            mark,
            if count > 0 {
                attention_style(color)
            } else {
                style
            },
        ),
        Span::styled(format!(" {}", tmt_cli_style::table::escape(name)), style),
    ];
    if count > 0 {
        spans.push(Span::styled(format!(" {count}"), style));
    }
    if attention.waiting > 0 && attention.blocked > 0 {
        spans.push(Span::styled(" ", style));
        spans.push(Span::styled(
            format!("{}{}", Mark::Failed.symbol(), attention.blocked),
            attention_style(&colors.blocked),
        ));
    }
    Line::from(spans).style(style)
}

pub(super) fn tab(
    look: crate::look::Look,
    name: &str,
    selected: bool,
    attention: Attention,
    colors: &TabColors,
) -> Line<'static> {
    let style = if selected {
        let selection = look.selection();
        Style {
            bg: selection.bg,
            ..look
                .role(Role::Accent)
                .add_modifier(Modifier::BOLD | selection.add_modifier)
        }
    } else {
        look.role(Role::Muted)
    };
    let mut label = tab_label(look, name, attention, colors, style);
    label.spans.push(Span::styled(" ", style));
    label
}

/// The first header line: only the tabs. Each
/// tab's place is recorded for clicks and drags. When the tabs do not fit,
/// the line scrolls to keep the current tab in view, as little as possible
/// from the last frame, and counts the tabs off each end (`‹ 3`, `5 ›`),
/// colored by the most pressing state among them.
pub(super) fn tab_line(app: &App, area: Rect) -> Line<'_> {
    let look = app.look();
    let default = TabColors::default();
    let colors = app.view.as_ref().map_or(&default, |view| &view.tab_colors);
    let attention = |key: &String| app.attention.get(key).copied().unwrap_or_default();
    let labels: Vec<Line> = app
        .tabs
        .iter()
        .map(|key| {
            let selected = Some(key) == app.current.as_ref();
            tab(
                look,
                crate::board::tabs::label(key),
                selected,
                attention(key),
                colors,
            )
        })
        .collect();
    let widths: Vec<u16> = labels
        .iter()
        .map(|label| label.width() as u16 + 1)
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
            let label = format!("{} (hidden)", crate::board::tabs::label(key));
            Some(tab(look, &label, true, attention(key), colors))
        }
        _ => None,
    };
    let reserved = shown_hidden
        .as_ref()
        .map_or(0, |label| label.width() as u16 + 1);
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
            _ => look.role(Role::Dim),
        }
    };
    let mut line = Vec::new();
    let mut x = area.x;
    if let Some(span) = shown_hidden {
        x = x.saturating_add(reserved);
        line.extend(span.spans);
        line.push(Span::raw(" "));
    }
    let mut labels: Vec<Option<Line>> = labels.into_iter().map(Some).collect();
    let mut draw = |index: usize, line: &mut Vec<Span<'static>>, x: &mut u16| {
        app.tab_hits.borrow_mut().push(TabHit {
            y: area.y,
            x: *x,
            width: widths[index] - 1,
            tab: index,
        });
        *x = x.saturating_add(widths[index]);
        line.extend(labels[index].take().expect("each tab is drawn once").spans);
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
            look.role(Role::Dim),
        ));
    }
    Line::from(line)
}

/// The tabs `[start, end)` that fit in `room` columns with the overflow
/// counts, keeping `current` in view and starting as near `previous` as it
/// allows. A tab wider than the whole line still shows, cut at the edge.
pub(super) fn tab_window(
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
