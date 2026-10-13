//! Plain-text reference completion over acquired board data. No lookup performs
//! IO and choosing a candidate never changes the composer's recipient.
use super::app::{App, Compose, Effect, MESSAGE_LIMIT};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    text::{Line, Span},
};
use std::cell::RefCell;
use tmt_cli_style::Role;
use tmt_tui::components::{Completion, Editor, strip};

#[derive(Default)]
pub(super) struct State {
    pub completion: Completion,
    hits: RefCell<Vec<(Rect, usize)>>,
}

impl State {
    pub(super) fn invalidate(&self) {
        self.hits.borrow_mut().clear();
    }
}

impl App {
    pub(super) fn reference_editor(&self) -> Option<&Editor> {
        if matches!(self.input.as_ref()?.compose, Compose::Status) {
            self.status_draft
                .as_ref()
                .filter(|draft| draft.editable_reason())
                .map(|draft| &draft.reason)
        } else {
            Some(&self.input.as_ref()?.text)
        }
    }
    pub(super) fn reference_editor_mut(&mut self) -> Option<&mut Editor> {
        if matches!(self.input.as_ref()?.compose, Compose::Status) {
            self.status_draft
                .as_mut()
                .filter(|draft| draft.editable_reason())
                .map(|draft| &mut draft.reason)
        } else {
            Some(&mut self.input.as_mut()?.text)
        }
    }
    fn accept_reference(&mut self) -> bool {
        let mut completion = std::mem::take(&mut self.mentions.completion);
        let limit = if self
            .input
            .as_ref()
            .is_some_and(|input| matches!(input.compose, Compose::Status))
        {
            super::app::INPUT_LIMIT
        } else {
            MESSAGE_LIMIT
        };
        let accepted = self
            .reference_editor_mut()
            .is_some_and(|editor| completion.accept(editor, limit));
        self.mentions.completion = completion;
        accepted
    }
    pub(super) fn message_field(&self) -> bool {
        self.input
            .as_ref()
            .is_some_and(|input| match input.compose {
                Compose::Talk { .. }
                | Compose::Reply { .. }
                | Compose::Annotate { .. }
                | Compose::AskLead { .. }
                | Compose::Leads { .. } => true,
                Compose::Status => self.reference_editor().is_some(),
                Compose::Cron => self
                    .cron_draft
                    .as_ref()
                    .is_some_and(|draft| draft.message_field()),
                _ => false,
            })
    }
    pub(super) fn update_mentions(&mut self) {
        if !self.message_field() {
            self.mentions = State::default();
            return;
        }
        let mut completion = std::mem::take(&mut self.mentions.completion);
        if let Some(editor) = self.reference_editor() {
            completion.update(editor, &self.references.candidates);
        }
        self.mentions.completion = completion;
        self.mentions.hits.borrow_mut().clear();
    }
    pub(super) fn mention_key(&mut self, key: KeyEvent) -> Option<Effect> {
        if !self.mentions.completion.open() {
            return None;
        }
        match key.code {
            KeyCode::Up => self.mentions.completion.move_by(-1),
            KeyCode::Down => self.mentions.completion.move_by(1),
            KeyCode::PageUp => self.mentions.completion.move_by(-6),
            KeyCode::PageDown => self.mentions.completion.move_by(6),
            KeyCode::Enter | KeyCode::Tab => {
                self.accept_reference();
            }
            KeyCode::Esc => {
                let mut completion = std::mem::take(&mut self.mentions.completion);
                if let Some(editor) = self.reference_editor() {
                    completion.dismiss(editor);
                }
                self.mentions.completion = completion;
            }
            _ => return None,
        }
        Some(Effect::None)
    }
    pub(super) fn mention_mouse(&mut self, event: MouseEvent) -> bool {
        if !self.mentions.completion.open() {
            return false;
        }
        let hit = self
            .mentions
            .hits
            .borrow()
            .iter()
            .find(|(rect, _)| rect.contains((event.column, event.row).into()))
            .map(|(_, index)| *index);
        let Some(index) = hit else {
            return false;
        };
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.mentions.completion.selected = index;
                self.accept_reference();
            }
            MouseEventKind::ScrollUp => self.mentions.completion.move_by(-1),
            MouseEventKind::ScrollDown => self.mentions.completion.move_by(1),
            _ => {}
        }
        true
    }
}

pub(super) fn render(frame: &mut Frame, app: &App) {
    app.mentions.hits.borrow_mut().clear();
    let completion = &app.mentions.completion;
    if !completion.open() {
        return;
    }
    let Some((editor, _)) = app.input_viewport.get() else {
        return;
    };
    let screen = frame.area();
    let count = completion.matches.len().clamp(1, 6);
    let height = (count as u16 + 1).min(screen.height);
    let y = if editor.y.saturating_sub(screen.y) >= height {
        editor.y - height
    } else {
        editor.bottom().min(screen.bottom().saturating_sub(height))
    };
    let width = screen.width.min(68);
    let x = editor.x.min(screen.right().saturating_sub(width));
    let look = app.look();
    let start = completion.selected.saturating_sub(count - 1);
    for row in 0..height {
        let rect = Rect::new(x, y + row, width, 1);
        let line = if row == height - 1 {
            Line::styled(
                "Mentions are text only · ↑↓ choose · Enter insert · Esc close",
                look.role(Role::Muted),
            )
        } else if let Some(candidate) = completion.matches.get(start + usize::from(row)) {
            let selected = start + usize::from(row) == completion.selected;
            let style = if selected {
                look.role(Role::Accent)
                    .add_modifier(ratatui::style::Modifier::REVERSED)
            } else {
                look.role(Role::Text)
            };
            app.mentions
                .hits
                .borrow_mut()
                .push((rect, start + usize::from(row)));
            Line::from(vec![Span::styled(
                format!(
                    "{} {:5}  {}",
                    if selected { "›" } else { " " },
                    candidate.kind,
                    candidate.label
                ),
                style,
            )])
        } else {
            Line::styled(
                if app.references.error.is_some() {
                    "Agent inventory unavailable; squad references only"
                } else {
                    "No matching agent or squad"
                },
                look.role(Role::Muted),
            )
        };
        // Explicit opaque cell fill prevents underlying pane text leaking into
        // short candidates; strip owns escaping and cell clipping.
        for col in rect.x..rect.right() {
            frame.buffer_mut()[(col, rect.y)].reset();
        }
        strip::paint_left(frame.buffer_mut(), rect, line);
    }
}

#[cfg(test)]
mod tests;
