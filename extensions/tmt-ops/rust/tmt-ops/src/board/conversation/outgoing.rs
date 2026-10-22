//! Bounded session feedback for explicit sends. Typed outcomes and exact request
//! IDs own correlation; this is neither durable history nor a delivery service.
use super::{ConversationKey, Session};
use crate::{
    board::{
        app::{App, Input, Request},
        row_detail::{Reply, Target},
    },
    core::SquadError,
    look::Look,
    send::{Accepted, Disposition},
};

// Sends stay in the session until this many have scrolled past; the byte budget
// below still bounds the total.
const MAX_ITEMS: usize = 512;
/// R1 for the sending row: nothing for the first 400 ms, then `sending…` for at
/// least 400 ms, so a fast send goes straight to its next state without a flash.
const SENDING_DELAY_MS: u64 = 400;
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
        if let Ok(accepted) = &self.outcome
            && let Some(warning) = &accepted.warning
        {
            return warning.clone();
        }
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

pub(super) struct Entry {
    ticket: Ticket,
    pub(super) started_ms: u64,
    outcome: Option<Result<Accepted, SquadError>>,
    /// The agent question that opened this send's group, when it arrived after
    /// earlier sends; the first group's question is `Session::before_send`.
    pub(super) lead_in: Option<Reply>,
    pub(super) reply: Option<Reply>,
    capture: Option<crate::board::row_detail::Capture>,
    feedback: String,
    feedback_clock: Option<FeedbackClock>,
    /// The first frame time at which `sending…` was painted.
    sending_shown_ms: Option<u64>,
    /// A settled send keeps `sending…` until it has been visible long enough.
    sending_held: bool,
}
impl Entry {
    pub(super) fn pending(&self) -> bool {
        self.outcome.is_none()
    }
    pub(super) fn request(&self) -> Option<&str> {
        let outcome = self.outcome.as_ref()?;
        outcome.as_ref().map_or_else(
            |error| error.request.as_deref(),
            |accepted| Some(accepted.request.as_str()),
        )
    }
    fn bytes(&self) -> usize {
        self.ticket.text.len()
            + self.reply.as_ref().map_or(0, |reply| reply.body.len())
            + self.lead_in.as_ref().map_or(0, |reply| reply.body.len())
            + self.capture.as_ref().map_or(0, |capture| match capture {
                crate::board::row_detail::Capture::Answered { text, .. } => text.len(),
                _ => 0,
            })
    }
    /// Time-driven only: advances the R1 sending row; returns whether it changed.
    fn observe_sending(&mut self, now_ms: u64) -> bool {
        let before = (self.sending_shown_ms, self.sending_held);
        if self.outcome.is_none()
            && self.sending_shown_ms.is_none()
            && now_ms.saturating_sub(self.started_ms) >= SENDING_DELAY_MS
        {
            self.sending_shown_ms = Some(now_ms);
        }
        self.sending_held = self.outcome.is_some()
            && self
                .sending_shown_ms
                .is_some_and(|shown| now_ms < shown + SENDING_DELAY_MS);
        before != (self.sending_shown_ms, self.sending_held)
    }
    /// When the sending row next changes without any other event.
    fn sending_boundary(&self) -> Option<u64> {
        match (&self.outcome, self.sending_shown_ms) {
            (None, None) => Some(self.started_ms + SENDING_DELAY_MS),
            (Some(_), Some(shown)) => Some(shown + SENDING_DELAY_MS),
            _ => None,
        }
    }
    fn status(&self) -> String {
        if self.sending_held {
            return "sending…".into();
        }
        match &self.outcome {
            None if self.sending_shown_ms.is_some() => "sending…".into(),
            None => String::new(),
            Some(Ok(accepted))
                if accepted.passthrough
                    && (self.capture.is_some() || accepted.disposition == Disposition::Sent) =>
            {
                match self.capture.as_ref() {
                    Some(
                        crate::board::row_detail::Capture::Pending
                        | crate::board::row_detail::Capture::Bound,
                    ) => "⠹ working…".into(),
                    _ => "✓ delivered".into(),
                }
            }
            Some(Ok(accepted)) => match accepted.disposition {
                Disposition::Sent => self.feedback.clone(),
                Disposition::Queued => "◌ queued · inbox".into(),
                Disposition::Offline => "○ offline · queued".into(),
                Disposition::Held => "◌ held · digest".into(),
                Disposition::Uncertain => "? unknown · do not resend".into(),
            },
            Some(Err(error)) if error.request.is_some() => "? unknown · do not resend".into(),
            Some(Err(_)) => "✗ not delivered – draft kept".into(),
        }
    }
}

