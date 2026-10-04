//! The `c` list: every squad's jobs in one modal list. The overlay owns only its
//! caller-side component state; opening a squad and every job effect belong to
//! the board controller.

use super::{
    State,
    line::{clock, first_line},
    rows::{Columns, key_of, project, row_id},
    surface::modal,
};
use crate::{board::picker_surface, look::Look};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::Rect,
};
use serde_json::json;
use std::cell::RefCell;
use tmt_tui::components::{ListRow, PickerEvent, PickerField, PickerInput, surface::Template};

pub(in crate::board) enum Input {
    /// Consumed by the list, or not meant for it.
    None,
    Close,
    /// Enter or a click on this row id.
    Open(String),
    /// A job control (`n e p x o d`) with the selected row id, if any.
    Job(char, Option<String>),
}

/// What the compiled template depends on: columns, modal width and height.
type Shape = (Columns, u16, u16);

pub(in crate::board) struct List {
    surface: RefCell<picker_surface::State>,
    template: RefCell<Option<(Shape, Template<()>)>>,
}

fn list_rows(state: &State) -> Vec<ListRow> {
    state
        .cron
        .iter()
        .flat_map(|cron| &cron.jobs)
        .map(|view| ListRow {
            id: row_id(&key_of(view)),
            disabled: false,
        })
        .collect()
}

impl List {
    pub fn open(state: &State, selected: Option<&str>) -> Self {
        Self {
            surface: RefCell::new(picker_surface::State::new(None, list_rows(state), selected)),
            template: RefCell::default(),
        }
    }

    pub fn invalidate(&self) {
        self.surface.borrow_mut().invalidate();
    }

    pub fn selected(&self) -> Option<String> {
        self.surface
            .borrow()
            .picker
            .list
            .selected()
            .map(str::to_owned)
    }

    pub fn render(
        &self,
        state: &State,
        now_ms: i64,
        place: Option<&str>,
        frame: &mut Frame,
        look: Look,
        body: Rect,
    ) {
        // A docked modal is a full-width sheet below 100 columns, else nine tenths.
        let width = if body.width < 100 {
            body.width
        } else {
            (u32::from(body.width) * 9 / 10) as u16
        };
        let inside = width.saturating_sub(4);
        let columns = Columns::for_width(true, inside);
        let jobs: Vec<_> = state.cron.iter().flat_map(|cron| &cron.jobs).collect();
        // Content height: border 2, one line per job (one for the
        // empty note), position, status and footer lines; the modal caps it.
        let height = (jobs.len().max(1) as u16 + 5).min(body.height);
        let shape = (columns, width, height);
        let mut template = self.template.borrow_mut();
        if template.as_ref().is_none_or(|(old, _)| *old != shape) {
            *template = Some((shape, modal(columns, width, height)));
        }
        let mut status = match &state.cron {
            Some(cron) => format!(
                "{} jobs · {}",
                jobs.len(),
                clock(&cron.clock, now_ms, false, state.clock_note(place)).0
            ),
            None => "no jobs read".into(),
        };
        if let Some(failure) = &state.failure {
            status.push_str(&format!(" · ! {}", first_line(failure)));
        }
        let value = json!({
            "rows": project(&jobs, None, now_ms),
            "query": "",
            "status": status,
            "footer": super::hints::overlay(usize::from(inside)),
            "notes": [],
            // The cached scene is keyed by value, and the template depends on width.
            "columns": format!("{columns:?}/{height}"),
        });
        let (_, template) = template.as_ref().expect("compiled");
        self.surface
            .borrow_mut()
            .render("squad.cron.xml", template, value, frame, look, body);
    }

    pub fn input(&self, event: &Event) -> Input {
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Input::Close,
                KeyCode::Enter => {
                    return self.selected().map_or(Input::None, Input::Open);
                }
                KeyCode::Char(job @ ('n' | 'e' | 'p' | 'x' | 'o' | 'd')) => {
                    return Input::Job(job, self.selected());
                }
                _ => {}
            }
        }
        match self.surface.borrow_mut().input(event, PickerField::List) {
            Some(PickerInput::Event(PickerEvent::Confirm(id))) => Input::Open(id),
            Some(PickerInput::Event(PickerEvent::Cancel)) => Input::Close,
            _ => Input::None,
        }
    }
}

#[cfg(test)]
mod tests;
