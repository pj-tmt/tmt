//! The needs-you and blocked sections: a rule, then one line per member.
use super::{
    controller::HomeEntry,
    paint::{age_label, rule},
    scene::{self, Kept, Key, Painted, Part},
};
use crate::{
    board::{app::App, view::fit},
    look::Look,
};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use serde_json::{Value, json};
use std::{ops::Range, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align, table::escape};
use tmt_tui::binding::{Schema, Template};

const FILE: &str = "squad.home.attention.xml";

/// Lines before the rows (a blank, the rule) and, per row, the line itself, then
/// whatever follows it: the `✓ sent` line and the lines reserved for the composer.
const MARKUP: &str = r#"<tmt-view version="1">
<tmt-repeat each="$.head" as="line"><tmt-text id-bind="line.id" bind="line.text" token-bind="line.role" class="w-full h-1"/></tmt-repeat>
<tmt-repeat each="$.rows" as="row"><tmt-col id-bind="row.id" class="w-full">
<tmt-row id="line" class="w-full">
<tmt-text id="mark" bind="row.mark" token-bind="row.mark_role" class="shrink-0"/>
<tmt-text id="name" bind="row.name" token="text" class="shrink-0"/>
<tmt-text id="squad" bind="row.squad" token="muted" class="shrink-0"/>
<tmt-text id="age" bind="row.age" token="dim" class="shrink-0"/>
<tmt-text id="fill" class="grow h-1"/>
</tmt-row>
<tmt-repeat each="row.after" as="extra"><tmt-row id-bind="extra.id" class="h-1"><tmt-text id="text" bind="extra.text" token-bind="extra.role" class="shrink-0"/></tmt-row></tmt-repeat>
</tmt-col></tmt-repeat>
</tmt-view>"#;

fn schema() -> Schema {
    use std::collections::BTreeMap;
    let object = |fields: &[(&str, Schema)]| {
        Schema::Object(
            fields
                .iter()
                .map(|(name, schema)| ((*name).to_owned(), schema.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    };
    let list = |item| Schema::Collection(Box::new(item));
    let line = || {
        object(&[
            ("id", Schema::StableId),
            ("text", Schema::Scalar),
            ("role", Schema::Scalar),
        ])
    };
    object(&[
        ("head", list(line())),
        (
            "rows",
            list(object(&[
                ("id", Schema::StableId),
                ("mark", Schema::Scalar),
                ("mark_role", Schema::Scalar),
                ("name", Schema::Scalar),
                ("squad", Schema::Scalar),
                ("age", Schema::Scalar),
                ("after", list(line())),
            ])),
        ),
    ])
}

fn template() -> &'static Template<()> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| scene::compile(FILE, MARKUP, &schema()))
}

/// What the section reports back, relative to its own first line.
#[derive(Clone)]
pub(super) struct Block {
    pub lines: Vec<Line<'static>>,
    pub rows: Vec<RowSpan>,
}

#[derive(Clone)]
pub(super) struct RowSpan {
    /// The entry's ordinal within the section; the section's `first` is the
    /// caller's, so a shifted section keeps its cached block.
    pub local: usize,
    /// The member line, and everything the entry owns through its reserved lines.
    pub start: usize,
    pub end: usize,
    pub reserve: Option<Range<usize>>,
}

pub(super) struct Section<'a> {
    pub first: usize,
    pub entries: &'a [HomeEntry<'a>],
    /// A blank, then this rule; or the rule alone when `blank` is false.
    pub title: String,
    pub role: Role,
    pub blank: bool,
}

fn head(section: &Section<'_>, width: usize) -> Vec<Value> {
    let mut head = Vec::new();
    if section.blank {
        head.push(json!({"id": "gap", "text": null, "role": null}));
    }
    head.push(json!({
        "id": "rule",
        "text": rule(&section.title, width),
        "role": section.role.name(),
    }));
    head
}

