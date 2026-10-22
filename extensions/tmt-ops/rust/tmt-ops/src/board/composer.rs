//! Session-only editor state keyed by sender and recipient identity. Routing and
//! send admission remain in App/RowSend; a retained draft grants no permission.
use ratatui::layout::Rect;
use std::cell::RefCell;
use std::collections::BTreeMap;
use tmt_tui::components::Editor;
use unicode_width::UnicodeWidthStr;

use super::app::{App, Compose, Input, RowTarget};

mod chat_quotes;
pub(super) mod files;
pub(super) mod paste;
pub(super) use chat_quotes::ChatQuote;

type DraftKey = (String, String);
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Draft {
    text: Editor,
    quotes: Vec<super::response::Quote>,
    chat_quotes: Vec<ChatQuote>,
}
pub(super) struct Projection<'a> {
    pub key: Option<DraftKey>,
    pub recipient: &'a str,
    pub text: Option<&'a Editor>,
    pub quotes: &'a [super::response::Quote],
    pub chat_quotes: &'a [ChatQuote],
    pub chat_quote_focus: Option<usize>,
    pub focused: bool,
}
#[derive(Clone)]
pub(super) struct SendHit {
    pub area: Rect,
    pub key: DraftKey,
}
#[derive(Clone)]
pub(super) struct QuoteHit {
    pub area: Rect,
    pub key: DraftKey,
    pub source: super::home_leads::MessageKey,
    pub range: std::ops::Range<usize>,
}

#[derive(Clone)]
pub(super) struct ChatQuoteHit {
    pub area: Rect,
    pub key: DraftKey,
    pub index: usize,
    pub quote: ChatQuote,
}

