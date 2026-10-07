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
            r#"<tmt-view version="1"><tmt-modal id="action-menu" title="{title}" placement="center" class="w-48 h-{}"><tmt-scroll id="body"><tmt-list id="choices" bind="$.rows" empty="(no actions)"><tmt-row class="flex-row gap-0"><tmt-cell bind="row.key" class="w-{key_width} shrink-0" token="accent"/><tmt-cell bind="row.cursor" class="w-1 shrink-0" token="text"/><tmt-cell bind="row.label" class="truncate"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#,
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
                picker_surface::schema(&["key", "cursor", "label"]),
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
                serde_json::json!({"id": row.id, "disabled": false, "key": entry.key, "cursor": if self.state.picker.list.selected() == Some(row.id.as_str()) { "›" } else { "" }, "label": entry.label})
            })
            .collect();
        let value = serde_json::json!({"rows": rows, "query": "", "footer": FOOTER, "status": "", "notes": []});
        self.state
            .render(FILE, &self.template, value, frame, look, body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::app::Choice;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn menu_cursor_preserves_every_key_cell_and_label_start() {
        let entries = vec![
            MenuEntry {
                key: "Ctrl-Shift-X".into(),
                label: "First long action label".into(),
                choice: Choice::Dismiss,
            },
            MenuEntry {
                key: "2".into(),
                label: "Second action".into(),
                choice: Choice::Dismiss,
            },
        ];
        for (variant, look) in picker_surface::evidence::looks().into_iter().enumerate() {
            for (width, height) in [(80, 30), (100, 30), (160, 30), (180, 30), (80, 8)] {
                let mut menu = MenuSurface::new("fixture actions", &entries);
                for selected in [0, 1] {
                    let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
                    screen
                        .draw(|frame| menu.render(&entries, selected, frame, look, frame.area()))
                        .unwrap();
                    let buffer = screen.backend().buffer();
                    for (index, entry) in entries.iter().enumerate() {
                        let row = picker_surface::evidence::row(&menu.state, &row_id(index));
                        if row.height == 0 {
                            continue;
                        }
                        for (offset, ch) in entry.key.chars().enumerate() {
                            assert_eq!(
                                buffer[(row.x + offset as u16, row.y)].symbol(),
                                ch.to_string()
                            );
                        }
                        assert_eq!(
                            buffer[(row.x + 12, row.y)].symbol(),
                            if index == selected { "›" } else { " " }
                        );
                        assert_eq!(
                            buffer[(row.x + 13, row.y)].symbol(),
                            if index == 0 { "F" } else { "S" }
                        );
                    }
                    picker_surface::evidence::capture(
                        &format!("menu-{width}x{height}-{variant}-{selected}"),
                        buffer,
                        &menu.state,
                    );
                }
            }
        }
    }
}
