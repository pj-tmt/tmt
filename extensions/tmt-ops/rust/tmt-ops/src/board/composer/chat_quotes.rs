//! Literal chat attachments have no request/source authority. Exact row-note
//! attachments retain the independent response::Quote owner.
use super::super::home_leads::MessageKey;
use super::*;
use std::ops::Range;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum QuoteSource {
    Literal {
        author: String,
    },
    Exact {
        key: MessageKey,
        range: Range<usize>,
    },
}

fn excerpt(text: &str) -> String {
    let raw: String = text.chars().take(120).collect();
    let mut text = super::super::notes::single_line(&raw);
    if text.chars().count() > 60 {
        text = text.chars().take(59).collect::<String>() + "…";
    }
    text
}

impl Input {
    /// Insert after the current logical line. A retained fold is one unit even
    /// when its source contains newlines; no insertion can split that source.
    pub(in crate::board) fn insert_quote(&mut self, text: &str, source: QuoteSource) -> bool {
        if text.is_empty() {
            return false;
        }
        let limit = super::super::app::MESSAGE_LIMIT;
        let mut end = self.text.cursor();
        loop {
            if let Some(fold) = self
                .text
                .folds()
                .iter()
                .find(|fold| fold.source.contains(&end))
            {
                end = fold.source.end;
            }
            let next = self.text.as_str()[end..]
                .find(['\r', '\n'])
                .map(|at| end + at);
            match next {
                Some(at)
                    if self
                        .text
                        .folds()
                        .iter()
                        .any(|fold| fold.source.contains(&at)) =>
                {
                    end = self
                        .text
                        .folds()
                        .iter()
                        .find(|fold| fold.source.contains(&at))
                        .unwrap()
                        .source
                        .end;
                }
                Some(at) => {
                    end = at;
                    break;
                }
                None => {
                    end = self.text.as_str().len();
                    break;
                }
            }
        }
        let prefix = if end == 0 || self.text.as_str()[..end].ends_with(['\r', '\n']) {
            ""
        } else {
            "\n"
        };
        if self
            .text
            .as_str()
            .len()
            .saturating_add(prefix.len())
            .saturating_add(text.len())
            .saturating_add(1)
            > limit
        {
            return false;
        }
        let author = match &source {
            QuoteSource::Literal { author } => {
                super::super::notes::single_line(&author.chars().take(40).collect::<String>())
            }
            QuoteSource::Exact { .. } => "response".into(),
        };
        let label = format!("Quote · {author}: {} ×", excerpt(text));
        // Work on a copy so even an unexpected grapheme-boundary refusal is atomic.
        let mut editor = self.text.clone();
        if !editor.replace_range(end..end, prefix, limit)
            || !editor.insert_folded(text, label, limit)
        {
            return false;
        }
        let Some(id) = editor
            .folds()
            .iter()
            .find(|fold| fold.source.start == end + prefix.len())
            .map(|fold| fold.id)
        else {
            return false;
        };
        if !editor.insert("\n", limit) {
            return false;
        }
        self.text = editor;
        self.inline_quotes.insert(id, source);
        true
    }

    /// Reconcile metadata with Editor after deletion/replacement; metadata alone
    /// never formats text or grants exact-source authority.
    pub(in crate::board) fn retain_quote_sources(&mut self) {
        self.inline_quotes
            .retain(|id, _| self.text.folds().iter().any(|fold| fold.id == *id));
    }

    pub(in crate::board) fn exact_quotes(
        &self,
    ) -> impl Iterator<Item = super::super::response::Quote> + '_ {
        self.text.folds().iter().filter_map(|fold| {
            let QuoteSource::Exact { key, range } = self.inline_quotes.get(&fold.id)? else {
                return None;
            };
            Some(super::super::response::Quote {
                key: key.clone(),
                range: range.clone(),
                text: self.text.as_str()[fold.source.clone()].to_owned(),
            })
        })
    }

    pub(in crate::board) fn quoted_message(&self) -> String {
        let mut body = String::new();
        let mut end = 0;
        for fold in self.text.folds() {
            let Some(source) = self.inline_quotes.get(&fold.id) else {
                continue;
            };
            body.push_str(&self.text.as_str()[end..fold.source.start]);
            let text = &self.text.as_str()[fold.source.clone()];
            body.push_str(&match source {
                QuoteSource::Literal { .. } => crate::requests::message::inline_quote(text),
                QuoteSource::Exact { .. } => crate::requests::message::blockquote(text),
            });
            end = fold.source.end;
        }
        body.push_str(&self.text.as_str()[end..]);
        body
    }
}

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
        if !input.insert_quote(&text, QuoteSource::Literal { author }) {
            return self.say("Quote exceeds the 1 MiB attachment limit; draft unchanged.");
        }
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
