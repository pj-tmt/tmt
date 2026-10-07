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
use tmt_tui::components::{
    ListRow, Modal, PickerEvent, PickerField, PickerInput, Placement, surface::Template,
};

pub(in crate::board) enum Input {
    /// Consumed by the list without an effect (a move, a boundary press).
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
        // The modal owns its sheet width; ask it rather than repeating its rule.
        let width = Modal {
            title: String::new(),
            placement: Placement::Docked,
        }
        .areas(body, [body.width, body.height], true, true)
        .outer
        .width;
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
        // Reconcile before projecting the cursor, including when refresh removes
        // the selected job. The list remains the sole selection owner.
        let mut surface = self.surface.borrow_mut();
        surface.reconcile(list_rows(state));
        let selected = surface.picker.list.selected().map(str::to_owned);
        let value = json!({
            "rows": project(&jobs, None, selected.as_deref(), now_ms),
            "query": "",
            "status": status,
            "footer": super::hints::overlay(usize::from(inside)),
            "notes": [],
            // The cached scene is keyed by value, and the template depends on width.
            "columns": format!("{columns:?}/{height}"),
        });
        let (_, template) = template.as_ref().expect("compiled");
        surface.render("squad.cron.xml", template, value, frame, look, body);
        if let Some(map) = &surface.frame {
            let footer = map.areas.footer;
            let text = super::hints::overlay(usize::from(footer.width));
            tmt_tui::components::strip::paint_left(
                frame.buffer_mut(),
                footer,
                crate::board::view::footer::hint_line(&text, look),
            );
        }
    }

    /// `None` is an event the list does not take, so the caller's router can offer
    /// it elsewhere; a consumed event must come back as `Some`, or the router asks
    /// again and a move is applied twice.
    pub fn input(&self, event: &Event) -> Option<Input> {
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Some(Input::Close),
                KeyCode::Enter => {
                    return Some(self.selected().map_or(Input::None, Input::Open));
                }
                KeyCode::Char(job @ ('n' | 'e' | 'p' | 'x' | 'o' | 'd')) => {
                    return Some(Input::Job(job, self.selected()));
                }
                _ => {}
            }
        }
        match self.surface.borrow_mut().input(event, PickerField::List)? {
            PickerInput::Event(PickerEvent::Confirm(id)) => Some(Input::Open(id)),
            PickerInput::Event(PickerEvent::Cancel) => Some(Input::Close),
            _ => Some(Input::None),
        }
    }
}

#[cfg(test)]
mod tests;
