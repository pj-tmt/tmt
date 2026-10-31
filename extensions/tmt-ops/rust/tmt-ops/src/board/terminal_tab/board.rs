//! What the board does for the Terminal tab: which member it is about, when core is asked where
//! the stream is, and what the loop keeps open. The state itself is [`Tab`](super::tab::Tab).
use super::{discovery::Located, stream::StreamEvent};
use crate::{
    action::{OpsUiAction, Verb},
    board::{
        app::{App, CopyFeedback, Effect, Request, event_name},
        shell::{OpsTab, RightTab},
    },
};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::Position,
};
use std::time::Duration;
use tmt_tui::components::terminal::{MouseMode, is_leave, key_bytes, paste_bytes, wheel_bytes};

impl App {
    /// After a draw: a program that hides its cursor while typing still has the real cursor
    /// moved to where it parked its own, kept hidden, so an input method draws there.
    pub(in crate::board) fn park_hidden_caret<B: ratatui::backend::Backend>(
        &self,
        screen: &mut ratatui::Terminal<B>,
    ) -> Result<(), B::Error> {
        match self.terminal.caret() {
            Some(caret) if !caret.shown => {
                let backend = screen.backend_mut();
                backend.set_cursor_position(caret.at)?;
                backend.flush()
            }
            _ => Ok(()),
        }
    }

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
        let id = self.terminal_identity()?;
        if !self.terminal.shown_for(&id) && !self.scrolls.visible(crate::config::Pane::Replies) {
            return None;
        }
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

    /// `j` on a built-in member: its own screen is its Terminal view, not a host pane, and `j`
    /// again returns to the chat. `None` for any other member, which keeps the host jump.
    pub(in crate::board) fn jump_to_terminal(&mut self, row: &serde_json::Value) -> Option<Effect> {
        if self.terminal_available() && self.terminal_identity().as_deref() == row["id"].as_str() {
            // A second `j` leads back to the chat, so the keyboard alone moves both ways.
            return Some(self.show_terminal(!self.terminal_showing()));
        }
        (row["pane"]["location"]["host"] == crate::board::new_member::model::BUILT_IN_HOST).then(|| {
            self.say("This built-in member's terminal is not available yet; select it on the Agents tab.")
        })
    }

