//! The leads section: a rule, then one boxed list with a heading line per lead and,
//! while replies show, its latest exchange. The HOME stream owns placement,
//! scrolling and hits; this section reports where each lead landed.
use super::{
    controller::HomeEntry,
    paint::rule,
    scene::{self, Part},
};
use crate::{
    board::{
        app::{App, Compose, RowTarget},
        home_leads::{Kind, Lead},
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
use serde_json::{Value, json};
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
<tmt-repeat each="lead.separator" as="sep"><tmt-row id="separator" class="w-full h-1"><tmt-text id="left" bind="$.left" class="shrink-0"/><tmt-text id="fill" class="grow h-1"/><tmt-text id="right" bind="$.right" class="shrink-0"/></tmt-row></tmt-repeat>
<tmt-row id="head" class="w-full h-1">
<tmt-text id="left" bind="$.left" class="shrink-0"/>
<tmt-text id="mark" bind="lead.mark" token-bind="lead.mark_role" class="shrink-0"/>
<tmt-text id="name" bind="lead.name" token="text" class="shrink-0"/>
<tmt-text id="squad" bind="lead.squad" token="muted" class="shrink-0" hide-below="md"/>
<tmt-text id="fill" class="grow h-1"/>
<tmt-text id="age" bind="lead.age" token="dim" class="shrink-0"/>
<tmt-text id="right" bind="$.right" class="shrink-0"/>
</tmt-row>
<tmt-repeat each="lead.after" as="line"><tmt-row id-bind="line.id" class="w-full h-1"><tmt-text id="left" bind="$.left" class="shrink-0"/><tmt-text id="text" bind="line.text" token-bind="line.role" class="shrink-0"/><tmt-text id="fill" class="grow h-1"/><tmt-text id="right" bind="$.right" class="shrink-0"/></tmt-row></tmt-repeat>
</tmt-col></tmt-repeat>
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
        ("top", Schema::Scalar),
        ("bottom", Schema::Scalar),
        ("left", Schema::Scalar),
        ("right", Schema::Scalar),
        (
            "leads",
            list(object(vec![
                ("id", Schema::StableId),
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
pub(super) struct Block {
    pub lines: Vec<Line<'static>>,
    pub leads: Vec<LeadSpan>,
}

pub(super) struct LeadSpan {
    /// The entry's ordinal in `App::home_entries`.
    pub index: usize,
    /// The heading line.
    pub start: usize,
    /// The heading and the preview under it: the lead's hits.
    pub hit: Range<usize>,
    /// Everything the lead owns through its reserved lines.
    pub end: usize,
    pub reserve: Option<Range<usize>>,
}

pub(super) struct Section<'a> {
    pub first: usize,
    pub entries: &'a [HomeEntry<'a>],
    pub leads: &'a [&'a Lead],
    pub replies: bool,
}

fn id(local: usize) -> String {
    format!("lead-{local}")
}

/// The one line under a heading: the latest exchange, or why there is none.
fn preview(lead: &Lead, width: u16) -> Value {
    let (text, role) = lead.exchange.as_ref().map_or_else(
        || {
            (
                if lead.failure.is_some() {
                    "(exchange unavailable)"
                } else {
                    "–"
                }
                .into(),
                Role::Dim,
            )
        },
        |exchange| match exchange.kind {
            Kind::Question => (format!("asks: {}", exchange.preview), Role::Waiting),
            Kind::Asked => (format!("no reply yet to: {}", exchange.preview), Role::Dim),
            Kind::Reply => (exchange.preview.clone(), Role::Text),
        },
    );
    json!({
        "id": "preview",
        "text": format!(
            "  {}",
            fit(&text, usize::from(width.saturating_sub(5))).trim_end()
        ),
        "role": role.name(),
    })
}

fn heading(lead: &Lead, width: u16, now: u64) -> Value {
    let (mark, role) = match lead.exchange.as_ref().map(|exchange| exchange.kind) {
        Some(Kind::Question) => ("◆", Role::Waiting),
        Some(Kind::Asked) => ("…", Role::Dim),
        Some(Kind::Reply) => ("✓", Role::Working),
        None => (" ", Role::Dim),
    };
    let inner = usize::from(width.saturating_sub(2));
    let age = lead
        .exchange
        .as_ref()
        .and_then(|exchange| exchange.since_ms)
        .map_or_else(|| "–".into(), |at| crate::requests::age(now, at));
    let age = fit(&age, inner.saturating_sub(4)).trim_end().to_owned();
    let available = inner.saturating_sub(4 + unicode_width::UnicodeWidthStr::width(age.as_str()));
    let name_width = available.min(24);
    // Whether the squad column shows at all is the `md` step of the markup;
    // whether any room is left for it is a fit.
    let squad = if available > name_width + 3 {
        format!(
            "   {}",
            fit(&escape(&lead.squad), available - name_width - 3).trim_end()
        )
    } else {
        String::new()
    };
    json!({
        "mark": format!(" {mark} "),
        "mark_role": role.name(),
        "name": fit(&escape(lead.name()), name_width),
        "squad": squad,
        "age": format!("{age} "),
    })
}

pub(super) fn paint(app: &App, look: Look, area: Rect, now: u64, section: &Section<'_>) -> Block {
    let chrome = Chrome::new(area.width, look);
    let inner = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(2),
        ..area
    };
    let mut previous_exchange = false;
    let mut reserving = Vec::new();
    let rows =
        section
            .leads
            .iter()
            .enumerate()
            .map(|(local, lead)| {
                let index = section.first + local;
                let target = RowTarget::Home(section.entries[local].target.clone());
                let reading = app.input.as_ref().is_some_and(|input| {
                    matches!(input.compose, Compose::ReadLead { .. })
                        && input
                            .row_send
                            .as_ref()
                            .is_some_and(|send| send.target == target)
                });
                // One blank boxed line keeps a lead with an exchange apart from the
                // next lead, whether that one has an exchange or not.
                let separator =
                    local > 0 && section.replies && previous_exchange && lead.exchange.is_some();
                previous_exchange = lead.exchange.is_some();
                let mut after = Vec::new();
                if section.replies && lead.exchange.is_some() && !reading {
                    after.push(preview(lead, area.width));
                }
                if app
                    .sent
                    .as_ref()
                    .is_some_and(|feedback| feedback.target == target)
                {
                    after.push(json!({"id": "sent", "text": "  ✓ sent", "role": "working"}));
                }
                let reserved = crate::board::view::waiting::reserved_lines(app, index, inner)
                    .unwrap_or_default();
                reserving.push(reserved);
                after.extend((0..reserved).map(
                    |line| json!({"id": format!("reserve-{line}"), "text": null, "role": null}),
                ));
                let mut row = heading(lead, area.width, now);
                row["id"] = json!(id(local));
                row["separator"] = json!(if separator {
                    vec![json!({"id": "gap"})]
                } else {
                    Vec::new()
                });
                row["after"] = json!(after);
                row
            })
            .collect::<Vec<_>>();
    let selected = app
        .selected
        .checked_sub(section.first)
        .filter(|local| *local < section.leads.len())
        .map(id);
    let painted = scene::paint(
        FILE,
        template(),
        &json!({
            "head": [
                {"id": "gap", "text": null, "role": null},
                {
                    "id": "rule",
                    "text": rule(
                        &format!(
                            "leads · latest from each · t {} replies",
                            if section.replies { "hides" } else { "shows" }
                        ),
                        usize::from(area.width),
                    ),
                    "role": Role::Muted.name(),
                },
            ],
            "top": chrome.top,
            "bottom": chrome.bottom,
            "left": chrome.left,
            "right": chrome.right,
            "leads": rows,
        }),
        area.width,
        &mut |Part {
                  id: node,
                  scope,
                  role,
              }| {
            let part = node.and_then(<[String]>::last).map(String::as_str);
            let line = node.and_then(|node| node.get(1)).map(String::as_str);
            let lead = scope.and_then(<[String]>::first).map(String::as_str);
            // Only a heading is ever selected; the lines under it keep their own style.
            let selected =
                selected.as_deref().is_some_and(|id| Some(id) == lead) && line == Some("head");
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
                (Some("squad" | "age"), _) => {
                    boxed(look.row_span(selected, look.role(role), false))
                }
                (Some("fill"), _) if selected => look.selection(),
                (Some("fill"), _) => look.role(Role::Text),
                (Some("text"), Some("sent")) => look.role(Role::Working),
                (Some("text"), _) => look.role(role),
                _ => Style::new(),
            };
            (style, Align::Left)
        },
    );
    let leads = (0..section.leads.len())
        .map(|local| {
            let lead = painted.lines(&[&id(local)]).expect("every lead is a block");
            let head = painted
                .lines(&[&id(local), "head"])
                .expect("every lead has a heading");
            let preview = painted.lines(&[&id(local), "preview"]);
            LeadSpan {
                index: section.first + local,
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
