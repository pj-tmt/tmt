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
        if self.ticket.is_none() {
            return self.outcome.as_ref().map_or_else(
                |error| error.message.clone(),
                |accepted| accepted.describe(&self.what, &self.to),
            );
        }
        match &self.outcome {
            Ok(accepted) => {
                let name = crate::board::notes::sanitize(&self.to);
                match accepted.disposition {
                    Disposition::Sent => format!("Sent to {name}"),
                    Disposition::Queued => format!("Queued for {name}"),
                    Disposition::Offline => format!("{name} offline; queued"),
                    Disposition::Held => format!("Held for {name}; digest active"),
                    Disposition::Uncertain => "Delivery unknown; do not resend".into(),
                }
            }
            Err(error) => error.message.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Answered {
    pub ticket: Ticket,
    pub outcome: Result<(), SquadError>,
    pub to: String,
}

enum Outcome {
    Send(Result<Accepted, SquadError>),
    Answer(Result<(), SquadError>),
}

pub(super) struct Entry {
    ticket: Ticket,
    pub(super) started_ms: u64,
    outcome: Option<Outcome>,
    question: Option<String>,
    pub(super) reply: Option<Reply>,
}
impl Entry {
    pub(super) fn pending(&self) -> bool {
        self.outcome.is_none()
    }
    fn request(&self) -> Option<&str> {
        let Outcome::Send(outcome) = self.outcome.as_ref()? else {
            return None;
        };
        outcome.as_ref().map_or_else(
            |error| error.request.as_deref(),
            |accepted| Some(accepted.request.as_str()),
        )
    }
    fn bytes(&self) -> usize {
        self.ticket.text.len() + self.reply.as_ref().map_or(0, |reply| reply.body.len())
    }
    fn status(&self) -> String {
        match &self.outcome {
            None => "◌ sending…".into(),
            Some(Outcome::Send(Ok(accepted))) => match accepted.disposition {
                Disposition::Sent => "✓ delivered".into(),
                Disposition::Queued => "◌ queued · inbox".into(),
                Disposition::Offline => "○ offline · queued".into(),
                Disposition::Held => "◌ held · digest".into(),
                Disposition::Uncertain => "? unknown · do not resend".into(),
            },
            Some(Outcome::Send(Err(error))) if error.request.is_some() => {
                "? unknown · do not resend".into()
            }
            Some(Outcome::Send(Err(_))) => "✗ not delivered – draft kept".into(),
            Some(Outcome::Answer(Ok(()))) => "✓ replied".into(),
            Some(Outcome::Answer(Err(_))) => "✗ reply failed – draft kept".into(),
        }
    }
}

impl Session {
    pub(super) fn local_finals(&self) -> impl Iterator<Item = &Reply> {
        self.outgoing
            .iter()
            .filter_map(|entry| entry.reply.as_ref())
    }
    fn trim_outgoing(&mut self) {
        while self.outgoing.len() > MAX_ITEMS
            || self.outgoing.iter().map(Entry::bytes).sum::<usize>() > MAX_BYTES
        {
            self.outgoing.pop_front();
        }
    }
    pub(super) fn correlate(&mut self, incoming: Option<&Reply>) {
        if let Some(reply) = incoming.filter(|reply| {
            reply.acquired
                && reply
                    .key
                    .as_ref()
                    .is_some_and(|key| key.kind == crate::board::home_leads::Kind::Question)
        }) && self
            .before_send
            .as_ref()
            .is_some_and(|old| super::same_source(old, reply))
        {
            let changed = self
                .before_send
                .as_ref()
                .is_some_and(|old| old.body != reply.body || !old.acquired);
            self.before_send = Some(reply.clone());
            self.reply = Some(reply.clone());
            if changed {
                self.refresh_outgoing();
            }
            return;
        }
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
                    at_ms: None,
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
    /// `state` picks the glyph; `words` are the STATE cell's words, so the
    /// status and the header never disagree.
    pub(super) fn agent_observed(&mut self, name: &str, state: &str, words: &str, stale: bool) {
        let glyph = match state {
            "working" => "●",
            "idle" => "◌",
            "ended" => "○",
            "blocked" => "✗",
            "waiting" => "◆",
            _ => "?",
        };
        let mark = format!(
            "{glyph} {} {}{}",
            crate::board::notes::sanitize(name),
            crate::board::notes::sanitize(words),
            if stale && state != "stale" {
                " · stale"
            } else {
                ""
            }
        );
        if self.agent_mark != mark {
            self.agent_mark = mark;
            if !self.outgoing.is_empty() {
                self.refresh_outgoing();
            }
        }
    }
    pub(super) fn body_ranges(&self) -> Vec<super::body::Body> {
        use super::{
            AuthoredLine,
            body::{Body, Key},
        };
        let latest = if self.outgoing.is_empty() {
            self.reply
                .as_ref()
                .and_then(|reply| reply.key.as_ref())
                .filter(|key| key.kind == crate::board::home_leads::Kind::Reply)
        } else {
            self.response_spans
                .iter()
                .rev()
                .map(|(_, key, _)| key)
                .find(|key| key.kind == crate::board::home_leads::Kind::Reply)
        };
        let key_of = |key: &crate::board::home_leads::MessageKey| {
            Key::Reply(format!("{:?}:{}", key.kind, key.request))
        };
        let mut bodies = if self.outgoing.is_empty() {
            self.reply
                .as_ref()
                .and_then(|reply| reply.key.as_ref())
                .map(|key| Body {
                    key: key_of(key),
                    sources: 0..self.text.split('\n').count(),
                    newest_reply: latest == Some(key),
                })
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            self.response_spans
                .iter()
                .map(|(_, key, range)| Body {
                    key: key_of(key),
                    sources: range.clone(),
                    newest_reply: latest == Some(key),
                })
                .collect()
        };
        let headers = self.authored_lines.iter().filter_map(|(line, kind)| {
            matches!(kind, AuthoredLine::Outgoing { .. }).then_some(*line)
        });
        for (entry, header) in self.outgoing.iter().zip(headers) {
            if let Some((status, _)) = self
                .authored_lines
                .range(header + 1..)
                .find(|(_, kind)| matches!(kind, AuthoredLine::Status))
            {
                bodies.push(Body {
                    key: Key::Outgoing(entry.ticket.sequence),
                    sources: header + 2..status.saturating_sub(1),
                    newest_reply: false,
                });
            }
        }
        bodies
    }

    fn refresh_outgoing(&mut self) {
        let mut text = self.before_send.as_ref().map_or_else(String::new, |reply| {
            crate::board::notes::sanitize(&reply.body)
        });
        self.response_spans.clear();
        if let Some(reply) = self
            .before_send
            .as_ref()
            .filter(|reply| reply.acquired && reply.source)
            && let Some(key) = &reply.key
        {
            self.response_spans.push((
                None,
                key.clone(),
                0..text.bytes().filter(|byte| *byte == b'\n').count() + 1,
            ));
        }
        self.authored_lines.clear();
        self.compact_separators.clear();
        for entry in &self.outgoing {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let header_line = text.bytes().filter(|byte| *byte == b'\n').count();
            self.authored_lines.insert(
                header_line,
                super::AuthoredLine::Outgoing {
                    answer: entry.question.is_some(),
                },
            );
            self.compact_separators.insert(header_line + 1);
            text.push_str("You · now\n\n");
            text.push_str(crate::board::notes::sanitize(&entry.ticket.text).trim_end_matches('\n'));
            let message_end = text.bytes().filter(|byte| *byte == b'\n').count();
            self.compact_separators.insert(message_end + 1);
            self.authored_lines
                .insert(message_end + 2, super::AuthoredLine::Status);
            text.push_str("\n\n");
            text.push_str(&entry.status());
            text.push_str(" · ");
            text.push_str(&self.agent_mark);
            if let Some(reply) = &entry.reply {
                text.push_str("\n\n");
                let header_line = text.bytes().filter(|byte| *byte == b'\n').count();
                self.authored_lines
                    .insert(header_line, super::AuthoredLine::ReplyHeading);
                self.compact_separators.insert(header_line + 1);
                text.push_str(&format!(
                    "{} · now\n\n",
                    crate::board::notes::sanitize(&reply.from)
                ));
                let start = text.bytes().filter(|byte| *byte == b'\n').count();
                text.push_str(crate::board::notes::sanitize(&reply.body).trim_end_matches('\n'));
                if reply.acquired
                    && reply.source
                    && let Some(key) = &reply.key
                {
                    self.response_spans.push((
                        Some(entry.ticket.sequence),
                        key.clone(),
                        start..text.bytes().filter(|byte| *byte == b'\n').count() + 1,
                    ));
                }
            }
        }
        self.text = text;
        self.rendered = None;
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
                | crate::board::app::Compose::Reply { .. }
        ) {
            return;
        }
        let Some(key) = input.draft_key.clone() else {
            return;
        };
        let text = text.to_owned();
        let incoming = input
            .target()
            .filter(|target| {
                self.target_row(target).and_then(|row| row["id"].as_str()) == Some(key.1.as_str())
            })
            .and_then(|target| {
                if let crate::board::app::Compose::Reply { request, .. } = &input.compose {
                    self.pending_question(target, Some(request))
                } else {
                    self.detail(&Target::Row(target.clone()))
                        .and_then(|detail| detail.reply)
                }
            });
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
            session.before_send = session.reply.clone();
        }
        session.outgoing.push_back(Entry {
            ticket: ticket.clone(),
            started_ms: crate::status::now_ms(),
            outcome: None,
            question: match &input.compose {
                crate::board::app::Compose::Reply { request, .. } => Some(request.clone()),
                _ => None,
            },
            reply: None,
        });
        session.trim_outgoing();
        session.follow = true;
        session.change = super::Change::Current;
        session.refresh_outgoing();
        state.pending_send = Some(ticket);
    }

    pub(in crate::board) fn outgoing_ticket(&mut self, request: &Request) -> Option<Ticket> {
        let question = if let Request::Reply { request, .. } = request {
            Some(request.as_str())
        } else {
            None
        };
        let text = match request {
            Request::Talk { text, .. }
            | Request::DirectTalk { text, .. }
            | Request::Reply { text, .. }
            | Request::Annotate { text, .. } => text.clone(),
            _ => return None,
        };
        let mut state = self.conversation.borrow_mut();
        if state.pending_send.as_ref().is_some_and(|ticket| {
            ticket.text == text
                && state
                    .sessions
                    .get(&(Some(ticket.key.0.clone()), ticket.key.1.clone()))
                    .is_some_and(|session| {
                        session.outgoing.iter().any(|entry| {
                            entry.ticket == *ticket && entry.question.as_deref() == question
                        })
                    })
        }) {
            state.pending_send.take()
        } else {
            None
        }
    }

    pub(in crate::board) fn finished_answer(&mut self, completed: Answered) {
        let ticket = &completed.ticket;
        let identity = (Some(ticket.key.0.clone()), ticket.key.1.clone());
        let current = self
            .conversation
            .borrow()
            .sessions
            .get(&identity)
            .is_some_and(|session| {
                session.outgoing.iter().any(|entry| {
                    entry.ticket == *ticket && entry.question.is_some() && entry.outcome.is_none()
                })
            });
        if !current {
            return;
        }
        self.finish_draft(completed.outcome.is_ok());
        self.notice = Some(completed.outcome.as_ref().map_or_else(
            |error| error.message.clone(),
            |()| {
                format!(
                    "Replied to {}",
                    crate::board::notes::sanitize(&completed.to)
                )
            },
        ));
        let mut state = self.conversation.borrow_mut();
        let session = state
            .sessions
            .get_mut(&identity)
            .expect("verified answer session");
        let entry = session
            .outgoing
            .iter_mut()
            .find(|entry| entry.ticket == *ticket)
            .expect("verified answer ticket");
        entry.outcome = Some(Outcome::Answer(completed.outcome));
        session.refresh_outgoing();
    }
    pub(in crate::board) fn finished_send(&mut self, completed: Completed) {
        let Some(ticket) = completed.ticket.as_ref() else {
            self.set_notice(Some(completed.notice()), false);
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
        self.set_notice(Some(completed.notice()), false);
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
        entry.outcome = Some(Outcome::Send(completed.outcome));
        session.refresh_outgoing();
    }
}

#[cfg(test)]
mod tests;
