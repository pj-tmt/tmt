//! Fold controls have no send authority and yield to editors and authored keys.
use super::*;
use crate::board::app::Effect;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

impl App {
    pub(in crate::board) fn invalidate_body_geometry(&self) {
        self.conversation.borrow_mut().fold_hits.clear();
    }

    pub(in crate::board) fn conversation_body_key(&mut self, event: KeyEvent) -> Option<Effect> {
        // Only Tab enters fold focus. Ordinary reader navigation must reach
        // its owner without resolving the workspace's scoped binding context.
        let entering = matches!(event.code, KeyCode::Tab | KeyCode::BackTab);
        if !entering
            && !matches!(
                event.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Enter | KeyCode::Esc
            )
        {
            return None;
        }
        {
            let state = self.conversation.borrow();
            let focused = state
                .active
                .as_ref()
                .and_then(|active| state.sessions.get(active)?.folds.focused.as_ref());
            if focused.is_none() && (!entering || state.fold_hits.is_empty()) {
                return None;
            }
        }
        if self.input.is_some()
            || self.searching
            || self.modal_open()
            || self.focused() != Pane::Replies
            || event.kind == KeyEventKind::Release
            || !matches!(event.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
            || crate::board::app::event_name(event).is_some_and(|key| {
                self.workspace_binding_sources()
                    .and_then(|sources| sources.get(&key).cloned())
                    .is_some_and(|source| source != "workspace default")
            })
        {
            return None;
        }
        let (expected, _) = self.receiving_conversation()?;
        let mut state = self.conversation.borrow_mut();
        let active = state.active.clone()?;
        if active != expected {
            return None;
        }
        let mut keys: Vec<_> = state
            .fold_hits
            .iter()
            .filter(|hit| hit.conversation == active)
            .map(|hit| hit.key.clone())
            .collect();
        let session = state.sessions.get_mut(&active)?;
        if session.exact_source.is_some() {
            return None;
        }
        // Relative key bursts may arrive before repaint. Retained keyboard
        // focus has no pointer authority and must not fall through to Send.
        if keys.is_empty()
            && let Some(key) = &session.folds.focused
        {
            keys.push(key.clone());
        }
        if keys.is_empty() {
            return None;
        }
        let index = session
            .folds
            .focused
            .as_ref()
            .and_then(|key| keys.iter().position(|candidate| candidate == key));
        match event.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let previous =
                    event.code == KeyCode::BackTab || event.modifiers == KeyModifiers::SHIFT;
                let next = index.map_or(if previous { keys.len() - 1 } else { 0 }, |index| {
                    if previous {
                        (index + keys.len() - 1) % keys.len()
                    } else {
                        (index + 1) % keys.len()
                    }
                });
                session.folds.focused = Some(keys[next].clone());
            }
            KeyCode::Up | KeyCode::Down if index.is_some() && event.modifiers.is_empty() => {
                let index = index.unwrap();
                let next = if event.code == KeyCode::Up {
                    (index + keys.len() - 1) % keys.len()
                } else {
                    (index + 1) % keys.len()
                };
                session.folds.focused = Some(keys[next].clone());
            }
            KeyCode::Enter if index.is_some() && event.modifiers.is_empty() => {
                session.folds.toggle(&keys[index.unwrap()]);
                session.follow = false;
            }
            KeyCode::Esc if index.is_some() => session.folds.focused = None,
            _ => return None,
        }
        session.rendered = None;
        drop(state);
        self.invalidate_frame_geometry();
        Some(Effect::None)
    }

    pub(in crate::board) fn conversation_body_mouse(
        &mut self,
        event: MouseEvent,
    ) -> Option<Effect> {
        if event.kind != MouseEventKind::Down(MouseButton::Left)
            || self.modal_open()
            || self.searching
        {
            return None;
        }
        let (expected, _) = self.receiving_conversation()?;
        let mut state = self.conversation.borrow_mut();
        let active = state.active.clone()?;
        if active != expected {
            return None;
        }
        let hit = state
            .fold_hits
            .iter()
            .find(|hit| {
                hit.conversation == active && hit.area.contains((event.column, event.row).into())
            })
            .cloned()?;
        let session = state.sessions.get_mut(&active)?;
        if session.exact_source.is_some() || !session.folds.toggle(&hit.key) {
            return None;
        }
        session.folds.focused = Some(hit.key);
        session.follow = false;
        session.rendered = None;
        drop(state);
        self.invalidate_frame_geometry();
        Some(Effect::None)
    }
}
