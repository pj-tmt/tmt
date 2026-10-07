//! Tab-line display after arrangement: pure window admission, then spans and hits.

use super::fit;
use crate::board::{
    app::{App, TabHit},
    tabs,
};
use crate::{attention::Attention, config::TabColors, look::Look};
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
            tmt_cli_style::table::escape(name),
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
/// A tab has one fixed mark slot; span 2 owns only its escaped name.
pub(super) fn tab_label(
    look: crate::look::Look,
    name: &str,
    attention: Attention,
    colors: &TabColors,
    style: Style,
) -> Line<'static> {
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
                tab_attention_style(look, color, style)
            } else {
                style
            },
        ),
        Span::styled(" ", style),
        Span::styled(tmt_cli_style::table::escape(name), style),
    ];
    if count > 0 {
        spans.push(Span::styled(format!(" {count}"), style));
    }
    if attention.waiting > 0 && attention.blocked > 0 {
        spans.push(Span::styled(" ", style));
        spans.push(Span::styled(
            format!("{} {}", Mark::Failed.symbol(), attention.blocked),
            tab_attention_style(look, &colors.blocked, style),
        ));
    }
    Line::from(spans).style(style)
}

fn tab_attention_style(look: Look, color: &str, style: Style) -> Style {
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
}

/// Fit the shared label through the grid owner, preserving the styles of the
/// retained prefix. Widths elsewhere come from this same rendered Line::width.
pub(super) fn fit_tab_label(mut line: Line<'static>, width: usize) -> Line<'static> {
    if line.width() <= width {
        line.spans
            .push(Span::styled(" ".repeat(width - line.width()), line.style));
        return line;
    }
    let text = line.to_string();
    let fitted = fit(&text, width);
    let mut retained: usize = text
        .chars()
        .zip(fitted.chars())
        .take_while(|(original, shown)| original == shown)
        .map(|(original, _)| original.len_utf8())
        .sum();
    let prefix = retained;
    let mut spans = Vec::new();
    for span in line.spans {
        let kept = retained.min(span.content.len());
        if kept > 0 {
            spans.push(Span::styled(span.content[..kept].to_owned(), span.style));
            retained -= kept;
        }
        if retained == 0 {
            break;
        }
    }
    spans.push(Span::styled(fitted[prefix..].to_owned(), line.style));
    Line::from(spans).style(line.style)
}

/// Fit the shown name before its semantic suffix when a name grapheme fits.
/// Otherwise retain the ordinary prefix fallback; a one-cell attention mark wins.
fn fit_shown_tab_label(mut line: Line<'static>, width: usize) -> Line<'static> {
    if line.width() <= width {
        return fit_tab_label(line, width);
    }
    let name = line.spans[2].content.to_string();
    let fixed = line.width() - line.spans[2].width();
    let room = width.saturating_sub(fixed);
    let clipped = tmt_tui::text::fit_line(
        &name,
        room.min(usize::from(u16::MAX)) as u16,
        tmt_tui::style::TextFlow::Clip,
        tmt_cli_style::grid::Align::Left,
    );
    if !clipped.trim().is_empty() {
        let fitted = fit(&name, room);
        // At the minimum width an ellipsis alone would identify no name.
        let fitted = if fitted.trim() == "…" {
            clipped
        } else {
            fitted
        };
        line.spans[2].content = fitted.into();
        line
    } else {
        line.spans[2].content = name.into();
        if width == 1 && line.spans[0].width() == 1 {
            // The fixed attention slot wins over an ellipsis at one cell.
            line.spans.truncate(1);
            line
        } else {
            fit_tab_label(line, width)
        }
    }
}

fn tab_style(look: Look, selected: bool, pending: bool) -> Style {
    if selected {
        let selection = look.selection();
        Style {
            bg: selection.bg,
            ..look
                .role(Role::Text)
                .add_modifier(Modifier::BOLD | selection.add_modifier)
        }
    } else if pending {
        look.role(Role::Muted).add_modifier(Modifier::UNDERLINED)
    } else {
        look.role(Role::Muted)
    }
}

/// Selection covers the entire tab; attention decorates only its marks.
pub(super) fn tab(
    look: crate::look::Look,
    name: &str,
    selected: bool,
    pending: bool,
    attention: Attention,
    colors: &TabColors,
) -> Line<'static> {
    let style = tab_style(look, selected, pending);
    tab_label(look, name, attention, colors, style)
}

/// The home keeps its public `all` key; focus changes only its presentation.
fn home_tab(
    look: crate::look::Look,
    selected: bool,
    pending: bool,
    attention: Attention,
    colors: &TabColors,
) -> Line<'static> {
    let style = tab_style(look, selected, pending);
    let mut spans = vec![
        Span::styled(" ", style),
        Span::styled("▚ ", style),
        Span::styled("tmt", style),
    ];
    for (count, mark, color) in [
        (attention.waiting, Mark::Decision, &colors.waiting),
        (attention.blocked, Mark::Failed, &colors.blocked),
    ] {
        if count > 0 {
            spans.push(Span::styled(" ", style));
            spans.push(Span::styled(
                format!("{} {count}", mark.symbol()),
                tab_attention_style(look, color, style),
            ));
        }
    }
    spans.push(Span::styled(" ", style));
    Line::from(spans).style(style)
}

/// Numeric display inputs, measured from the same labels the painter uses.
struct Widths {
    full: usize,
    grouped: usize,
    prefix: Option<String>,
    header: usize,
    overflow: usize,
    tier: u8,
}

/// A drawn tab's grouping and boundary decisions, shared by cost and paint.
struct Placement {
    index: usize,
    grouped: bool,
    header: bool,
}

fn group_at<'a>(widths: &'a [Widths], indices: &[usize], offset: usize) -> Option<&'a str> {
    let index = indices[offset];
    widths[index].prefix.as_deref().filter(|name| {
        let adjacent = |other: usize| {
            index.abs_diff(other) == 1 && widths[other].prefix.as_deref() == Some(*name)
        };
        offset
            .checked_sub(1)
            .is_some_and(|previous| adjacent(indices[previous]))
            || indices.get(offset + 1).is_some_and(|&next| adjacent(next))
    })
}

fn placements(widths: &[Widths], pins: &[usize], start: usize, end: usize) -> Vec<Placement> {
    let indices: Vec<_> = pins.iter().copied().chain(start..end).collect();
    indices
        .iter()
        .enumerate()
        .map(|(offset, &index)| {
            let group = group_at(widths, &indices, offset);
            Placement {
                index,
                grouped: group.is_some(),
                header: group.is_some_and(|name| {
                    offset == 0
                        || indices[offset - 1] + 1 != index
                        || widths[indices[offset - 1]].prefix.as_deref() != Some(name)
                }),
            }
        })
        .collect()
}

/// Admission output only; no application state, styles or hit-map mutation.
struct Window {
    pins: Vec<usize>,
    start: usize,
    end: usize,
    hidden: Vec<usize>,
    reserved: usize,
}

