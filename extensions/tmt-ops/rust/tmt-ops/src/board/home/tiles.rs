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

pub(in crate::board) struct TileItem<'a> {
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

/// One budget owner for the table and its header. Sampling admits the
/// longest-window token/share columns, including explicit missing readings.
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
    /// The text budgets are shared by header, rows, detail visibility and counters.
    fn new(items: &[TileItem<'_>], width: u16, _wide: bool) -> Self {
        let sampled = items.iter().any(|item| item.usage.is_some());
        let mixed = mixed_windows(items);
        let indices = if sampled { vec![2] } else { Vec::new() };
        let share = sampled;
        let room = width.saturating_sub(3);
        let mut result = Self {
            name: 13.min(room),
            lead: if width >= 100 {
                19
            } else if width >= 80 {
                17
            } else {
                0
            },
            model: 0,
            members: if width >= 100 {
                16
            } else if width >= 80 {
                12
            } else {
                0
            },
            values: if sampled {
                if mixed { 24 } else { 18 }
            } else {
                0
            },
            indices,
            share,
            mixed,
            sampled,
        };
        if width >= 100 {
            // Use spare columns for the member marks and full count label.
            let wanted = items
                .iter()
                .map(|item| {
                    let counts = item.members;
                    let marks = counts.waiting
                        + counts.blocked
                        + counts.review
                        + counts.working
                        + counts.idle;
                    unicode_width::UnicodeWidthStr::width(member_label(counts).as_str())
                        .saturating_add(1)
                        .saturating_add(marks.saturating_mul(2))
                        .min(usize::from(u16::MAX)) as u16
                })
                .max()
                .unwrap_or(0);
            let available = room.saturating_sub(result.name + result.lead + result.values);
            result.members = result.members.max(wanted).min(available);
        }
        while result.name + result.lead + result.members + result.values > room {
            if result.name > 2 {
                result.name -= 1;
            } else if result.lead > 2 {
                result.lead -= 1;
            } else if result.members > 0 {
                result.members -= 1;
            } else if result.lead > 0 {
                result.lead -= 1;
            } else {
                result.values = result.values.saturating_sub(1);
                break;
            }
        }
        result
    }
    fn slots(&self) -> usize {
        (self.indices.len() + usize::from(self.share)).max(usize::from(self.sampled))
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
        ('◌', counts.idle, Role::Dim),
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
        (
            column(
                item.squad
                    .squad
                    .strip_prefix("tmt-")
                    .unwrap_or(&item.squad.squad),
                columns.name,
            ),
            Role::Accent,
        ),
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
    let used = 3 + columns.name + columns.lead + columns.model + columns.members + columns.values;
    pieces.push((
        " ".repeat(usize::from(width.saturating_sub(used))),
        Role::Text,
    ));
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

fn header(items: &[TileItem<'_>], width: u16, columns: &Columns) -> String {
    let mut text = format!(
        "   {}{}{}",
        column("squad", columns.name),
        column("lead", columns.lead),
        column("members", columns.members)
    );
    use unicode_width::UnicodeWidthStr;
    let label = if columns.mixed {
        "tok window".into()
    } else {
        let window = items
            .iter()
            .find_map(|item| item.usage.as_ref().map(|usage| usage.windows[2]))
            .unwrap_or(TokenWindow::DEFAULTS[2]);
        format!("tok {}", window.label())
    };
    if columns.sampled {
        text.push_str(
            &" ".repeat(
                usize::from(width).saturating_sub(text.width() + usize::from(columns.values)),
            ),
        );
        let first = columns.values / 2 + columns.values % 2;
        text.push_str(&format!(
            " {} {}",
            tmt_tui::text::fit_line(
                &label,
                first.saturating_sub(1),
                TextFlow::Truncate,
                Align::Right
            ),
            tmt_tui::text::fit_line(
                "lead %",
                (columns.values / 2).saturating_sub(1),
                TextFlow::Truncate,
                Align::Right
            )
        ));
    }
    fit(&text, width)
}

pub(in crate::board) fn hidden_squad(
    item: &TileItem<'_>,
    width: u16,
) -> Vec<(&'static str, String)> {
    let columns = Columns::new(std::slice::from_ref(item), width, width >= 80);
    let mut fields = Vec::new();
    let survives = |value: &str, cells: u16| {
        cells > 0
            && crate::board::row_detail::uncut(
                value,
                usize::from(cells.saturating_sub(1)),
                TextFlow::Truncate,
            )
    };
    if !survives(&item.squad.squad, columns.name) {
        fields.push(("squad", item.squad.squad.clone()));
    }
    if !survives(lead(item), columns.lead) {
        fields.push(("lead", lead(item).into()));
    }
    if let Some(model) = model(item) {
        fields.push(("model", model.into()));
    }
    let counts = item.members;
    let states = format!(
        "◆ {} need you · ✗ {} blocked · ◐ {} review · ● {} working · ◌ {} idle",
        counts.waiting, counts.blocked, counts.review, counts.working, counts.idle
    );
    fields.push((
        "members",
        if !survives(&member_label(counts), columns.members) {
            format!("{} · {states}", member_label(counts))
        } else {
            states
        },
    ));
    if let Some(usage) = &item.usage {
        for index in 0..2 {
            fields.push((
                "lead usage",
                format!(
                    "{} {}",
                    usage.windows[index].label(),
                    if usage.lead[index].is_some() {
                        format!("{} tok", tokens(item, index))
                    } else {
                        "no reading".into()
                    }
                ),
            ));
        }
        if usage.lead[2].is_none() {
            fields.push((
                "lead usage",
                format!("{} no reading", usage.windows[2].label()),
            ));
        }
        if usage.share.is_none() {
            fields.push(("share", "no share reading".into()));
        }
    }
    fields
}

/// Names are display text and may hold anything; the ordinal is the frame's identity.
fn row_id(item: usize) -> String {
    format!("squad-{item}")
}

/// The rule and rows of one branch of the `md` switch.
fn branch(items: &[TileItem<'_>], width: u16, wide: bool, digits: Option<&[[String; 3]]>) -> Value {
    let columns = Columns::new(items, width, wide);
    let legend = if columns.mixed {
        "windows vary".into()
    } else {
        String::new()
    };
    let title = format!(
        "Squads · {}{}",
        items.len(),
        if legend.is_empty() {
            String::new()
        } else {
            format!(" · {legend}")
        }
    );
    json!({
        "rule": rule(&title, usize::from(width)),
        "header": if width >= 80 { vec![json!({"text": header(items, width, &columns)})] } else { Vec::new() },
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
            r#"<tmt-text id="rule" bind="$.{branch}.rule" token="muted" class="w-full"/><tmt-repeat each="$.{branch}.header" as="header"><tmt-text id="header" bind="header.text" token="muted" class="w-full h-1"/></tmt-repeat><tmt-repeat each="$.{branch}.rows" as="row"><tmt-row id-bind="row.id" class="w-full"><tmt-repeat each="row.cells" as="cell"><tmt-text id-bind="cell.id" bind="cell.text" token-bind="cell.role" class="shrink-0"/></tmt-repeat></tmt-row></tmt-repeat>"#
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
                "header",
                Schema::Collection(Box::new(object(vec![("text", Schema::Scalar)]))),
            ),
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
                Some([rule]) if rule == "rule" || rule == "header" => look.role(role),
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
    let header = usize::from(key.width >= 80);
    let rows = lines.split_off(1 + header);
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
    let mut x = width.saturating_sub(columns.values);
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
