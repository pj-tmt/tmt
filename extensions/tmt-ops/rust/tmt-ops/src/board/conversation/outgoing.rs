//! Bounded session feedback for explicit sends. Typed outcomes and exact request
//! IDs own correlation; this is neither durable history nor a delivery service.
use super::{ConversationKey, Session};
use crate::{
    board::{
        app::{App, Input, Request},
        row_detail::{Reply, Target},
    },
    core::SquadError,
    send::{Accepted, Disposition},
};

const MAX_ITEMS: usize = 16;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub key: (String, String),
    pub sequence: u64,
    pub text: String,
    pub squad: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Completed {
    pub ticket: Option<Ticket>,
    pub outcome: Result<Accepted, SquadError>,
    pub what: String,
    pub to: String,
}

impl Completed {
    pub(in crate::board) fn accepted(&self) -> bool {
        self.outcome
            .as_ref()
            .map_or_else(|error| error.request.is_some(), |_| true)
    }
    pub(in crate::board) fn notice(&self) -> String {
        self.outcome.as_ref().map_or_else(
            |error| error.message.clone(),
            |accepted| accepted.describe(&self.what, &self.to),
        )
    }
}

pub(super) struct Entry {
    ticket: Ticket,
    outcome: Option<Result<Accepted, SquadError>>,
    reply: Option<Reply>,
}
impl Entry {
    fn request(&self) -> Option<&str> {
        self.outcome.as_ref()?.as_ref().map_or_else(
            |error| error.request.as_deref(),
            |accepted| Some(accepted.request.as_str()),
        )
    }
    fn bytes(&self) -> usize {
        self.ticket.text.len() + self.reply.as_ref().map_or(0, |reply| reply.body.len())
    }
    fn status(&self) -> String {
        match &self.outcome {
            None => "sending".into(),
            Some(Ok(accepted)) => match accepted.disposition {
                Disposition::Sent => "delivered · written to the pane".into(),
                Disposition::Queued => "queued · waiting in the inbox".into(),
                Disposition::Offline => "offline · queued in the inbox".into(),
                Disposition::Held => "held · until the digest checklist ends".into(),
                Disposition::Uncertain => "unconfirmed, do not resend".into(),
            },
            Some(Err(error)) => match &error.request {
                Some(request) => format!(
                    "{} · request {request} retained; do not resend",
                    error.message
                ),
                None => format!("{} · draft retained", error.message),
            },
        }
    }
}

impl Session {
    fn trim_outgoing(&mut self) {
        while self.outgoing.len() > MAX_ITEMS
            || self.outgoing.iter().map(Entry::bytes).sum::<usize>() > MAX_BYTES
        {
            self.outgoing.pop_front();
        }
    }
    pub(super) fn correlate(&mut self, incoming: Option<&Reply>) {
        let Some(reply) = incoming.filter(|reply| reply.acquired && reply.source) else {
            return;
        };
        let Some(key) = &reply.key else {
            return;
        };
        let Some(entry) = self.outgoing.iter_mut().find(|entry| {
            entry.ticket.key.0 == key.sender
                && entry.ticket.key.1 == key.lead
                && entry.request() == Some(key.request.as_str())
                && key.kind == crate::board::home_leads::Kind::Reply
        }) else {
            return;
        };
        if entry
            .reply
            .as_ref()
            .is_none_or(|old| !super::same_source(old, reply) || old.body != reply.body)
        {
            self.reply = Some(reply.clone());
            entry.reply = Some(reply.clone());
            self.trim_outgoing();
            self.refresh_outgoing();
        }
    }
    pub(super) fn correlate_loaded(&mut self, app: &App, name: &str) {
        let mut changed = false;
        for entry in &mut self.outgoing {
            let Some(request) = entry.request() else {
                continue;
            };
            let key = crate::board::home_leads::MessageKey {
                sender: entry.ticket.key.0.clone(),
                lead: entry.ticket.key.1.clone(),
                squad: entry.ticket.squad.clone(),
                request: request.to_owned(),
                kind: crate::board::home_leads::Kind::Reply,
            };
            let Some(body) = app.loaded_reply_body(&key) else {
                continue;
            };
            if entry.reply.as_ref().is_none_or(|reply| reply.body != body) {
                let reply = Reply {
                    from: name.to_owned(),
                    age: "now".into(),
                    body: body.to_owned(),
                    key: Some(key),
                    acquired: true,
                    source: true,
                };
                self.reply = Some(reply.clone());
                entry.reply = Some(reply);
                changed = true;
            }
        }
        if changed {
            self.trim_outgoing();
            self.refresh_outgoing();
        }
    }
    pub(super) fn agent_observed(&mut self, state: &str, stale: bool) {
        let mark = format!(
            "Agent {} · {}",
            crate::board::notes::sanitize(state),
            if stale { "stale" } else { "observed" }
        );
        if self.agent_mark != mark {
            self.agent_mark = mark;
            if !self.outgoing.is_empty() {
                self.refresh_outgoing();
            }
        }
    }
    fn refresh_outgoing(&mut self) {
        let mut text = self.before_send.clone();
        for entry in &self.outgoing {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str("You · now  \n");
            text.push_str(&crate::board::notes::sanitize(&entry.ticket.text));
            text.push_str("\n\n");
            text.push_str(&crate::board::notes::sanitize(&entry.status()));
            text.push_str(" · ");
            text.push_str(&self.agent_mark);
            if let Some(reply) = &entry.reply {
                text.push_str("\n\n");
                text.push_str(&format!(
                    "{} · reply {}\n\n",
                    crate::board::notes::sanitize(&reply.from),
                    entry.request().unwrap()
                ));
                text.push_str(&crate::board::notes::sanitize(&reply.body));
            }
        }
        self.text = text;
        self.rendered = None;
        self.anchor.reconcile(&self.text);
    }
}

