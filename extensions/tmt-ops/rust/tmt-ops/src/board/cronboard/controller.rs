//! Cron's board controller: opening the list and routing its events. Presentation
//! and projections stay in their modules; this is where input becomes an effect.

use super::{ListInput, rows::key_of, rows::row_id};
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
    /// Timers needs a read, or at least its failure to show.
    pub(in crate::board) fn cron_shown(&self) -> bool {
        self.cron.cron.is_some() || self.cron.failure.is_some()
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
                if job == 'n'
                    && id.is_none()
                    && self
                        .cron_list
                        .as_ref()
                        .is_some_and(|list| list.scope.is_none())
                {
                    return Some(
                        self.say("Select an explicit squad or job before creating a timer."),
                    );
                }
                // Forms and the delete confirmation need the base layer's input
                // line and menu; a send or pause keeps the list open to show it.
                if matches!(job, 'n' | 'E' | 'o' | 'd') {
                    self.cron_list = None;
                }
                Some(self.cron_key(job, id.as_deref()))
            }
            ListInput::Open(id) => {
                let room = self
                    .cron
                    .cron
                    .iter()
                    .flat_map(|cron| &cron.jobs)
                    .map(key_of)
                    .find(|key| row_id(key) == id)
                    .map(|key| key.room_id)?;
                let target = crate::board::workspace::OpsTarget::Squad(room);
                let tab = self.shell.tab;
                self.shell.tab = crate::board::shell::OpsTab::Agents;
                let index = self
                    .workspace_rows()
                    .iter()
                    .position(|row| row.target == target);
                let Some(index) = index else {
                    self.shell.tab = tab;
                    return Some(self.say("Timer squad is not visible or no longer observed. Refresh or clear the filter before opening it."));
                };
                self.cron_list = None;
                self.shell.tab = crate::board::shell::OpsTab::Agents;
                self.focus = 0;
                self.select(index);
                Some(Effect::None)
            }
        }
    }

    /// Where the clock runs as `session:window`, when the worker could resolve the
    /// holder's pane; otherwise the caller shows the pane id.
    pub(in crate::board) fn clock_place(&self) -> Option<&str> {
        self.cron.cron.as_ref()?.place.as_deref()
    }
}
