//! The action menu's surface: one modal list over the board. A menu's title is
//! the member, request or job it is about, so its template is compiled once per
//! menu rather than once per process; its entries never change while it is open.
use super::{
    app::{Choice, MenuEntry},
    picker_surface,
    workspace::manage::Kind,
};
use crate::look::Look;
use ratatui::{
    Frame,
    crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use tmt_cli_style::table::escape;
use tmt_tui::components::{ListFrame, ListRow, collection, surface};
use unicode_width::UnicodeWidthStr;

const FILE: &str = "squad.menu.xml";
const FOOTER: &str = "Enter runs · Esc closes";
/// Rows beyond the entries: two borders, the position line and the footer.
const CHROME: usize = 4;

#[derive(Clone, Copy)]
enum Presentation {
    Actions,
    Dropdown,
}

enum Template {
    Modal(surface::Template<()>),
    Dropdown {
        template: collection::Table<()>,
        scene: Option<(serde_json::Value, collection::ListSurface)>,
        frame: Option<ListFrame>,
    },
}

pub(in crate::board) struct MenuSurface {
    template: Template,
    rows: Vec<ListRow>,
    state: picker_surface::State,
    room_picker_footer: Option<String>,
}

/// What the pointer asked of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::board) enum Pointer {
    /// A click on an entry runs it, as Enter would.
    Choose(usize),
    /// The wheel moves the highlight by this many entries.
    Wheel(isize),
}

fn row_id(index: usize) -> String {
    // Entry keys may repeat or be numeric; the menu never reorders, so its own
    // position names a row for the whole life of the surface.
    format!("entry-{index}")
}

impl MenuSurface {
    pub fn invalidate(&mut self) {
        self.state.invalidate();
        if let Template::Dropdown { frame, .. } = &mut self.template {
            *frame = None;
        }
    }