impl Window {
    fn new(
        widths: &[Widths],
        pinned: usize,
        pins: Vec<usize>,
        start: usize,
        end: usize,
        width: usize,
    ) -> Self {
        let mut hidden: Vec<_> = (0..pinned)
            .filter(|index| !pins.contains(index))
            .chain(end..widths.len())
            .collect();
        // Stable sorting retains arrangement order within each attention tier.
        hidden.sort_by_key(|&index| widths[index].tier);
        let reserved = hidden.first().map_or(0, |&first| {
            (format!("+{} › ", hidden.len()).width() + widths[first].overflow + 2)
                .min(28)
                .min(width / 2)
        });
        Self {
            pins,
            start,
            end,
            hidden,
            reserved,
        }
    }

    fn cost(&self, widths: &[Widths], pinned: usize, hidden_width: usize) -> usize {
        placements(widths, &self.pins, self.start, self.end)
            .iter()
            .map(|place| {
                let tab = &widths[place.index];
                (if place.grouped { tab.grouped } else { tab.full })
                    + 1
                    + if place.header { tab.header } else { 0 }
            })
            .sum::<usize>()
            + hidden_width
            + self.left(pinned).width()
            + self.reserved
    }

    fn left(&self, pinned: usize) -> String {
        if self.start > pinned {
            format!("‹ {} ", self.start - pinned)
        } else {
            String::new()
        }
    }
}

/// Keep the current visible, stepping excess pins aside without changing order.
fn window(
    widths: &[Widths],
    pinned: usize,
    position: Option<usize>,
    previous: usize,
    hidden_width: usize,
    width: usize,
) -> Window {
    let mut pins: Vec<_> = (0..pinned).collect();
    let current = position.filter(|&index| index >= pinned);
    let mut start = (previous + pinned).min(widths.len());
    start = current
        .map_or(start, |current| start.min(current))
        .max(pinned);
    let mut end = current.map_or(start, |current| current + 1);
    if end == start && start < widths.len() {
        end += 1;
    }
    let all = Window::new(widths, pinned, pins.clone(), pinned, widths.len(), width);
    if all.cost(widths, pinned, hidden_width) <= width {
        return all;
    }
    while Window::new(widths, pinned, pins.clone(), start, end, width).cost(
        widths,
        pinned,
        hidden_width,
    ) > width
    {
        if start < current.unwrap_or(start) {
            start += 1;
        } else if current.is_none() && end > start {
            end = start;
        } else if let Some(drop) = pins.iter().rposition(|index| Some(*index) != position) {
            pins.remove(drop);
        } else {
            break;
        } // The current label alone is fitted by the painter.
    }
    while end < widths.len()
        && Window::new(widths, pinned, pins.clone(), start, end + 1, width).cost(
            widths,
            pinned,
            hidden_width,
        ) <= width
    {
        end += 1;
    }
    Window::new(widths, pinned, pins, start, end, width)
}

struct Label {
    full: Line<'static>,
    grouped: Line<'static>,
    header: Line<'static>,
    name: String,
    marks: String,
    attention: Attention,
}

fn prefix(key: &str) -> Option<(&str, &str)> {
    (!tabs::aggregate(key))
        .then(|| key.split_once('-'))
        .flatten()
        .filter(|(prefix, suffix)| !prefix.is_empty() && !suffix.is_empty())
}

fn label(
    key: &str,
    selected: bool,
    pending: bool,
    attention: Attention,
    look: Look,
    colors: &TabColors,
) -> Label {
    let full = if key == tabs::ALL {
        home_tab(look, selected, pending, attention, colors)
    } else {
        tab(look, tabs::label(key), selected, pending, attention, colors)
    };
    let (grouped, header) = prefix(key).map_or_else(
        || (full.clone(), Line::default()),
        |(name, suffix)| {
            (
                tab(look, suffix, selected, pending, attention, colors),
                Line::from(vec![
                    Span::styled(tmt_cli_style::table::escape(name), look.role(Role::Dim)),
                    Span::styled(" · ", look.role(Role::Dim)),
                ]),
            )
        },
    );
    let name = tmt_cli_style::table::escape(tabs::label(key));
    let mut marks = String::new();
    for (count, mark) in [
        (attention.waiting, Mark::Decision),
        (attention.blocked, Mark::Failed),
    ] {
        if count > 0 {
            marks.push_str(&format!(" {} {count}", mark.symbol()));
        }
    }
    Label {
        full,
        grouped,
        header,
        name,
        marks,
        attention,
    }
}

fn widths(key: &str, label: &Label) -> Widths {
    Widths {
        full: label.full.width(),
        grouped: label.grouped.width(),
        prefix: prefix(key).map(|(name, _)| name.to_owned()),
        header: label.header.width(),
        overflow: label.name.width() + label.marks.width(),
        tier: if label.attention.waiting > 0 {
            0
        } else if label.attention.blocked > 0 {
            1
        } else {
            2
        },
    }
}

fn attention_style(attention: Attention, look: Look, colors: &TabColors) -> Style {
    if attention.waiting > 0 {
        look.named(&colors.waiting)
    } else if attention.blocked > 0 {
        look.named(&colors.blocked)
    } else {
        look.role(Role::Dim)
    }
}

fn overflow(
    labels: &[Label],
    hidden: &[usize],
    room: usize,
    look: Look,
    colors: &TabColors,
) -> Line<'static> {
    let mut line = Line::from(Span::styled(
        format!("+{} › ", hidden.len()),
        look.role(Role::Dim),
    ));
    for &index in hidden {
        let label = &labels[index];
        let available = room.saturating_sub(line.width() + 2);
        if available <= label.marks.width() {
            break;
        }
        let fitted = fit(
            &label.name,
            label.name.width().min(available - label.marks.width()),
        );
        let shortened = fitted != label.name;
        if shortened && line.spans.len() > 1 {
            break;
        }
        line.spans
            .push(Span::styled(fitted, look.role(Role::Muted)));
        for (count, mark, color) in [
            (label.attention.waiting, Mark::Decision, &colors.waiting),
            (label.attention.blocked, Mark::Failed, &colors.blocked),
        ] {
            if count > 0 {
                line.spans.push(Span::styled(
                    format!(" {} {count}", mark.symbol()),
                    look.named(color).add_modifier(Modifier::BOLD),
                ));
            }
        }
        line.spans.push(Span::raw(" "));
        if shortened {
            break;
        }
    }
    line.spans.push(Span::styled("…", look.role(Role::Dim)));
    fit_tab_label(line, room)
}

