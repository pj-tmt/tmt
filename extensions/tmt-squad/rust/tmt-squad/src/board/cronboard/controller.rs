//! Cron's board controller: opening the list and routing its events. Presentation
//! and projections stay in their modules; this is where input becomes an effect.

use super::{List, ListInput, rows::key_of, rows::row_id};
use crate::board::app::{App, Effect};
use ratatui::crossterm::event::Event;

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
}
