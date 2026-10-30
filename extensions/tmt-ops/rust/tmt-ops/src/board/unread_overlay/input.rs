//! Overlay-local selection, focus and explicit read actions; effects stay with App's writer.
use super::{Answer, Overlay};
use crate::board::app::{App, Effect};
use crate::board::unread::publish::Seen;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
use std::collections::BTreeSet;
use tmt_tui::components::{ListEvent, Step};

pub(super) struct Confirmation {
    actor: String,
    members: BTreeSet<String>,
    answers: Vec<Answer>,
    before: Option<serde_json::Value>,
}

impl Confirmation {
    pub(super) fn members(&self) -> &BTreeSet<String> {
        &self.members
    }
}

enum Input {
    None,
    Close,
    Chat(String),
    Mark(Seen),
    Unread(Seen),
    All(Confirmation),
}

impl Overlay {
    pub(in crate::board) fn pointer(&self, at: Option<ratatui::layout::Position>) -> bool {
        let next = at.filter(|_| self.confirmation.is_none()).and_then(|at| {
            let frame = self.list_frame.borrow();
            let list = self.list.borrow();
            frame
                .as_ref()
                .filter(|frame| frame.rows == list.rows() && frame.offset == list.scroll.offset())
                .and_then(|frame| frame.geometry.iter().find(|row| row.visible.contains(at)))
                .and_then(|row| row.id.strip_prefix("answer-")?.parse().ok())
        });
        next != self.hover.replace(next)
    }

    fn selection_changed(&mut self) {
        self.scroll.borrow_mut().step(Step::Top);
        self.painted.borrow_mut().clear();
        self.hover.set(None);
        self.bump();
    }

    fn confirmation_input(&mut self, event: &Event) -> Input {
        let confirm = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(false),
                KeyCode::Enter
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    Some(true)
                }
                _ => None,
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let at = (mouse.column, mouse.row).into();
                self.confirm_hits.get().and_then(|(yes, no)| {
                    if yes.contains(at) {
                        Some(true)
                    } else if no.contains(at) {
                        Some(false)
                    } else {
                        None
                    }
                })
            }
            _ => None,
        };
        match confirm {
            Some(false) => {
                self.confirmation = None;
                self.confirm_hits.set(None);
                self.painted.borrow_mut().clear();
                Input::None
            }
            Some(true) if self.confirm_hits.get().is_some() => {
                let confirmation = self.confirmation.take().expect("shown confirmation");
                self.confirm_hits.set(None);
                self.painted.borrow_mut().clear();
                Input::All(confirmation)
            }
            _ => Input::None,
        }
    }

    fn input(&mut self, event: &Event, local: &crate::board::unread::local::State) -> Input {
        if self.confirmation.is_some() {
            return self.confirmation_input(event);
        }
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Esc => {
                    if self.narrow.get() && self.detail_open {
                        self.detail_open = false;
                        self.detail_focus = false;
                        self.painted.borrow_mut().clear();
                        return Input::None;
                    }
                    return Input::Close;
                }
                KeyCode::Char('q') => return Input::Close,
                KeyCode::Enter => {
                    if self.narrow.get() && !self.detail_open {
                        self.detail_open = true;
                        self.detail_focus = true;
                        self.selection_changed();
                        return Input::None;
                    }
                    return self
                        .selected_answer()
                        .map_or(Input::None, |answer| Input::Chat(answer.member.clone()));
                }
                KeyCode::Tab => {
                    self.detail_focus = !self.detail_focus;
                    return Input::None;
                }
                KeyCode::BackTab => {
                    self.detail_focus = false;
                    return Input::None;
                }
                KeyCode::Char('r') => {
                    if self.page_failed {
                        self.page_failed = false;
                        return Input::None;
                    }
                    let Some(answer) = self.selected_answer().cloned() else {
                        return Input::None;
                    };
                    if self.failed.contains_key(&answer.member) {
                        self.failed.remove(&answer.member);
                        self.asked.remove(&answer.member);
                        self.pages.remove(&answer.member);
                        self.bump();
                        return Input::None;
                    }
                    return match &self.actor {
                        Some(actor) => {
                            let marked = local.contains(actor, &answer);
                            let read = answer.acknowledged && !marked;
                            let seen = Seen {
                                actor: actor.clone(),
                                answer,
                            };
                            if read {
                                Input::Unread(seen)
                            } else {
                                Input::Mark(seen)
                            }
                        }
                        None => Input::None,
                    };
                }
                KeyCode::Char('a') => {
                    if let Some(actor) = &self.actor {
                        self.confirmation = Some(Confirmation {
                            actor: actor.clone(),
                            members: self.members.iter().cloned().collect(),
                            answers: self
                                .items
                                .values()
                                .filter(|answer| {
                                    !answer.acknowledged || local.contains(actor, answer)
                                })
                                .cloned()
                                .collect(),
                            before: self.next_before.clone(),
                        });
                        self.confirm_hits.set(None);
                        self.painted.borrow_mut().clear();
                    }
                    return Input::None;
                }
                _ => {}
            }
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let at = (mouse.column, mouse.row).into();
            if self
                .list_frame
                .borrow()
                .as_ref()
                .is_some_and(|frame| frame.viewport.contains(at))
            {
                self.detail_focus = false;
            } else if self.detail_visible() && self.scroll.borrow().viewport().contains(at) {
                self.detail_focus = true;
            }
        }
        let detail_input = self.detail_visible()
            && ((self.narrow.get() && self.detail_open)
                || self.detail_focus
                || matches!(event, Event::Mouse(mouse) if self.scroll.borrow().viewport().contains((mouse.column,mouse.row).into())));
        if detail_input && self.scroll.borrow_mut().input(event) {
            return Input::None;
        }
        let selected = self
            .list_frame
            .borrow()
            .as_ref()
            .cloned()
            .and_then(|frame| self.list.borrow_mut().input(event, &frame));
        if matches!(selected, Some(ListEvent::Changed(_))) {
            self.selection_changed();
        }
        Input::None
    }
}

impl App {
    pub(in crate::board) fn unread_overlay_event(&mut self, event: &Event) -> Option<Effect> {
        let actor = self.view.as_ref().and_then(|view| view.me_id.as_deref());
        let overlay = self.unread_overlay.as_mut()?;
        if overlay.actor.as_deref() != actor {
            return Some(Effect::None);
        }
        Some(match overlay.input(event, &self.unread.local) {
            Input::None => Effect::None,
            Input::Close => {
                self.unread_overlay = None;
                self.unread.displayed.clear();
                Effect::None
            }
            Input::Chat(member) => {
                self.unread_overlay = None;
                self.unread.displayed.clear();
                self.open_member_conversation(&member)
            }
            Input::Mark(seen) => {
                self.explicit_answers_read(&seen.actor, vec![seen.answer]);
                Effect::None
            }
            Input::Unread(seen) => match self.unread.local.mark(&seen.actor, &seen.answer) {
                Ok(()) => {
                    self.local_unread_changed();
                    Effect::None
                }
                Err(error) => self.say(error),
            },
            Input::All(confirmation) => {
                if self.view.as_ref().and_then(|view| view.me_id.as_deref())
                    != Some(&confirmation.actor)
                {
                    return Some(Effect::None);
                }
                self.explicit_answers_read(&confirmation.actor, confirmation.answers);
                if let Some(before) = confirmation.before {
                    self.unread.writer.mark_scope(
                        &confirmation.actor,
                        confirmation.members,
                        before,
                    );
                }
                Effect::None
            }
        })
    }
}
