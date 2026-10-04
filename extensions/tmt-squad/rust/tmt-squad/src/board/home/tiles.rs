//! The squad section returns lines and local regions to the single home painter.
use super::{Counts, SquadLine};
use crate::{board::app::HomeUsage, config::TokenWindow, look::Look};
use ratatui::text::{Line, Span};
use std::ops::Range;
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::style::TextFlow;

pub(super) struct TileItem<'a> {
    pub squad: &'a SquadLine,
    /// Exclusive non-lead state counts, projected from acquired memberships.
    pub members: &'a Counts,
    /// Public session observation, available even when token sampling is off.
    pub lead_model: Option<&'a str>,
    /// Already sampled runtime windows; this section only formats observations.
    pub usage: Option<HomeUsage<'a>>,
}

impl TileItem<'_> {
    fn windows(&self) -> [TokenWindow; 3] {
        self.usage
            .as_ref()
            .map_or(TokenWindow::DEFAULTS, |usage| usage.windows)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TileRegion {
    pub item: usize,
    pub lines: Range<usize>,
    pub x: u16,
    pub width: u16,
}

pub(super) struct TilePaint {
    pub lines: Vec<Line<'static>>,
    pub regions: Vec<TileRegion>,
}

fn compact(width: u16) -> bool {
    width < 100
}

/// A single full-width column; local line ranges are also the scroll/hit map.
fn placement(width: u16, count: usize) -> Vec<TileRegion> {
    if width == 0 {
        return Vec::new();
    }
    let height = if compact(width) { 1 } else { 3 };
    let gap = usize::from(!compact(width));
    (0..count)
        .map(|item| {
            let start = item * (height + gap);
            TileRegion {
                item,
                lines: start..start + height,
                x: 0,
                width,
            }
        })
        .collect()
}

fn fit(value: &str, width: u16) -> String {
    tmt_tui::text::fit_line(value, width, TextFlow::Truncate, Align::Left)
}

fn span(text: String, role: Role, selected: bool, look: Look) -> Span<'static> {
    let emphasize = matches!(
        role,
        Role::Accent | Role::Waiting | Role::Blocked | Role::Review | Role::Working
    );
    let style = look.row_span(selected, look.role(role), emphasize);
    let style = if selected {
        look.selection().patch(style)
    } else {
        style
    };
    Span::styled(text, style)
}

fn marks(counts: &Counts, width: u16, selected: bool, look: Look) -> Vec<Span<'static>> {
    let mut remaining = usize::from(width);
    let mut spans = Vec::new();
    for (mark, count, role) in [
        ('◆', counts.waiting, Role::Waiting),
        ('✗', counts.blocked, Role::Blocked),
        ('◐', counts.review, Role::Review),
        ('●', counts.working, Role::Working),
        ('○', counts.idle, Role::Dim),
    ] {
        let shown = count.min(remaining / 2);
        spans.push(span(format!("{mark} ").repeat(shown), role, selected, look));
        remaining -= shown * 2;
    }
    spans.push(span(" ".repeat(remaining), Role::Text, selected, look));
    spans
}

fn member_line(counts: &Counts, width: u16, selected: bool, look: Look) -> Line<'static> {
    let known = counts.waiting + counts.blocked + counts.review + counts.working + counts.idle;
    let other = counts.members.saturating_sub(known);
    let other = if other == 0 {
        String::new()
    } else {
        format!("{other} other · ")
    };
    let label = format!(
        " {other}{} member{}",
        counts.members,
        if counts.members == 1 { "" } else { "s" }
    );
    let label_width =
        unicode_width::UnicodeWidthStr::width(label.as_str()).min(usize::from(width)) as u16;
    let mut spans = marks(counts, width - label_width, selected, look);
    spans.push(span(fit(&label, label_width), Role::Dim, selected, look));
    Line::from(spans)
}

