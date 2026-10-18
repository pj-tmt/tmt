//! Exact source selection and quote provenance. Display escaping never changes
//! stored byte ranges; selecting or attaching a quote acknowledges no request.
use super::home_leads::MessageKey;
use std::ops::Range;
use tmt_tui::components::Editor;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Quote {
    pub key: MessageKey,
    pub range: Range<usize>,
    pub text: String,
}
pub(super) struct Selection {
    pub key: MessageKey,
    pub source: Editor,
}
impl Selection {
    pub fn new(key: MessageKey, source: String) -> Self {
        let mut source = Editor::from(source);
        source.key(
            ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Home,
                ratatui::crossterm::event::KeyModifiers::CONTROL,
            ),
            80,
            usize::MAX,
        );
        Self { key, source }
    }
    pub fn quote(&self) -> Option<Quote> {
        let range = self.source.selection();
        (!range.is_empty()).then(|| Quote {
            key: self.key.clone(),
            text: self.source.as_str()[range.clone()].to_owned(),
            range,
        })
    }
}

/// The caller owns its source surface. Opening a composer remains observable
/// even when a retained draft already uses the complete attachment budget.
pub(super) enum Attachment {
    Composer(super::app::Effect),
    Opened {
        effect: super::app::Effect,
        reason: &'static str,
    },
    Refused(&'static str),
    Unavailable,
}
impl Attachment {
    pub(super) fn finish(self, app: &mut super::app::App) -> super::app::Effect {
        match self {
            Self::Composer(effect) => effect,
            Self::Opened { effect, reason } => {
                app.set_notice(Some(reason.into()), false);
                effect
            }
            Self::Refused(reason) => app.say(reason),
            Self::Unavailable => super::app::Effect::None,
        }
    }
}

impl super::app::App {
    pub(super) fn response_quote_current(
        &self,
        target: &super::row_detail::Target,
        quote: &Quote,
    ) -> bool {
        let reply = if quote.key.kind == super::home_leads::Kind::Question {
            let super::row_detail::Target::Row(target) = target else {
                return false;
            };
            // Compact placement can move the composer to Agents. The held
            // request, not that tab's displayed response, owns this source.
            self.pending_question(target, Some(&quote.key.request))
        } else {
            self.detail(target).and_then(|detail| detail.reply)
        };
        reply.is_some_and(|reply| {
            reply.acquired
                && reply.source
                && reply
                    .key
                    .as_ref()
                    .is_some_and(|key| key.same_source(&quote.key))
                && reply.body.get(quote.range.clone()) == Some(quote.text.as_str())
        })
    }

    pub(super) fn attach_response_quote(
        &mut self,
        target: super::row_detail::Target,
        quote: Quote,
    ) -> Attachment {
        use super::app::RowTarget;
        if !self.response_quote_current(&target, &quote) {
            return Attachment::Refused(
                "The response source changed; reopen it before annotating.",
            );
        }
        let super::row_detail::Target::Row(target) = target else {
            return Attachment::Unavailable;
        };
        let squad = match &target {
            RowTarget::Workspace(_) => self.target_squad(&target).unwrap_or_default(),
            RowTarget::Member { squad, .. } | RowTarget::Lead { squad, .. } => squad.clone(),
        };
        let retained = self
            .input
            .as_ref()
            .filter(|input| input.target() == Some(&target));
        let effect = if let Some(input) = retained {
            if input
                .row_send
                .as_ref()
                .is_none_or(|send| !send.valid(self, input))
            {
                return Attachment::Refused(
                    "The row target, sender, lead or request changed; quote not attached.",
                );
            }
            // Attach to the admitted composer, without moving a draft to a
            // different recipient just because its source belongs to this row.
            self.workspace_focus_conversation();
            super::app::Effect::None
        } else {
            let Some(send) = self.send_target(target, !squad.is_empty()) else {
                return Attachment::Refused(
                    "The agent is no longer available; quote not attached.",
                );
            };
            // A quote grants no squad membership. Common pool identities keep
            // the existing explicit Talk follow-up to the same UUID.
            let verb = if squad.is_empty() {
                crate::action::Verb::Talk
            } else {
                crate::action::Verb::Annotate
            };
            self.compose_row(send, verb, squad)
        };
        if let Some(input) = &mut self.input {
            let bytes = input
                .response_quotes
                .iter()
                .map(|quote| quote.text.len())
                .sum::<usize>();
            if bytes.saturating_add(quote.text.len()) > super::app::MESSAGE_LIMIT {
                return Attachment::Opened {
                    effect,
                    reason: "Quote exceeds the 1 MiB attachment limit; not attached.",
                };
            } else {
                input.response_quotes.push(quote);
            }
        }
        Attachment::Composer(effect)
    }

