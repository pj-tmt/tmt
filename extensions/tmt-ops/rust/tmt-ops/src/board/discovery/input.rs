//! Input uses only current shared geometry and emits typed jobs.
use super::*;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use tmt_tui::components::{PickerEvent, PickerField, PickerInput};
impl Controller {
    pub fn input(&mut self, event: &Event) -> Option<Task> {
        if !self.active {
            return None;
        }
        if matches!(event, Event::Resize(..)) {
            self.changed();
            return None;
        }
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                return None;
            }
            // Esc hides the dialog with its screen; the confirm is its own layer and cancels
            // first.
            if key.code == KeyCode::Esc {
                if self.screen == Screen::Confirm
                    && !self.pending.as_ref().is_some_and(|t| t.job.writes())
                {
                    self.choose("cancel");
                } else {
                    self.close();
                }
                return None;
            }
            if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
                return None;
            }
            // ↓ from the field or past the end of the reading reaches the actions; ↑ on the first
            // action goes back.
            if matches!(
                self.screen,
                Screen::Form | Screen::Details | Screen::Results
            ) && key.modifiers.is_empty()
            {
                let first = self
                    .rows()
                    .into_iter()
                    .find(|row| !row.disabled)
                    .map(|row| row.id);
                let ui = self.ui.borrow();
                let scroll = &ui.picker.list.scroll;
                let to_actions = self.form_focus
                    && key.code == KeyCode::Down
                    && (self.screen == Screen::Form || scroll.offset() >= scroll.limit());
                let to_field = !self.form_focus
                    && key.code == KeyCode::Up
                    && first.is_some()
                    && ui.picker.list.selected() == first.as_deref();
                drop(ui);
                if to_actions || to_field {
                    self.form_focus = to_field;
                    self.invalidate();
                    return None;
                }
            }
            if key.code == KeyCode::Char('r') && key.modifiers == KeyModifiers::CONTROL {
                return self.scan();
            }
        }
        if self.screen == Screen::Form {
            if let Event::Mouse(mouse) = event
                && matches!(
                    mouse.kind,
                    MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Drag(MouseButton::Left)
                )
                && let Some(area) = self
                    .editor_area
                    .get()
                    .filter(|area| area.contains((mouse.column, mouse.row).into()))
            {
                self.form_focus = true;
                self.editor.place(
                    area.width,
                    self.editor_offset.get() + usize::from(mouse.row - area.y),
                    usize::from(mouse.column - area.x),
                    matches!(mouse.kind, MouseEventKind::Drag(_))
                        || mouse.modifiers.contains(KeyModifiers::SHIFT),
                );
                self.invalidate();
                return None;
            }
            if self.form_focus {
                match event {
                    Event::Paste(text) => {
                        if text.chars().any(char::is_control) || !self.editor.insert(text, 1024) {
                            self.failure = Some(SquadError::new(
                                "USAGE_ERROR",
                                "Enter one name or UUID, up to 1,024 bytes; control characters are not allowed.",
                            ));
                        }
                        self.invalidate();
                        return None;
                    }
                    Event::Key(key) if key.code == KeyCode::Enter => return self.choose("review"),
                    Event::Key(key) => {
                        let width = self.editor_area.get().map_or(80, |area| area.width);
                        if self.editor.key(*key, width, 1024) {
                            self.invalidate();
                            return None;
                        }
                    }
                    _ => {}
                }
            }
        }
        if matches!(self.screen, Screen::Details | Screen::Results)
            && self.form_focus
            && !matches!(event,Event::Mouse(m) if !matches!(m.kind,MouseEventKind::ScrollUp|MouseEventKind::ScrollDown))
        {
            let mut ui = self.ui.borrow_mut();
            if ui.picker.list.scroll.input(event) {
                ui.invalidate();
                self.readable.set(false);
                return None;
            }
        }
        let rows = self.rows();
        let mut ui = self.ui.borrow_mut();
        ui.reconcile(
            rows.iter()
                .map(|r| tmt_tui::components::ListRow {
                    id: r.id.clone(),
                    disabled: r.disabled,
                })
                .collect(),
        );
        let clicked = if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            ui.frame
                .as_ref()
                .and_then(|map| map.list.as_ref())
                .filter(|frame| {
                    frame.offset == ui.picker.list.scroll.offset()
                        && frame.viewport.contains((mouse.column, mouse.row).into())
                })
                .and_then(|frame| {
                    frame
                        .geometry
                        .iter()
                        .rev()
                        .find(|r| r.visible.contains((mouse.column, mouse.row).into()))
                })
                .and_then(|hit| rows.iter().find(|r| r.id == hit.id && !r.disabled))
                .map(|r| r.id.clone())
        } else {
            None
        };
        let field = PickerField::List;
        let input = ui.input(event, field);
        drop(ui);
        if let Some(id) = clicked {
            return self.choose(&id);
        }
        match input {
            Some(PickerInput::Event(PickerEvent::Changed(id)))
                if self.screen == Screen::Candidates && id.starts_with("candidate:") =>
            {
                self.invalidate();
                self.selected = id
                    .strip_prefix("candidate:")
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| self.observation.as_ref()?.panes.get(i))
                    .map(|p| p.key.clone());
                None
            }
            Some(PickerInput::Event(PickerEvent::Confirm(id))) => self.choose(&id),
            Some(PickerInput::Event(PickerEvent::Cancel)) => {
                self.choose("back");
                None
            }
            _ => None,
        }
    }
}
