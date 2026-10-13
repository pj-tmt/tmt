//! Session-only editor state keyed by sender and recipient identity. Routing and
//! send admission remain in App/RowSend; a retained draft grants no permission.
use std::collections::BTreeMap;
use tmt_tui::components::Editor;

use super::app::{App, Compose, Input};

type DraftKey = (String, String);
#[derive(Clone, PartialEq, Eq)]
struct Draft {
    text: Editor,
    quotes: Vec<super::response::Quote>,
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
}

impl State {
    pub(super) fn has_drafts(&self) -> bool {
        !self.drafts.is_empty()
    }
}
impl App {
    fn resolve_draft(&self, input: &Input) -> Option<DraftKey> {
        let name = match &input.compose {
            Compose::Talk { to } | Compose::AskLead { to, .. } | Compose::Annotate { to, .. } => to,
            Compose::Reply { from, .. } => from,
            _ => return None,
        };
        let sender = match &input.compose {
            Compose::AskLead { sender, .. } => sender.clone(),
            _ => input
                .row_send
                .as_ref()
                .map(|send| send.sender.clone())
                .or_else(|| self.view.as_ref()?.me.clone())?,
        };
        let sender = input
            .draft_key
            .as_ref()
            .map(|key| key.0.clone())
            .or_else(|| {
                let view = self.view.as_ref()?;
                (view.me.as_deref() == Some(sender.as_str()))
                    .then(|| view.me_id.clone())
                    .flatten()
            })?;
        let recipient = self
            .rows()
            .into_iter()
            .find_map(|(_, row)| {
                (row["name"].as_str() == Some(name.as_str()))
                    .then(|| row["id"].as_str().map(str::to_owned))
                    .flatten()
            })
            .or_else(|| {
                let lead = &self.view.as_ref()?.document["squad"]["lead"];
                (lead["name"].as_str() == Some(name.as_str()))
                    .then(|| lead["id"].as_str().map(str::to_owned))
                    .flatten()
            })?;
        Some((sender, recipient))
    }
    pub(super) fn bind_draft(&mut self) {
        let key = self
            .input
            .as_ref()
            .and_then(|input| self.resolve_draft(input));
        if let Some(input) = &mut self.input {
            input.draft_key = key;
        }
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
        if let Some((key, sent)) = self.composer.pending.take()
            && success
            && self.composer.drafts.get(&key) == Some(&sent)
        {
            self.composer.drafts.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests;