/// The fold consumes existing attention projections; it never reacquires or reclassifies rows.
fn unpicked(app: &App, budget: usize, look: Look, colors: &TabColors) -> Line<'static> {
    let keys = app.unpicked_squads();
    if keys.is_empty() || budget == 0 {
        return Line::default();
    }
    let attention = keys.iter().filter_map(|key| app.attention.get(*key)).fold(
        Attention::default(),
        |sum, one| Attention {
            waiting: sum.waiting + one.waiting,
            blocked: sum.blocked + one.blocked,
        },
    );
    let marks: Vec<_> = [
        (attention.waiting, Mark::Decision, &colors.waiting),
        (attention.blocked, Mark::Failed, &colors.blocked),
    ]
    .into_iter()
    .filter(|(count, _, _)| *count > 0)
    .map(|(count, mark, color)| {
        Span::styled(
            format!(" {} {count}", mark.symbol()),
            look.named(color).add_modifier(Modifier::BOLD),
        )
    })
    .collect();
    let marks_width = marks.iter().map(Span::width).sum::<usize>();
    let text = format!("{} not on this board", keys.len());
    let text = fit(
        &text,
        text.width().min(budget.saturating_sub(3 + marks_width)),
    );
    let mut spans = vec![
        Span::styled(" │ ", look.role(Role::Dim)),
        Span::styled(text, attention_style(attention, look, colors)),
    ];
    spans.extend(marks);
    let line = Line::from(spans);
    let fitted_width = line.width().min(budget);
    fit_tab_label(line, fitted_width)
}