fn heading(item: &TileItem<'_>, width: u16, selected: bool, look: Look) -> Line<'static> {
    let mut badges = Vec::new();
    for (mark, count, role) in [
        ('◆', item.squad.counts.waiting, Role::Waiting),
        ('✗', item.squad.counts.blocked, Role::Blocked),
    ] {
        if count > 0 {
            badges.push(span(format!(" {mark} {count}"), role, selected, look));
        }
    }
    let badge_width: usize = badges.iter().map(Span::width).sum();
    let room = usize::from(width).saturating_sub(2 + badge_width);
    let name = &item.squad.squad;
    let name_width = unicode_width::UnicodeWidthStr::width(name.as_str()).min(room) as u16;
    let mut spans = vec![span(fit("─ ", width.min(2)), Role::Dim, selected, look)];
    spans.push(span(fit(name, name_width), Role::Accent, selected, look));
    if badge_width + 2 <= usize::from(width) {
        spans.extend(badges);
    }
    let remaining = usize::from(width).saturating_sub(spans.iter().map(Span::width).sum());
    if remaining > 0 {
        spans.push(span(
            format!(" {}", "─".repeat(remaining - 1)),
            Role::Dim,
            selected,
            look,
        ));
    }
    Line::from(spans)
}

fn lead<'a>(item: &'a TileItem<'_>) -> &'a str {
    item.squad
        .lead
        .as_ref()
        .and_then(|lead| lead["name"].as_str())
        .unwrap_or("no lead")
}

fn model<'a>(item: &'a TileItem<'_>) -> Option<&'a str> {
    item.usage
        .as_ref()
        .and_then(|usage| usage.lead_model)
        .or(item.lead_model)
        .map(crate::source::model_name)
}

fn mixed_windows(items: &[TileItem<'_>]) -> bool {
    let mut windows = items
        .iter()
        .filter_map(|item| item.usage.as_ref().map(|usage| usage.windows));
    windows
        .next()
        .is_some_and(|first| windows.any(|windows| windows != first))
}

/// The home painter owns the section heading; column names appear there once.
pub(super) fn legend(items: &[TileItem<'_>], width: u16) -> String {
    if mixed_windows(items) {
        return "lead tokens · windows vary".into();
    }
    let Some(windows) = items
        .iter()
        .find_map(|item| item.usage.as_ref().map(|usage| usage.windows))
    else {
        return String::new();
    };
    let labels = windows[usize::from(compact(width))..]
        .iter()
        .map(|window| window.label())
        .collect::<Vec<_>>()
        .join(" · ");
    if compact(width) {
        format!("lead tokens · {labels}")
    } else {
        format!("lead tokens · {labels} · share ({})", windows[2].label())
    }
}

fn tokens(item: &TileItem<'_>, index: usize) -> String {
    item.usage
        .as_ref()
        .and_then(|usage| usage.lead[index])
        .map_or_else(
            || "–".into(),
            |reading| {
                format!(
                    "{}{}",
                    if reading.partial { "~" } else { "" },
                    crate::source::render_value(
                        &serde_json::Value::String(reading.tokens.to_string()),
                        crate::source::Format::Tokens,
                        0
                    )
                    .expect("token count is numeric")
                )
            },
        )
}

fn share(item: &TileItem<'_>) -> String {
    item.usage
        .as_ref()
        .and_then(|usage| usage.share.as_ref())
        .map_or_else(
            || "–".into(),
            |share| {
                format!(
                    "{}{:.0}%",
                    if share.partial { "~" } else { "" },
                    share.fraction * 100.0
                )
            },
        )
}

fn value_span(value: &str, width: u16, selected: bool, look: Look) -> Span<'static> {
    span(
        tmt_tui::text::fit_line(value, width, TextFlow::Truncate, Align::Right),
        Role::Text,
        selected,
        look,
    )
}

fn lead_line(
    item: &TileItem<'_>,
    width: u16,
    all_windows: bool,
    mixed: bool,
    selected: bool,
    look: Look,
) -> Line<'static> {
    let model = model(item);
    let first = usize::from(!all_windows);
    let unavailable = item
        .usage
        .as_ref()
        .is_some_and(|usage| usage.lead.iter().all(Option::is_none));
    let values = if item.usage.is_none() {
        Vec::new()
    } else if unavailable {
        vec!["–".into()]
    } else {
        (first..3)
            .map(|index| {
                let value = tokens(item, index);
                if mixed {
                    format!("{}:{value}", item.windows()[index].label())
                } else {
                    value
                }
            })
            .chain(std::iter::once(if mixed {
                format!("{}:{}", item.windows()[2].label(), share(item))
            } else {
                share(item)
            }))
            .collect::<Vec<_>>()
    };
    let token_width = if mixed { 10 } else { 7 };
    let values_width = if unavailable {
        2
    } else {
        values.len() as u16 * token_width
    };
    let model_width = model.map_or(0, |name| {
        unicode_width::UnicodeWidthStr::width(name).min(8) as u16
    });
    let model_gap = u16::from(model_width > 0);
    let name_width = width
        .saturating_sub(values_width + model_width + model_gap)
        .min(24);
    let mut spans = vec![span(
        fit(lead(item), name_width),
        Role::Text,
        selected,
        look,
    )];
    if let Some(model) = model {
        spans.push(span(" ".into(), Role::Text, selected, look));
        spans.push(span(fit(model, model_width), Role::Muted, selected, look));
    }
    for value in values {
        let available = width.saturating_sub(spans.iter().map(Span::width).sum::<usize>() as u16);
        if available == 0 {
            break;
        }
        spans.push(span(" ".into(), Role::Text, selected, look));
        let value_width = available.saturating_sub(1).min(token_width - 1);
        spans.push(if unavailable {
            span(fit(&value, value_width), Role::Dim, selected, look)
        } else {
            value_span(&value, value_width, selected, look)
        });
    }
    Line::from(spans)
}

fn tile(
    item: &TileItem<'_>,
    width: u16,
    all_windows: bool,
    mixed: bool,
    selected: bool,
    look: Look,
) -> Vec<Line<'static>> {
    vec![
        heading(item, width, selected, look),
        lead_line(item, width, all_windows, mixed, selected, look),
        member_line(item.members, width, selected, look),
    ]
}

