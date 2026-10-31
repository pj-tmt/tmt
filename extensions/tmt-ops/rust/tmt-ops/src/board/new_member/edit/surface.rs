//! Edit fields use the same Editor and choice vocabulary as New member.
use super::*;
use ratatui::{Frame, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::{Dialog, Focus as DialogFocus, Placement, strip};
impl Controller {
    pub(in crate::board::new_member) fn render_edit(
        &self,
        frame: &mut Frame,
        app: &App,
        look: crate::look::Look,
        body: Rect,
    ) {
        let Some(edit) = &self.edit else { return };
        if self.list.is_some() {
            self.render_edit_list(frame, app, look, body);
            return;
        }
        self.fields.borrow_mut().clear();
        self.readable.set(false);
        let fields = edit.fields(&self.draft, self.loaded.as_ref());
        let target_error = (self.stage == Stage::Form)
            .then(|| edit.current_error(app))
            .flatten();
        let buttons = edit.buttons(self, target_error.is_none());
        let dialog = Dialog {
            title: edit.title(),
            kind: DialogKind::Form,
            placement: Placement::Center,
            buttons,
        };
        let preview = if let Stage::Result(text) = &self.stage {
            text.lines().map(str::to_owned).collect::<Vec<_>>()
        } else if self.stage == Stage::Launching {
            vec![
                "◌ Saving the reviewed changes…".into(),
                "Closing this form keeps the operation running.".into(),
            ]
        } else {
            self.loaded
                .as_ref()
                .map(|l| {
                    edit.plan(&self.draft, l)
                        .map(|p| p.lines(&l.display_executable))
                        .unwrap_or_else(|e| vec![format!("✗ {e}")])
                })
                .unwrap_or_else(|| vec!["◌ Loading edit settings…".into()])
        };
        let probe = dialog.areas(
            body,
            [100, 30],
            self.notice.is_some() || target_error.is_some(),
        );
        let mut lines = Vec::new();
        let mut field_rows = Vec::new();
        if self.stage == Stage::Form {
            lines.push("↑↓ field · ←→ value · Enter choose · Esc keeps this draft".to_owned());
            lines.push(String::new());
            for _field in &fields {
                field_rows.push(lines.len());
                lines.push(String::new());
                lines.push(String::new());
            }
            if let Target::Member { memberships, .. } = &edit.target {
                lines.push(format!(
                    "Host · {} (fixed)",
                    self.draft.text(super::super::model::Slot::Host)
                ));
                lines.push(format!(
                    "Session · {} (fixed)",
                    self.draft.text(super::super::model::Slot::Session)
                ));
                if memberships.len() > 1 {
                    let Target::Member { source, .. } = &edit.target else {
                        unreachable!()
                    };
                    let names = memberships
                        .iter()
                        .filter(|id| source.as_ref().is_none_or(|room| &room.id != *id))
                        .map(|id| {
                            self.draft
                                .rooms
                                .iter()
                                .find(|room| &room.id == id)
                                .map_or_else(
                                    || format!("unavailable squad ({id})"),
                                    |room| room.name.clone(),
                                )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.extend(crate::board::notes::wrap(
                        &format!(
                            "{} squads stay · {names}",
                            if source.is_some() {
                                "Other"
                            } else {
                                "Existing"
                            }
                        ),
                        usize::from(probe.content.width),
                    ));
                }
                lines.extend(crate::board::notes::wrap(
                    "Model and effort keep the recorded resume settings.",
                    usize::from(probe.content.width),
                ));
                if let Target::Member { observation, .. } = &edit.target {
                    let last = observation["resume"]["observedModel"]["model"]
                        .as_str()
                        .unwrap_or("unknown");
                    lines.extend(crate::board::notes::wrap(
                        &format!("Last observed model at open · {last}"),
                        usize::from(probe.content.width),
                    ));
                }
                lines.push(String::new());
            }
            lines.push("Runs".into());
        }
        for line in preview {
            lines.extend(crate::board::notes::wrap(
                &crate::board::notes::sanitize(&line),
                usize::from(probe.content.width),
            ));
        }
        if self.reopened && self.stage == Stage::Form {
            lines.push("Kept draft · Discard starts a fresh review".into());
        }
        let areas = dialog.areas(
            body,
            [100, (lines.len() + 4).min(u16::MAX as usize) as u16],
            self.notice.is_some() || target_error.is_some(),
        );
        let mut hits = crate::board::buttons::paint_dialog(
            app,
            frame.buffer_mut(),
            &dialog,
            areas,
            &edit.dialog,
            crate::board::app::Overlay::NewMember,
        );
        app.input_band.set(Some(areas.outer));
        app.peek_areas.set(
            super::super::super::app::Overlay::NewMember,
            Some(areas.outer),
        );
        let selected = if self.stage != Stage::Form {
            None
        } else {
            match edit.dialog.focus {
                DialogFocus::Field(i) => field_rows.get(i).map(|r| *r..*r + 1),
                DialogFocus::Buttons(_) => {
                    (!lines.is_empty()).then(|| lines.len() - 1..lines.len())
                }
            }
        };
        let mut scroll = self.scroll.borrow_mut();
        scroll.update(areas.content, lines.len(), selected);
        let offset = scroll.offset();
        for (i, line) in lines
            .iter()
            .enumerate()
            .skip(offset)
            .take(usize::from(areas.content.height))
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + (i - offset) as u16,
                    areas.content.width,
                    1,
                ),
                Line::styled(crate::board::notes::sanitize(line), look.role(Role::Muted)),
            );
        }
        for (i, field) in fields.iter().enumerate() {
            let row = field_rows.get(i).copied().unwrap_or(usize::MAX);
            if self.stage != Stage::Form
                || row < offset
                || row >= offset + usize::from(areas.content.height)
            {
                hits.fields.push(Rect::default());
                continue;
            }
            let area = Rect::new(
                areas.content.x,
                areas.content.y + (row - offset) as u16,
                areas.content.width,
                1,
            );
            let value_area = Rect::new(area.x + 12, area.y, area.width.saturating_sub(12), 1);
            hits.fields.push(value_area);
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(area.x, area.y, 12.min(area.width), 1),
                Line::styled(field.label(), look.role(Role::Muted)),
            );
            let focused = edit.dialog.focus == DialogFocus::Field(i);
            let text = match field {
                Field::Member(super::super::model::Slot::Squad)
                    if self.draft.text(super::super::model::Slot::Squad).is_empty()
                        && matches!(&edit.target,Target::Member{source:None,memberships,..}if !memberships.is_empty()) =>
                {
                    "add to a squad".into()
                }
                Field::Member(slot) => self.draft.shown(*slot).unwrap_or_else(|| {
                    if *slot == super::super::model::Slot::Name {
                        "agent name".into()
                    } else if matches!(
                        slot,
                        super::super::model::Slot::Model | super::super::model::Slot::Effort
                    ) && self.draft.text(super::super::model::Slot::Tool)
                        != edit.initial[super::super::model::Slot::Tool.index()]
                    {
                        "remembered / default".into()
                    } else {
                        "keep current / unknown".into()
                    }
                }),
                Field::Alert => if edit.enabled { "On" } else { "Off" }.into(),
                Field::After => edit.after.as_str().into(),
                Field::Hidden => if edit.hidden { "On" } else { "Off" }.into(),
            };
            if field.kind() == FieldKind::Text {
                let editor = match field {
                    Field::Member(slot) => &self.draft.fields[slot.index()],
                    Field::After => &edit.after,
                    _ => unreachable!(),
                };
                let text_area = if *field == Field::After {
                    Rect::new(
                        value_area.x,
                        value_area.y,
                        value_area.width.saturating_sub(2),
                        1,
                    )
                } else {
                    value_area
                };
                if *field == Field::After && value_area.width >= 2 {
                    strip::paint_left(
                        frame.buffer_mut(),
                        Rect::new(value_area.right() - 1, value_area.y, 1, 1),
                        Line::styled("▾", look.role(Role::Muted)),
                    );
                }
                let cursor = editor.layout(text_area.width).paint(
                    frame.buffer_mut(),
                    text_area,
                    0,
                    editor.selection(),
                    look.role(if focused { Role::Accent } else { Role::Text }),
                    look.selection(),
                );
                if focused {
                    self.editor_width.set(text_area.width);
                    if let Some(cursor) = cursor {
                        frame.set_cursor_position(cursor);
                    }
                }
            } else {
                strip::paint_left(
                    frame.buffer_mut(),
                    value_area,
                    Line::styled(
                        format!("{text} ▾"),
                        if focused {
                            look.selection()
                        } else {
                            look.role(Role::Text)
                        },
                    ),
                );
            }
        }
        self.readable.set(
            areas.content.width >= 34 && offset + usize::from(areas.content.height) >= lines.len(),
        );
        if let Some(notice) = target_error.as_ref().or(self.notice.as_ref()) {
            strip::paint_left(
                frame.buffer_mut(),
                areas.status,
                Line::styled(
                    crate::board::notes::sanitize(notice),
                    look.role(Role::Waiting),
                ),
            );
        }
        *edit.hits.borrow_mut() = hits;
    }
    fn render_edit_list(&self, frame: &mut Frame, app: &App, look: crate::look::Look, body: Rect) {
        let list = self.list.as_ref().unwrap();
        let rows = self.rows();
        let dialog = Dialog {
            title: format!("{} · choose", list.field.label()),
            kind: DialogKind::List,
            placement: Placement::Center,
            buttons: vec![Button::primary("Back")],
        };
        let state = DialogState::new(&[], &dialog.buttons);
        let areas = dialog.areas(
            body,
            [100, (rows.len() + 6).min(u16::MAX as usize) as u16],
            false,
        );
        let mut hits = crate::board::buttons::paint_dialog(
            app,
            frame.buffer_mut(),
            &dialog,
            areas,
            &state,
            crate::board::app::Overlay::NewMember,
        );
        app.input_band.set(Some(areas.outer));
        app.peek_areas.set(
            super::super::super::app::Overlay::NewMember,
            Some(areas.outer),
        );
        self.fields.borrow_mut().clear();
        let mut scroll = self.scroll.borrow_mut();
        scroll.update(
            areas.content,
            rows.len() + 2,
            Some(list.row + 2..list.row + 3),
        );
        let offset = scroll.offset();
        if offset == 0 && areas.content.height > 0 {
            let area = Rect::new(areas.content.x, areas.content.y, areas.content.width, 1);
            let cursor = list.filter.layout(area.width).paint(
                frame.buffer_mut(),
                area,
                0,
                list.filter.selection(),
                look.role(Role::Accent),
                look.selection(),
            );
            self.editor_width.set(area.width);
            hits.fields.push(area);
            if let Some(cursor) = cursor {
                frame.set_cursor_position(cursor);
            }
            if list.filter.is_empty() {
                strip::paint_left(
                    frame.buffer_mut(),
                    area,
                    Line::styled("type to filter", look.role(Role::Muted)),
                );
            }
        }
        for (i, choice) in rows.iter().enumerate() {
            let row = i + 2;
            if row < offset || row >= offset + usize::from(areas.content.height) {
                continue;
            }
            let area = Rect::new(
                areas.content.x,
                areas.content.y + (row - offset) as u16,
                areas.content.width,
                1,
            );
            strip::paint_left(
                frame.buffer_mut(),
                area,
                Line::styled(
                    format!("{} {}", if i == list.row { "›" } else { " " }, choice.label),
                    if i == list.row {
                        look.selection()
                    } else {
                        look.role(Role::Text)
                    },
                ),
            );
            self.fields
                .borrow_mut()
                .push((super::super::Field::Choice(i), area));
        }
        *self.edit.as_ref().unwrap().hits.borrow_mut() = hits;
    }
}
