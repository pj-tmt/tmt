//! The blocked section: a gap, a rule, then one line per member.
use super::{
    controller::HomeEntry,
    paint::age_label,
    scene::{self, Kept, Key, Painted, Part, rule},
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
use unicode_width::UnicodeWidthStr;

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
<tmt-repeat each="row.digest" as="digest"><tmt-text id="digest-gap" class="shrink-0"> </tmt-text><tmt-text id="digest-word" bind="digest.word" token="text" class="shrink-0"/><tmt-text id="digest-suffix" bind="digest.suffix" token="muted" class="shrink-0"/></tmt-repeat>
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
                (
                    "digest",
                    list(object(&[
                        ("word", Schema::Scalar),
                        ("suffix", Schema::Scalar),
                    ])),
                ),
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
}

fn head(section: &Section<'_>, width: usize) -> Vec<Value> {
    vec![
        json!({"id": "gap", "text": null, "role": null}),
        json!({
            "id": "rule",
            "text": rule(&format!("✗ blocked · {}", section.entries.len()), width),
            "role": Role::Blocked.name(),
        }),
    ]
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
            let age = entry.age.map(|age| age_label(age, now)).unwrap_or_default();
            let age_width = unicode_width::UnicodeWidthStr::width(age.as_str());
            let available = width.saturating_sub(4 + age_width);
            let digest = crate::digest::pieces(entry.row, now, available / 2);
            let digest_width = digest.as_array().unwrap().first().map_or(0, |piece| {
                piece["word"].as_str().unwrap().width() + piece["suffix"].as_str().unwrap().width() + 1
            });
            let name_width = (available.saturating_sub(digest_width) / 2).min(24);
            let sent = app.sent.as_ref().and_then(|feedback| {
                feedback.line(&crate::board::app::RowTarget::Home(entry.target.clone()))
            });
            let mut after = Vec::new();
            if let Some(text) = sent {
                after.push(json!({"id": "sent", "text": format!("   {text}"), "role": "working"}));
            }
            for line in
                0..crate::board::view::waiting::reserved_lines(app, index, area).unwrap_or_default()
            {
                after.push(json!({"id": format!("reserve-{line}"), "text": null, "role": null}));
            }
            let budget = available / 2;
            let visible = if crate::digest::fitted(entry.row, now, budget).2 {
                vec!["digest"]
            } else {
                vec![]
            };
            json!({
                "digest":digest,
                "id": id(local),
                "mark": " ✗ ",
                "mark_role": Role::Blocked.name(),
                "name": fit(&escape(entry.row["name"].as_str().unwrap_or_default()), name_width),
                "squad": if digest_width == 0 {
                    fit(&escape(&entry.target.squad), available.saturating_sub(name_width))
                } else {
                    format!(" {}", fit(&escape(&entry.target.squad), available.saturating_sub(name_width + digest_width + 1)))
                },
                "age": format!(" {age}"),
                "after": after,
                "detail": app.detail_value(index,&visible),
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
                Some("name" | "squad" | "age" | "digest-word" | "digest-suffix" | "digest-gap") => {
                    base.patch(span(look.role(role), false))
                }
                Some("fill") => base,
                _ => Style::new(),
            };
            (style, Align::Left)
        },
    );
    let mut rows: Vec<RowSpan> = (0..reserving.len())
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
    let mut lines = painted.lines;
    let mut shift = 0;
    for row in &mut rows {
        row.start += shift;
        row.end += shift;
        if let Some(range) = &mut row.reserve {
            range.start += shift;
            range.end += shift;
        }
        let count = crate::board::row_detail::insert(
            &mut lines,
            row.start + 1,
            &key.data["rows"][row.local]["detail"],
            key.width,
            3,
            look,
            selected == Some(id(row.local).as_str()),
            false,
        );
        row.end += count;
        if let Some(range) = &mut row.reserve {
            range.start += count;
            range.end += count;
        }
        shift += count;
    }
    Block { lines, rows }
}
