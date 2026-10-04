//! The action menu's surface: one modal list over the board. A menu's title is
//! the member, request or job it is about, so its template is compiled once per
//! menu rather than once per process; its entries never change while it is open.
use super::{app::MenuEntry, picker_surface};
use crate::look::Look;
use ratatui::{Frame, layout::Rect};
use tmt_cli_style::table::escape;
use tmt_tui::components::{ListRow, surface};
use unicode_width::UnicodeWidthStr;

const FILE: &str = "squad.menu.xml";
const FOOTER: &str = "Enter runs · Esc closes";
/// Rows beyond the entries: two borders, the position line and the footer.
const CHROME: usize = 4;

pub(in crate::board) struct MenuSurface {
    template: surface::Template<()>,
    rows: Vec<ListRow>,
    state: picker_surface::State,
}

fn row_id(index: usize) -> String {
    // Entry keys may repeat or be numeric; the menu never reorders, so its own
    // position names a row for the whole life of the surface.
    format!("entry-{index}")
}

impl MenuSurface {
    pub fn new(title: &str, entries: &[MenuEntry]) -> Self {
        let key_width = entries
            .iter()
            .map(|entry| entry.key.width())
            .max()
            .unwrap_or(1);
        let title = escape(title)
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;");
        let title = if title.trim().is_empty() {
            "actions"
        } else {
            &title
        };
        let markup = format!(
            r#"<tmt-view version="1"><tmt-modal id="action-menu" title="{title}" placement="center" class="w-48 h-{}"><tmt-scroll id="body"><tmt-list id="choices" bind="$.rows" empty="(no actions)"><tmt-row class="flex-row gap-1"><tmt-cell bind="row.key" class="w-{key_width} shrink-0" token="accent"/><tmt-cell bind="row.label" class="truncate"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#,
            entries.len() + CHROME
        );
        let rows: Vec<ListRow> = (0..entries.len())
            .map(|index| ListRow {
                id: row_id(index),
                disabled: false,
            })
            .collect();
        Self {
            template: picker_surface::compile(
                FILE,
                &markup,
                picker_surface::schema(&["key", "label"]),
            ),
            state: picker_surface::State::new(None, rows.clone(), None),
            rows,
        }
    }

    pub fn render(
        &mut self,
        entries: &[MenuEntry],
        selected: usize,
        frame: &mut Frame,
        look: Look,
        body: Rect,
    ) {
        let id = row_id(selected);
        if self.state.picker.list.selected() != Some(id.as_str()) {
            self.state.select(&id);
        }
        let rows: Vec<_> = entries
            .iter()
            .zip(&self.rows)
            .map(|(entry, row)| {
                serde_json::json!({"id": row.id, "disabled": false, "key": entry.key, "label": entry.label})
            })
            .collect();
        let value = serde_json::json!({"rows": rows, "query": "", "footer": FOOTER, "status": "", "notes": []});
        self.state
            .render(FILE, &self.template, value, frame, look, body);
    }
}
