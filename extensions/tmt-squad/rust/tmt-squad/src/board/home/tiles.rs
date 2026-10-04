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

/// A single full-width column; local line ranges are also the scroll/hit map.
fn placement(width: u16, count: usize) -> Vec<TileRegion> {
    if width == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|item| TileRegion {
            item,
            lines: item..item + 1,
            x: 0,
            width,
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
    let label = format!(" {}", member_label(counts));
    let label_width =
        unicode_width::UnicodeWidthStr::width(label.as_str()).min(usize::from(width)) as u16;
    let mut spans = marks(counts, width - label_width, selected, look);
    spans.push(span(fit(&label, label_width), Role::Dim, selected, look));
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

fn observed(item: &TileItem<'_>) -> bool {
    item.usage
        .as_ref()
        .is_some_and(|usage| usage.lead.iter().any(Option::is_some) || usage.share.is_some())
}

fn mixed_windows(items: &[TileItem<'_>]) -> bool {
    let mut windows = items
        .iter()
        .filter(|item| observed(item))
        .map(TileItem::windows);
    windows
        .next()
        .is_some_and(|first| windows.any(|windows| windows != first))
}

/// One admission/budget owner for the table and its legend. Sampling alone
/// reserves the first unavailable slot; observed data admits named columns.
struct Columns {
    name: u16,
    lead: u16,
    model: u16,
    members: u16,
    values: u16,
    indices: Vec<usize>,
    share: bool,
    mixed: bool,
    sampled: bool,
}

fn member_label(counts: &Counts) -> String {
    let known = counts.waiting + counts.blocked + counts.review + counts.working + counts.idle;
    let other = counts.members.saturating_sub(known);
    format!(
        "{}{} member{}",
        if other > 0 {
            format!("{other} other · ")
        } else {
            String::new()
        },
        counts.members,
        if counts.members == 1 { "" } else { "s" }
    )
}

impl Columns {
    fn new(items: &[TileItem<'_>], width: u16) -> Self {
        use unicode_width::UnicodeWidthStr;
        let wide = width >= 100;
        let indices = (usize::from(!wide)..3)
            .filter(|&index| {
                items.iter().any(|item| {
                    item.usage
                        .as_ref()
                        .is_some_and(|usage| usage.lead[index].is_some())
                })
            })
            .collect::<Vec<_>>();
        let share = wide
            && items.iter().any(|item| {
                item.usage
                    .as_ref()
                    .is_some_and(|usage| usage.share.is_some())
            });
        let sampled = items.iter().any(|item| item.usage.is_some());
        let mixed = mixed_windows(items);
        let slots = (indices.len() + usize::from(share)).max(usize::from(sampled));
        let desired_values = (slots * if mixed { 11 } else { 7 }) as u16;
        let room = width.saturating_sub(3);
        let max_width = |values: Vec<&str>, cap: usize| {
            values
                .into_iter()
                .map(UnicodeWidthStr::width)
                .max()
                .unwrap_or(0)
                .min(cap) as u16
        };
        let name = max_width(
            items.iter().map(|item| item.squad.squad.as_str()).collect(),
            if wide { 24 } else { 18 },
        ) + u16::from(!items.is_empty());
        let lead = max_width(items.iter().map(lead).collect(), if wide { 28 } else { 20 })
            + u16::from(!items.is_empty());
        let model = max_width(items.iter().filter_map(model).collect(), 8);
        let model = model + u16::from(model > 0);
        let members = items
            .iter()
            .map(|item| {
                let counts = item.members;
                let known =
                    counts.waiting + counts.blocked + counts.review + counts.working + counts.idle;
                (known.saturating_mul(2).min(20) + member_label(counts).width() + 1) as u16
            })
            .max()
            .unwrap_or(0)
            .min(44);
        let mut result = Self {
            name,
            lead,
            model,
            members,
            values: desired_values.min(room / 2),
            indices,
            share,
            mixed,
            sampled,
        };
        while result.name + result.lead + result.model + result.members + result.values > room {
            if result.name > 2 || result.lead > 2 {
                if result.name >= result.lead {
                    result.name -= 1;
                } else {
                    result.lead -= 1;
                }
            } else if result.members > 0 {
                result.members -= 1;
            } else if result.model > 0 {
                result.model -= 1;
            } else if result.lead > 0 {
                result.lead -= 1;
            } else if result.name > 0 {
                result.name -= 1;
            } else {
                result.values -= 1;
            }
        }
        result
    }
    fn slots(&self) -> usize {
        (self.indices.len() + usize::from(self.share)).max(usize::from(self.sampled))
    }
}

/// The home painter owns the section heading; admit only rendered observations.
pub(super) fn legend(items: &[TileItem<'_>], width: u16) -> String {
    let columns = Columns::new(items, width);
    if columns.indices.is_empty() && !columns.share || columns.values < columns.slots() as u16 * 2 {
        return String::new();
    }
    if columns.mixed {
        return "lead tokens · windows vary".into();
    }
    let windows = items
        .iter()
        .find(|item| observed(item))
        .map(TileItem::windows)
        .expect("admitted observation");
    let mut labels = columns
        .indices
        .iter()
        .map(|&index| windows[index].label())
        .collect::<Vec<_>>();
    let share = format!("share ({})", windows[2].label());
    if columns.share {
        labels.push(share);
    }
    format!("lead tokens · {}", labels.join(" · "))
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

fn column(value: &str, width: u16) -> String {
    if width == 0 {
        String::new()
    } else {
        format!("{} ", fit(value, width - 1))
    }
}

fn table_line(
    item: &TileItem<'_>,
    width: u16,
    columns: &Columns,
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
    let mut spans = vec![
        span(fit(badge, width.min(3)), role, selected, look),
        span(
            column(&item.squad.squad, columns.name),
            Role::Accent,
            selected,
            look,
        ),
        span(column(lead(item), columns.lead), Role::Text, selected, look),
        span(
            column(model(item).unwrap_or_default(), columns.model),
            Role::Muted,
            selected,
            look,
        ),
    ];
    spans.extend(member_line(item.members, columns.members, selected, look).spans);
    let unavailable = item
        .usage
        .as_ref()
        .is_some_and(|usage| usage.lead.iter().all(Option::is_none) && usage.share.is_none());
    let mut values = columns
        .indices
        .iter()
        .map(|&index| {
            let value = tokens(item, index);
            if columns.mixed {
                format!("{}:{value}", item.windows()[index].label())
            } else {
                value
            }
        })
        .collect::<Vec<_>>();
    if columns.share {
        let value = share(item);
        values.push(if columns.mixed {
            format!("{}:{value}", item.windows()[2].label())
        } else {
            value
        });
    }
    values.resize(columns.slots(), String::new());
    let count = values.len() as u16;
    for (index, value) in values.into_iter().enumerate() {
        let cell_width =
            columns.values / count + u16::from((index as u16) < columns.values % count);
        if cell_width == 0 {
            continue;
        }
        spans.push(span(" ".into(), Role::Text, selected, look));
        let value = if item.usage.is_none() || unavailable && index > 0 {
            String::new()
        } else if unavailable {
            "–".into()
        } else {
            value
        };
        spans.push(if unavailable {
            span(
                tmt_tui::text::fit_line(&value, cell_width - 1, TextFlow::Truncate, Align::Right),
                Role::Dim,
                selected,
                look,
            )
        } else {
            value_span(&value, cell_width - 1, selected, look)
        });
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
    let columns = Columns::new(items, width);
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
        let rows = vec![table_line(item, text_width, &columns, selected, look)];
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
