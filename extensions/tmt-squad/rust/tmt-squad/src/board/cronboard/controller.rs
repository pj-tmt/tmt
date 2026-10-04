//! Cron's board controller: opening the list and routing its events. Presentation
//! and projections stay in their modules; this is where input becomes an effect.

use super::{List, ListInput, rows::key_of, rows::row_id};
use crate::board::app::{App, Effect, Request, event_name};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use tmt_tui::components::ListEvent;

impl App {
    /// The ⑤ line and `c` need a read, or at least its failure to show.
    pub(in crate::board) fn cron_shown(&self) -> bool {
        self.cron.cron.is_some() || self.cron.failure.is_some()
    }

    pub(in crate::board) fn open_cron_list(&mut self, selected: Option<&str>) -> Effect {
        if !self.cron_shown() {
            return self.say("Cron jobs have not been read yet.");
        }
        self.cron_list = Some(List::open(&self.cron, selected));
        Effect::None
    }

    pub(in crate::board) fn cron_list_event(&mut self, event: &Event) -> Option<Effect> {
        let input = self.cron_list.as_ref()?.input(event);
        match input {
            ListInput::None => None,
            ListInput::Close => {
                self.cron_list = None;
                Some(Effect::None)
            }
            ListInput::Job(job, id) => {
                // Forms and the delete confirmation need the base layer's input
                // line and menu; a send or pause keeps the list open to show it.
                if matches!(job, 'n' | 'e' | 'o' | 'd') {
                    self.cron_list = None;
                }
                Some(self.cron_key(job, id.as_deref()))
            }
            ListInput::Open(id) => {
                let squad = self
                    .cron
                    .cron
                    .iter()
                    .flat_map(|cron| &cron.jobs)
                    .map(key_of)
                    .find(|key| row_id(key) == id)
                    .map(|key| key.squad)?;
                self.cron_list = None;
                Some(self.go(squad))
            }
        }
    }

    /// Where the clock runs as `session:window`, when the worker could resolve the
    /// holder's pane; otherwise the caller shows the pane id.
    pub(in crate::board) fn clock_place(&self) -> Option<&str> {
        self.cron.cron.as_ref()?.place.as_deref()
    }

    /// Whether the jobs half was drawn on the last frame, so Tab can enter it.
    pub(in crate::board) fn jobs_painted(&self) -> bool {
        !self.jobs_area.get().is_empty()
    }

    fn jobs_room(&self) -> Option<String> {
        super::half::room(self).map(str::to_owned)
    }

    /// The jobs half is a base-layer field of the shared focus stack: its keys
    /// route through `app::route` like an overlay's, and only what it does not
    /// handle falls through to the board. A user `[bind]` on a key wins.
    pub(in crate::board) fn jobs_event(&mut self, event: &Event) -> Option<Effect> {
        use tmt_tui::app::{Routed, route};
        if !self.jobs_focus {
            return None;
        }
        if !self.jobs_painted() {
            self.jobs_focus = false;
            return None;
        }
        if !matches!(event, Event::Key(_))
            || self.input.is_some()
            || self.menu.is_some()
            || self.searching
        {
            return None;
        }
        let mut focus = tmt_tui::app::FocusStack::new(vec![vec!["cron-jobs".into()]]);
        match route(&mut focus, event, |_, event| self.jobs_input(event)) {
            Routed::Handled(effect) => Some(effect),
            Routed::Quit => Some(Effect::Quit),
            Routed::Captured | Routed::Unhandled => None,
        }
    }

    fn jobs_input(&mut self, event: &Event) -> Option<Effect> {
        let Event::Key(key) = event else { return None };
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if event_name(*key).is_some_and(|name| {
            self.view
                .as_ref()
                .is_some_and(|view| view.configured_bindings.contains_key(&name))
        }) {
            return None;
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.jobs_leave(key.code == KeyCode::BackTab);
                Some(Effect::None)
            }
            KeyCode::Char('c') => {
                let selected = self.jobs_selected();
                Some(self.open_cron_list(selected.as_deref()))
            }
            KeyCode::Char(job @ ('n' | 'e' | 'p' | 'x' | 'o' | 'd'))
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let selected = self.jobs_selected();
                Some(self.cron_key(job, selected.as_deref()))
            }
            _ => {
                let room = self.jobs_room()?;
                let event = self.jobs.borrow_mut().get_mut(&room)?.input(event)?;
                match event {
                    ListEvent::Confirm(id) => Some(self.jobs_open_owner(&id)),
                    ListEvent::Changed(_) => Some(Effect::None),
                }
            }
        }
    }

    pub(in crate::board) fn jobs_selected(&self) -> Option<String> {
        let room = self.jobs_room()?;
        self.jobs
            .borrow()
            .get(&room)?
            .list
            .selected()
            .map(str::to_owned)
    }

    /// Enter on a job goes to its current owner's pane.
    fn jobs_open_owner(&mut self, id: &str) -> Effect {
        let owner = self
            .cron
            .cron
            .iter()
            .flat_map(|cron| &cron.jobs)
            .find(|view| super::rows::row_id(&super::rows::key_of(view)) == id)
            .map(|view| view.owner_name.clone());
        match owner {
            Some(Some(name)) => Effect::Act(Request::Jump(name)),
            Some(None) => self.say("This job has no owner; o reassigns it."),
            None => self.say("The job no longer exists."),
        }
    }

    /// Pointer input over the jobs half focuses it and drives its list; a press
    /// anywhere else gives the focus back.
    pub(in crate::board) fn jobs_mouse(
        &mut self,
        event: ratatui::crossterm::event::MouseEvent,
    ) -> Option<Effect> {
        let inside = self
            .jobs_area
            .get()
            .contains((event.column, event.row).into());
        if !inside {
            if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.jobs_focus = false;
            }
            return None;
        }
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
            self.jobs_focus = true;
        }
        let room = self.jobs_room()?;
        let listed = self
            .jobs
            .borrow_mut()
            .get_mut(&room)?
            .input(&Event::Mouse(event));
        Some(match listed {
            Some(ListEvent::Confirm(id)) => self.jobs_open_owner(&id),
            _ => Effect::None,
        })
    }
}
