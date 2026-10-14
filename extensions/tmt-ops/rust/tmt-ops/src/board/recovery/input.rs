//! Input emits typed jobs; nothing here starts a command.
use super::*;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
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
            if key.code == KeyCode::Esc
                || key.code == KeyCode::Backspace && self.screen != Screen::Save
            {
                if self.screen == Screen::List
                    || self.pending.as_ref().is_some_and(|t| t.job.writes())
                {
                    self.close();
                } else {
                    self.confirmation = None;
                    self.screen = Screen::List;
                    self.invalidate();
                }
                return None;
            }
            if self.pending.as_ref().is_some_and(|t| t.job.writes()) {
                return None;
            }
            if self.screen == Screen::Save && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                self.form_focus = !self.form_focus;
                return None;
            }
            if matches!(self.screen, Screen::Preview | Screen::Results) && key.code == KeyCode::Tab
            {
                self.form_focus = !self.form_focus;
                return None;
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
        let input = state.input(event, field);
        drop(state);
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
