//! The Timers modal template. The list controller owns selection and scroll;
//! shared row projection supplies the columns and expanded detail.

use super::rows::{Columns, schema_rows};
use crate::board::picker_surface;
use tmt_tui::{binding::Schema, components::surface};

const FILE: &str = "squad.cron.xml";

fn schema() -> Schema {
    let mut schema = picker_surface::schema(&[]);
    let Schema::Object(root) = &mut schema else {
        unreachable!("picker schema is an object")
    };
    root.insert("rows".into(), schema_rows());
    schema
}

/// The `c` list's modal template for these columns.
///
/// Docked like #1607's prompt band: the modal takes `height` lines (its chrome plus
/// one per job), never a full-body sheet over a short list.
pub(in crate::board) fn modal(
    columns: Columns,
    width: u16,
    height: u16,
    title: &str,
) -> surface::Template<()> {
    let title = tmt_cli_style::table::escape(title)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;");
    let markup = format!(
        r#"<tmt-view version="1"><tmt-modal id="cron-list" title="{title}" placement="docked" class="w-{width} h-{height}"><tmt-scroll id="body"><tmt-list id="choices" bind="$.rows" empty="(no jobs · n new)">{}</tmt-list></tmt-scroll><tmt-text slot="status" bind="$.status" token="muted" class="truncate"/><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#,
        columns.markup()
    );
    picker_surface::compile(FILE, &markup, schema())
}