/// Block identity is private to one frame; entry names may hold anything, so the
/// ordinal stands in for them.
fn id(index: usize) -> String {
    format!("entry-{index}")
}

/// The section's block, painted again only when its key changed.
pub(super) fn paint(
    app: &App,
    look: Look,
    area: Rect,
    now: u64,
    section: &Section<'_>,
    slot: &mut Kept<Block>,
) -> Block {
    let width = usize::from(area.width);
    let rows = section
        .entries
        .iter()
        .enumerate()
        .map(|(local, entry)| {
            let index = section.first + local;
            let waiting = entry.target.section == "needs-you";
            let age = entry.age.map(|age| age_label(age, now)).unwrap_or_default();
            let age_width = unicode_width::UnicodeWidthStr::width(age.as_str());
            let available = width.saturating_sub(4 + age_width);
            let name_width = (available / 2).min(24);
            let sent = app.sent.as_ref().is_some_and(|feedback| {
                feedback.target == crate::board::app::RowTarget::Home(entry.target.clone())
            });
            let mut after = Vec::new();
            if sent {
                after.push(json!({"id": "sent", "text": "   ✓ sent", "role": "working"}));
            }
            for line in
                0..crate::board::view::waiting::reserved_lines(app, index, area).unwrap_or_default()
            {
                after.push(json!({"id": format!("reserve-{line}"), "text": null, "role": null}));
            }
            json!({
                "id": id(local),
                "mark": format!(" {} ", if waiting { "◆" } else { "✗" }),
                "mark_role": if waiting { Role::Waiting } else { Role::Blocked }.name(),
                "name": fit(&escape(entry.row["name"].as_str().unwrap_or_default()), name_width),
                "squad": fit(&escape(&entry.target.squad), available.saturating_sub(name_width)),
                "age": format!(" {age}"),
                "after": after,
            })
        })
        .collect::<Vec<_>>();
    let selected = app
        .selected
        .checked_sub(section.first)
        .filter(|local| *local < section.entries.len())
        .map(id);
    let key = Key {
        width: area.width,
        look,
        selected,
        data: json!({"head": head(section, width), "rows": rows}),
    };
    slot.get(key, build).clone()
}

fn build(key: &Key) -> Block {
    let look = key.look;
    let selected = key.selected.as_deref();
    let reserving = key.data["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            row["after"].as_array().map_or(0, |after| {
                after.iter().filter(|line| line["text"].is_null()).count()
            })
        })
        .collect::<Vec<_>>();
    let painted: Painted = scene::paint(
        FILE,
        template(),
        &key.data,
        key.width,
        &mut |Part {
                  id: node,
                  scope,
                  role,
              }| {
            let part = node.and_then(<[String]>::last).map(String::as_str);
            let row = scope.and_then(<[String]>::first).map(String::as_str);
            let selected = selected.is_some_and(|id| Some(id) == row);
            let base = if selected {
                look.selection().add_modifier(Modifier::BOLD)
            } else {
                look.role(Role::Text)
            };
            let span = |style: Style, emphasize| look.row_span(selected, style, emphasize);
            let style = match part {
                Some("rule") => look.role(role),
                Some("text")
                    if node.is_some_and(|id| id.get(1).is_some_and(|extra| extra == "sent")) =>
                {
                    look.role(Role::Working)
                }
                Some("mark") => base.patch(span(look.role(role), true)),
                Some("name" | "squad" | "age") => base.patch(span(look.role(role), false)),
                Some("fill") => base,
                _ => Style::new(),
            };
            (style, Align::Left)
        },
    );
    let rows = (0..reserving.len())
        .map(|local| {
            let block = painted
                .lines(&[&id(local)])
                .expect("every entry is a scene block");
            let reserve = (reserving[local] > 0).then(|| block.end - reserving[local]..block.end);
            RowSpan {
                local,
                start: block.start,
                end: block.end,
                reserve,
            }
        })
        .collect();
    Block {
        lines: painted.lines,
        rows,
    }
}
