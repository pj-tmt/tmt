//! One boxed-list scene serves HOME leads and squad members. Each caller supplies
//! its row projection and owns scrolling/hit translation; the scene reports local
//! row spans and shared-band reservations.
use super::scene::{self, Key, Part, rule};
use crate::{
    board::{
        app::{App, Compose},
        view::fit,
    },
    look::Look,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use serde_json::json;
use std::{ops::Range, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align, table::escape};
use tmt_tui::{
    binding::{Schema, Template},
    components::Outline,
};

const FILE: &str = "squad.home.leads.xml";

/// A boxed line is its two border cells around the content, with a filler that
/// carries the line's base style. The squad column of a heading is the `md` step.
const MARKUP: &str = r#"<tmt-view version="1">
<tmt-repeat each="$.head" as="line"><tmt-text id-bind="line.id" bind="line.text" token-bind="line.role" class="w-full h-1"/></tmt-repeat>
<tmt-text id="top" bind="$.top" class="w-full h-1"/>
<tmt-repeat each="$.leads" as="lead"><tmt-col id-bind="lead.id" class="w-full">
<tmt-repeat each="lead.before" as="line"><tmt-row id-bind="line.id" class="w-full h-1"><tmt-text id="left" bind="$.left" class="shrink-0"/><tmt-text id="text" bind="line.text" token="dim" class="grow h-1"/><tmt-text id="right" bind="$.right" class="shrink-0"/></tmt-row></tmt-repeat>
<tmt-repeat each="lead.separator" as="sep"><tmt-row id="separator" class="w-full h-1"><tmt-text id="left" bind="$.left" class="shrink-0"/><tmt-text id="fill" class="grow h-1"/><tmt-text id="right" bind="$.right" class="shrink-0"/></tmt-row></tmt-repeat>
<tmt-row id="head" class="w-full h-1">
<tmt-text id="left" bind="$.left" class="shrink-0"/>
<tmt-text id="mark" bind="lead.mark" token-bind="lead.mark_role" class="shrink-0"/>
<tmt-text id="name" bind="lead.name" token="text" class="shrink-0"/>
<tmt-text id="tag" bind="lead.tag" token="dim" class="shrink-0"/>
<tmt-text id="state" bind="lead.state" token-bind="lead.state_role" class="shrink-0"/>
<tmt-text id="squad" bind="lead.squad" token="muted" class="shrink-0" hide-below="md"/>
<tmt-text id="fill" class="grow h-1"/>
<tmt-text id="model" bind="lead.model" token="muted" class="shrink-0"/>
<tmt-text id="age" bind="lead.age" token="dim" class="shrink-0"/>
<tmt-text id="right" bind="$.right" class="shrink-0"/>
</tmt-row>
<tmt-repeat each="lead.after" as="line"><tmt-row id-bind="line.id" class="w-full h-1"><tmt-text id="left" bind="$.left" class="shrink-0"/><tmt-text id="text" bind="line.text" token-bind="line.role" class="shrink-0"/><tmt-text id="fill" class="grow h-1"/><tmt-text id="right" bind="$.right" class="shrink-0"/></tmt-row></tmt-repeat>
</tmt-col></tmt-repeat>
<tmt-repeat each="$.tail" as="line"><tmt-row id-bind="line.id" class="w-full h-1"><tmt-text id="left" bind="$.left" class="shrink-0"/><tmt-text id="text" bind="line.text" token-bind="line.role" class="grow h-1"/><tmt-text id="right" bind="$.right" class="shrink-0"/></tmt-row></tmt-repeat>
<tmt-text id="bottom" bind="$.bottom" class="w-full h-1"/>
</tmt-view>"#;

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
    let list = |item| Schema::Collection(Box::new(item));
    let line = || {
        object(vec![
            ("id", Schema::StableId),
            ("text", Schema::Scalar),
            ("role", Schema::Scalar),
        ])
    };
    object(vec![
        ("head", list(line())),
        ("tail", list(line())),
        ("top", Schema::Scalar),
        ("bottom", Schema::Scalar),
        ("left", Schema::Scalar),
        ("right", Schema::Scalar),
        (
            "leads",
            list(object(vec![
                ("id", Schema::StableId),
                ("before", list(line())),
                ("tag", Schema::Scalar),
                ("state", Schema::Scalar),
                ("state_role", Schema::Scalar),
                ("model", Schema::Scalar),
                ("separator", list(object(vec![("id", Schema::StableId)]))),
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

/// The box's four border strings. `Outline` owns the square glyphs; the section
/// reads them from a painted scratch buffer and styles them itself.
struct Chrome {
    top: String,
    bottom: String,
    left: String,
    right: String,
}

impl Chrome {
    fn new(width: u16, look: Look) -> Self {
        let area = Rect::new(0, 0, width, 3);
        let mut buffer = Buffer::empty(area);
        Outline {
            title: Line::default(),
            border: look.role(Role::Dim),
            title_style: look.role(Role::Dim),
        }
        .paint(area, &mut buffer);
        let row = |y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
        };
        let side = |x| {
            if width > 0 {
                buffer[(x, 1)].symbol().to_owned()
            } else {
                String::new()
            }
        };
        Self {
            top: row(0),
            bottom: row(2),
            left: side(0),
            right: if width > 1 {
                side(width - 1)
            } else {
                String::new()
            },
        }
    }
}

/// What the section reports back, relative to its own first line.
#[derive(Clone)]
pub(in crate::board) struct Block {
    pub lines: Vec<Line<'static>>,
    pub leads: Vec<LeadSpan>,
}

#[derive(Clone)]
pub(in crate::board) struct LeadSpan {
    /// The lead's ordinal within the section; the section's `first` is the
    /// caller's, so a shifted section keeps its cached block.
    pub local: usize,
    /// The heading line.
    pub start: usize,
    /// The heading and the preview under it: the lead's hits.
    pub hit: Range<usize>,
    /// Everything the lead owns through its reserved lines.
    pub end: usize,
    pub reserve: Option<Range<usize>>,
}

pub(in crate::board) fn id(local: usize) -> String {
    format!("lead-{local}")
}

/// The box's border strings are a function of the width alone, so they are read
/// only when the section is painted.
pub(in crate::board) fn build(key: &Key) -> Block {
    let look = key.look;
    let selected = key.selected.as_deref();
    let chrome = Chrome::new(key.width, look);
    let reserving = key.data["leads"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|lead| {
            lead["after"].as_array().map_or(0, |after| {
                after.iter().filter(|line| line["text"].is_null()).count()
            })
        })
        .collect::<Vec<_>>();
    let mut data = key.data.clone();
    data["top"] = json!(chrome.top);
    data["bottom"] = json!(chrome.bottom);
    data["left"] = json!(chrome.left);
    data["right"] = json!(chrome.right);
    let painted = scene::paint(
        FILE,
        template(),
        &data,
        key.width,
        &mut |Part {
                  id: node,
                  scope,
                  role,
              }| {
            let part = node.and_then(<[String]>::last).map(String::as_str);
            let line = node.and_then(|node| node.get(1)).map(String::as_str);
            let lead = scope.and_then(<[String]>::first).map(String::as_str);
            // HOME selects the heading; squad members also select the task line.
            let selected = selected.is_some_and(|id| Some(id) == lead)
                && (line == Some("head") || (key.data["members"] == true && line == Some("task")));
            let boxed = |style: Style| {
                if selected {
                    look.selection().patch(style)
                } else {
                    style
                }
            };
            let style = match (part, line) {
                (Some("rule"), _) => look.role(role),
                (Some("top" | "bottom" | "left" | "right"), _) => look.role(Role::Dim),
                (Some("mark"), _) => boxed(look.row_span(selected, look.role(role), true)),
                (Some("name"), _) => boxed(look.row_span(
                    selected,
                    look.role(Role::Text).add_modifier(Modifier::BOLD),
                    true,
                )),
                (Some("squad" | "state" | "tag" | "model" | "age"), _) => {
                    boxed(look.row_span(selected, look.role(role), false))
                }
                (Some("fill"), _) if selected => look.selection(),
                (Some("fill"), _) => look.role(Role::Text),
                (Some("text"), Some("sent")) => look.role(Role::Working),
                (Some("text"), _) => boxed(look.row_span(selected, look.role(role), false)),
                _ => Style::new(),
            };
            (style, Align::Left)
        },
    );
    let leads = (0..reserving.len())
        .map(|local| {
            let lead = painted.lines(&[&id(local)]).expect("every lead is a block");
            let head = painted
                .lines(&[&id(local), "head"])
                .expect("every lead has a heading");
            let preview = painted
                .lines(&[&id(local), "preview"])
                .or_else(|| painted.lines(&[&id(local), "task"]));
            LeadSpan {
                local,
                start: head.start,
                hit: head.start..preview.map_or(head.end, |preview| preview.end),
                end: lead.end,
                reserve: (reserving[local] > 0).then(|| lead.end - reserving[local]..lead.end),
            }
        })
        .collect();
    Block {
        lines: painted.lines,
        leads,
    }
}

/// Named squad rows use the HOME box template and block builder. Display order,
/// pane scrolling and the shared band remain the existing application owners.
pub(in crate::board) fn render_squad(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    use crate::{
        config::Pane,
        display_rows::{Item, RowOrigin},
    };
    let Some(view) = &app.view else { return };
    let look = app.look();
    let inner = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(2),
        ..area
    };
    let now = crate::status::now_ms();
    let selected_target = app.row_target(app.selected);
    let reading = app.input.as_ref().is_some_and(|input| {
        matches!(input.compose, Compose::ReadRow { .. })
            && input.target() == selected_target.as_ref()
    });
    let reserved =
        crate::board::view::waiting::reserved_lines(app, app.selected, inner).unwrap_or_default();
    let sent_row = app.sent.as_ref().and_then(|sent| {
        (0..app.rows().len()).find(|index| app.row_target(*index).as_ref() == Some(&sent.target))
    });
    let mut before = Vec::new();
    let mut rows = Vec::new();
    for item in app.items() {
        let (origin, row) = match item {
            Item::Rule(rule) => {
                before.push(json!({"id": "members", "text": rule.label(), "role": "dim"}));
                continue;
            }
            Item::Section(Some(title)) => {
                before.push(json!({"id": format!("section-{}", before.len()), "text": title, "role": "muted"}));
                continue;
            }
            Item::Section(None) => continue,
            Item::Row(origin, row) => (origin, row),
        };
        let index = rows.len();
        let reading = reading && index == app.selected;
        let waits = crate::attention::waits_on_you(row);
        let state = if waits {
            "waits on you"
        } else {
            row["state"].as_str().unwrap_or("–")
        };
        let (mark, role) = if waits {
            ("◆", Role::Waiting)
        } else {
            match state {
                "working" => ("●", Role::Working),
                "blocked" => ("✗", Role::Blocked),
                "review" | "testing" => ("◐", Role::Review),
                "idle" => ("○", Role::Dim),
                _ => (" ", Role::Text),
            }
        };
        let model = row["fields"]["model"]
            .as_str()
            .map(crate::source::model_name)
            .unwrap_or_default();
        let model = fit(model, 8).trim_end().to_owned();
        let age = row["staleness"]["unchangedSinceMs"]
            .as_u64()
            .filter(|since| *since > 0 && *since <= now)
            .map(|since| now - since)
            .or_else(|| row["staleness"]["ageMs"].as_u64())
            .filter(|_| matches!(row["staleness"]["state"].as_str(), Some("fresh" | "stale")))
            .map(tmt_cli_style::value::relative_time)
            .unwrap_or_else(|| "–".into());
        let width = usize::from(inner.width);
        let right = format!("{}  {} ", model, age);
        let available =
            width.saturating_sub(3 + unicode_width::UnicodeWidthStr::width(right.as_str()));
        let tag = if origin == RowOrigin::Lead {
            "  lead"
        } else {
            ""
        };
        let tag = if available >= 26 + tag.len() + 3 {
            tag
        } else {
            ""
        };
        let name_width = available.saturating_sub(tag.len() + 3).min(26);
        let name = fit(&escape(row["name"].as_str().unwrap_or("–")), name_width);
        let state_width = available.saturating_sub(name_width + tag.len() + 2);
        let mut after = Vec::new();
        if !reading {
            after.push(json!({"id": "task", "text": format!("   {}", fit(&super::super::notes::sanitize(row["fields"]["task"].as_str().unwrap_or_default()), width.saturating_sub(3)).trim_end()), "role": "text"}));
        }
        if sent_row == Some(index) {
            after.push(json!({"id": "sent", "text": "   ✓ sent", "role": "working"}));
        }
        let reserve = if index == app.selected { reserved } else { 0 };
        after.extend(
            (0..reserve)
                .map(|line| json!({"id": format!("reserve-{line}"), "text": null, "role": null})),
        );
        for line in &mut before {
            line["text"] = json!(rule(line["text"].as_str().unwrap_or_default(), width));
        }
        rows.push(json!({"id": id(index), "before": std::mem::take(&mut before), "separator": [],
            "mark": format!(" {mark} "), "mark_role": role.name(), "name": name, "tag": tag,
            "state": format!("  {}", fit(&escape(state), state_width).trim_end()),
            "state_role": if waits { "waiting" } else { row["colors"]["state"].as_str().and_then(crate::look::role).unwrap_or(Role::Text).name() },
            "squad": "", "model": if model.is_empty() { String::new() } else { format!("{model}  ") },
            "age": format!("{age} "), "after": after}));
    }
    // A trailing rule/section is still meaningful when search hides all members.
    // Attach it as an unselectable tail after the last row, before the box closes.
    if rows.is_empty() {
        before.push(json!({"id": "empty", "text": if app.search.is_empty() { "(no members)" } else { "(no matching members)" }, "role": "dim"}));
    }
    let tail = before.iter().enumerate().map(|(index, line)| json!({"id": format!("tail-{index}"), "text": rule(line["text"].as_str().unwrap_or_default(), usize::from(inner.width)), "role": "dim"})).collect::<Vec<_>>();
    let key = Key {
        width: area.width,
        look,
        selected: Some(id(app.selected)),
        data: json!({"head": [], "leads": rows, "members": true, "tail": tail}),
    };
    let block = view
        .derived
        .borrow_mut()
        .member_list
        .get(key, build)
        .clone();
    let mut input = None;
    let mut selected = 0..0;
    for row in &block.leads {
        app.row_starts.borrow_mut().push(row.start);
        if row.local == app.selected {
            selected = row.start..row.end;
        }
        if let Some(reserve) = &row.reserve {
            input = Some(reserve.clone());
        }
    }
    if app.follow {
        app.scrolls
            .reveal_range(Pane::Rows, selected, area, block.lines.len());
    }
    let (offset, viewport) = app
        .scrolls
        .show(frame, Pane::Rows, area, &block.lines, look);
    crate::board::view::waiting::place_input(app, input, inner, offset, viewport);
    for row in block.leads {
        for line in row.hit.start.max(offset)..row.hit.end.min(offset + viewport) {
            if inner.width > 0 {
                app.hits.borrow_mut().push(crate::board::app::Hit {
                    y: area.y + (line - offset) as u16,
                    x: inner.x,
                    width: inner.width,
                    row: row.local,
                });
            }
        }
    }
}
