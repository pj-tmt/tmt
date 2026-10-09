//! The squad section returns lines and local regions to the single home painter.
use super::{
    Counts, SquadLine,
    scene::{self, Kept, Key, Part, rule},
};
use crate::{board::app::HomeUsage, config::TokenWindow, look::Look};
use ratatui::{style::Style, text::Line};
use serde_json::{Value, json};
use std::{ops::Range, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{
    binding::{Schema, Template},
    style::TextFlow,
};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TileRegion {
    pub item: usize,
    pub lines: Range<usize>,
    pub x: u16,
    pub width: u16,
}

#[derive(Clone)]
pub(super) struct TilePaint {
    /// The section rule, a line above the rows.
    pub head: Vec<Line<'static>>,
    /// One compact row per squad; local line ranges are also the scroll/hit map.
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
    /// `wide` is the `md` step: the markup's switch picks the branch that was
    /// built with it; nothing here measures the terminal against a number.
    fn new(items: &[TileItem<'_>], width: u16, wide: bool) -> Self {
        use unicode_width::UnicodeWidthStr;
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
pub(super) fn legend(items: &[TileItem<'_>], width: u16, wide: bool) -> String {
    let columns = Columns::new(items, width, wide);
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

fn column(value: &str, width: u16) -> String {
    if width == 0 {
        String::new()
    } else {
        format!("{} ", fit(value, width - 1))
    }
}

type Piece = (String, Role);

/// The marks of one squad's members, then the count label, in `width` cells.
fn member_pieces(counts: &Counts, width: u16) -> Vec<Piece> {
    let label = format!(" {}", member_label(counts));
    let label_width =
        unicode_width::UnicodeWidthStr::width(label.as_str()).min(usize::from(width)) as u16;
    let mut remaining = usize::from(width - label_width);
    let mut pieces = Vec::new();
    for (mark, count, role) in [
        ('◆', counts.waiting, Role::Waiting),
        ('✗', counts.blocked, Role::Blocked),
        ('◐', counts.review, Role::Review),
        ('●', counts.working, Role::Working),
        ('○', counts.idle, Role::Dim),
    ] {
        let shown = count.min(remaining / 2);
        pieces.push((format!("{mark} ").repeat(shown), role));
        remaining -= shown * 2;
    }
    pieces.push((" ".repeat(remaining), Role::Text));
    pieces.push((fit(&label, label_width), Role::Dim));
    pieces
}

/// One squad's row as role-tagged pieces whose widths add up to `width`.
fn pieces(
    item: &TileItem<'_>,
    width: u16,
    columns: &Columns,
    digits: Option<&[String; 3]>,
) -> Vec<Piece> {
    let (badge, role) = if item.squad.counts.waiting > 0 {
        (" ◆ ", Role::Waiting)
    } else if item.squad.counts.blocked > 0 {
        (" ✗ ", Role::Blocked)
    } else {
        ("   ", Role::Dim)
    };
    let mut pieces = vec![
        (fit(badge, width.min(3)), role),
        (column(&item.squad.squad, columns.name), Role::Accent),
        (column(lead(item), columns.lead), Role::Text),
        (
            column(model(item).unwrap_or_default(), columns.model),
            Role::Muted,
        ),
    ];
    pieces.extend(member_pieces(item.members, columns.members));
    let unavailable = item
        .usage
        .as_ref()
        .is_some_and(|usage| usage.lead.iter().all(Option::is_none) && usage.share.is_none());
    let mut values = columns
        .indices
        .iter()
        .map(|&index| {
            let value = digits.map_or_else(|| tokens(item, index), |digits| digits[index].clone());
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
        pieces.push((" ".into(), Role::Text));
        let value = if item.usage.is_none() || unavailable && index > 0 {
            String::new()
        } else if unavailable {
            "–".into()
        } else {
            value
        };
        pieces.push((
            tmt_tui::text::fit_line(&value, cell_width - 1, TextFlow::Truncate, Align::Right),
            if unavailable { Role::Dim } else { Role::Text },
        ));
    }
    let used: usize = pieces
        .iter()
        .map(|(text, _)| unicode_width::UnicodeWidthStr::width(text.as_str()))
        .sum();
    pieces.push((
        " ".repeat(usize::from(width).saturating_sub(used)),
        Role::Text,
    ));
    pieces.retain(|(text, _)| !text.is_empty());
    pieces
}

/// Names are display text and may hold anything; the ordinal is the frame's identity.
fn row_id(item: usize) -> String {
    format!("squad-{item}")
}

/// The rule and rows of one branch of the `md` switch.
fn branch(items: &[TileItem<'_>], width: u16, wide: bool, digits: Option<&[[String; 3]]>) -> Value {
    let columns = Columns::new(items, width, wide);
    let legend = legend(items, width, wide);
    let title = format!(
        "squads · {}{}",
        items.len(),
        if legend.is_empty() {
            String::new()
        } else {
            format!(" · {legend}")
        }
    );
    json!({
        "rule": rule(&title, usize::from(width)),
        "rows": items.iter().enumerate().map(|(at, item)| {
            json!({
                "id": row_id(at),
                "cells": pieces(item, width, &columns, digits.map(|digits| &digits[at])).into_iter().enumerate().map(|(at, (text, role))| {
                    json!({"id": format!("c{at}"), "text": text, "role": role.name()})
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    })
}

const FILE: &str = "squad.home.squads.xml";

fn markup() -> String {
    let body = |branch: &str| {
        format!(
            r#"<tmt-text id="rule" bind="$.{branch}.rule" token="muted" class="w-full"/><tmt-repeat each="$.{branch}.rows" as="row"><tmt-row id-bind="row.id" class="w-full"><tmt-repeat each="row.cells" as="cell"><tmt-text id-bind="cell.id" bind="cell.text" token-bind="cell.role" class="shrink-0"/></tmt-repeat></tmt-row></tmt-repeat>"#
        )
    };
    format!(
        r#"<tmt-view version="1"><tmt-switch><tmt-case min="md">{}</tmt-case><tmt-default>{}</tmt-default></tmt-switch></tmt-view>"#,
        body("wide"),
        body("narrow")
    )
}

fn schema() -> Schema {
    use std::collections::BTreeMap;
    let object = |fields: Vec<(&str, Schema)>| {
        Schema::Object(
            fields
                .into_iter()
                .map(|(name, schema)| (name.to_owned(), schema))
                .collect::<BTreeMap<_, _>>(),
        )
    };
    let branch = || {
        object(vec![
            ("rule", Schema::Scalar),
            (
                "rows",
                Schema::Collection(Box::new(object(vec![
                    ("id", Schema::StableId),
                    (
                        "cells",
                        Schema::Collection(Box::new(object(vec![
                            ("id", Schema::StableId),
                            ("text", Schema::Scalar),
                            ("role", Schema::Scalar),
                        ]))),
                    ),
                ]))),
            ),
        ])
    };
    object(vec![("wide", branch()), ("narrow", branch())])
}

fn template() -> &'static Template<()> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| scene::compile(FILE, &markup(), &schema()))
}

/// The squads section: its rule and one full-width row per squad. A single
/// column at every width; `md` decides which columns the table has.
#[cfg(test)]
pub(super) fn paint(
    items: &[TileItem<'_>],
    width: u16,
    look: Look,
    selected: Option<usize>,
) -> TilePaint {
    paint_in(&mut Kept::default(), items, width, look, selected)
}

/// The section, painted again only when its key changed.
#[cfg(test)]
pub(super) fn paint_in(
    slot: &mut Kept<TilePaint>,
    items: &[TileItem<'_>],
    width: u16,
    look: Look,
    selected: Option<usize>,
) -> TilePaint {
    paint_display_in(slot, items, width, look, selected, None)
}

pub(super) fn paint_display_in(
    slot: &mut Kept<TilePaint>,
    items: &[TileItem<'_>],
    width: u16,
    look: Look,
    selected: Option<usize>,
    digits: Option<&[[String; 3]]>,
) -> TilePaint {
    if width == 0 {
        return TilePaint {
            head: vec![Line::default()],
            lines: Vec::new(),
            regions: Vec::new(),
        };
    }
    let key = Key {
        width,
        look,
        selected: selected.map(row_id),
        data: json!({"wide": branch(items, width, true, digits), "narrow": branch(items, width, false, digits)}),
    };
    slot.get(key, build).clone()
}

fn build(key: &Key) -> TilePaint {
    let look = key.look;
    let selected = key.selected.as_deref();
    let painted = scene::paint(
        FILE,
        template(),
        &key.data,
        key.width,
        &mut |Part { id, scope, role }| {
            let row = scope.and_then(<[String]>::first).map(String::as_str);
            let style = match id {
                Some([rule]) if rule == "rule" => look.role(role),
                Some([_, _]) => {
                    let selected = selected.is_some_and(|id| Some(id) == row);
                    let emphasize = matches!(
                        role,
                        Role::Accent | Role::Waiting | Role::Blocked | Role::Review | Role::Working
                    );
                    let style = look.row_span(selected, look.role(role), emphasize);
                    if selected {
                        look.selection().patch(style)
                    } else {
                        style
                    }
                }
                _ => Style::new(),
            };
            (style, Align::Left)
        },
    );
    let mut lines = painted.lines;
    let rows = lines.split_off(1);
    TilePaint {
        head: lines,
        regions: placement(
            key.width,
            key.data["wide"]["rows"].as_array().map_or(0, Vec::len),
        ),
        lines: rows,
    }
}

#[cfg(test)]
mod tests;

/// Counter slots reuse the same table budget used by the admitted branch.
pub(super) fn counter_slots(items: &[TileItem<'_>], width: u16) -> Vec<(usize, u16, u16, bool)> {
    let columns = Columns::new(items, width, width >= tmt_cli_style::breakpoint::MD.cells);
    let slots = columns.slots() as u16;
    if slots == 0 {
        return Vec::new();
    }
    let mut x = 3 + columns.name + columns.lead + columns.model + columns.members;
    columns
        .indices
        .iter()
        .enumerate()
        .map(|(slot, &index)| {
            let cell = columns.values / slots + u16::from((slot as u16) < columns.values % slots);
            let result = (
                index,
                x.saturating_add(1),
                cell.saturating_sub(1),
                columns.mixed,
            );
            x += cell;
            result
        })
        .collect()
}