/// Prepare labels, admit a pure window, then paint its spans and exact hit cells.
pub(super) fn paint(app: &App, area: Rect) -> Line<'static> {
    let look = app.look();
    if app.view.is_none() && app.tabs.is_empty() {
        return Line::styled(
            "  Squad",
            look.role(Role::Accent).add_modifier(Modifier::BOLD),
        );
    }
    let default = TabColors::default();
    let colors = app.view.as_ref().map_or(&default, |view| &view.tab_colors);
    let indices = app.picked_indices();
    let keys: Vec<_> = indices.iter().map(|&index| &app.tabs[index]).collect();
    // A requested tab may still be loading while the retained view is on screen.
    // Selection follows the content the user can see, not the pending request.
    let shown = app.shown_tab().or(app.current.as_deref());
    let position = shown.and_then(|key| keys.iter().position(|tab| tab.as_str() == key));
    let pinned = indices
        .iter()
        .take_while(|&&index| index < app.pinned)
        .count();
    let fold = unpicked(app, usize::from(area.width) / 2, look, colors);
    let fold_width = fold.width();
    let width = usize::from(area.width).saturating_sub(fold_width);
    let labels: Vec<_> = keys
        .iter()
        .enumerate()
        .map(|(index, key)| {
            label(
                key,
                position == Some(index),
                app.loading() && app.current.as_deref() == Some(key.as_str()),
                app.attention.get(*key).copied().unwrap_or_default(),
                look,
                colors,
            )
        })
        .collect();
    let widths: Vec<_> = keys
        .iter()
        .zip(&labels)
        .map(|(key, label)| widths(key, label))
        .collect();
    let shown_hidden = shown.filter(|_| position.is_none()).map(|key| {
        let mut label = tab(
            look,
            tabs::label(key),
            true,
            false,
            app.attention.get(key).copied().unwrap_or_default(),
            colors,
        );
        label
            .spans
            .insert(3, Span::styled(" (hidden)", label.style));
        label
    });
    let hidden_width = shown_hidden.as_ref().map_or(0, |label| label.width() + 1);
    let window = window(
        &widths,
        pinned,
        position,
        app.tab_start.get(),
        hidden_width,
        width,
    );
    app.tab_start.set(window.start - pinned);
    let mut spans = Vec::new();
    let mut used = 0;
    if let Some(label) = shown_hidden {
        let fitted = fit_shown_tab_label(label, hidden_width.saturating_sub(1).min(width));
        used += fitted.width();
        spans.extend(fitted.spans);
        if used < width {
            spans.push(Span::raw(" "));
            used += 1;
        }
    }
    let left = window.left(pinned);
    let left_attention =
        labels[pinned..window.start]
            .iter()
            .fold(Attention::default(), |sum, label| Attention {
                waiting: sum.waiting + label.attention.waiting,
                blocked: sum.blocked + label.attention.blocked,
            });
    let left_style = attention_style(left_attention, look, colors);
    for place in placements(&widths, &window.pins, window.start, window.end) {
        let label = &labels[place.index];
        if place.index == window.start && !left.is_empty() {
            used += left.width();
            spans.push(Span::styled(left.clone(), left_style));
        }
        if place.header {
            used += label.header.width();
            spans.extend(label.header.spans.clone());
        }
        let line = if place.grouped {
            &label.grouped
        } else {
            &label.full
        };
        let label_width = line
            .width()
            .min(width.saturating_sub(used + window.reserved));
        let fitted = if position == Some(place.index) {
            fit_shown_tab_label(line.clone(), label_width)
        } else {
            fit_tab_label(line.clone(), label_width)
        };
        if label_width > 0 {
            app.tab_hits.borrow_mut().push(TabHit {
                y: area.y,
                x: area.x.saturating_add(used as u16),
                width: label_width as u16,
                tab: indices[place.index],
            });
            spans.extend(fitted.spans);
            used += label_width;
            if used < width {
                spans.push(Span::raw(" "));
                used += 1;
            }
        }
    }
    if window.start == window.end && !left.is_empty() {
        let fitted = fit(&left, left.width().min(width.saturating_sub(used)));
        used += fitted.width();
        spans.push(Span::styled(fitted, left_style));
    }
    app.tabs_overflow.set(!window.hidden.is_empty());
    if !window.hidden.is_empty() && used < width {
        let overflow = overflow(&labels, &window.hidden, width - used, look, colors);
        used += overflow.width();
        spans.extend(overflow.spans);
    }
    if fold_width > 0 {
        app.unpicked_hit.set(Some(Rect::new(
            area.x.saturating_add(used as u16),
            area.y,
            fold_width as u16,
            1,
        )));
        spans.extend(fold.spans);
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::view::{
        render,
        tests::{board, draw},
    };
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{Terminal, backend::TestBackend, widgets::Paragraph};
    use serde_json::json;

    fn tabline_board() -> App {
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs = [
            crate::board::ALL,
            crate::board::LEADS,
            "mamezu",
            "tmt-colab",
            "tmt-core",
            "tmt-infra",
            "tmt-remote",
            "tmt-squad",
            "tmt-design",
            "docs",
            "perf",
            "tools",
            "long-running-squad",
            "quiet",
            "ops",
            "test",
        ]
        .map(String::from)
        .to_vec();
        app.set_tab_focus_for_test(crate::board::ALL);
        app.pinned = 1;
        for (name, waiting, blocked) in [
            (crate::board::ALL, 3, 2),
            ("tmt-colab", 1, 0),
            ("tmt-core", 0, 1),
            ("tmt-remote", 2, 1),
            ("perf", 0, 2),
        ] {
            app.attention
                .insert(name.into(), Attention { waiting, blocked });
        }
        app
    }

    #[test]
    fn shown_name_fitting_keeps_graphemes_and_semantic_suffixes_at_exact_widths() {
        let look = Look::default();
        let colors = TabColors::default();
        for (name, minimum, smallest) in [
            ("product", 3, "p"),
            ("界界", 4, "界"),
            ("e\u{301}clair", 3, "e\u{301}"),
        ] {
            for attention in [
                Attention::default(),
                Attention {
                    waiting: 2,
                    blocked: 1,
                },
            ] {
                let label = tab(look, name, true, false, attention, &colors);
                let fixed = label.width() - label.spans[2].width();
                let minimum = minimum + fixed - 2;
                for width in 0..=label.width() + 1 {
                    let fitted = fit_shown_tab_label(label.clone(), width);
                    let text = fitted.to_string();
                    assert_eq!(
                        fitted.width(),
                        width,
                        "{name:?}/{attention:?}/{width}: {text:?}"
                    );
                    assert_eq!(
                        text.matches('[').count(),
                        text.matches(']').count(),
                        "{text}"
                    );
                    if width >= minimum {
                        assert!(!text.contains('[') && !text.contains(']'), "{text}");
                        if attention.waiting > 0 {
                            assert!(
                                text.starts_with("◆ ") && text.trim_end().ends_with(" 2 ✗ 1"),
                                "{text}"
                            );
                            assert_eq!(fitted.spans[0].style.fg, look.role(Role::Waiting).fg);
                            assert_eq!(
                                fitted
                                    .spans
                                    .iter()
                                    .find(|span| span.content.starts_with("✗"))
                                    .unwrap()
                                    .style
                                    .fg,
                                look.role(Role::Blocked).fg
                            );
                        }
                        if width == minimum {
                            assert!(text.contains(smallest), "{text:?}");
                        }
                    } else {
                        assert!(
                            !text.contains('[') && !text.contains(']'),
                            "fitted names have no decoration: {text}"
                        );
                        if width == 1 && attention.waiting > 0 {
                            assert_eq!(text, "◆");
                        }
                    }
                }
            }
        }
        for width in 0..=24 {
            let line = fit_shown_tab_label(
                home_tab(
                    look,
                    true,
                    false,
                    Attention {
                        waiting: 2,
                        blocked: 1,
                    },
                    &colors,
                ),
                width,
            );
            let text = line.to_string();
            assert_eq!(line.width(), width);
            assert_eq!(text.matches('[').count(), text.matches(']').count());
            if width >= 13 {
                assert!(text.contains("▚ t"), "{text}");
            }
        }
    }

    #[test]
    fn authored_tab_brackets_and_escaped_controls_are_preserved() {
        let look = Look::default();
        let colors = TabColors::default();
        let name = "[authored]\n界e\u{301}";
        let escaped = tmt_cli_style::table::escape(name);
        for selected in [false, true] {
            let label = tab(look, name, selected, false, Attention::default(), &colors);
            assert_eq!(label.spans[2].content, escaped);
            assert_eq!(label.width(), 2 + escaped.width());
        }
        assert_eq!(pane_tab(look, "[detail]", true).content, "[detail]");
    }

    #[test]
    fn home_and_named_selection_follow_user_text_background_and_reverse_overrides() {
        for settings in [
            vec![
                ("base", "tmt-light"),
                ("text", "#123456"),
                ("selection", "#234567"),
            ],
            vec![("base", "terminal"), ("text", "blue")],
        ] {
            let look = Look {
                theme: tmt_cli_style::Theme::parse("theme", settings).unwrap(),
                depth: tmt_cli_style::Depth::TrueColor,
            };
            let home = home_tab(
                look,
                true,
                false,
                Attention::default(),
                &TabColors::default(),
            );
            let named = tab(
                look,
                "product",
                true,
                false,
                Attention::default(),
                &TabColors::default(),
            );
            assert_eq!(home.style, named.style);
            assert_eq!(home.style.fg, look.role(Role::Text).fg);
            assert_eq!(home.style.bg, look.selection().bg);
            assert_eq!(
                home.style.add_modifier.contains(Modifier::REVERSED),
                look.selection().bg.is_none()
            );
            assert!(home.style.add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn shown_names_are_inside_measured_hits_and_overflow_remains_reachable() {
        for key in [tabs::ALL, tabs::LEADS, "@tab:authored", "tmt-core"] {
            let mut app = tabline_board();
            app.tabs.insert(2, "@tab:authored".into());
            app.pinned = 3;
            app.set_tab_focus_for_test(key);
            let document = app.view.as_ref().unwrap().document.clone();
            for width in [80, 100, 160, 180, 32, 180] {
                let line = draw(&app, width, 8)[0].clone();
                let hits = app.tab_hits.borrow().clone();
                let index = app.tabs.iter().position(|tab| tab == key).unwrap();
                let hit = hits.iter().find(|hit| hit.tab == index).unwrap();
                let shown: String = line
                    .chars()
                    .skip(hit.x as usize)
                    .take(hit.width as usize)
                    .collect();
                assert!(
                    !shown.contains('[') && !shown.contains(']'),
                    "{key}/{width}: {line}"
                );
                let name = if key == tabs::ALL {
                    "tmt"
                } else if key == "tmt-core" && line.contains("tmt ·") {
                    "core"
                } else {
                    tabs::label(key)
                };
                assert!(shown.contains(name), "{key}/{width}: {shown}");
                let name_start = shown.find(name).unwrap() as u16 + hit.x;
                for x in [hit.x, name_start, hit.x + hit.width - 1] {
                    assert_eq!(
                        app.mouse(
                            MouseEvent {
                                kind: MouseEventKind::Down(MouseButton::Left),
                                column: x,
                                row: hit.y,
                                modifiers: KeyModifiers::NONE
                            },
                            std::time::Instant::now()
                        ),
                        crate::board::app::Effect::None
                    );
                    assert_eq!(
                        app.current.as_deref(),
                        Some(key),
                        "exact shown hit at {x}: {line}"
                    );
                }
                assert!(hits.iter().all(|h| h.x + h.width <= width));
                assert!(
                    hits.windows(2)
                        .all(|pair| pair[0].x + pair[0].width <= pair[1].x)
                );
                assert_eq!(app.view.as_ref().unwrap().document, document);
                if app.tabs_overflow.get() {
                    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
                    assert!(app.switcher_keys().iter().any(|tab| tab == key));
                    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                    assert!(app.switcher.is_none());
                }
            }
        }
    }

    /// Separately labeled deterministic chrome evidence; no fixture writes or core reads.
    #[test]
    #[ignore = "explicit chrome evidence output"]
    fn capture_flat_chrome() {
        let output = std::env::var("TMT_CHROME_OUTPUT").expect("task-owned output path");
        crate::status::with_now_ms(1_900_000_000_000, || {
            let mut captures = Vec::new();
            for (base, depth) in [
                (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
                (
                    tmt_cli_style::Base::TmtLight,
                    tmt_cli_style::Depth::TrueColor,
                ),
                (tmt_cli_style::Base::Terminal, tmt_cli_style::Depth::Ansi16),
                (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
            ] {
                for (width, height) in [(80, 30), (100, 30), (160, 30), (180, 30), (80, 8)] {
                    for current in [
                        tabs::ALL,
                        tabs::LEADS,
                        "@tab:authored",
                        "tmt-core",
                        "hidden-squad",
                    ] {
                        let mut app = tabline_board();
                        app.tabs.insert(2, "@tab:authored".into());
                        app.pinned = 3;
                        app.hidden = vec!["hidden-squad".into()];
                        app.set_tab_focus_for_test(current);
                        app.view.as_mut().unwrap().look = Look {
                            theme: tmt_cli_style::Theme::new(base),
                            depth,
                        };
                        let lines = draw(&app, width, height);
                        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                        terminal.draw(|frame| render(frame, &app)).unwrap();
                        let cells: Vec<_> = terminal.backend().buffer().content.iter().map(|cell| json!({
                            "symbol":cell.symbol(), "fg":format!("{:?}",cell.fg), "bg":format!("{:?}",cell.bg), "modifier":format!("{:?}",cell.modifier)
                        })).collect();
                        captures.push(json!({"current":current, "base":base.name(), "depth":format!("{depth:?}"), "width":width,"height":height,"lines":lines,"cells":cells,"hits":format!("{:?}", app.tab_hits.borrow()),"overflow":app.tabs_overflow.get(),"document":app.view.as_ref().unwrap().document}));
                    }
                }
            }
            std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
        });
    }

    #[test]
    fn picks_regroup_visible_tabs_preserve_canonical_hits_and_separate_fold_attention() {
        for (base, depth) in [
            (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
            (
                tmt_cli_style::Base::TmtLight,
                tmt_cli_style::Depth::TrueColor,
            ),
            (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
        ] {
            let mut app = tabline_board();
            app.view.as_mut().unwrap().look = Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            app.tabs = [
                tabs::ALL,
                tabs::LEADS,
                "tmt-a",
                "tmt-gap",
                "tmt-b",
                "docs",
                "@tab:view",
            ]
            .map(String::from)
            .to_vec();
            app.hidden = vec!["global-hidden".into()];
            app.picks =
                crate::board::pick::Picks::parse(Some("@all,leads,tmt-a,tmt-b"), &app.switchable())
                    .unwrap();
            app.attention.insert(
                "tmt-gap".into(),
                Attention {
                    waiting: 2,
                    blocked: 1,
                },
            );
            app.attention.insert(
                "docs".into(),
                Attention {
                    waiting: 0,
                    blocked: 2,
                },
            );
            app.attention.insert(
                "global-hidden".into(),
                Attention {
                    waiting: 100,
                    blocked: 100,
                },
            );
            for width in [160, 100, 80, 32, 160] {
                let line = draw(&app, width, 6)[0].clone();
                assert!(line.contains("◆ 2 ✗ 3"), "{width}: {line}");
                assert!(!line.contains("100"));
                if width >= 80 {
                    assert!(line.contains("2 not on this board"), "{line}");
                    assert!(line.contains("tmt ·") && !line.contains("tmt-gap"));
                    assert_eq!(
                        app.tab_hits
                            .borrow()
                            .iter()
                            .map(|hit| hit.tab)
                            .collect::<Vec<_>>(),
                        [0, 1, 2, 4]
                    );
                }
                let rect = app.unpicked_hit.get().unwrap();
                assert!(rect.x + rect.width <= width);
                assert!(
                    app.tab_hits
                        .borrow()
                        .iter()
                        .all(|hit| hit.x + hit.width <= rect.x)
                );
                let mut terminal = Terminal::new(TestBackend::new(width, 6)).unwrap();
                terminal.draw(|frame| render(frame, &app)).unwrap();
                for (symbol, role) in [("◆ 2", Role::Waiting), ("✗ 3", Role::Blocked)] {
                    let column = line[..line.find(symbol).unwrap()].chars().count() as u16;
                    assert_eq!(
                        terminal.backend().buffer()[(column, 0)].fg,
                        app.look().role(role).fg.unwrap_or_default()
                    );
                }
            }
            let rect = app.unpicked_hit.get().unwrap();
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: rect.x,
                    row: rect.y,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now(),
            );
            assert_eq!(app.switcher_keys(), ["tmt-gap", "docs"]);
            app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
            assert!(app.picks.contains("tmt-gap"));
            assert_eq!(app.switcher_keys(), ["docs"]);
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(app.switcher.is_none());
            let line = draw(&app, 80, 6)[0].clone();
            assert!(line.contains("1 not on this board ✗ 2"), "{line}");
            assert!(!line.contains("◆ 2"));
        }
    }

    #[test]
    fn quiet_fold_is_dim_and_omitted_aggregate_tabs_do_not_count_as_squads() {
        let mut app = tabline_board();
        app.tabs = [tabs::ALL, "product", "quiet", "@tab:view"]
            .map(String::from)
            .to_vec();
        app.picks = crate::board::pick::Picks::parse(Some("product"), &app.switchable()).unwrap();
        app.set_tab_focus_for_test("product");
        app.attention.clear();
        let line = draw(&app, 80, 6)[0].clone();
        assert!(line.contains("1 not on this board"), "{line}");
        assert!(!line.contains("◆") && !line.contains("✗"));
        let mut terminal = Terminal::new(TestBackend::new(80, 6)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let column = line[..line.find("1 not").unwrap()].chars().count() as u16;
        assert_eq!(
            terminal.backend().buffer()[(column, 0)].fg,
            app.look().role(Role::Dim).fg.unwrap_or_default()
        );
        assert_eq!(app.tab_hits.borrow()[0].tab, 1);
    }

    #[test]
    fn tab_hits_cover_the_slot_and_name_of_the_rendered_label() {
        use crate::board::app::Effect;
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        for attention in [
            Attention::default(),
            Attention {
                waiting: 1,
                blocked: 2,
            },
        ] {
            for offset in [0, 2, 14] {
                let mut app = board(json!([{"title": null, "rows": []}]));
                app.attention.insert("reviews".into(), attention);
                draw(&app, 80, 8);
                let hit = app.tab_hits.borrow()[1];
                let label = tab(
                    app.look(),
                    "reviews",
                    false,
                    false,
                    attention,
                    &TabColors::default(),
                );
                assert_eq!(usize::from(hit.width), label.width());
                let x = hit.x + offset.min(hit.width - 1);
                assert_eq!(
                    app.mouse(
                        MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column: x,
                            row: hit.y,
                            modifiers: KeyModifiers::NONE,
                        },
                        std::time::Instant::now()
                    ),
                    Effect::Load("reviews".into())
                );
                assert_eq!(app.current.as_deref(), Some("reviews"));
            }
        }
    }

    #[test]
    fn default_attention_color_does_not_inherit_the_selected_name_foreground() {
        let look = crate::look::Look::default();
        let label = tab(
            look,
            "product",
            true,
            false,
            Attention {
                waiting: 1,
                blocked: 1,
            },
            &TabColors {
                waiting: "default".into(),
                blocked: "default".into(),
            },
        );
        for rendered in [label.clone(), Line::from(label.spans)] {
            let width = rendered.width() as u16;
            let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
            terminal
                .draw(|frame| frame.render_widget(Paragraph::new(rendered), frame.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(0, 0)].fg, Style::new().fg.unwrap_or_default());
            assert_eq!(buffer[(12, 0)].fg, Style::new().fg.unwrap_or_default());
            assert_eq!(buffer[(2, 0)].fg, look.role(Role::Text).fg.unwrap());
            for x in 0..width {
                assert_eq!(buffer[(x, 0)].bg, look.selection().bg.unwrap());
            }
        }
    }

    #[test]
    fn many_tabs_scroll_to_keep_the_current_one_and_count_the_rest() {
        let names: Vec<String> = (0..9).map(|n| format!("sq{n}")).collect();
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs = names.clone();
        app.set_tab_focus_for_test("sq4");
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
            line.contains(" sq4 "),
            "the current tab stays in view: {line:?}"
        );
        assert!(line.contains(" ›") && line.contains("sq8 ◆ 2"), "{line:?}");
        // The left count hides a blocked tab, the right one a waiting tab.
        assert_eq!(
            buffer[(0, 0)].fg,
            app.look().role(Role::Blocked).fg.unwrap()
        );
        let right = line[..line.find("◆ 2").unwrap()].chars().count() as u16;
        assert_eq!(
            buffer[(right, 0)].fg,
            app.look().role(Role::Waiting).fg.unwrap()
        );
        // Only shown tabs can be clicked, at their drawn places.
        let hits = app.tab_hits.borrow().clone();
        assert!(hits.iter().all(|hit| hit.tab >= app.tab_start.get()));
        let current = hits.iter().find(|hit| hit.tab == 4).unwrap();
        let at = line[..line.find("  sq4").unwrap()].chars().count() as u16;
        assert_eq!(current.x, at);
    }

    #[test]
    fn home_tabline_snapshots_at_160_100_80_in_dark_light_and_no_color() {
        for (base, depth) in [
            (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
            (
                tmt_cli_style::Base::TmtLight,
                tmt_cli_style::Depth::TrueColor,
            ),
            (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
        ] {
            let mut app = tabline_board();
            app.view.as_mut().unwrap().look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            let mut differences = Vec::new();
            for (width, expected) in [
                (
                    160,
                    " ▚ tmt ◆ 3 ✗ 2    leads   mamezu tmt · ◆ colab 1 ✗ core 1   infra ◆ remote 2 ✗ 1   squad   design   docs ✗ perf 2   tools   long-running-squad +3 › quiet ops …",
                ),
                (
                    100,
                    " ▚ tmt ◆ 3 ✗ 2    leads   mamezu tmt · ◆ colab 1 ✗ core 1   infra ◆ remote 2 ✗ 1 +9 › perf ✗ 2 …",
                ),
                (
                    80,
                    " ▚ tmt ◆ 3 ✗ 2    leads   mamezu ◆ tmt-colab 1 +12 › tmt-remote ◆ 2 ✗ 1 …",
                ),
            ] {
                for (current, selected) in [(tabs::ALL, true), ("tmt-colab", false)] {
                    app.set_tab_focus_for_test(current);
                    let line = draw(&app, width, 6)[0].clone();
                    if selected {
                        if line != expected {
                            differences.push(format!("{width}: {line}"));
                        }
                    } else {
                        assert!(line.starts_with(" ▚ tmt ◆ 3 ✗ 2 "), "{line}");
                    }
                    let hits = app.tab_hits.borrow().clone();
                    assert_eq!(hits[0].tab, 0);
                    assert!(hits.iter().all(|hit| hit.x + hit.width <= width));
                    let mut terminal = Terminal::new(TestBackend::new(width, 6)).unwrap();
                    terminal.draw(|frame| render(frame, &app)).unwrap();
                    let buffer = terminal.backend().buffer();
                    let home = &buffer[(1, 0)];
                    assert_eq!(
                        home.fg,
                        app.look()
                            .role(if selected { Role::Text } else { Role::Muted })
                            .fg
                            .unwrap_or_default()
                    );
                    assert_eq!(
                        home.bg,
                        if selected {
                            app.look().selection().bg.unwrap_or_default()
                        } else {
                            app.look().role(Role::Muted).bg.unwrap_or_default()
                        }
                    );
                    assert_eq!(
                        home.modifier.contains(Modifier::REVERSED),
                        selected && app.look().selection().bg.is_none()
                    );
                    assert!(!home.modifier.contains(Modifier::UNDERLINED));
                    // Counts stay inside the label and keep their attention roles.
                    for (symbol, role) in [("◆", Role::Waiting), ("✗", Role::Blocked)] {
                        let column = line[..line.find(symbol).unwrap()].chars().count() as u16;
                        assert_eq!(
                            buffer[(column, 0)].fg,
                            app.look().role(role).fg.unwrap_or_default()
                        );
                        assert_eq!(
                            buffer[(column, 0)].modifier.contains(Modifier::REVERSED),
                            selected && app.look().selection().bg.is_none()
                        );
                    }
                }
            }
            assert!(differences.is_empty(), "{differences:#?}");
        }
    }

    #[test]
    fn tab_selection_follows_the_retained_home_view_until_the_requested_tab_loads() {
        for (base, depth) in [
            (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
            (
                tmt_cli_style::Base::TmtLight,
                tmt_cli_style::Depth::TrueColor,
            ),
            (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
        ] {
            let mut snapshot = crate::board::app::tests::snapshot(tabs::ALL, json!([]));
            snapshot.tabs = vec![tabs::ALL.into(), "ux".into()];
            snapshot.pinned = 1;
            snapshot.view.as_mut().unwrap().home = Some(crate::board::home::Home {
                windows: crate::config::TokenWindow::DEFAULTS,
                summary: Default::default(),
                sections: Vec::new(),
                squads: Vec::new(),
                failures: Vec::new(),
                incomplete: false,
            });
            snapshot.view.as_mut().unwrap().look = Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            let mut app = App::new(Some(tabs::ALL.into()));
            app.apply(snapshot);

            let assert_selected = |app: &App, expected: usize| {
                let width = 113;
                let line = draw(app, width, 20)[0].clone();
                assert!(line.contains("▚ tmt") && line.contains("ux"), "{line}");
                let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
                terminal.draw(|frame| render(frame, app)).unwrap();
                let buffer = terminal.backend().buffer();
                let selection = app.look().selection();
                let selected: Vec<_> = app
                    .tab_hits
                    .borrow()
                    .iter()
                    .filter(|hit| {
                        let cell = &buffer[(hit.x + 1, hit.y)];
                        if let Some(bg) = selection.bg {
                            cell.bg == bg
                        } else {
                            cell.modifier.contains(Modifier::REVERSED)
                        }
                    })
                    .map(|hit| hit.tab)
                    .collect();
                assert_eq!(selected, [expected], "{base:?} {depth:?}: {line}");
            };

            assert_eq!(app.shown_tab(), Some(tabs::ALL));
            assert_selected(&app, 0);
            assert_eq!(
                app.go("ux".into()),
                crate::board::app::Effect::Load("ux".into())
            );
            assert!(app.loading() && app.view.as_ref().unwrap().home.is_some());
            assert!(
                app.switcher.is_none(),
                "this is a pending load, not a switcher cursor"
            );
            assert_eq!(app.current.as_deref(), Some("ux"));
            assert_eq!(app.shown_tab(), Some(tabs::ALL));
            assert_selected(&app, 0);

            let mut terminal = Terminal::new(TestBackend::new(113, 20)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let target = app
                .tab_hits
                .borrow()
                .iter()
                .find(|hit| hit.tab == 1)
                .copied()
                .unwrap();
            assert!(
                terminal.backend().buffer()[(target.x + 2, 0)]
                    .modifier
                    .contains(Modifier::UNDERLINED),
                "the pending ux tab needs an immediate cue"
            );

            let failed = crate::board::app::Snapshot {
                squad_keys: Vec::new(),
                tabs: Vec::new(),
                hidden: Vec::new(),
                pinned: 0,
                attention: Default::default(),
                squad: Some("ux".into()),
                view: Err("ux: room not found".into()),
            };
            app.apply(failed);
            assert!(!app.loading() && app.loading_since.is_none());
            assert_eq!(app.current.as_deref(), Some(tabs::ALL));
            assert_eq!(app.shown_tab(), Some(tabs::ALL));
            assert!(app.view.as_ref().unwrap().home.is_some());
            assert_selected(&app, 0);
            assert!(draw(&app, 113, 20)[19].contains("ux: room not found"));

            assert_eq!(
                app.go("ux".into()),
                crate::board::app::Effect::Load("ux".into())
            );
            assert!(app.error.is_none());

            let mut loaded = crate::board::app::tests::snapshot("ux", json!([]));
            loaded.tabs = vec![tabs::ALL.into(), "ux".into()];
            loaded.pinned = 1;
            loaded.view.as_mut().unwrap().look = Look {
                theme: tmt_cli_style::Theme::new(base),
                depth,
            };
            app.apply(loaded);
            assert_eq!(app.shown_tab(), Some("ux"));
            assert_selected(&app, 1);
        }
    }

    #[test]
    fn grouped_squads_have_individual_marks_hits_and_full_navigation_keys() {
        let mut app = tabline_board();
        app.tabs.truncate(8);
        let original = app.tabs.clone();
        let line = draw(&app, 160, 6)[0].clone();
        assert_eq!(line.matches("tmt ·").count(), 1, "{line}");
        assert!(!line.contains('│'), "{line}");
        for (index, name, mark) in [
            (3, "colab", "◆"),
            (4, "core", "✗"),
            (5, "infra", " "),
            (6, "remote", "◆"),
        ] {
            let hit = app
                .tab_hits
                .borrow()
                .iter()
                .find(|hit| hit.tab == index)
                .copied()
                .unwrap();
            let tab_text: String = line
                .chars()
                .skip(usize::from(hit.x))
                .take(usize::from(hit.width))
                .collect();
            assert!(tab_text.starts_with(mark), "{tab_text}");
            assert!(tab_text.contains(name), "{tab_text}");
            assert_eq!(
                app.mouse(
                    MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: hit.x,
                        row: 0,
                        modifiers: KeyModifiers::NONE
                    },
                    std::time::Instant::now()
                ),
                crate::board::app::Effect::Load(original[index].clone())
            );
        }
        let prefix = line[..line.find("tmt ·").unwrap()].chars().count() as u16;
        let mut terminal = Terminal::new(TestBackend::new(160, 6)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(prefix, 0)].fg,
            app.look().role(Role::Dim).fg.unwrap()
        );
        assert!(!buffer[(prefix, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(
            buffer[(prefix + 3, 0)].fg,
            app.look().role(Role::Dim).fg.unwrap()
        );
        for (column, _) in line
            .chars()
            .enumerate()
            .filter(|(_, symbol)| *symbol == '│')
        {
            assert_eq!(
                buffer[(column as u16, 0)].fg,
                app.look().role(Role::Dim).fg.unwrap()
            );
            assert!(
                app.tab_hits
                    .borrow()
                    .iter()
                    .all(|hit| !(hit.x..hit.x + hit.width).contains(&(column as u16)))
            );
        }
        assert!(
            app.tab_hits
                .borrow()
                .iter()
                .all(|hit| !(hit.x..hit.x + hit.width).contains(&prefix))
        );
        assert_eq!(app.tabs, original, "grouping and clicks do not reorder");
        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert!(
            draw(&app, 100, 18).join("\n").contains("tmt-colab"),
            "switcher retains full names"
        );
    }

    #[test]
    fn group_labels_use_dim_prefixes_and_one_gap_between_tab_hits() {
        let mut app = tabline_board();
        app.pinned = 0;
        app.set_tab_focus_for_test("tmt-a");
        for names in [
            vec!["tmt-a", "tmt-b", "docs", "ops-a", "ops-b", "notes"],
            vec!["tmt-a", "tmt-b", "ops-a", "ops-b"],
            vec!["tmt-a", "tmt-b"],
        ] {
            app.tabs = names.iter().map(|name| (*name).into()).collect();
            let line = draw(&app, 160, 6)[0].clone();
            assert!(!line.contains('│'), "{line}");
            let hits = app.tab_hits.borrow();
            for pair in hits.windows(2) {
                let between: String = line
                    .chars()
                    .skip(usize::from(pair[0].x + pair[0].width))
                    .take(usize::from(pair[1].x - pair[0].x - pair[0].width))
                    .collect();
                let expected = if app.tabs[pair[1].tab] == "ops-a" {
                    " ops · "
                } else {
                    " "
                };
                assert_eq!(between, expected, "{line}");
            }
        }
        app.tabs = (0..30).map(|index| format!("tmt-squad{index}")).collect();
        let first = app.tabs[0].clone();
        app.set_tab_focus_for_test(&first);
        let line = draw(&app, 80, 6)[0].clone();
        assert!(line.contains(" › "), "{line}");
        assert!(!line.contains('│'), "{line}");
    }

    #[test]
    fn aggregate_tabs_and_interrupted_or_single_prefixes_never_group() {
        let mut app = tabline_board();
        app.pinned = 0;
        app.tabs = [
            "tmt-a",
            crate::board::ALL,
            "tmt-b",
            "@tab:tmt-view",
            "tmt-c",
            "other-d",
            "tmt-e",
            "tmt-f",
        ]
        .map(String::from)
        .to_vec();
        let line = draw(&app, 160, 6)[0].clone();
        for name in ["tmt-a", "tmt-b", "tmt-view", "tmt-c", "other-d"] {
            assert!(line.contains(name), "{line}");
        }
        assert_eq!(line.matches("tmt ·").count(), 1, "{line}");
        assert_eq!(app.tab_hits.borrow().len(), app.tabs.len());
    }

    #[test]
    fn every_current_tab_survives_resize_and_excessive_pins_with_bounded_hits() {
        for pinned in [1, 8, 16] {
            let mut app = tabline_board();
            app.pinned = pinned;
            let original = app.tabs.clone();
            for index in 0..app.tabs.len() {
                let current = app.tabs[index].clone();
                app.set_tab_focus_for_test(&current);
                for width in [160, 100, 80, 32, 160] {
                    let line = draw(&app, width, 6)[0].clone();
                    let hits = app.tab_hits.borrow();
                    let current = hits
                        .iter()
                        .find(|hit| hit.tab == index)
                        .unwrap_or_else(|| panic!("{pinned} {index} {width}: {line}"));
                    assert!(current.width > 2, "the name remains visible: {line}");
                    assert!(hits.iter().all(|hit| hit.x + hit.width <= width));
                    assert_eq!(app.tabs, original);
                    if width == 160 {
                        assert!(hits.len() > 1, "widening restores tabs");
                    }
                }
            }
        }
    }

    #[test]
    fn pinned_current_keeps_left_overflow_when_no_scrolling_tab_fits() {
        let app = tabline_board();
        app.tab_start.set(10);
        let line = draw(&app, 32, 6)[0].clone();
        assert!(line.contains("▚ tmt"), "{line}");
        assert!(line.contains("‹ 10"), "{line}");
        let hits = app.tab_hits.borrow();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].tab, 0);
        assert!(hits[0].x + hits[0].width <= 32);
    }

    #[test]
    fn hidden_current_and_wide_long_labels_have_no_invisible_hit_cells() {
        let mut app = tabline_board();
        app.set_tab_focus_for_test("hidden-squad");
        app.hidden = vec!["hidden-squad".into()];
        app.pinned = app.tabs.len();
        let line = draw(&app, 32, 6)[0].clone();
        assert!(line.starts_with("  hidden-squad (hidden)"), "{line}");
        assert!(
            app.tab_hits.borrow().is_empty(),
            "hidden current leaves no room for pins"
        );
        app.pinned = 0;
        app.tabs = ["界界界界界界界界界界界界界界界界界界界界", "next"]
            .map(String::from)
            .to_vec();
        let first = app.tabs[0].clone();
        app.set_tab_focus_for_test(&first);
        let line = paint(&app, Rect::new(4, 3, 32, 1));
        assert!(line.width() <= 32, "{line}");
        let hit = *app.tab_hits.borrow().last().unwrap();
        assert_eq!((hit.x, hit.y, hit.tab), (4, 3, 0));
        assert!(hit.width <= 32);
        assert!(line.to_string().contains('…'));
    }

    #[test]
    fn overflow_prioritizes_waiting_then_blocked_then_quiet_without_changing_order() {
        let mut app = tabline_board();
        app.tabs = [
            "current",
            "this-name-is-far-too-long-to-show-in-the-visible-window-at-any-of-the-expected-capture-widths",
            "quiet",
            "blocked",
            "waiting",
            "other-waiting",
        ]
        .map(String::from)
        .to_vec();
        app.pinned = 0;
        app.set_tab_focus_for_test("current");
        app.attention.insert(
            "waiting".into(),
            Attention {
                waiting: 2,
                blocked: 1,
            },
        );
        app.attention.insert(
            "other-waiting".into(),
            Attention {
                waiting: 1,
                blocked: 0,
            },
        );
        app.attention.insert(
            "blocked".into(),
            Attention {
                waiting: 0,
                blocked: 3,
            },
        );
        let original = app.tabs.clone();
        let line = draw(&app, 80, 6)[0].clone();
        assert!(
            line.contains("+5 › waiting ◆ 2 ✗ 1 other-waiting ◆ 1 blocked ✗ 3"),
            "{line}"
        );
        assert!(line.ends_with('…'), "{line}");
        assert_eq!(app.tabs, original);
        assert_eq!(app.tab_hits.borrow().len(), 1, "overflow has no tab hits");
    }

    #[test]
    fn a_hidden_squad_being_shown_leads_the_tab_line_selected() {
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs = (0..9).map(|n| format!("sq{n}")).collect();
        app.hidden = vec!["quiet".into()];
        app.set_tab_focus_for_test("quiet");
        app.attention.insert(
            "quiet".into(),
            Attention {
                waiting: 1,
                blocked: 2,
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(42, 6)).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let line: String = (0..42)
            .map(|x| buffer[(x, 0)].symbol().to_owned())
            .collect();
        assert!(line.starts_with("◆ quiet (hidden) 1 ✗ 2 "), "{line:?}");
        assert!(buffer[(2, 0)].modifier.contains(Modifier::BOLD));
        // The selected tab's name is a word on the selection background.
        assert_eq!(buffer[(2, 0)].fg, app.look().role(Role::Text).fg.unwrap());
        assert_eq!(
            buffer[(0, 0)].fg,
            app.look().role(Role::Waiting).fg.unwrap()
        );
        assert_eq!(
            buffer[(19, 0)].fg,
            app.look().role(Role::Blocked).fg.unwrap()
        );
        assert!(
            line.contains(" › ") && line.trim_end().ends_with("…"),
            "{line:?}"
        );
        // It is not one of the tabs, so it cannot be clicked or dragged, and
        // the tabs after it are hit where they are drawn.
        let hits = app.tab_hits.borrow().clone();
        let first = hits.iter().find(|hit| hit.tab == 0).unwrap();
        let at = line[..line.find("  sq0").unwrap()].chars().count() as u16;
        assert_eq!(first.x, at, "{line:?}");
        assert!(hits.iter().all(|hit| hit.x >= at));
    }

    #[test]
    fn pinned_tabs_stay_in_view_and_keep_their_pin_order() {
        use crate::board::app::Effect;
        let mut app = board(json!([{"title": null, "rows": []}]));
        app.tabs = std::iter::once(crate::board::ALL.to_owned())
            .chain((0..9).map(|n| format!("sq{n}")))
            .collect();
        app.pinned = 1;
        app.set_tab_focus_for_test("sq8");
        let line = draw(&app, 36, 6)[0].clone();
        assert!(
            line.starts_with(" ▚ tmt  ‹ "),
            "the pin stays first: {line:?}"
        );
        assert!(
            line.contains(" sq8"),
            "the current tab is in view: {line:?}"
        );
        let hits = app.tab_hits.borrow().clone();
        assert_eq!(hits[0].tab, 0);
        assert_eq!(hits[0].x, 0);
        // A pin neither moves nor is passed; the other tabs move among
        // themselves. (A saved `order` could not reorder the pins.)
        let shift = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
        let refused = Some("Pinned tabs keep the order in [tabs] pin.");
        app.set_tab_focus_for_test("sq0");
        assert_eq!(app.key(shift(KeyCode::Left)), Effect::None);
        assert_eq!(app.notice.as_deref(), refused);
        app.pinned = 2;
        app.set_tab_focus_for_test(crate::board::ALL);
        app.notice = None;
        assert_eq!(app.key(shift(KeyCode::Right)), Effect::None);
        assert_eq!(app.notice.as_deref(), refused);
        assert_eq!(app.tabs[..2], [crate::board::ALL, "sq0"]);
        app.pinned = 1;
        app.set_tab_focus_for_test("sq0");
        assert!(matches!(app.key(shift(KeyCode::Right)), Effect::Act(_)));
        assert_eq!(app.tabs[..3], [crate::board::ALL, "sq1", "sq0"]);
    }
}