    /// The `terminal` action (no default key): the chat and the terminal swap places in the
    /// Talk pane. Without a stream it does nothing, and says nothing.
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
        let selected = self.terminal_identity();
        if let Some(id) = &selected {
            self.terminal.selected(id);
        }
        let wanted = selected
            .filter(|id| self.terminal.showing(id))
            .and_then(|id| {
                let address = self.terminal.address(&id)?.clone();
                Some((id, address))
            });
        let changed = self
            .terminal
            .sync(wanted.as_ref().map(|(id, address)| (id.as_str(), address)));
        self.terminal.fit_window();
        changed
    }

    /// The selected member's name, for the lines under the screen.
    pub(in crate::board) fn terminal_member_name(&self) -> String {
        self.conversation_target()
            .and_then(|target| {
                self.target_row(&target)?["name"]
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "agent".to_owned())
    }

    /// `⏎` on a shown terminal: keys go to the program until Ctrl-\. After the stream ended it
    /// attaches again instead.
    pub(in crate::board) fn type_into_terminal(&mut self) -> Effect {
        if self
            .terminal_view()
            .is_some_and(|view| view.condition.ended())
        {
            self.terminal.reattach();
        } else {
            self.terminal.start_typing();
        }
        Effect::None
    }

    /// The tab fills the board and back. Only a shown terminal can fill it.
    pub(in crate::board) fn toggle_terminal_zoom(&mut self) -> Effect {
        let zoom = !self.terminal.zoomed() && self.terminal_showing();
        self.terminal.set_zoomed(zoom);
        self.invalidate_frame_geometry();
        Effect::None
    }

    pub(in crate::board) fn terminal_zoomed(&self) -> bool {
        self.terminal.zoomed() && self.terminal_showing()
    }

    /// The key is bound to `terminal-zoom`; with `authored` only a binding the user wrote
    /// counts, since the default `z` is a key the program may need while typing.
    fn zoom_key(&self, key: KeyEvent, authored: bool) -> bool {
        let Some(name) = event_name(key) else {
            return false;
        };
        self.workspace_bindings()
            .get(&name)
            .is_some_and(|action| action.verb == Verb::Ui(OpsUiAction::TerminalZoom))
            && (!authored
                || self
                    .workspace_binding_sources()
                    .and_then(|sources| sources.get(&name))
                    .is_some_and(|source| source != "workspace default"))
    }

    /// While typing, every key but the way out and an authored zoom key is the program's, byte
    /// for byte. While zoomed and not typing, only the way out of zoom and `⏎` mean anything.
    pub(in crate::board) fn terminal_key(&mut self, key: KeyEvent) -> Option<Effect> {
        if self.modal_open() {
            return None;
        }
        if self.terminal.typing() {
            if is_leave(&key) {
                self.terminal.stop_typing();
            } else if self.zoom_key(key, true) {
                return Some(self.toggle_terminal_zoom());
            } else if let Some(bytes) = key_bytes(&key, self.terminal.modes()) {
                self.terminal.send(&bytes);
            }
            return Some(Effect::None);
        }
        if !self.terminal_zoomed() {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return None;
        }
        match key.code {
            KeyCode::Enter => return Some(self.type_into_terminal()),
            KeyCode::Esc => return Some(self.toggle_terminal_zoom()),
            _ if self.zoom_key(key, false) => return Some(self.toggle_terminal_zoom()),
            _ => {}
        }
        Some(Effect::None)
    }

    /// A left drag over a shown screen selects its text and the release copies it, unless the
    /// program is being typed into and asked for the mouse itself. While typing, the wheel over
    /// the screen is the program's and a press elsewhere ends typing (and then lands as usual).
    pub(in crate::board) fn terminal_mouse(&mut self, event: MouseEvent) -> Option<Effect> {
        if self.modal_open() || !self.terminal_showing() {
            return None;
        }
        let area = self.terminal.area()?;
        let typing = self.terminal.typing();
        if !area.contains(Position::new(event.column, event.row)) {
            if typing && matches!(event.kind, MouseEventKind::Down(_)) {
                self.terminal.stop_typing();
            }
            return None;
        }
        let at = (event.column - area.x, event.row - area.y);
        let program_mouse = typing && self.terminal.modes().mouse != MouseMode::Off;
        match event.kind {
            // The press only anchors a selection; it still lands on the board (focus, menus).
            MouseEventKind::Down(MouseButton::Left) if !program_mouse => {
                self.terminal.select_from(at);
                if !typing {
                    return None;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if !program_mouse => {
                self.terminal.select_to(at);
                if !typing && !self.terminal.dragged() {
                    return None;
                }
            }
            MouseEventKind::Up(MouseButton::Left) if !program_mouse => {
                self.terminal.select_to(at);
                if let Some(text) = self.terminal.selected_text() {
                    return Some(Effect::Act(Request::Copy {
                        text,
                        program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
                        feedback: CopyFeedback::Selection,
                    }));
                }
                if !typing {
                    return None;
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if typing => {
                if let Some(bytes) = wheel_bytes(
                    event.kind == MouseEventKind::ScrollUp,
                    at.0,
                    at.1,
                    self.terminal.modes(),
                ) {
                    self.terminal.send(&bytes);
                }
            }
            // Watching, anything else over the screen is the board's as before.
            _ if !typing => return None,
            _ => {}
        }
        Some(Effect::None)
    }

    /// While typing, a paste is the program's.
    pub(in crate::board) fn terminal_paste(&mut self, text: &str) -> Option<Effect> {
        if !self.terminal.typing() || self.modal_open() {
            return None;
        }
        self.terminal
            .send(&paste_bytes(text, self.terminal.modes()));
        Some(Effect::None)
    }

    pub(in crate::board) fn apply_terminal(&mut self, epoch: u64, event: StreamEvent) -> bool {
        self.terminal.apply(epoch, event)
    }
}
