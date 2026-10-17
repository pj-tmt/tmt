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

mod progress;

/// Generated metadata is literal inside the Markdown transcript. Message and
/// reply bodies retain their original source and Markdown behavior.
fn push_literal(text: &mut String, value: &str) {
    for ch in value.chars() {
        if ch.is_ascii_punctuation() {
            text.push('\\');
        }
        text.push(ch);
    }
}

/// A painted feedback label, independent of conversation acquisition or authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct FeedbackClock {
    progress: progress::Progress,
    name: String,
    motion: bool,
}

impl FeedbackClock {
    pub(in crate::board) fn mark(&self, now_ms: u64) -> String {
        self.progress.text(
            &self.name,
            now_ms,
            crate::board::view::feedback_glyph(now_ms, self.motion),
        )
    }

    pub(in crate::board) fn animates(&self, now_ms: u64) -> bool {
        self.motion
            && matches!(self.progress, progress::Progress::Working { since_ms } if since_ms <= now_ms)
    }
}

pub(super) fn motion_allowed(app: &App, target: &super::RowTarget) -> bool {
    if app.look().depth == tmt_cli_style::Depth::None {
        return false;
    }
    let Some(view) = &app.view else {
        return false;
    };
    let reduced = match target {
        super::RowTarget::Workspace(crate::board::workspace::OpsTarget::Member {
            room_id: Some(room),
            ..
        }) => view.home.as_ref().and_then(|home| {
            home.workspace
                .roster
                .squads
                .iter()
                .find(|squad| &squad.room_id == room)
                .and_then(|squad| view.home_rate.get(&squad.name))
                .map(|rate| rate.settings.reduced_motion)
        }),
        _ => None,
    }
    .unwrap_or_else(|| view.home.as_ref().is_none_or(|home| home.reduced_motion));
    !reduced
}

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
    feedback: String,
    feedback_clock: Option<FeedbackClock>,
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
            None => "sending…".into(),
            Some(Outcome::Send(Ok(accepted))) => match accepted.disposition {
                Disposition::Sent => self.feedback.clone(),
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
    pub(super) fn observe_progress(
        &mut self,
        name: &str,
        observation: Option<crate::board::activity::ObservedActivity>,
        now_ms: u64,
        motion: bool,
    ) {
        let mut changed = false;
        for entry in &mut self.outgoing {
            if entry.reply.is_some()
                || !matches!(
                    &entry.outcome,
                    Some(Outcome::Send(Ok(Accepted {
                        disposition: Disposition::Sent,
                        ..
                    })))
                )
            {
                continue;
            }
            let mut progress = entry
                .feedback_clock
                .as_ref()
                .map_or(Default::default(), |clock| clock.progress);
            progress.observe(observation, entry.started_ms, now_ms);
            let clock = FeedbackClock {
                progress,
                name: crate::board::notes::single_line(name),
                motion,
            };
            // Spinner frames decorate one visible glyph; keep the body cache.
            let feedback = progress.text(name, now_ms, "⠹");
            entry.feedback_clock = (progress != progress::Progress::Unknown).then_some(clock);
            if entry.feedback != feedback {
                entry.feedback = feedback;
                changed = true;
            }
        }
        // Keep the full transcript cache while the small status labels agree.
        if changed {
            self.refresh_outgoing();
        }
    }

    /// Only visible authored status rows admit elapsed/animation clock demand.
    pub(super) fn admit_feedback_clock(
        &self,
        app: &App,
        source: usize,
        line: &ratatui::text::Line<'_>,
    ) {
        if app.modal_open() {
            return;
        }
        let Some(clock) = self.feedback_clock(source) else {
            return;
        };
        let mut clock = clock.clone();
        // A wrapped continuation can show elapsed text while the spinner is
        // above the viewport. It needs no animation-frequency wakeup.
        clock.motion &= line
            .spans
            .iter()
            .find(|span| !span.content.is_empty())
            .is_some_and(|span| span.content.starts_with("⠹"));
        let mut clocks = app.frame_clocks.borrow_mut();
        if let Some(held) = clocks.iter_mut().find_map(|surface| match surface {
            crate::board::view::ClockSurface::OutgoingFeedback(held)
                if held.progress == clock.progress && held.name == clock.name =>
            {
                Some(held)
            }
            _ => None,
        }) {
            held.motion |= clock.motion;
        } else {
            clocks.push(crate::board::view::ClockSurface::OutgoingFeedback(clock));
        }
    }

    fn feedback_clock(&self, source: usize) -> Option<&FeedbackClock> {
        let source = self.rendered.as_ref().and_then(|rendered| {
            rendered
                .response_sources
                .as_ref()
                .map_or(Some(source), |sources| {
                    sources.get(source).copied().flatten()
                })
        })?;
        let Some(super::AuthoredLine::Status { sequence }) = self.authored_lines.get(&source)
        else {
            return None;
        };
        self.outgoing
            .iter()
            .find(|entry| entry.ticket.sequence == *sequence && entry.reply.is_none())
            .and_then(|entry| entry.feedback_clock.as_ref())
    }

    pub(super) fn feedback_line<'a>(
        &self,
        source: usize,
        line: &ratatui::text::Line<'a>,
        now_ms: u64,
    ) -> ratatui::text::Line<'a> {
        let mut painted = line.clone();
        if self
            .feedback_clock(source)
            .is_some_and(|clock| clock.animates(now_ms))
            && let Some(span) = painted
                .spans
                .iter_mut()
                .find(|span| !span.content.is_empty())
            && let Some(rest) = span.content.strip_prefix("⠹")
        {
            span.content =
                format!("{}{rest}", crate::board::view::feedback_glyph(now_ms, true)).into();
        }
        painted
    }
    fn trim_outgoing(&mut self) {
        while self.outgoing.len() > MAX_ITEMS
            || self.outgoing.iter().map(Entry::bytes).sum::<usize>() > MAX_BYTES
        {
            self.outgoing.pop_front();
        }
    }
    pub(super) fn retained_response(
        &self,
        key: &crate::board::home_leads::MessageKey,
        sequence: Option<u64>,
    ) -> Option<&Reply> {
        let reply = if self.outgoing.is_empty() {
            self.reply.as_ref()
        } else if let Some(sequence) = sequence {
            self.outgoing
                .iter()
                .find(|entry| entry.ticket.sequence == sequence)
                .and_then(|entry| entry.reply.as_ref())
        } else {
            self.before_send.as_ref()
        }?;
        (reply.acquired
            && reply.source
            && reply.key.as_ref().is_some_and(|held| held.same_source(key)))
        .then_some(reply)
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
            if entry.reply.is_none() {
                self.compact_separators.insert(message_end + 1);
                self.authored_lines.insert(
                    message_end + 2,
                    super::AuthoredLine::Status {
                        sequence: entry.ticket.sequence,
                    },
                );
                text.push_str("\n\n");
                push_literal(&mut text, &entry.status());
            }
            if let Some(reply) = &entry.reply {
                text.push_str("\n\n");
                let header_line = text.bytes().filter(|byte| *byte == b'\n').count();
                self.authored_lines
                    .insert(header_line, super::AuthoredLine::ReplyHeading);
                self.compact_separators.insert(header_line + 1);
                push_literal(
                    &mut text,
                    &format!(
                        "{} · now\n\n",
                        crate::board::notes::single_line(&reply.from)
                    ),
                );
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
            feedback: String::new(),
            feedback_clock: None,
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
        entry.feedback =
            progress::Progress::Unknown.text(&completed.to, crate::status::now_ms(), "⠹");
        entry.outcome = Some(Outcome::Send(completed.outcome));
        session.refresh_outgoing();
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;
