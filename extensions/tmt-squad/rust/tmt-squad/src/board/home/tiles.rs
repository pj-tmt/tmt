//! The squad section returns lines and local regions to the single home painter.
use super::{Counts, SquadLine};
use crate::look::Look;
use ratatui::text::{Line, Span};
use std::ops::Range;
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{
    binding::{self, Schema, Schemas, Scopes, Sources},
    geometry,
    style::TextFlow,
};

pub(super) struct TileItem<'a> {
    pub squad: &'a SquadLine,
    /// Exclusive non-lead state counts, projected from acquired memberships.
    pub members: &'a Counts,
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

struct Literals;
impl Sources for Literals {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("tile geometry has no acquired sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("tile geometry binds no runtime values".into())
    }
}

fn compact(width: u16, count: usize) -> bool {
    width < 100 || count >= 10
}

fn placement(width: u16, names: &[&str]) -> Result<Vec<(TileRegion, u16)>, String> {
    if names.is_empty() || width == 0 {
        return Ok(Vec::new());
    }
    let short = compact(width, names.len());
    let columns = if width >= 150 {
        if short { 2 } else { 3 }
    } else if short {
        1
    } else {
        2
    };
    let tile_height = if short { 1 } else { 3 };
    let gap = usize::from(!short);
    let height = names.len().div_ceil(columns) * (tile_height + gap) - gap;
    let height = u16::try_from(height).map_err(|_| "tile section is too tall")?;
    let tracks = vec!["minmax(0,1fr)"; columns].join("_");
    let file = "squad.home.tiles";
    let xml = format!(
        "<tmt-view version='1' class='grid grid-cols-[{tracks}] gap-x-2 gap-y-{gap}'><tmt-cell class='min-w-0 h-{tile_height}'/></tmt-view>"
    );
    let mut root = binding::compile(
        file,
        &tmt_tui::parse(file, &xml).map_err(|e| e.to_string())?,
        &Schema::Object(Default::default()),
        &Literals,
    )
    .and_then(|template| template.materialize(file, &serde_json::json!({}), &Literals))
    .map_err(|e| e.to_string())?;
    let prototype = root.children.remove(0);
    root.children = names
        .iter()
        .map(|name| {
            let mut node = prototype.clone();
            node.id = Some(vec![(*name).into()]);
            node
        })
        .collect();
    geometry::layout(&root, [width, height], |_, _, _| [0, 0])?
        .into_iter()
        .filter(|cell| cell.node.id.is_some())
        .enumerate()
        .map(|(item, cell)| {
            let rect = cell.clip;
            let y = usize::try_from(rect.y).map_err(|_| "negative tile line")?;
            Ok((
                TileRegion {
                    item,
                    lines: y..y + rect.height as usize,
                    x: u16::try_from(rect.x).map_err(|_| "negative tile column")?,
                    width: rect.width as u16,
                },
                cell.text_width,
            ))
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
        let shown = count.min(remaining);
        spans.push(span(mark.to_string().repeat(shown), role, selected, look));
        remaining -= shown;
    }
    spans.push(span(" ".repeat(remaining), Role::Text, selected, look));
    spans
}

fn member_line(counts: &Counts, width: u16, selected: bool, look: Look) -> Line<'static> {
    let label = format!(
        " {} member{}",
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
            badges.push(span(format!(" {mark}{count}"), role, selected, look));
        }
    }
    let badge_width: usize = badges.iter().map(Span::width).sum();
    let name_width = width.saturating_sub(badge_width as u16 + 2);
    let mut spans = vec![span(fit("─ ", width.min(2)), Role::Dim, selected, look)];
    spans.push(span(
        fit(&item.squad.squad, name_width),
        Role::Accent,
        selected,
        look,
    ));
    if badge_width + 2 <= usize::from(width) {
        spans.extend(badges);
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

fn tile(item: &TileItem<'_>, width: u16, selected: bool, look: Look) -> Vec<Line<'static>> {
    vec![
        heading(item, width, selected, look),
        Line::from(span(fit(lead(item), width), Role::Text, selected, look)),
        member_line(item.members, width, selected, look),
    ]
}

fn compact_line(item: &TileItem<'_>, width: u16, selected: bool, look: Look) -> Line<'static> {
    let (badge, role) = if item.squad.counts.waiting > 0 {
        (" ◆ ", Role::Waiting)
    } else if item.squad.counts.blocked > 0 {
        (" ✗ ", Role::Blocked)
    } else {
        ("   ", Role::Dim)
    };
    let badge_width = width.min(3);
    let room = width - badge_width;
    let name_width = (room / 4).min(18);
    let lead_width = (room / 4).min(20);
    let mut spans = vec![
        span(fit(badge, badge_width), role, selected, look),
        span(
            fit(&item.squad.squad, name_width),
            Role::Accent,
            selected,
            look,
        ),
        span(fit(lead(item), lead_width), Role::Text, selected, look),
    ];
    spans.extend(member_line(item.members, room - name_width - lead_width, selected, look).spans);
    Line::from(spans)
}

/// Regions and lines share one geometry result; no scroll, cursor or core effects.
pub(super) fn paint(
    items: &[TileItem<'_>],
    width: u16,
    look: Look,
    selected: Option<usize>,
) -> Result<TilePaint, String> {
    let names = items
        .iter()
        .map(|item| item.squad.squad.as_str())
        .collect::<Vec<_>>();
    let placements = placement(width, &names)?;
    let height = placements
        .iter()
        .map(|(region, _)| region.lines.end)
        .max()
        .unwrap_or(0);
    let mut lines = vec![Line::default(); height];
    let mut regions = Vec::new();
    for (region, text_width) in placements {
        let item = &items[region.item];
        let selected = selected == Some(region.item);
        let rows = if compact(width, items.len()) {
            vec![compact_line(item, text_width, selected, look)]
        } else {
            tile(item, text_width, selected, look)
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
    Ok(TilePaint { lines, regions })
}

#[cfg(test)]
mod tests;
