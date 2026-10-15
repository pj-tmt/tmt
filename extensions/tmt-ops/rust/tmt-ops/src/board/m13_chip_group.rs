//! M13 ChipGroup: shared admitted options, selected padding and clipped hit IDs.
use crate::look::Look;
use ratatui::{buffer::Buffer, layout::Rect};
use serde_json::json;
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{
    binding::{Node, Schema, Schemas, Scopes, Sources, Template},
    geometry, paint, text,
};
const FILE: &str = "ops.board.chip-group.xml";
const MARKUP: &str = r#"<tmt-view version="1" class="w-full"><tmt-repeat each="$.rows" as="row"><tmt-row class="h-1 w-full"><tmt-text bind="row.label" class="shrink-0"/><tmt-repeat each="row.options" as="option"><tmt-text id-bind="option.id" bind="option.text" class="shrink-0"/><tmt-text bind="option.gap" class="shrink-0"/></tmt-repeat></tmt-row></tmt-repeat></tmt-view>"#;
struct Data;
impl Sources for Data {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("controls use bound values only".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("controls use bound values only".into())
    }
}

#[derive(Clone)]
pub(in crate::board) struct Choice {
    pub id: String,
    pub actionable: bool,
    pub text: String,
    pub gap: String,
    pub selected: bool,
    pub disabled: bool,
    pub cursor: bool,
}
pub(in crate::board) struct ChipGroup {
    pub label: String,
    pub choices: Vec<Choice>,
}
pub(in crate::board) struct Hit {
    pub area: Rect,
    pub id: String,
}
fn node(groups: &[ChipGroup]) -> Node {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    let template = TEMPLATE.get_or_init(|| {
        let option = Schema::Object(BTreeMap::from([
            ("id".into(), Schema::StableId),
            ("text".into(), Schema::Scalar),
            ("gap".into(), Schema::Scalar),
        ]));
        let row = Schema::Object(BTreeMap::from([
            ("label".into(), Schema::Scalar),
            ("options".into(), Schema::Collection(Box::new(option))),
        ]));
        let schema = Schema::Object(BTreeMap::from([(
            "rows".into(),
            Schema::Collection(Box::new(row)),
        )]));
        let parsed = tmt_tui::parse(FILE, MARKUP).expect("embedded usage controls");
        tmt_tui::binding::compile(FILE, &parsed, &schema, &Data).expect("typed usage controls")
    });
    let rows:Vec<_>=groups.iter().map(|group|json!({"label":group.label,"options":group.choices.iter().map(|choice|json!({"id":choice.id,"text":choice.text,"gap":choice.gap})).collect::<Vec<_>>()})).collect();
    template
        .materialize(FILE, &json!({"rows":rows}), &Data)
        .expect("admitted shared chip group")
}
pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    groups: &[ChipGroup],
    look: Look,
) -> Vec<Hit> {
    let choices: Vec<_> = groups
        .iter()
        .flat_map(|group| group.choices.iter())
        .collect();
    let node = node(groups);
    let mut cells = geometry::layout(&node, [area.width, area.height], text::measure)
        .expect("usage control geometry");
    for cell in &mut cells {
        for rect in [&mut cell.rect, &mut cell.content, &mut cell.clip] {
            rect.x += i32::from(area.x);
            rect.y += i32::from(area.y);
        }
    }
    let hits = paint::paint_with(&cells, buffer, |index, _, _| {
        let id = cells[index].node.id.as_ref().and_then(|path| path.last());
        let choice = id.and_then(|id| choices.iter().find(|choice| choice.id == *id));
        let (selected, disabled, cursor) = choice.map_or((false, false, false), |choice| {
            (choice.selected, choice.disabled, choice.cursor)
        });
        let mut style = look.role(if disabled { Role::Dim } else { Role::Muted });
        if selected {
            style = style.add_modifier(ratatui::style::Modifier::REVERSED);
        }
        if cursor {
            style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
        }
        (style, Align::Left)
    });
    hits.into_iter()
        .filter_map(|hit| {
            let id = hit.id?.last()?.clone();
            choices
                .iter()
                .any(|choice| choice.id == id && choice.actionable)
                .then_some(Hit {
                    area: Rect::new(
                        hit.rect.x.try_into().ok()?,
                        hit.rect.y.try_into().ok()?,
                        hit.rect.width.try_into().ok()?,
                        hit.rect.height.try_into().ok()?,
                    ),
                    id,
                })
        })
        .collect()
}
