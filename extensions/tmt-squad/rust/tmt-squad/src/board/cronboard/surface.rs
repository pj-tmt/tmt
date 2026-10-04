//! Component state and painting for job rows: the squad tab's jobs half (an
//! ordinary list pane) and the `c` list (a modal list). Both bind the same row
//! projection and keep selection and scroll in caller-owned `ListState`.

use super::rows::{Columns, schema_rows};
use crate::{board::picker_surface, look::Look};
use ratatui::{Frame, crossterm::event::Event, layout::Rect};
use serde_json::{Value, json};
use tmt_tui::{
    binding::Schema,
    components::{
        ListEvent, ListFrame, ListState, Table, collection,
        surface::{self, RenderStyle},
    },
};

const FILE: &str = "squad.cron.xml";

fn schema() -> Schema {
    let mut schema = picker_surface::schema(&[]);
    let Schema::Object(root) = &mut schema else {
        unreachable!("picker schema is an object")
    };
    root.insert("rows".into(), schema_rows());
    schema
}

/// The jobs half: an ordinary list pane below the squad's members.
#[derive(Default)]
pub(in crate::board) struct Pane {
    pub list: ListState,
    frame: Option<ListFrame>,
    table: Option<(Columns, &'static str, Table<()>)>,
}

impl Pane {
    pub fn render(
        &mut self,
        rows: Vec<Value>,
        columns: Columns,
        empty: &'static str,
        frame: &mut Frame,
        look: Look,
        area: Rect,
    ) {
        if self
            .table
            .as_ref()
            .is_none_or(|(old, note, _)| *old != columns || *note != empty)
        {
            let markup = format!(
                r#"<tmt-view version="1"><tmt-list id="jobs" bind="$.rows" empty="{empty}">{}</tmt-list></tmt-view>"#,
                columns.markup()
            );
            self.table = Some((
                columns,
                empty,
                picker_surface::collection(FILE, &markup, schema()),
            ));
        }
        let (_, _, table) = self.table.as_ref().expect("compiled");
        let scene = match table.materialize(FILE, &json!({"rows": rows}), &picker_surface::Data) {
            Ok(scene) => scene,
            Err(error) => {
                // A projection the markup refuses must not take the board down.
                self.frame = None;
                let role = look.role(tmt_cli_style::Role::Blocked);
                frame.buffer_mut().set_stringn(
                    area.x,
                    area.y,
                    format!("✗ cron rows unavailable: {}", error.message),
                    usize::from(area.width),
                    role,
                );
                return;
            }
        };
        self.frame = collection::render(
            &scene,
            &mut self.list,
            area,
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
        .ok();
    }

    /// Stale geometry never activates: a resize or new model discards it.
    pub fn invalidate(&mut self) {
        self.frame = None;
    }

    pub fn input(&mut self, event: &Event) -> Option<ListEvent> {
        let frame = self.frame.clone()?;
        self.list.input(event, &frame)
    }

    pub fn viewport(&self) -> Option<Rect> {
        self.frame.as_ref().map(|frame| frame.viewport)
    }
}

/// The `c` list's modal template for these columns.
pub(in crate::board) fn modal(columns: Columns) -> surface::Template<()> {
    let markup = format!(
        r#"<tmt-view version="1"><tmt-modal id="cron-list" title="cron · all squads" placement="body"><tmt-scroll id="body"><tmt-list id="choices" bind="$.rows" empty="(no jobs · n new)">{}</tmt-list></tmt-scroll><tmt-text slot="status" bind="$.status" token="muted" class="truncate"/><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#,
        columns.markup()
    );
    picker_surface::compile(FILE, &markup, schema())
}