impl Session {
    /// Durable sends from storage that this session has no live entry for.
    pub(super) fn restored_sends(&self) -> impl Iterator<Item = &crate::board::row_detail::Sent> {
        self.sent.iter().filter(|send| {
            !self
                .outgoing
                .iter()
                .any(|entry| entry.request() == Some(send.request.as_str()))
        })
    }

    pub(super) fn local_finals(&self) -> impl Iterator<Item = &Reply> {
        self.outgoing
            .iter()
            .filter_map(|entry| entry.reply.as_ref())
    }

    pub(super) fn grouped_feedback(
        &self,
        sequence: u64,
        width: usize,
        look: Look,
    ) -> Option<Vec<ratatui::text::Line<'static>>> {
        let entry = self
            .outgoing
            .iter()
            .find(|entry| entry.ticket.sequence == sequence)?;
        Some(super::super::markdown::wrap_styled(
            super::super::exchange_view::feedback_content(&entry.status(), look),
            width,
        ))
    }

    pub(super) fn observe_progress(
        &mut self,
        name: &str,
        observation: Option<crate::board::activity::ObservedActivity>,
        now_ms: u64,
        motion: bool,
    ) {
        let mut changed = false;
        let pending = &self.pending_replies;
        for entry in &mut self.outgoing {
            changed |= entry.observe_sending(now_ms);
            if matches!(&entry.outcome, Some(Ok(accepted)) if accepted.passthrough) {
                entry.feedback_clock = None;
                continue;
            }
            // The reply exists and is being read: keep the line as painted.
            if entry
                .request()
                .is_some_and(|request| pending.contains(request))
                || entry.reply.is_some()
                || !matches!(
                    &entry.outcome,
                    Some(Ok(Accepted {
                        disposition: Disposition::Sent,
                        ..
                    }))
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
            entry.feedback_clock = matches!(
                progress,
                progress::Progress::Working { .. }
                    | progress::Progress::Earlier { .. }
                    | progress::Progress::Finished { .. }
            )
            .then_some(clock);
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
        if let Some(until_ms) = self.status_entry(source).and_then(Entry::sending_boundary) {
            crate::board::view::ClockSurface::admit_delay_gate(app, until_ms);
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
        self.status_entry(source)
            .filter(|entry| entry.reply.is_none())
            .and_then(|entry| entry.feedback_clock.as_ref())
    }

    /// The sent message's age, like every other stamp; it admits its own clock.
    pub(super) fn outgoing_stamp(&self, app: &App, sequence: u64) -> String {
        let Some(entry) = self.outgoing.iter().find(|e| e.ticket.sequence == sequence) else {
            return "now".into();
        };
        let mut clocks = app.frame_clocks.borrow_mut();
        let at = entry.started_ms;
        if !clocks.iter().any(|held| {
            matches!(held, crate::board::view::ClockSurface::ActivityAge(known) if *known == at)
        }) {
            clocks.push(crate::board::view::ClockSurface::ActivityAge(at));
        }
        super::exchange::age_stamp(&crate::requests::age(crate::status::now_ms(), at))
    }

    /// The send whose status row paints at this source line.
    fn status_entry(&self, source: usize) -> Option<&Entry> {
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
            .find(|entry| entry.ticket.sequence == *sequence)
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
        if let Some(reply) = incoming.filter(|reply| {
            reply
                .key
                .as_ref()
                .is_some_and(|key| key.kind == crate::board::home_leads::Kind::Question)
        }) {
            let held = self
                .tail
                .iter_mut()
                .chain(self.outgoing.iter_mut().filter_map(|e| e.lead_in.as_mut()))
                .find(|held| super::same_source(held, reply));
            if let Some(held) = held {
                if reply.acquired && (held.body != reply.body || !held.acquired) {
                    *held = reply.clone();
                    if self
                        .reply
                        .as_ref()
                        .is_some_and(|old| super::same_source(old, reply))
                    {
                        self.reply = Some(reply.clone());
                    }
                    self.refresh_outgoing();
                }
                return;
            }
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
    /// The same admitted provider final may also appear in responses.recent.
    /// Suppress only the exact observation already paired with a request; this
    /// comparison grants no request/source authority to recent output.
    pub(super) fn captures_output(&self, item: &crate::board::recent_responses::Item) -> bool {
        self.sent.iter().filter_map(|send| send.capture.as_ref())
            .chain(self.outgoing.iter().filter_map(|entry| entry.capture.as_ref()))
            .any(|capture| matches!(capture, crate::board::row_detail::Capture::Answered { text, provider, ended_ms, truncated }
                if text == &item.text && provider == &item.provider && ended_ms == &item.ended_ms && truncated == &item.truncated))
    }

    pub(super) fn observe_captures(&mut self) {
        let mut changed = false;
        for entry in &mut self.outgoing {
            if let Some(send) = self
                .sent
                .iter()
                .find(|send| Some(send.request.as_str()) == entry.request())
                && entry.capture != send.capture
            {
                entry.capture = send.capture.clone();
                entry.feedback_clock = None;
                changed = true;
            }
        }
        if changed {
            self.trim_outgoing();
            self.refresh_outgoing();
        }
    }

    pub(super) fn refresh_outgoing(&mut self) {
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
            if let Some(question) = &entry.lead_in {
                push_question(
                    &mut text,
                    &mut self.authored_lines,
                    &mut self.compact_separators,
                    &mut self.response_spans,
                    question,
                );
            }
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let header_line = text.bytes().filter(|byte| *byte == b'\n').count();
            self.authored_lines.insert(
                header_line,
                super::AuthoredLine::Outgoing {
                    answer: false,
                    sequence: entry.ticket.sequence,
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
            if let Some(capture) = &entry.capture
                && !matches!(
                    capture,
                    crate::board::row_detail::Capture::Pending
                        | crate::board::row_detail::Capture::Bound
                )
            {
                text.push_str("\n\n");
                text.push_str(&capture.markdown());
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
        if let Some(question) = &self.tail {
            push_question(
                &mut text,
                &mut self.authored_lines,
                &mut self.compact_separators,
                &mut self.response_spans,
                question,
            );
        }
        self.text = text;
        self.stale = true;
    }
}

/// An agent question shown between or after sends, headed like an agent reply.
fn push_question(
    text: &mut String,
    authored_lines: &mut std::collections::BTreeMap<usize, super::AuthoredLine>,
    compact_separators: &mut std::collections::BTreeSet<usize>,
    response_spans: &mut Vec<(
        Option<u64>,
        crate::board::home_leads::MessageKey,
        std::ops::Range<usize>,
    )>,
    question: &Reply,
) {
    let lines = |text: &str| text.bytes().filter(|byte| *byte == b'\n').count();
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    let header_line = lines(text);
    authored_lines.insert(header_line, super::AuthoredLine::ReplyHeading);
    compact_separators.insert(header_line + 1);
    text.push_str(&format!(
        "{} · now\n\n",
        crate::board::notes::sanitize(&question.from)
    ));
    let start = lines(text);
    text.push_str(crate::board::notes::sanitize(&question.body).trim_end_matches('\n'));
    if question.acquired
        && question.source
        && let Some(key) = &question.key
    {
        response_spans.push((None, key.clone(), start..lines(text) + 1));
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

    pub(in crate::board) fn chat_fallback(
        &self,
        key: &(String, String),
        name: &str,
    ) -> Option<String> {
        let state = self.conversation.borrow();
        let entry = state
            .sessions
            .get(&(Some(key.0.clone()), key.1.clone()))?
            .outgoing
            .back()?;
        let Some(Ok(accepted)) = &entry.outcome else {
            return None;
        };
        accepted.fallback.as_ref()?;
        Some(format!(
            "Sent as a request: {} can't take a direct message right now.",
            crate::board::notes::single_line(name),
        ))
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
        let text = text.to_owned();
        let incoming = input
            .target()
            .filter(|target| {
                self.target_row(target).and_then(|row| row["id"].as_str()) == Some(key.1.as_str())
            })
            .and_then(|target| {
                self.detail(&Target::Row(target.clone()))
                    .and_then(|detail| detail.reply)
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
        let lead_in = if session.outgoing.is_empty() {
            session.update(incoming);
            session.before_send = session.reply.clone();
            None
        } else {
            session.tail.take()
        };
        session.outgoing.push_back(Entry {
            ticket: ticket.clone(),
            started_ms: crate::status::now_ms(),
            outcome: None,
            lead_in,
            reply: None,
            capture: None,
            feedback: String::new(),
            feedback_clock: None,
            sending_shown_ms: None,
            sending_held: false,
        });
        session.trim_outgoing();
        session.follow = true;
        session.change = super::Change::Current;
        session.refresh_outgoing();
        state.pending_send = Some(ticket);
    }

    pub(in crate::board) fn outgoing_ticket(&mut self, request: &Request) -> Option<Ticket> {
        let text = match request {
            Request::Talk { text, .. }
            | Request::Chat { text, .. }
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
                        session.outgoing.iter().any(|entry| entry.ticket == *ticket)
                    })
        }) {
            state.pending_send.take()
        } else {
            None
        }
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
        entry.feedback = progress::Progress::Sent.text(&completed.to, crate::status::now_ms(), "⠹");
        entry.outcome = Some(completed.outcome);
        session.observe_captures();
        session.refresh_outgoing();
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;