fn compact_line(
    item: &TileItem<'_>,
    width: u16,
    mixed: bool,
    selected: bool,
    look: Look,
) -> Line<'static> {
    let (badge, role) = if item.squad.counts.waiting > 0 {
        (" ◆ ", Role::Waiting)
    } else if item.squad.counts.blocked > 0 {
        (" ✗ ", Role::Blocked)
    } else {
        ("   ", Role::Dim)
    };
    let badge_width = width.min(3);
    let room = width - badge_width;
    let unavailable = item
        .usage
        .as_ref()
        .is_some_and(|usage| usage.lead.iter().all(Option::is_none));
    let values = if item.usage.is_none() {
        Vec::new()
    } else if unavailable {
        vec!["–".into()]
    } else if mixed {
        vec![
            format!("{}:{}", item.windows()[1].label(), tokens(item, 1)),
            format!("{}:{}", item.windows()[2].label(), tokens(item, 2)),
            format!("{}:{}", item.windows()[2].label(), share(item)),
        ]
    } else {
        vec![tokens(item, 1), tokens(item, 2)]
    };
    let value_width = if values.is_empty() {
        0
    } else if unavailable {
        3
    } else if mixed {
        28
    } else {
        15
    }
    .min(room / 2);
    let model_width = model(item).map_or(0, |_| 9.min(room - value_width));
    let room = room - value_width - model_width;
    let name_width = (room / 3).min(18);
    let lead_width = (room / 3).min(20);
    let mut spans = vec![
        span(fit(badge, badge_width), role, selected, look),
        span(
            fit(&item.squad.squad, name_width),
            Role::Accent,
            selected,
            look,
        ),
        span(fit(lead(item), lead_width), Role::Text, selected, look),
        span(
            fit(
                &format!(" {}", model(item).unwrap_or_default()),
                model_width,
            ),
            Role::Muted,
            selected,
            look,
        ),
    ];
    spans.extend(member_line(item.members, room - name_width - lead_width, selected, look).spans);
    let prefix = value_width.min(1);
    spans.push(span(
        " ".repeat(prefix as usize),
        Role::Text,
        selected,
        look,
    ));
    let room = value_width - prefix;
    let count = values.len() as u16;
    for (index, value) in values.iter().enumerate() {
        let width = room / count + u16::from((index as u16) < room % count);
        if width > 0 {
            spans.push(span(" ".into(), Role::Text, selected, look));
            spans.push(if unavailable {
                span(fit(value, width - 1), Role::Dim, selected, look)
            } else {
                value_span(value, width - 1, selected, look)
            });
        }
    }
    Line::from(spans)
}

/// Regions and lines share one geometry result; no scroll, cursor or core effects.
pub(super) fn paint(
    items: &[TileItem<'_>],
    width: u16,
    look: Look,
    selected: Option<usize>,
) -> TilePaint {
    let placements = placement(width, items.len());
    let mixed = mixed_windows(items);
    let height = placements
        .iter()
        .map(|region| region.lines.end)
        .max()
        .unwrap_or(0);
    let mut lines = vec![Line::default(); height];
    let mut regions = Vec::new();
    for region in placements {
        let text_width = region.width;
        let item = &items[region.item];
        let selected = selected == Some(region.item);
        let rows = if compact(width) {
            vec![compact_line(item, text_width, mixed, selected, look)]
        } else {
            tile(item, text_width, true, mixed, selected, look)
        };
        for (index, mut row) in region.lines.clone().zip(rows) {
            let target = &mut lines[index];
            let gap = usize::from(region.x).saturating_sub(target.width());
            target.spans.push(Span::raw(" ".repeat(gap)));
            let padding = usize::from(region.width).saturating_sub(row.width());
            row.spans
                .push(span(" ".repeat(padding), Role::Text, selected, look));
            target.spans.extend(row.spans);
        }
        regions.push(region);
    }
    TilePaint { lines, regions }
}

#[cfg(test)]
mod tests;
