//! Session-only editor state keyed by sender and recipient identity. Routing and
//! send admission remain in App/RowSend; a retained draft grants no permission.
use ratatui::layout::Rect;
use std::cell::RefCell;
use std::collections::BTreeMap;
use tmt_tui::components::Editor;

use super::app::{App, Compose, Input, RowTarget};

type DraftKey = (String, String);
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Draft {
    text: Editor,
    quotes: Vec<super::response::Quote>,
}
pub(super) struct Projection<'a> {
    pub key: Option<DraftKey>,
    pub recipient: &'a str,
    pub text: Option<&'a Editor>,
    pub quotes: &'a [super::response::Quote],
    pub focused: bool,
}
#[derive(Clone)]
pub(super) struct SendHit {
    pub area: Rect,
    pub key: DraftKey,
}
impl Draft {
    fn from_input(input: &Input) -> Self {
        Self {
            text: input.text.clone(),
            quotes: input.response_quotes.clone(),
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    drafts: BTreeMap<DraftKey, Draft>,
    pending: Option<(DraftKey, Draft)>,
    pub send_hit: RefCell<Option<SendHit>>,
}

impl State {
    pub(super) fn invalidate(&self) {
        *self.send_hit.borrow_mut() = None;
    }
    pub(super) fn has_drafts(&self) -> bool {
        !self.drafts.is_empty()
    }
}
impl App {
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
            focused: false,
        })
    }
    pub(super) fn remember_draft(&mut self) {
        if let Some(input) = self.input.as_ref()
            && let Some(key) = input.draft_key.clone()
        {
            if input.text.is_empty() && input.response_quotes.is_empty() {
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
                }) {
                    input.text = sent.text.clone();
                    input.response_quotes = sent.quotes.clone();
                }
                self.composer.drafts.entry(key).or_insert(sent);
            }
        }
    }
}

#[cfg(test)]
mod tests;
