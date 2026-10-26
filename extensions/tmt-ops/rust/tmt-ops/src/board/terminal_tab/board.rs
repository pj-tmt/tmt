//! What the board does for the Terminal tab: which member it is about, when core is asked where
//! the stream is, and what the loop keeps open. The state itself is [`Tab`](super::tab::Tab).
use super::{discovery::Located, stream::StreamEvent};
use crate::board::{
    app::{App, Effect},
    shell::{OpsTab, RightTab},
};
use std::time::Duration;

impl App {
    /// The member the Terminal tab is about: the selected agent while the Agents tab shows Talk.
    fn terminal_identity(&self) -> Option<String> {
        if self.shell.tab != OpsTab::Agents || self.right_tab != RightTab::Talk {
            return None;
        }
        let target = self.conversation_target()?;
        let id = self.target_row(&target)?["id"].as_str()?;
        crate::board::recent_responses::canonical_identity(id).then(|| id.to_owned())
    }

    /// What the open stream shows, when it is the selected member's.
    pub(in crate::board) fn terminal_view(&self) -> Option<&super::tab::View> {
        self.terminal.view(&self.terminal_identity()?)
    }

    /// The selected member has a stream, so the tab can be shown.
    pub(in crate::board) fn terminal_available(&self) -> bool {
        self.terminal_identity()
            .is_some_and(|id| self.terminal.address(&id).is_some())
    }

    /// The Terminal tab, not the chat, is what the Talk pane shows.
    pub(in crate::board) fn terminal_showing(&self) -> bool {
        self.terminal_identity()
            .is_some_and(|id| self.terminal.showing(&id))
    }

    /// The member core should be asked about next, when it has not answered or its answer is old.
    pub(in crate::board) fn terminal_locate(&self) -> Option<String> {
        // Like every background read, only for a pane that is on screen; a shown terminal is.
        if !self.terminal.shown() && !self.scrolls.visible(crate::config::Pane::Replies) {
            return None;
        }
        let id = self.terminal_identity()?;
        self.terminal
            .needs_locating(&id, crate::status::now_ms())
            .then_some(id)
    }

    /// How long until the selected member's answer is old enough to ask again.
    pub(in crate::board) fn terminal_wait(&self) -> Option<Duration> {
        let id = self.terminal_identity()?;
        self.terminal.locating_wait(&id, crate::status::now_ms())
    }

    pub(in crate::board) fn apply_terminal_located(&mut self, identity: &str, located: Located) {
        self.terminal
            .located(identity, located, crate::status::now_ms());
    }

    /// `t`: the chat and the terminal swap places in the Talk pane. Without a stream it does
    /// nothing, and says nothing.
    pub(in crate::board) fn toggle_terminal(&mut self) -> Effect {
        if let Some(id) = self.terminal_identity()
            && self.terminal.address(&id).is_some()
        {
            self.terminal.toggle(&id);
        }
        Effect::None
    }

    /// A click on a chip: the terminal when `show`, the chat otherwise.
    pub(in crate::board) fn show_terminal(&mut self, show: bool) -> Effect {
        if self.terminal_showing() != show {
            return self.toggle_terminal();
        }
        Effect::None
    }

    /// Opens, replaces or drops the one stream so it matches what is shown. True when it changed.
    pub(in crate::board) fn terminal_sync(&mut self) -> bool {
        let wanted = self
            .terminal_identity()
            .filter(|id| self.terminal.showing(id))
            .and_then(|id| {
                let address = self.terminal.address(&id)?.clone();
                Some((id, address))
            });
        self.terminal
            .sync(wanted.as_ref().map(|(id, address)| (id.as_str(), address)))
    }

    pub(in crate::board) fn apply_terminal(&mut self, epoch: u64, event: StreamEvent) -> bool {
        self.terminal.apply(epoch, event)
    }
}
