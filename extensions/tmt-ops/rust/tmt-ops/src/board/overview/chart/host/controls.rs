//! Direct Each/Range options: input rectangles come from admitted/clipped nodes.
use super::super::model::Selection;
use super::{Focus, Hit, Target};
use crate::look::Look;
use ratatui::{buffer::Buffer, layout::Rect};
use serde_json::json;
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{
    binding::{Node, Schema, Schemas, Scopes, Sources, Template},
    geometry, paint, text,
};
const FILE: &str = "ops.usage.controls.xml";
const MARKUP: &str = r#"<tmt-view version="1" class="w-full"><tmt-repeat each="$.rows" as="row"><tmt-row class="h-1 w-full"><tmt-text bind="row.label" class="shrink-0"/><tmt-repeat each="row.options" as="option"><tmt-text id-bind="option.id" bind="option.text" class="shrink-0"/></tmt-repeat></tmt-row></tmt-repeat></tmt-view>"#;
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
fn node(selection: Selection) -> Node {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    let template = TEMPLATE.get_or_init(|| {
        let option = Schema::Object(BTreeMap::from([
            ("id".into(), Schema::StableId),
            ("text".into(), Schema::Scalar),
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
    let rows: Vec<_> = [("Each  ", "each", Selection::EACH.as_slice(), selection.each_minutes), ("Range ", "range", Selection::RANGE.as_slice(), selection.range_minutes)].into_iter().map(|(label, prefix, values, selected)| {
        let options: Vec<_> = values.iter().map(|value| json!({"id":format!("{prefix}-{value}"), "text": if *value == selected { format!("[{value:>2}m] ") } else { format!(" {value:>2}m  ") }})).collect();
        json!({"label":label,"options":options})
    }).collect();
    template
        .materialize(FILE, &json!({"rows":rows}), &Data)
        .expect("admitted usage controls")
}
pub(super) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    selection: Selection,
    focus: Option<Focus>,
    draft: usize,
    look: Look,
) -> Vec<Hit> {
    let node = node(selection);
    let mut cells = geometry::layout(&node, [area.width, area.height], text::measure)
        .expect("usage control geometry");
    for cell in &mut cells {
        for rect in [&mut cell.rect, &mut cell.content, &mut cell.clip] {
            rect.x += i32::from(area.x);
            rect.y += i32::from(area.y);
        }
    }
    let target = |id: &str| {
        let (prefix, value) = id.split_once('-')?;
        let value = value.parse().ok()?;
        match prefix {
            "each" => Some(Target::Each(value)),
            "range" => Some(Target::Range(value)),
            _ => None,
        }
    };
    let hits = paint::paint_with(&cells, buffer, |index, _, _| {
        let choice = cells[index]
            .node
            .id
            .as_ref()
            .and_then(|id| id.last())
            .and_then(|id| target(id));
        let (selected, disabled, cursor) = match choice {
            Some(Target::Each(v)) => (
                v == selection.each_minutes,
                v > selection.range_minutes,
                focus == Some(Focus::Each) && Selection::EACH[draft] == v,
            ),
            Some(Target::Range(v)) => (
                v == selection.range_minutes,
                v < selection.each_minutes,
                focus == Some(Focus::Range) && Selection::RANGE[draft] == v,
            ),
            _ => (false, false, false),
        };
        let style = if cursor {
            look.selection()
        } else {
            look.role(if disabled {
                Role::Dim
            } else if selected {
                Role::Accent
            } else {
                Role::Muted
            })
        };
        (style, Align::Left)
    });
    hits.into_iter()
        .filter_map(|hit| {
            let target = target(hit.id?.last()?)?;
            Some(Hit {
                area: Rect::new(
                    hit.rect.x.try_into().ok()?,
                    hit.rect.y.try_into().ok()?,
                    hit.rect.width.try_into().ok()?,
                    hit.rect.height.try_into().ok()?,
                ),
                target,
            })
        })
        .collect()
}
