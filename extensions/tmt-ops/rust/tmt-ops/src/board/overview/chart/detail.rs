//! Read-only interval evidence captured from the chart at inspection time.
//! The shared modal and scroll components own geometry and event behavior.
use crate::board::app::App;
use ratatui::{Frame, crossterm::event::Event, layout::Rect};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_tui::{
    binding::{Schema, Schemas, Scopes, Sources},
    components::{
        ScrollState,
        surface::{self, ModalSurface, RenderStyle},
    },
};

const FILE: &str = "ops.usage-details.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="usage-details" title="Usage · exact interval" placement="body"><tmt-scroll id="usage-body"><tmt-repeat each="$.lines" as="line"><tmt-text bind="line.text" token="text" wrap="true"/></tmt-repeat></tmt-scroll><tmt-text slot="footer" token="muted" bind="$.footer"/></tmt-modal></tmt-view>"#;
struct Data;
impl Sources for Data {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("usage details have no field sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("usage details have no field sources".into())
    }
}
fn template() -> &'static surface::Template<()> {
    static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let parsed = tmt_tui::parse(FILE, MARKUP).expect("embedded usage details markup");
        let schema = Schema::Object(BTreeMap::from([
            (
                "lines".into(),
                Schema::Collection(Box::new(Schema::Object(BTreeMap::from([(
                    "text".into(),
                    Schema::Scalar,
                )])))),
            ),
            ("footer".into(), Schema::Scalar),
        ]));
        surface::compile(FILE, &parsed, &schema, &Data).expect("embedded usage details schema")
    })
}
#[derive(Default)]
pub(in crate::board) struct Details {
    pub(super) lines: Vec<String>,
    pub(in crate::board) scroll: ScrollState,
    scene: Option<(Value, ModalSurface)>,
}
impl Details {
    pub fn new(lines: Vec<String>) -> Self {
        Self {
            lines,
            ..Self::default()
        }
    }
    pub fn input(&mut self, event: &Event) {
        if self.scroll.viewport().height > 0 {
            self.scroll.input(event);
        }
    }
    pub fn invalidate(&mut self) {
        self.scroll
            .update(Rect::default(), self.scroll.content(), None);
    }
    pub fn render(&mut self, frame: &mut Frame, app: &App, body: Rect) {
        let value = json!({"lines": self.lines.iter().map(|text| json!({"text":text})).collect::<Vec<_>>(), "footer":"↑↓ scroll · PgUp/PgDn page · Esc close"});
        if self.scene.as_ref().is_none_or(|(old, _)| old != &value) {
            self.scene = Some((
                value.clone(),
                template()
                    .materialize(FILE, &value, &Data)
                    .expect("typed usage details"),
            ));
        }
        let look = app.look();
        surface::render(
            &self.scene.as_ref().unwrap().1,
            &mut self.scroll,
            body,
            frame.buffer_mut(),
            RenderStyle {
                theme: &look.theme,
                depth: look.depth,
            },
            |role| look.role(role),
        )
        .expect("admitted usage details surface");
    }
}