/// Bound untrusted text before sanitizing or painting, independently of the
/// full attachment retained for submission.
pub(super) fn quote_excerpt(quote: &super::response::Quote) -> String {
    let mut chars = quote.text.chars();
    let text: String = chars.by_ref().take(120).collect();
    let mut excerpt = super::notes::sanitize(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if chars.next().is_some() {
        excerpt.push('…');
    }
    excerpt
}
impl Draft {
    fn from_input(input: &Input) -> Self {
        Self {
            text: input.text.clone(),
            quotes: input.response_quotes.clone(),
            chat_quotes: input.chat_quotes.clone(),
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    drafts: BTreeMap<DraftKey, Draft>,
    pending: Option<(DraftKey, Draft)>,
    pub send_hit: RefCell<Option<SendHit>>,
    pub quote_hit: RefCell<Option<QuoteHit>>,
    pub chat_quote_hits: RefCell<Vec<ChatQuoteHit>>,
    pub(super) paste_hits: RefCell<Vec<paste::Hit>>,
    pub(super) paste_preview: RefCell<Option<paste::Preview>>,
    /// What is known of the files dropped in this session, and the worker that learns more.
    pub(super) files: RefCell<files::Table>,
    pub(in crate::board) probe: Option<files::Probe>,
}

impl State {
    pub(super) fn invalidate(&self) {
        *self.send_hit.borrow_mut() = None;
        *self.quote_hit.borrow_mut() = None;
        self.chat_quote_hits.borrow_mut().clear();
        self.paste_hits.borrow_mut().clear();
        if let Some(preview) = self.paste_preview.borrow().as_ref() {
            preview.invalidate();
        }
    }
    /// Relabels the file chips over `path` in every kept draft.
    pub(super) fn relabel_drafts(&mut self, path: &str, label: &str) -> bool {
        let mut changed = false;
        for draft in self
            .drafts
            .values_mut()
            .chain(self.pending.iter_mut().map(|(_, draft)| draft))
        {
            changed |= paste::relabel(&mut draft.text, path, label);
        }
        changed
    }
    pub(super) fn has_drafts(&self) -> bool {
        !self.drafts.is_empty()
    }
}
impl App {
    /// Rows the composer band needs at this band width: the content-sized box,
    /// the info line and any quote rows.
    pub(in crate::board) fn composer_rows(&self, band_width: u16, padded: bool) -> u16 {
        self.composer_projection().map_or(0, |projection| {
            super::input_field::rows(projection.text, band_width, padded)
                + u16::from(!projection.quotes.is_empty())
                + u16::from(!projection.chat_quotes.is_empty())
        })
    }
    pub(in crate::board) fn composer_info<'a>(
        &'a self,
        projection: &Projection<'a>,
    ) -> super::input_field::Info<'a> {
        let identity = projection.key.as_ref().map(|(_, id)| id.as_str());
        let row = identity.and_then(|identity| {
            // The usual recipient is the painted conversation target; use its
            // held row directly instead of walking the roster on every frame.
            self.conversation_target()
                .and_then(|target| self.target_row(&target))
                .filter(|row| row["id"].as_str() == Some(identity))
                .or_else(|| {
                    self.workspace_snapshot().and_then(|snapshot| {
                        snapshot
                            .rows
                            .values()
                            .find(|row| row["id"].as_str() == Some(identity))
                    })
                })
                .or_else(|| {
                    self.rows()
                        .into_iter()
                        .map(|(_, row)| row)
                        .find(|row| row["id"].as_str() == Some(identity))
                })
        });
        super::input_field::Info {
            name: projection.recipient,
            newline: self.newline_key(),
            unread: identity.map_or_else(Default::default, |id| self.unread_count(id)),
            open: row
                .and_then(|row| row["waitingOnYou"].as_array())
                .map(Vec::len),
            context: identity
                .and_then(|id| self.references.agents.get(id))
                .and_then(|agent| agent.context),
        }
    }
    pub(in crate::board) fn quote_author(&self, identity: &str) -> Option<&str> {
        self.workspace_snapshot()?
            .roster
            .members
            .get(identity)
            .map(|member| member.name.as_str())
    }
    pub(in crate::board) fn quote_caption(
        &self,
        quote: &super::response::Quote,
        width: usize,
    ) -> String {
        if width < 12 {
            return super::view::fit("Quote", width);
        }
        let name = self.quote_author(&quote.key.lead).unwrap_or("response");
        let name = super::notes::sanitize(name);
        let name = super::view::fit(&name, width.saturating_sub(16).min(name.width()));
        let prefix = format!("Quote · {}: \"", name.trim_end());
        let excerpt = super::view::fit(
            &quote_excerpt(quote),
            width.saturating_sub(prefix.width() + 1),
        );
        format!("{prefix}{}\"", excerpt.trim_end())
    }
    /// The final Overview field advertises an admitted message key, retaining
    /// authored Enter even when it chooses Talk instead of the factory Reply.
    pub(in crate::board) fn composer_start(&self) -> Option<(String, crate::action::Action)> {
        use crate::action::Verb;
        let bindings = self.bindings();
        bindings
            .get("enter")
            .filter(|action| {
                !self.member_link_owns_enter()
                    && matches!(action.verb, Verb::Reply | Verb::Talk)
                    && super::view::footer::row_allows(self, action)
            })
            .map(|action| ("enter".into(), action.clone()))
            .or_else(|| {
                bindings.into_iter().find(|(key, action)| {
                    !(key == "enter" && self.member_link_owns_enter())
                        && matches!(action.verb, Verb::Reply | Verb::Talk)
                        && super::view::footer::row_allows(self, action)
                })
            })
    }
    pub(in crate::board) fn focus_composer(&mut self) -> super::app::Effect {
        let action = if matches!(self.shell.tab, super::shell::OpsTab::Agents) {
            self.composer_start()
                .map(|(_, action)| action)
                .unwrap_or_else(|| crate::action::Action::parse("reply").expect("built-in Reply"))
        } else {
            crate::action::Action::parse("talk").expect("built-in Talk")
        };
        self.perform(&action)
    }
    pub(in crate::board) fn composer_focus_hint(&self) -> String {
        let bindings = self.bindings();
        let key = bindings
            .get("enter")
            .filter(|action| action.verb == crate::action::Verb::Talk)
            .map(|_| "⏎")
            .or_else(|| {
                bindings
                    .iter()
                    .find(|(_, action)| action.verb == crate::action::Verb::Talk)
                    .map(|(key, _)| key.as_str())
            });
        key.map_or_else(|| "click to write".into(), |key| format!("{key} write"))
    }

    /// Details consumes frozen display lines; composer owns UUID admission and
    /// the attachment bounds, with no name-based recipient lookup.
    pub(in crate::board) fn quote_provenance(&self, identity: &str) -> Vec<String> {
        let Some(projection) = self
            .composer_projection()
            .filter(|projection| projection.key.as_ref().is_some_and(|key| key.1 == identity))
        else {
            return Vec::new();
        };
        let matching = projection
            .quotes
            .iter()
            .filter(|quote| quote.key.lead == identity);
        let count = matching.clone().count();
        let mut lines: Vec<_> = matching
            .take(8)
            .map(|quote| {
                format!(
                    "Quote · request {} · source {} · bytes {}..{} · {}",
                    quote.key.request,
                    quote.key.lead,
                    quote.range.start,
                    quote.range.end,
                    quote_excerpt(quote)
                )
            })
            .collect();
        if count > 8 {
            lines.push(format!("{} more attached quotes", count - 8));
        }
        lines
    }

    pub(in crate::board) fn remove_painted_quote(&mut self, hit: &QuoteHit) {
        let current = self.composer_projection().is_some_and(|projection| {
            projection.key.as_ref() == Some(&hit.key)
                && projection
                    .quotes
                    .last()
                    .is_some_and(|quote| quote.key == hit.source && quote.range == hit.range)
        });
        if !current {
            return;
        }
        if self.input.is_none() {
            self.focus_composer();
        }
        if let Some(input) = self.input.as_mut().filter(|input| {
            input.draft_key.as_ref() == Some(&hit.key)
                && input
                    .response_quotes
                    .last()
                    .is_some_and(|quote| quote.key == hit.source && quote.range == hit.range)
        }) {
            input.response_quotes.pop();
            self.remember_draft();
        }
        self.invalidate_frame_geometry();
    }

    pub(super) fn draft_identity(&self, input: &Input) -> Option<DraftKey> {
        if let Compose::AskLead { sender, to } = &input.compose {
            let view = self.view.as_ref()?;
            let lead = &view.document["squad"]["lead"];
            return (view.me.as_ref() == Some(sender) && lead["name"].as_str() == Some(to))
                .then(|| Some((view.me_id.clone()?, lead["id"].as_str()?.to_owned())))?;
        }
        let send = input.row_send.as_ref()?;
        let recipient = match &input.compose {
            Compose::Talk { .. } | Compose::Reply { .. } => match &send.target {
                RowTarget::Workspace(super::workspace::OpsTarget::Member {
                    identity_id, ..
                }) => identity_id.clone(),
                RowTarget::Member { id, .. } | RowTarget::Lead { id, .. } => id.clone(),
                _ => return None,
            },
            Compose::Annotate { .. } => send.note_recipient.as_ref()?.id.clone()?,
            _ => return None,
        };
        Some((send.sender_id.clone()?, recipient))
    }
    pub(super) fn bind_draft(&mut self) {
        let key = self
            .input
            .as_ref()
            .and_then(|input| self.draft_identity(input));
        if let Some(input) = &mut self.input {
            input.draft_key = key;
        }
    }
    pub(super) fn composer_projection(&self) -> Option<Projection<'_>> {
        let target = self.conversation_target()?;
        let row = self.target_row(&target)?;
        if let Some(input) = self
            .input
            .as_ref()
            .filter(|input| input.target() == Some(&target))
        {
            let recipient = match &input.compose {
                Compose::Talk { to } | Compose::Annotate { to, .. } => to.as_str(),
                Compose::Reply { from, .. } => from.as_str(),
                _ => return None,
            };
            return Some(Projection {
                key: input.draft_key.clone(),
                recipient,
                text: Some(&input.text),
                quotes: &input.response_quotes,
                chat_quotes: &input.chat_quotes,
                chat_quote_focus: input.chat_quote_focus,
                focused: true,
            });
        }
        if self.input.is_some() {
            return None;
        }
        let key = (
            self.view.as_ref()?.me_id.clone().unwrap_or_default(),
            row["id"].as_str()?.to_owned(),
        );
        let draft = self.composer.drafts.get(&key);
        Some(Projection {
            key: self.view.as_ref()?.me_id.as_ref().map(|_| key.clone()),
            recipient: row["name"].as_str()?,
            text: draft.map(|draft| &draft.text),
            quotes: draft.map_or(&[], |draft| draft.quotes.as_slice()),
            chat_quotes: draft.map_or(&[], |draft| draft.chat_quotes.as_slice()),
            chat_quote_focus: None,
            focused: false,
        })
    }
    pub(super) fn remember_draft(&mut self) {
        self.composer.paste_preview.get_mut().take();
        if let Some(input) = self.input.as_ref()
            && let Some(key) = input.draft_key.clone()
        {
            if input.text.is_empty()
                && input.response_quotes.is_empty()
                && input.chat_quotes.is_empty()
            {
                self.composer.drafts.remove(&key);
            } else {
                self.composer.drafts.insert(key, Draft::from_input(input));
            }
        }
    }
    pub(super) fn restore_draft(&mut self) {
        let key = self
            .input
            .as_ref()
            .and_then(|input| input.draft_key.clone());
        if let Some(text) = key.and_then(|key| self.composer.drafts.get(&key))
            && let Some(input) = self.input.as_mut().filter(|input| input.text.is_empty())
        {
            input.text = text.text.clone();
            input.response_quotes = text.quotes.clone();
            input.chat_quotes = text.chat_quotes.clone();
        }
    }
    pub(super) fn sending_draft(&mut self, input: &Input) {
        self.composer.pending = input
            .draft_key
            .clone()
            .map(|key| (key, Draft::from_input(input)));
    }
    pub(super) fn finish_draft(&mut self, success: bool) {
        if let Some((key, sent)) = self.composer.pending.take() {
            if success {
                if self.composer.drafts.get(&key) == Some(&sent) {
                    self.composer.drafts.remove(&key);
                }
            } else {
                if let Some(input) = self.input.as_mut().filter(|input| {
                    input.draft_key.as_ref() == Some(&key)
                        && input.text.is_empty()
                        && input.response_quotes.is_empty()
                        && input.chat_quotes.is_empty()
                }) {
                    input.text = sent.text.clone();
                    input.response_quotes = sent.quotes.clone();
                    input.chat_quotes = sent.chat_quotes.clone();
                }
                self.composer.drafts.entry(key).or_insert(sent);
            }
        }
    }
}

#[cfg(test)]
mod tests;
