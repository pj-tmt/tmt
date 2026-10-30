//! The board's confirm dialog: a question, a few lines about it and Cancel / the action. Choosing
//! the action applies its `Choice`; everything else closes it and changes nothing. The cursor
//! starts on Cancel, so a stray Enter never applies a destructive action.
use super::app::{App, Choice, Effect, Overlay};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode},
    layout::Rect,
    text::Line,
};
use std::cell::RefCell;
use tmt_cli_style::Role;
use tmt_tui::components::{
    Button, Dialog, DialogHits, DialogInput, DialogKind, DialogState, Focus, Placement, strip,
};

pub(super) struct Confirm {
    title: String,
    lines: Vec<String>,
    action: String,
    choice: Choice,
    state: DialogState,
    hits: RefCell<DialogHits>,
}

impl Confirm {
    #[cfg(test)]
    pub(super) fn title(&self) -> &str {
        &self.title
    }

    pub(super) fn invalidate(&self) {
        *self.hits.borrow_mut() = DialogHits::default();
    }

    pub(super) fn render(&self, frame: &mut Frame, app: &App, body: Rect) {
        let look = app.look();
        let dialog = Dialog {
            title: self.title.clone(),
            kind: DialogKind::Confirm,
            placement: Placement::Center,
            buttons: buttons(&self.action).into(),
        };
        let width = self
            .lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or_default()
            .max(self.title.chars().count() + 4)
            .saturating_add(4)
            .max(40);
        let height = self.lines.len().saturating_add(5);
        let areas = dialog.areas(
            body,
            [
                u16::try_from(width).unwrap_or(u16::MAX),
                u16::try_from(height).unwrap_or(u16::MAX),
            ],
            false,
        );
        let hits = super::buttons::paint_dialog(
            app,
            frame.buffer_mut(),
            &dialog,
            areas,
            &self.state,
            Overlay::Confirm,
        );
        app.peek_areas.set(Overlay::Confirm, Some(areas.outer));
        let content = areas.content;
        for (row, line) in self
            .lines
            .iter()
            .take(usize::from(content.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(content.x, content.y + row as u16, content.width, 1),
                Line::styled(
                    super::view::fit(line, usize::from(content.width)),
                    look.role(Role::Text),
                ),
            );
        }
        *self.hits.borrow_mut() = hits;
    }
}

impl App {
    /// Asks before `choice`: `action` names the button that applies it.
    pub(super) fn confirm(
        &mut self,
        title: String,
        lines: Vec<String>,
        action: String,
        choice: Choice,
    ) -> Effect {
        self.menu = None;
        self.confirm = Some(Confirm {
            title,
            lines,
            action,
            choice,
            state: DialogState {
                focus: Focus::Buttons(0),
            },
            hits: RefCell::default(),
        });
        Effect::None
    }

    pub(super) fn confirm_event(&mut self, event: &Event) -> Option<Effect> {
        let confirm = self.confirm.as_mut()?;
        let action = confirm.action.clone();
        let buttons = buttons(&action);
        let input = match event {
            Event::Key(key) => confirm.state.key(DialogKind::Confirm, key, &[], &buttons),
            Event::Mouse(mouse) => {
                let hits = confirm.hits.borrow().clone();
                confirm.state.mouse(mouse, &hits, &buttons)
            }
            _ => DialogInput::Ignored,
        };
        Some(match input {
            DialogInput::Cancel | DialogInput::Choose(0) => self.close_overlay(Overlay::Confirm),
            DialogInput::Choose(_) => {
                let confirm = self.confirm.take().expect("open confirm");
                // Only a painted button applies, never one from a stale or clipped frame.
                if confirm
                    .hits
                    .borrow()
                    .buttons
                    .get(1)
                    .copied()
                    .flatten()
                    .is_none()
                {
                    self.confirm = Some(confirm);
                    return Some(Effect::None);
                }
                self.choose(confirm.choice)
            }
            // The letters the menu this replaced used: n keeps, y applies.
            _ if matches!(event, Event::Key(key) if key.code == KeyCode::Char('n')) => {
                self.close_overlay(Overlay::Confirm)
            }
            _ if matches!(event, Event::Key(key) if key.code == KeyCode::Char('y')) => {
                let confirm = self.confirm.take().expect("open confirm");
                self.choose(confirm.choice)
            }
            _ => Effect::None,
        })
    }
}

fn buttons(action: &str) -> [Button<'_>; 2] {
    [Button::secondary("Cancel"), Button::primary(action)]
}

#[cfg(test)]
mod tests;
