//! ChipGroup: shared admitted options, selected padding and clipped hit IDs.
use super::app::{App, Overlay};
use ratatui::{buffer::Buffer, layout::Rect};
use serde_json::json;
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{
    binding::{Node, Schema, Schemas, Scopes, Sources, Template},
    geometry, text,
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
/// Shared option geometry delegates every actionable chip to the board button painter.
pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    groups: &[ChipGroup],
    app: &App,
    owner: Option<Overlay>,
) -> Vec<Hit> {
    use ratatui::text::Line;
    use tmt_tui::components::strip;
    let look = app.look();
    let choices: Vec<_> = groups.iter().flat_map(|group| &group.choices).collect();
    let node = node(groups);
    let cells = geometry::layout(&node, [area.width, area.height], text::measure)
        .expect("shared chip group geometry");
    let mut hits = Vec::new();
    for cell in cells {
        let Some(value) = cell.node.text.as_deref() else {
            continue;
        };
        let rect = cell.rect;
        let Ok(x) = u16::try_from(rect.x) else {
            continue;
        };
        let Ok(y) = u16::try_from(rect.y) else {
            continue;
        };
        let Ok(width) = u16::try_from(rect.width) else {
            continue;
        };
        let Ok(height) = u16::try_from(rect.height) else {
            continue;
        };
        let rect = Rect::new(
            area.x.saturating_add(x),
            area.y.saturating_add(y),
            width,
            height,
        )
        .intersection(area)
        .intersection(buffer.area);
        let choice = cell
            .node
            .id
            .as_ref()
            .and_then(|id| id.last())
            .and_then(|id| choices.iter().find(|choice| choice.id == *id));
        if let Some(choice) = choice.filter(|choice| choice.actionable) {
            if cell.cut {
                continue;
            }
            let label = value
                .strip_prefix(' ')
                .and_then(|value| value.strip_suffix(' '))
                .unwrap_or(value);
            let button = super::buttons::Button::secondary(label)
                .focused(choice.selected || choice.cursor)
                .enabled(!choice.disabled)
                .idle(look.role(Role::Muted));
            if let Some(rect) =
                super::buttons::paint(app, buffer, rect, &[button], Align::Left, owner)[0]
                && !choice.disabled
            {
                hits.push(Hit {
                    area: rect,
                    id: choice.id.clone(),
                });
            }
        } else {
            strip::paint_left(buffer, rect, Line::styled(value, look.role(Role::Muted)));
        }
    }
    hits
}
