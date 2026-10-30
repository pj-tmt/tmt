//! Input emits typed jobs; nothing here starts a command.
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
        if let Event::Key(key) = event {
            if key.kind != KeyEventKind::Press {
                return None;
            }
            // Esc hides the dialog with its screen; a confirm is its own layer and returns to
            // the list first.
            if key.code == KeyCode::Esc {
                if matches!(self.screen, Screen::Confirm | Screen::Delete)
                    && !self.pending.as_ref().is_some_and(|t| t.job.writes())
                {
                    self.confirmation = None;
                    self.screen = Screen::List;
                    self.invalidate();
                } else {
                    self.close();
                }
                return None;
            }
            if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
                return None;
            }
            // ↓ from the name field or past the end of the reading reaches the actions; ↑ on the
            // first action goes back.
            if matches!(
                self.screen,
                Screen::Preview | Screen::Results | Screen::Save
            ) && key.modifiers.is_empty()
            {
                let first = self
                    .rows()
                    .into_iter()
                    .find(|row| !row.disabled)
                    .map(|row| row.id);
                let panel = if self.screen == Screen::Save {
                    self.form.borrow()
                } else {
                    self.panel.borrow()
                };
                let scroll = &panel.picker.list.scroll;
                let to_actions = self.form_focus
                    && key.code == KeyCode::Down
                    && (self.screen == Screen::Save || scroll.offset() >= scroll.limit());
                let to_reading = !self.form_focus
                    && key.code == KeyCode::Up
                    && first.is_some()
                    && panel.picker.list.selected() == first.as_deref();
                drop(panel);
                if to_actions || to_reading {
                    self.form_focus = to_reading;
                    self.invalidate();
                    return None;
                }
            }
            if key.code == KeyCode::Char('r') && key.modifiers == KeyModifiers::CONTROL {
                return self.refresh();
            }
        }
        if matches!(self.screen, Screen::Preview | Screen::Results) && self.form_focus {
            if let Event::Mouse(mouse) = event
                && !matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                )
            {
                // Action clicks must reach the painted list, even while reading.
            } else {
                let mut reading = self.panel.borrow_mut();
                if reading.picker.list.scroll.input(event) {
                    reading.invalidate();
                    return None;
                }
            }
        }
        let rows = self.rows();
        let state = match self.screen {
            Screen::List => &self.list,
            Screen::Save => &self.form,
            _ => &self.panel,
        };
        let mut state = state.borrow_mut();
        state.reconcile(
            rows.iter()
                .map(|r| tmt_tui::components::ListRow {
                    id: r.id.clone(),
                    disabled: r.disabled,
                })
                .collect(),
        );
        let field = if self.screen == Screen::Save && self.form_focus {
            PickerField::Query
        } else {
            PickerField::List
        };
        // A list's shared click selects. Recovery action rows activate on that
        // same painted rectangle, including an already selected action. Never
        // invent geometry or activate a disabled/clipped/stale row.
        let clicked = if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            state
                .frame
                .as_ref()
                .and_then(|map| map.list.as_ref())
                .filter(|frame| {
                    frame.offset == state.picker.list.scroll.offset()
                        && frame.viewport.contains((mouse.column, mouse.row).into())
                })
                .and_then(|frame| {
                    frame
                        .geometry
                        .iter()
                        .rev()
                        .find(|row| row.visible.contains((mouse.column, mouse.row).into()))
                })
                .and_then(|hit| rows.iter().find(|row| row.id == hit.id && !row.disabled))
                .map(|row| row.id.clone())
        } else {
            None
        };
        let input = state.input(event, field);
        drop(state);
        if let Some(id) = clicked {
            return if self.screen == Screen::List && id.starts_with("snapshot:") {
                self.select(&id)
            } else {
                self.choose(&id)
            };
        }
        match input {
            Some(PickerInput::Event(PickerEvent::Changed(id)))
                if self.screen == Screen::List && id.starts_with("snapshot:") =>
            {
                self.select(&id)
            }
            Some(PickerInput::Event(PickerEvent::Confirm(id))) => self.choose(&id),
            Some(PickerInput::Event(PickerEvent::Cancel)) => {
                if self.screen == Screen::List {
                    self.close();
                } else {
                    self.choose("cancel");
                }
                None
            }
            _ => None,
        }
    }
}