impl App {
    /// Consume only retained response bytes already in the originator projection.
    /// The current detail path separately supplies its exact acquired body.
    fn loaded_reply_body(&self, key: &crate::board::home_leads::MessageKey) -> Option<&str> {
        let view = self
            .view
            .as_ref()
            .filter(|view| view.me_id.as_ref() == Some(&key.sender))?;
        self.home_leads
            .replies
            .iter()
            .chain(&view.replies)
            .find(|reply| {
                reply["recipientId"].as_str() == Some(&key.lead)
                    && reply["requestId"].as_str() == Some(&key.request)
            })?["response"]
            .as_str()
    }

    pub(in crate::board) fn begin_outgoing(&mut self, input: &Input, text: &str) {
        if !matches!(
            input.compose,
            crate::board::app::Compose::Talk { .. }
                | crate::board::app::Compose::AskLead { .. }
                | crate::board::app::Compose::Annotate { .. }
        ) {
            return;
        }
        let Some(key) = input.draft_key.clone() else {
            return;
        };
        let text = if let crate::board::app::Compose::Annotate { row, .. } = &input.compose {
            format!("{}{text}", crate::requests::tag(&input.squad, row))
        } else {
            text.to_owned()
        };
        let incoming = input
            .target()
            .and_then(|target| self.detail(&Target::Row(target.clone())))
            .and_then(|detail| detail.reply);
        let mut state = self.conversation.borrow_mut();
        state.sequence = state
            .sequence
            .checked_add(1)
            .expect("session send sequence exhausted");
        let ticket = Ticket {
            key: key.clone(),
            sequence: state.sequence,
            text,
            squad: input.squad.clone(),
        };
        let identity: ConversationKey = (Some(key.0), key.1);
        let session = state.sessions.entry(identity).or_default();
        if session.outgoing.is_empty() {
            session.update(incoming);
            session.before_send = if session.reply.is_some() {
                session.text.clone()
            } else {
                String::new()
            };
        }
        session.outgoing.push_back(Entry {
            ticket: ticket.clone(),
            outcome: None,
            reply: None,
        });
        session.trim_outgoing();
        session.follow = true;
        session.change = super::Change::Current;
        session.refresh_outgoing();
        state.pending_send = Some(ticket);
    }

    pub(in crate::board) fn outgoing_ticket(&mut self, request: &Request) -> Option<Ticket> {
        let text = match request {
            Request::Talk { text, .. } | Request::DirectTalk { text, .. } => text.clone(),
            Request::Annotate {
                text, squad, row, ..
            } => format!("{}{text}", crate::requests::tag(squad, row)),
            _ => return None,
        };
        let mut state = self.conversation.borrow_mut();
        if state
            .pending_send
            .as_ref()
            .is_some_and(|ticket| ticket.text == text)
        {
            state.pending_send.take()
        } else {
            None
        }
    }

    pub(in crate::board) fn finished_send(&mut self, completed: Completed) {
        let Some(ticket) = completed.ticket.as_ref() else {
            self.notice = Some(completed.notice());
            return;
        };
        let current = self
            .conversation
            .borrow()
            .sessions
            .get(&(Some(ticket.key.0.clone()), ticket.key.1.clone()))
            .is_some_and(|session| {
                session
                    .outgoing
                    .iter()
                    .any(|entry| entry.ticket == *ticket && entry.outcome.is_none())
            });
        if !current {
            return;
        }
        self.finish_draft(completed.accepted());
        self.notice = Some(completed.notice());
        let ticket = completed.ticket.unwrap();
        let mut state = self.conversation.borrow_mut();
        let identity = (Some(ticket.key.0.clone()), ticket.key.1.clone());
        let Some(session) = state.sessions.get_mut(&identity) else {
            return;
        };
        let Some(entry) = session
            .outgoing
            .iter_mut()
            .find(|entry| entry.ticket == ticket)
        else {
            return;
        };
        entry.outcome = Some(completed.outcome);
        session.refresh_outgoing();
    }
}

#[cfg(test)]
mod tests;
