//! One boxed-list scene serves HOME leads and squad members. Each caller supplies
//! its row projection and owns scrolling/hit translation; the scene reports local
//! row spans and shared-band reservations.
use super::{
    scene::{self, Key, Part, rule},
    stable::Stable,
};
use crate::{
    board::{app::App, row_chips, view::fit},
    look::Look,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use serde_json::{Value, json};
use std::{ops::Range, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align, table::escape};
use tmt_tui::{
    binding::{Schema, Template},
    components::Outline,
};
use unicode_width::UnicodeWidthStr;

const FILE: &str = "squad.home.leads.xml";

/// Cells of the age cell in a squad row: the widest relative age (`just now`,
/// `999d ago`). The age is right-aligned, so a placeholder `–` ends where a value does.
const AGE_CELLS: usize = 8;

/// A box narrower than this shows no model or age: the name, state and chips keep the room.
pub(in crate::board) const NARROW: usize = 40;

/// The right-aligned age cell and its trailing space, `cells` wide before the space;
/// no cells, no cell.
pub(in crate::board) fn age_cell(age: &str, cells: usize) -> String {
    if cells == 0 {
        return String::new();
    }
    format!(
        "{} ",
        tmt_tui::text::fit_line(
            age,
            cells.min(usize::from(u16::MAX)) as u16,
            tmt_tui::style::TextFlow::Truncate,
            Align::Right
        )
    )
}

/// The left-aligned model cell and its gap, blank at the same width when the row has none.
fn model_cell(model: &str, cells: usize) -> String {
    if cells == 0 {
        String::new()
    } else {
        format!("{}  ", fit(model, cells))
    }
}

/// A boxed line is its two border cells around the content, with a filler that
/// carries the line's base style. The squad column of a heading is the `md` step.
/// The top edge is drawn only for a section whose `head` has no rule of its own.
const MARKUP: &str = r#"<tmt-view version="1">
<tmt-repeat each="$.head" as="line"><tmt-text id-bind="line.id" bind="line.text" token-bind="line.role" class="w-full h-1"/></tmt-repeat>
<tmt-repeat each="$.top" as="edge"><tmt-text id="top" bind="edge.text" class="w-full h-1"/></tmt-repeat>
<tmt-repeat each="$.leads" as="lead"><tmt-col id-bind="lead.id" class="w-full">
<tmt-repeat each="lead.before" as="line"><tmt-row id-bind="line.id" class="w-full h-1"><tmt-text id="left" bind="$.left" class="shrink-0"/><tmt-text id="text" bind="line.text" token="dim" class="grow h-1"/><tmt-text id="right" bind="$.right" class="shrink-0"/></tmt-row></tmt-repeat>
<tmt-repeat each="lead.separator" as="sep"><tmt-row id="separator" class="w-full h-1"><tmt-text id="left" bind="$.left" class="shrink-0"/><tmt-text id="fill" class="grow h-1"/><tmt-text id="right" bind="$.right" class="shrink-0"/></tmt-row></tmt-repeat>
<tmt-row id="head" class="w-full h-1">
<tmt-text id="left" bind="$.left" class="shrink-0"/>
<tmt-text id="mark" bind="lead.mark" token-bind="lead.mark_role" class="shrink-0"/>
<tmt-text id="name" bind="lead.name" token-bind="lead.name_role" class="shrink-0"/>
<tmt-text id="tag" bind="lead.tag" token="dim" class="shrink-0"/>
<tmt-text id="state" bind="lead.state" token-bind="lead.state_role" class="shrink-0"/>
<tmt-text id="squad" bind="lead.squad" token="muted" class="shrink-0" hide-below="md"/>
<tmt-text id="fill" class="grow h-1"/>
<tmt-repeat each="lead.chips" as="chip"><tmt-text id-bind="chip.id" bind="chip.text" token-bind="chip.role" class="shrink-0"/></tmt-repeat>
<tmt-text id="model" bind="lead.model" token-bind="lead.model_role" class="shrink-0"/>
<tmt-text id="age" bind="lead.age" token-bind="lead.age_role" class="shrink-0"/>
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
        (
            "top",
            list(object(vec![
                ("id", Schema::StableId),
                ("text", Schema::Scalar),
            ])),
        ),
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
                ("model_role", Schema::Scalar),
                ("separator", list(object(vec![("id", Schema::StableId)]))),
                ("mark", Schema::Scalar),
                ("mark_role", Schema::Scalar),
                ("name", Schema::Scalar),
                ("name_role", Schema::Scalar),
                ("squad", Schema::Scalar),
                ("age", Schema::Scalar),
                ("age_role", Schema::Scalar),
                (
                    "chips",
                    list(object(vec![
                        ("id", Schema::StableId),
                        ("text", Schema::Scalar),
                        ("role", Schema::Scalar),
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

/// What a squad row shows for its model and age, with the layout-neutral facts
/// the detail line needs.
struct Reading {
    model: String,
    model_visible: bool,
    model_carried: bool,
    age: String,
}

fn reading(stable: &mut Stable, row: &Value, now: u64) -> Reading {
    let full = row["fields"]["model"].as_str().unwrap_or_default();
    let name = crate::source::model_name(full);
    let model_visible = name == full
        && crate::board::row_detail::uncut(name, 8, tmt_tui::style::TextFlow::Truncate);
    let id = row["id"].as_str();
    let model = stable.model(id, fit(name, 8).trim_end(), now);
    let staleness = &row["staleness"];
    let state = staleness["state"].as_str();
    let since = matches!(state, Some("fresh" | "stale"))
        .then(|| {
            staleness["unchangedSinceMs"]
                .as_u64()
                .filter(|since| *since > 0 && *since <= now)
                .or_else(|| {
                    staleness["ageMs"]
                        .as_u64()
                        .map(|age| now.saturating_sub(age))
                })
        })
        .flatten();
    // A row the board did not observe, or could not (`unknown`), has no age to
    // report; the last one read for it stands in. `disabled` has none.
    let since = stable.since(id, since, matches!(state, None | Some("unknown")), now);
    Reading {
        model: model.value,
        model_visible,
        model_carried: model.carried,
        age: since.value.map_or_else(
            || "–".into(),
            |since| tmt_cli_style::value::relative_time(now.saturating_sub(since)),
        ),
    }
}

/// The box's four border strings. `Outline` owns the flat glyphs; the section
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
        .paint_flat(area, &mut buffer);
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
    for row in data["leads"].as_array_mut().into_iter().flatten() {
        if row.get("chips").is_none() {
            row["chips"] = json!([]);
        }
    }
    // A section whose head carries its own rule is already bounded above.
    let headed = key.data["head"]
        .as_array()
        .is_some_and(|head| !head.is_empty());
    data["top"] = if headed {
        json!([])
    } else {
        json!([{"id": "top", "text": chrome.top}])
    };
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
            let part = node
                .and_then(<[String]>::last)
                .map(String::as_str)
                .map(|part| {
                    if part.starts_with("chip-") {
                        "chip"
                    } else {
                        part
                    }
                });
            let line = node.and_then(|node| node.get(1)).map(String::as_str);
            let lead = scope.and_then(<[String]>::first).map(String::as_str);
            // Preserve the collapsed entity styling; expanded detail stays outside it.
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
                    look.role(role).add_modifier(Modifier::BOLD),
                    true,
                )),
                (Some("squad" | "state" | "tag" | "model" | "age" | "chip"), _) => {
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
    let mut leads: Vec<LeadSpan> = (0..reserving.len())
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
    let mut lines = painted.lines;
    let mut shift = 0;
    for row in &mut leads {
        row.start += shift;
        row.hit.start += shift;
        row.hit.end += shift;
        row.end += shift;
        if let Some(range) = &mut row.reserve {
            range.start += shift;
            range.end += shift;
        }
        let at = row.hit.end;
        let count = crate::board::row_detail::insert(
            &mut lines,
            at,
            &key.data["leads"][row.local]["detail"],
            key.width,
            3,
            look,
            selected == Some(id(row.local).as_str()),
            true,
        );
        row.hit.end += count;
        row.end += count;
        if let Some(range) = &mut row.reserve {
            range.start += count;
            range.end += count;
        }
        shift += count;
    }
    Block { lines, leads }
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
    let reserved =
        crate::board::view::waiting::reserved_lines(app, app.selected, inner).unwrap_or_default();
    let sent_row = app.sent.as_ref().and_then(|sent| {
        (0..app.rows().len()).find_map(|index| {
            let text = sent.line(&app.row_target(index)?)?;
            Some((index, text))
        })
    });
    let width = usize::from(inner.width);
    // Model and age come first: their cells are as wide as the tab has needed,
    // and the name and state layout below depends on that width alone.
    let readings: Vec<_> = {
        let mut stable = app.stable.borrow_mut();
        app.items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Row(_, row) => Some(reading(&mut stable, row, now)),
                Item::Section(_) | Item::Rule(_) => None,
            })
            .collect()
    };
    let narrow = width < NARROW;
    let model_cells = app.stable.borrow_mut().width(
        "model",
        readings
            .iter()
            .map(|reading| reading.model.width())
            .max()
            .unwrap_or(0),
    );
    let model_cells = if narrow { 0 } else { model_cells };
    let age_cells = if narrow { 0 } else { AGE_CELLS };
    let right = if model_cells == 0 { 0 } else { model_cells + 2 }
        + if age_cells == 0 { 0 } else { age_cells + 1 };
    let available = width.saturating_sub(3 + right);
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
        let waits = crate::attention::waits_on_you(row);
        let reported = row["state"].as_str().filter(|state| !state.is_empty());
        // Observed presence is TMT's, not the member's: it names a row that reports no state
        // and marks a reported one that is no longer online.
        let offline = row["presence"].as_str() == Some("offline");
        let (state, observed) = if waits {
            ("waits on you".to_owned(), false)
        } else {
            match (reported, row["presence"].as_str()) {
                (Some(state), Some("offline")) => (format!("{state} · offline"), false),
                (Some(state), _) => (state.to_owned(), false),
                (None, Some("active")) => ("online".to_owned(), true),
                (None, Some("offline")) => ("offline".to_owned(), true),
                (None, _) => ("–".to_owned(), false),
            }
        };
        let (mark, role) = if waits {
            ("◆", Role::Waiting)
        } else {
            match reported {
                Some("working") => ("●", Role::Working),
                Some("blocked") => ("✗", Role::Blocked),
                Some("review" | "testing") => ("◐", Role::Review),
                Some("idle") => ("◌", Role::Dim),
                _ => (" ", Role::Text),
            }
        };
        let reading = &readings[index];
        let chips = row_chips::padded(row_chips::pieces(&app.labels, row, now, available / 2));
        let chips_width = row_chips::pieces_width(&chips);
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
        // The name takes at most half the row, so the chips always have room beside it.
        let name_width = available
            .saturating_sub(tag.len() + 3)
            .min(26)
            .min(available / 2);
        let name = fit(&escape(row["name"].as_str().unwrap_or("–")), name_width);
        let state_width = available.saturating_sub(chips_width + name_width + tag.len() + 2);
        let mut after = vec![
            json!({"id":"task","text":format!("   {}",fit(&super::super::notes::sanitize(row["fields"]["task"].as_str().unwrap_or_default()),width.saturating_sub(3)).trim_end()),"role":"text"}),
        ];
        if let Some((_, text)) = sent_row.as_ref().filter(|(row, _)| *row == index) {
            after.push(json!({"id": "sent", "text": format!("   {text}"), "role": "working"}));
        }
        let reserve = if index == app.selected { reserved } else { 0 };
        after.extend(
            (0..reserve)
                .map(|line| json!({"id": format!("reserve-{line}"), "text": null, "role": null})),
        );
        for line in &mut before {
            line["text"] = json!(rule(line["text"].as_str().unwrap_or_default(), width));
        }
        let task =
            super::super::notes::sanitize(row["fields"]["task"].as_str().unwrap_or_default());
        let mut visible = Vec::new();
        if crate::board::row_detail::uncut(
            &task,
            width.saturating_sub(3),
            tmt_tui::style::TextFlow::Truncate,
        ) {
            visible.push("task");
        }
        if reading.model_visible {
            visible.push("model");
        }
        if row_chips::fitted(&app.labels, row, now, available / 2).1 {
            visible.push("chips");
        }
        rows.push(json!({"chips": chips, "id": id(index), "before": std::mem::take(&mut before), "separator": [],
            "mark": format!(" {mark} "), "mark_role": role.name(), "name": name, "tag": tag,
            "state": if state_width == 0 { String::new() } else { format!("  {}", fit(&escape(&state), state_width).trim_end()) },
            "state_role": if waits { "waiting" } else if observed { Role::Dim.name() } else { row["colors"]["state"].as_str().and_then(crate::look::role).unwrap_or(Role::Text).name() },
            "name_role": if offline { Role::Dim.name() } else { Role::Text.name() },
            "squad": "", "model": model_cell(&reading.model, model_cells),
            "model_role": if reading.model_carried { Role::Dim } else { Role::Muted }.name(),
            "age": age_cell(&reading.age, age_cells), "age_role": Role::Dim.name(), "after": after, "detail":app.detail_value(index,&visible)}));
    }
    // A trailing rule/section is still meaningful when search hides all members.
    // Attach it as an unselectable tail after the last row, before the box closes.
    if rows.is_empty() {
        before.push(json!({"id": "empty", "text": if app.search.is_empty() { "(no members)" } else { "(no matching members)" }, "role": "dim"}));
    }
    let tail = before.iter().enumerate().map(|(index, line)| json!({"id": format!("tail-{index}"), "text": rule(line["text"].as_str().unwrap_or_default(), usize::from(inner.width)), "role": "dim"})).collect::<Vec<_>>();
    let details: Vec<_> = rows.iter().map(|row| row["detail"].clone()).collect();
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
        crate::board::row_detail::more_hit(
            app,
            row.local,
            &details[row.local],
            inner.width,
            3,
            row.start + 2,
            area,
            offset,
            viewport,
        );
        for line in row.hit.start.max(offset)..row.hit.end.min(offset + viewport) {
            if inner.width > 0 {
                app.hits.borrow_mut().push(crate::board::app::Hit {
                    y: area.y + (line - offset) as u16,
                    x: inner.x,
                    width: inner.width,
                    target: crate::board::app::HitTarget::Row(row.local),
                });
            }
        }
    }
}