    #[cfg(test)]
    pub fn new(title: &str, entries: &[MenuEntry]) -> Self {
        Self::with_anchor(title, entries, false)
    }
    pub fn with_anchor(title: &str, entries: &[MenuEntry], anchored: bool) -> Self {
        Self::build(title, entries, anchored, 48, Presentation::Actions)
    }
    pub(in crate::board) fn dropdown(title: &str, entries: &[MenuEntry]) -> Self {
        Self::build(title, entries, true, 18, Presentation::Dropdown)
    }
    fn build(
        title: &str,
        entries: &[MenuEntry],
        anchored: bool,
        width: u16,
        presentation: Presentation,
    ) -> Self {
        let key_role = if matches!(presentation, Presentation::Dropdown) {
            "muted"
        } else {
            "accent"
        };
        let placement = if anchored { "body" } else { "center" };
        let room_picker_footer = entries.first().and_then(|first| {
            let Choice::WorkspaceRoom { kind, .. } = &first.choice else {
                return None;
            };
            if !entries
                .iter()
                .all(|entry| matches!(&entry.choice, Choice::WorkspaceRoom { .. }))
            {
                return None;
            }
            let action = if *kind == Kind::Remove {
                "remove"
            } else {
                kind.label()
            };
            let action = super::notes::sanitize(action).to_ascii_lowercase();
            Some(format!("Enter {action} · Esc cancel"))
        });
        let room_picker_footer = if anchored && matches!(presentation, Presentation::Actions) {
            Some("j host · Enter Agents · Esc closes".into())
        } else {
            room_picker_footer
        };
        let cursor_width = if room_picker_footer.is_some() { 2 } else { 1 };
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
            r#"<tmt-view version="1"><tmt-modal id="action-menu" title="{title}" placement="{placement}" class="w-{width} h-{}"><tmt-scroll id="body"><tmt-list id="choices" bind="$.rows" empty="(no actions)"><tmt-row class="flex-row gap-0"><tmt-cell bind="row.key" class="w-{key_width} shrink-0" token="{key_role}"/><tmt-cell bind="row.cursor" class="w-{cursor_width} shrink-0" token="text"/><tmt-cell bind="row.label" class="truncate"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#,
            entries.len() + CHROME
        );
        let markup = if matches!(presentation, Presentation::Dropdown) {
            r#"<tmt-view version="1" class="w-full"><tmt-list id="choices" bind="$.rows" empty=""><tmt-row class="flex-row gap-0 h-1"><tmt-cell bind="row.key" class="w-3 shrink-0" token="muted"/><tmt-cell bind="row.label" class="truncate"/></tmt-row></tmt-list></tmt-view>"#.into()
        } else {
            markup
        };
        let rows: Vec<ListRow> = (0..entries.len())
            .map(|index| ListRow {
                id: row_id(index),
                disabled: matches!(entries[index].choice, Choice::Unavailable(_)),
            })
            .collect();
        Self {
            template: match presentation {
                Presentation::Actions => Template::Modal(picker_surface::compile(
                    FILE,
                    &markup,
                    picker_surface::schema(&["key", "cursor", "label"]),
                )),
                Presentation::Dropdown => {
                    let parsed = tmt_tui::parse(FILE, &markup).expect("embedded dropdown markup");
                    Template::Dropdown {
                        template: collection::compile(
                            FILE,
                            &parsed,
                            &picker_surface::schema(&["key", "cursor", "label"]),
                            &picker_surface::Data,
                        )
                        .expect("embedded dropdown schema"),
                        scene: None,
                        frame: None,
                    }
                }
            },
            state: picker_surface::State::new(None, rows.clone(), None),
            rows,
            room_picker_footer,
        }
    }

    fn list_frame(&self) -> Option<&ListFrame> {
        match &self.template {
            Template::Dropdown { frame, .. } => frame.as_ref(),
            Template::Modal(_) => self.state.frame.as_ref()?.list.as_ref(),
        }
    }
    pub(in crate::board) fn row_at(&self, mouse: MouseEvent) -> Option<usize> {
        let list = self.list_frame()?;
        let at = (mouse.column, mouse.row).into();
        if !list.viewport.contains(at) {
            return None;
        }
        list.geometry
            .iter()
            .rev()
            .find(|row| row.visible.contains(at))
            .and_then(|row| row.id.strip_prefix("entry-")?.parse().ok())
    }

    /// Reads a mouse event against what was last painted; a click outside the
    /// entries, and anything before the first paint, asks nothing.
    pub fn pointer(&self, mouse: MouseEvent) -> Option<Pointer> {
        let list = self.list_frame()?;
        let at = (mouse.column, mouse.row).into();
        if !list.viewport.contains(at) {
            return None;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.row_at(mouse).map(Pointer::Choose),
            MouseEventKind::ScrollUp => Some(Pointer::Wheel(-1)),
            MouseEventKind::ScrollDown => Some(Pointer::Wheel(1)),
            _ => None,
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
                serde_json::json!({"id": row.id, "disabled": matches!(entry.choice, Choice::Unavailable(_)), "key": if matches!(self.template, Template::Dropdown { .. }) { format!(" {}",entry.key) } else { entry.key.clone() }, "cursor": if self.state.picker.list.selected() == Some(row.id.as_str()) { "›" } else { "" }, "label": entry.label})
            })
            .collect();
        let footer = self.room_picker_footer.as_deref().unwrap_or(FOOTER);
        let value = serde_json::json!({"rows": rows, "query": "", "footer": footer, "status": "", "notes": []});
        match &mut self.template {
            Template::Modal(template) => {
                self.state.render(FILE, template, value, frame, look, body)
            }
            Template::Dropdown {
                template,
                scene,
                frame: painted,
            } => {
                if scene.as_ref().is_none_or(|(old, _)| old != &value) {
                    *scene = Some((
                        value.clone(),
                        template
                            .materialize(FILE, &value, &picker_surface::Data)
                            .expect("projected dropdown choices"),
                    ));
                }
                *painted = Some(
                    collection::render(
                        &scene.as_ref().unwrap().1,
                        &mut self.state.picker.list,
                        body,
                        frame.buffer_mut(),
                        surface::RenderStyle {
                            theme: &look.theme,
                            depth: look.depth,
                        },
                        |role| {
                            let style =
                                look.selection()
                                    .patch(look.row_span(true, look.role(role), false));
                            if look.depth == tmt_cli_style::Depth::None {
                                style.add_modifier(ratatui::style::Modifier::BOLD)
                            } else {
                                style
                            }
                        },
                    )
                    .expect("admitted dropdown list"),
                );
                // Collection paint owns text and selection. Supply the popup's
                // idle fill afterwards, preserving every explicit row style.
                let idle = super::input_field::style(look, false);
                let fill = ratatui::style::Style {
                    bg: idle.bg,
                    add_modifier: idle.add_modifier,
                    ..Default::default()
                };
                for y in body.y..body.bottom() {
                    for x in body.x..body.right() {
                        let cell = &mut frame.buffer_mut()[(x, y)];
                        if cell.bg == ratatui::style::Color::Reset {
                            cell.set_style(fill);
                        }
                    }
                }
            }
        }
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
