//! Cron's board controller: opening the list and routing its events. Presentation
//! and projections stay in their modules; this is where input becomes an effect.

use super::{List, ListInput, rows::key_of, rows::row_id};
use crate::board::app::{App, Effect};
use ratatui::crossterm::event::Event;

/// Room context retained by a named public Squad projection, when present.
pub(super) fn projected_room(app: &App) -> Option<&str> {
    app.view
        .as_ref()?
        .document
        .pointer("/squad/roomId")?
        .as_str()
}

impl App {
    /// The home cron line and `c` need a read, or at least its failure to show.
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
        // `None` is only an event the list does not take; one it consumed is
        // handled, or the router would offer it again and apply it twice.
        let input = self.cron_list.as_ref()?.input(event)?;
        match input {
            ListInput::None => Some(Effect::None),
            ListInput::Close => {
                self.cron_list = None;
                Some(Effect::None)
            }
            ListInput::Job(job, id) => {
                // Forms and the delete confirmation need the base layer's input
                // line and menu; a send or pause keeps the list open to show it.
                if matches!(job, 'n' | 'E' | 'o' | 'd') {
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
}
