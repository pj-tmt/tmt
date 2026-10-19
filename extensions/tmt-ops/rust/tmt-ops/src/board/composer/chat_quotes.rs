//! Literal chat attachments have no request/source authority. Exact row-note
//! attachments retain the independent response::Quote owner.
use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct ChatQuote {
    pub author: String,
    pub text: String,
}

impl App {
    pub(in crate::board) fn add_chat_quote(
        &mut self,
        author: String,
        text: String,
    ) -> super::super::app::Effect {
        use super::super::app::Effect;
        if text.is_empty() {
            return Effect::None;
        }
        if text.len() > super::super::app::MESSAGE_LIMIT {
            return self.say("Quote exceeds the 1 MiB attachment limit; draft unchanged.");
        }
        let Some(target) = self.conversation_target() else {
            return self.say("Select a member before quoting into chat.");
        };
        let effect = if self
            .input
            .as_ref()
            .is_some_and(|input| input.target() == Some(&target))
        {
            Effect::None
        } else {
            let Some(send) = self.send_target(target.clone(), false) else {
                return self.say("This member is no longer available; draft retained.");
            };
            let squad = self.target_squad(&target).unwrap_or_default();
            self.remember_draft();
            self.compose_row(send, crate::action::Verb::Talk, squad)
        };
        let Some(input) = &mut self.input else {
            return effect;
        };
        let bytes = input
            .chat_quotes
            .iter()
            .map(|quote| quote.text.len())
            .sum::<usize>();
        if bytes.saturating_add(text.len()) > super::super::app::MESSAGE_LIMIT {
            return self.say("Quote exceeds the 1 MiB attachment limit; draft unchanged.");
        }
        input.chat_quotes.push(ChatQuote { author, text });
        input.chat_quote_focus = None;
        self.remember_draft();
        self.workspace_focus_conversation();
        self.invalidate_frame_geometry();
        effect
    }

    pub(in crate::board) fn chat_quote_key(
        &mut self,
        key: KeyEvent,
    ) -> Option<super::super::app::Effect> {
        use super::super::app::Effect;
        let width = self.input_viewport.get().map_or(80, |(area, _)| area.width);
        let input = self.input.as_mut()?;
        if input.chat_quotes.is_empty() {
            input.chat_quote_focus = None;
            return None;
        }
        if key.kind == KeyEventKind::Release {
            return input.chat_quote_focus.map(|_| Effect::None);
        }
        if let Some(index) = input.chat_quote_focus {
            match key.code {
                KeyCode::Left => input.chat_quote_focus = Some(index.saturating_sub(1)),
                KeyCode::Right => {
                    input.chat_quote_focus = Some((index + 1).min(input.chat_quotes.len() - 1))
                }
                KeyCode::Down | KeyCode::Esc | KeyCode::Enter => input.chat_quote_focus = None,
                KeyCode::Char('x') | KeyCode::Delete if key.modifiers.is_empty() => {
                    input
                        .chat_quotes
                        .remove(index.min(input.chat_quotes.len() - 1));
                    input.chat_quote_focus = (!input.chat_quotes.is_empty())
                        .then(|| index.min(input.chat_quotes.len() - 1));
                }
                _ => {}
            }
            self.remember_draft();
            self.invalidate_frame_geometry();
            return Some(Effect::None);
        }
        if key.code == KeyCode::Up
            && key.modifiers == KeyModifiers::NONE
            && input.text.layout(width).cursor.0 == 0
        {
            input.chat_quote_focus = Some(0);
            self.invalidate_frame_geometry();
            return Some(Effect::None);
        }
        None
    }
}

impl App {
    pub(in crate::board) fn remove_chat_quote(&mut self, hit: &ChatQuoteHit) {
        if !self.composer_projection().is_some_and(|projection| {
            projection.key.as_ref() == Some(&hit.key)
                && projection.chat_quotes.get(hit.index) == Some(&hit.quote)
        }) {
            return;
        }
        if self.input.is_none() {
            self.focus_composer();
        }
        if let Some(input) = &mut self.input
            && input.draft_key.as_ref() == Some(&hit.key)
            && input.chat_quotes.get(hit.index) == Some(&hit.quote)
        {
            input.chat_quotes.remove(hit.index);
            input.chat_quote_focus = input.chat_quote_focus.and_then(|index| {
                (!input.chat_quotes.is_empty()).then(|| index.min(input.chat_quotes.len() - 1))
            });
            self.remember_draft();
            self.invalidate_frame_geometry();
        }
    }
}