    pub(super) fn reader_event(
        &mut self,
        event: &ratatui::crossterm::event::Event,
    ) -> super::app::Effect {
        use super::app::{Effect, Request};
        use ratatui::crossterm::event::{
            Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
        };
        if matches!(event, Event::Key(key) if matches!(key.code, KeyCode::Esc | KeyCode::Char('q' | 'v')))
        {
            self.row_details.reader = None;
            self.reconcile_row_details();
            return Effect::None;
        }
        let Some(target) = self
            .row_details
            .reader
            .as_ref()
            .map(|reader| reader.target.clone())
        else {
            return Effect::None;
        };
        if matches!(event, Event::Key(key) if key.code == KeyCode::Char('s') && key.modifiers.is_empty())
        {
            let reply = self
                .reader_reply()
                .filter(|reply| reply.acquired && reply.source);
            if let Some(reply) = reply
                && let Some(key) = reply.key.clone()
            {
                let reader = self.row_details.reader.as_mut().unwrap();
                reader.selection = Some(Selection::new(key, reply.body.clone()));
                reader.frozen = Some(reply);
                *reader.scroll.borrow_mut() = Default::default();
            } else {
                self.set_notice(
                    Some("Wait for the exact response source before selecting.".into()),
                    false,
                );
            }
            return Effect::None;
        }
        let quote = self
            .row_details
            .reader
            .as_ref()
            .and_then(|reader| reader.selection.as_ref())
            .and_then(Selection::quote);
        if matches!(event, Event::Key(key) if key.code == KeyCode::Char('y') && key.modifiers.is_empty())
        {
            return quote.map_or(Effect::None, |quote| {
                Effect::Act(Request::Copy {
                    text: quote.text,
                    program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
                })
            });
        }
        if matches!(event, Event::Key(key) if key.code == KeyCode::Char('a') && key.modifiers.is_empty())
        {
            let Some(quote) = quote else {
                self.set_notice(Some("Select a response range first.".into()), false);
                return Effect::None;
            };
            let attached = self.attach_response_quote(target, quote);
            if matches!(
                attached,
                Attachment::Composer(_) | Attachment::Opened { .. }
            ) {
                self.row_details.reader = None;
            }
            return attached.finish(self);
        }
        let reader = self.row_details.reader.as_mut().unwrap();
        if let Some(selection) = &mut reader.selection {
            let area = reader.source_area.get().unwrap_or_default();
            match event {
                Event::Key(key)
                    if key.kind != KeyEventKind::Release
                        && matches!(
                            key.code,
                            KeyCode::Left
                                | KeyCode::Right
                                | KeyCode::Up
                                | KeyCode::Down
                                | KeyCode::Home
                                | KeyCode::End
                                | KeyCode::Char('a')
                        ) =>
                {
                    // Plain a is handled above; Ctrl-A selects all. No character
                    // insertion or deletion reaches a read-only response.
                    if key.code == KeyCode::Char('a') && key.modifiers == KeyModifiers::CONTROL {
                        selection.source.key(
                            KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL),
                            area.width,
                            usize::MAX,
                        );
                        selection.source.key(
                            KeyEvent::new(
                                KeyCode::End,
                                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                            ),
                            area.width,
                            usize::MAX,
                        );
                    } else if !matches!(key.code, KeyCode::Char(_)) {
                        selection.source.key(*key, area.width, usize::MAX);
                    }
                    let row = selection.source.layout(area.width).cursor.0;
                    reader.scroll.borrow_mut().reveal(row..row + 1);
                    return Effect::None;
                }
                Event::Mouse(mouse)
                    if area.contains((mouse.column, mouse.row).into())
                        && matches!(
                            mouse.kind,
                            MouseEventKind::Down(MouseButton::Left)
                                | MouseEventKind::Drag(MouseButton::Left)
                        ) =>
                {
                    selection.source.place(
                        area.width,
                        reader.scroll.borrow().offset() + usize::from(mouse.row - area.y),
                        usize::from(mouse.column - area.x),
                        matches!(mouse.kind, MouseEventKind::Drag(_))
                            || mouse
                                .modifiers
                                .contains(ratatui::crossterm::event::KeyModifiers::SHIFT),
                    );
                    return Effect::None;
                }
                _ => {}
            }
        }
        reader.scroll.borrow_mut().input(event);
        Effect::None
    }
}
