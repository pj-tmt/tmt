//! Shared component state for Squad's settings, preview pickers and tab filter.
//! Controllers own projection, previews and persistence; this owner retains
//! only the component cursor, query, scene and currently painted hit map.
use crate::look::Look;
use ratatui::{Frame, crossterm::event::Event, layout::Rect};
use serde_json::Value;
use std::collections::BTreeMap;
use tmt_tui::{
    binding::{Schema, Schemas, Scopes, Sources},
    components::{
        ListFrame, ListRow, Picker, PickerField, PickerInput, RowGeometry, collection,
        surface::{self, FrameMap, ModalSurface, RenderStyle},
    },
};

pub(super) struct Data;
impl Sources for Data {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("picker projections have no field sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("picker projections have no field sources".into())
    }
}
pub(super) fn schema(fields: &[&str]) -> Schema {
    let mut row = BTreeMap::from([
        ("id".into(), Schema::StableId),
        ("disabled".into(), Schema::Boolean),
    ]);
    row.extend(fields.iter().map(|field| ((*field).into(), Schema::Scalar)));
    Schema::Object(BTreeMap::from([
        (
            "rows".into(),
            Schema::Collection(Box::new(Schema::Object(row))),
        ),
        ("query".into(), Schema::Scalar),
        ("footer".into(), Schema::Scalar),
        ("status".into(), Schema::Scalar),
        (
            "notes".into(),
            Schema::Collection(Box::new(Schema::Object(BTreeMap::from([
                ("id".into(), Schema::StableId),
                ("text".into(), Schema::Scalar),
            ])))),
        ),
    ]))
}
pub(super) fn compile(file: &str, markup: &str, schema: Schema) -> surface::Template<()> {
    let parsed = tmt_tui::parse(file, markup).expect("embedded picker markup");
    surface::compile(file, &parsed, &schema, &Data).expect("embedded picker schema")
}

pub(super) fn collection(file: &str, markup: &str, schema: Schema) -> collection::Table<()> {
    let parsed = tmt_tui::parse(file, markup).expect("embedded list markup");
    collection::compile(file, &parsed, &schema, &Data).expect("embedded list schema")
}

pub(super) struct State {
    pub picker: Picker,
    pub frame: Option<FrameMap>,
    scene: Option<(Value, ModalSurface)>,
}
impl State {
    pub fn new(query: Option<String>, rows: Vec<ListRow>, selected: Option<&str>) -> Self {
        let mut picker = Picker::new(query).expect("bounded initial query");
        picker
            .reconcile(rows)
            .expect("projected unique row identities");
        if let Some(id) = selected {
            picker.list.select(id);
        }
        Self {
            picker,
            frame: None,
            scene: None,
        }
    }
    pub fn reconcile(&mut self, rows: Vec<ListRow>) {
        if self.picker.list.rows() != rows {
            self.frame = None;
        }
        self.picker
            .reconcile(rows)
            .expect("projected unique row identities");
    }
    pub fn select(&mut self, id: &str) {
        self.picker.list.select(id);
        self.frame = None;
    }
    pub fn invalidate(&mut self) {
        self.frame = None;
    }
    pub fn input(&mut self, event: &Event, field: PickerField) -> Option<PickerInput> {
        let frame = match &self.frame {
            Some(frame) => frame.list.clone()?,
            // Keys may arrive before the first paint, notably in controller
            // tests. Only record navigation is available; never fabricate hits.
            None if matches!(event, Event::Key(_) | Event::Paste(_)) => ListFrame {
                rows: self.picker.list.rows().to_vec(),
                geometry: self
                    .picker
                    .list
                    .rows()
                    .iter()
                    .enumerate()
                    .map(|(index, row)| RowGeometry {
                        id: row.id.clone(),
                        lines: index..index + 1,
                        visible: Rect::default(),
                    })
                    .collect(),
                viewport: self.picker.list.scroll.viewport(),
                offset: self.picker.list.scroll.offset(),
            },
            None => return None,
        };
        self.picker.input_field(event, &frame, field)
    }
    pub fn render(
        &mut self,
        file: &str,
        template: &surface::Template<()>,
        value: Value,
        frame: &mut Frame,
        look: Look,
        body: Rect,
    ) {
        self.prepare(file, template, value);
        self.frame = Some(
            surface::render_list(
                &self.scene.as_ref().unwrap().1,
                &mut self.picker.list,
                body,
                frame.buffer_mut(),
                RenderStyle {
                    theme: &look.theme,
                    depth: look.depth,
                },
                |role| {
                    look.selection()
                        .patch(look.row_span(true, look.role(role), false))
                },
            )
            .expect("admitted picker surface"),
        );
    }
    pub fn render_modal(
        &mut self,
        file: &str,
        template: &surface::Template<()>,
        value: Value,
        frame: &mut Frame,
        look: Look,
        body: Rect,
    ) {
        self.prepare(file, template, value);
        self.frame = Some(
            surface::render(
                &self.scene.as_ref().unwrap().1,
                &mut self.picker.list.scroll,
                body,
                frame.buffer_mut(),
                RenderStyle {
                    theme: &look.theme,
                    depth: look.depth,
                },
                |role| look.role(role),
            )
            .expect("admitted modal surface"),
        );
    }
    fn prepare(&mut self, file: &str, template: &surface::Template<()>, value: Value) {
        if self.scene.as_ref().is_none_or(|(old, _)| old != &value) {
            self.frame = None;
            self.scene = Some((
                value.clone(),
                template
                    .materialize(file, &value, &Data)
                    .expect("typed picker projection"),
            ));
        }
    }
}

#[cfg(test)]
pub(super) mod evidence {
    use super::*;
    use ratatui::buffer::Buffer;
    use serde_json::json;
    use tmt_cli_style::{Base, Depth, Theme};

    pub fn looks() -> [Look; 4] {
        [
            (Base::Tmt, Depth::TrueColor),
            (Base::TmtLight, Depth::TrueColor),
            (Base::Terminal, Depth::Ansi16),
            (Base::Tmt, Depth::None),
        ]
        .map(|(base, depth)| Look {
            theme: Theme::new(base),
            depth,
        })
    }

    /// Optional task-owned evidence; normal regression runs write nothing.
    pub fn capture(name: &str, buffer: &Buffer, state: &State) {
        let Ok(directory) = std::env::var("TMT_OVERLAY_CUE_OUTPUT") else {
            return;
        };
        let map = state.frame.as_ref().expect("painted surface");
        let value = json!({
            "width": buffer.area.width, "height": buffer.area.height,
            "cells": buffer.content.iter().map(|cell| json!({"symbol":cell.symbol(),
                "fg":format!("{:?}", cell.fg), "bg":format!("{:?}", cell.bg),
                "modifier":format!("{:?}", cell.modifier)})).collect::<Vec<_>>(),
            "hits": map.hits.iter().map(|hit| json!({"id":hit.id,
                "row":hit.row_id,"rect":[hit.rect.x,hit.rect.y,hit.rect.width,hit.rect.height]})).collect::<Vec<_>>(),
            "list":format!("{:?}",map.list), "selected":state.picker.list.selected(),
            "query":state.picker.query(),
        });
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }

    pub fn row(state: &State, id: &str) -> Rect {
        state
            .frame
            .as_ref()
            .unwrap()
            .list
            .as_ref()
            .unwrap()
            .geometry
            .iter()
            .find(|row| row.id == id)
            .expect("stable row geometry")
            .visible
    }
}
