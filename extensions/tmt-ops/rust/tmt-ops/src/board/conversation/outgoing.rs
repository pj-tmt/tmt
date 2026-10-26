//! Bounded session feedback for explicit sends. Typed outcomes and exact request
//! IDs own correlation; this is neither durable history nor a delivery service.
use super::{ConversationKey, Session};
use crate::{
    board::{
        app::{App, Effect, Input, Request},
        row_detail::{Reply, Target},
    },
    core::SquadError,
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
            && matches!(
                self.progress,
                progress::Progress::Working { since_ms } | progress::Progress::Stated { since_ms }
                    if since_ms <= now_ms
            )
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
                if let Some(text) = &accepted.message_state_label {
                    return format!("{name}: {text}");
                }
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
        self.ticket.text.len() + self.lead_in.as_ref().map_or(0, |reply| reply.body.len())
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
    pub(super) fn text(&self) -> &str {
        &self.ticket.text
    }
    pub(super) fn sequence(&self) -> u64 {
        self.ticket.sequence
    }
    pub(super) fn has_core_state(&self) -> bool {
        matches!(&self.outcome, Some(Ok(accepted)) if accepted.message_state_label.is_some())
    }
    pub(super) fn status(&self) -> String {
        if let Some(Ok(accepted)) = &self.outcome
            && let Some(text) = &accepted.message_state_label
        {
            return text.clone();
        }
        if self.sending_held {
            return "sending…".into();
        }
        match &self.outcome {
            None if self.sending_shown_ms.is_some() => "sending…".into(),
            None => String::new(),
            Some(Ok(accepted))
                if accepted.passthrough && accepted.disposition == Disposition::Sent =>
            {
                if self.feedback.is_empty() {
                    "✓ delivered".into()
                } else {
                    self.feedback.clone()
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
    pub(super) fn needs_outgoing_prepare(&self) -> bool {
        self.stale && !self.outgoing.is_empty()
    }

    /// Durable sends from storage that this session has no live entry for.
    pub(super) fn restored_sends(&self) -> impl Iterator<Item = &crate::board::row_detail::Sent> {
        self.sent.iter().filter(|send| {
            !self
                .outgoing
                .iter()
                .any(|entry| entry.request() == Some(send.request.as_str()))
        })
    }

    /// The request a send-now applies to: the newest own message, and only when Core's state for it
    /// says so. A message the history has not listed yet is the newest, and nothing is offered
    /// while a send is still running, so an older held message is never acted on by mistake.
    pub(super) fn send_now_request(&self) -> Option<&str> {
        if self.outgoing.iter().any(Entry::pending) {
            return None;
        }
        let unlisted = self
            .outgoing
            .iter()
            .rev()
            .find_map(|entry| match &entry.outcome {
                Some(Ok(accepted))
                    if !self
                        .sent
                        .iter()
                        .any(|send| send.request == accepted.request) =>
                {
                    Some(accepted)
                }
                _ => None,
            });
        if let Some(accepted) = unlisted {
            return accepted.can_send_now.then_some(accepted.request.as_str());
        }
        let newest = self.sent.iter().max_by_key(|send| send.at_ms)?;
        newest.can_send_now.then_some(newest.request.as_str())
    }

    pub(super) fn local_finals(&self) -> impl Iterator<Item = &Reply> {
        self.finals.iter().filter(|reply| {
            reply.key.as_ref().is_some_and(|key| {
                self.outgoing
                    .iter()
                    .any(|entry| entry.request() == Some(key.request.as_str()))
            })
        })
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
        let answered = self
            .finals
            .iter()
            .filter_map(|reply| reply.key.as_ref())
            .filter(|key| key.kind == crate::board::home_leads::Kind::Reply)
            .map(|key| key.request.as_str())
            .collect::<std::collections::HashSet<_>>();
        for entry in &mut self.outgoing {
            changed |= entry.observe_sending(now_ms);
            if matches!(&entry.outcome, Some(Ok(accepted)) if accepted.passthrough || accepted.message_state_label.is_some())
            {
                // Core's words are the status; a `working` mark alone moves.
                entry.feedback_clock = matches!(
                    &entry.outcome,
                    Some(Ok(accepted)) if accepted.working
                )
                .then(|| FeedbackClock {
                    progress: progress::Progress::Stated {
                        since_ms: entry.started_ms,
                    },
                    name: crate::board::notes::single_line(name),
                    motion,
                });
                continue;
            }
            // The reply exists and is being read: keep the line as painted.
            if entry
                .request()
                .is_some_and(|request| pending.contains(request))
                || entry
                    .request()
                    .is_some_and(|request| answered.contains(request))
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
        clock.motion &= line.spans.iter().any(|span| span.content.starts_with("⠹"));
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
            .and_then(|entry| entry.feedback_clock.as_ref())
    }

    /// The send whose status row paints at this source line.
    fn status_entry(&self, source: usize) -> Option<&Entry> {
        let rendered = self.rendered.as_ref()?;
        let original = *rendered.anchor_sources.get(source)?;
        let sequence = rendered
            .feedback
            .iter()
            .find(|(range, _)| range.contains(&original))?
            .1;
        self.outgoing
            .iter()
            .find(|entry| entry.ticket.sequence == sequence)
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
                .find(|span| span.content.starts_with("⠹"))
            && let Some(rest) = span.content.strip_prefix("⠹")
        {
            span.content =
                format!("{}{rest}", crate::board::view::feedback_glyph(now_ms, true)).into();
        }
        painted
    }
    fn trim_outgoing(&mut self) {
        while self.outgoing.len() > MAX_ITEMS || self.outgoing_bytes() > MAX_BYTES {
            let Some(entry) = self.outgoing.pop_front() else {
                break;
            };
            if let Some(request) = entry.request() {
                self.sent.retain(|send| send.request != request);
                self.finals
                    .retain(|reply| reply.key.as_ref().is_none_or(|key| key.request != request));
            }
        }
    }
    fn outgoing_bytes(&self) -> usize {
        self.outgoing
            .iter()
            .map(|entry| {
                entry.bytes()
                    + entry.request().map_or(0, |request| {
                        self.finals
                            .iter()
                            .filter(|reply| {
                                reply.key.as_ref().is_some_and(|key| key.request == request)
                            })
                            .map(|reply| reply.body.len())
                            .sum::<usize>()
                            + self
                                .sent
                                .iter()
                                .filter(|send| send.request == request)
                                .filter_map(|send| send.capture.as_ref())
                                .map(|capture| match capture {
                                    crate::board::row_detail::Capture::Answered {
                                        text, ..
                                    } => text.len(),
                                    _ => 0,
                                })
                                .sum::<usize>()
                    })
            })
            .sum()
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
        if !self.outgoing.iter().any(|entry| {
            entry.ticket.key.0 == key.sender
                && entry.ticket.key.1 == key.lead
                && entry.request() == Some(key.request.as_str())
                && key.kind == crate::board::home_leads::Kind::Reply
        }) {
            return;
        }
        if self
            .finals
            .iter()
            .find(|old| super::same_source(old, reply))
            .is_none_or(|old| old.body != reply.body || !old.acquired)
        {
            self.finals.retain(|old| !super::same_source(old, reply));
            self.finals.push(reply.clone());
            self.reply = Some(reply.clone());
            self.trim_outgoing();
            self.refresh_outgoing();
        }
    }
    pub(super) fn correlate_loaded(&mut self, app: &App, name: &str) {
        let mut incoming = Vec::new();
        for entry in &self.outgoing {
            let Some(request) = entry.request() else {
                continue;
            };
            let key = crate::board::home_leads::MessageKey {
                sender: entry.ticket.key.0.clone(),
                lead: entry.ticket.key.1.clone(),
                squad: entry.ticket.squad.clone(),
                request: request.into(),
                kind: crate::board::home_leads::Kind::Reply,
            };
            let Some(body) = app.loaded_reply_body(&key) else {
                continue;
            };
            if self
                .finals
                .iter()
                .find(|reply| {
                    reply
                        .key
                        .as_ref()
                        .is_some_and(|held| held.same_source(&key))
                })
                .is_none_or(|reply| reply.body != body || !reply.acquired)
            {
                incoming.push(Reply {
                    from: name.into(),
                    age: "—".into(),
                    at_ms: app.loaded_reply_time(&key),
                    body: body.into(),
                    key: Some(key),
                    acquired: true,
                    source: true,
                });
            }
        }
        for reply in incoming {
            self.correlate(Some(&reply));
        }
    }
    pub(super) fn observe_captures(&mut self) {
        let mut changed = false;
        for entry in &mut self.outgoing {
            if let Some(send) = self
                .sent
                .iter()
                .find(|send| Some(send.request.as_str()) == entry.request())
            {
                if let Some(Ok(accepted)) = &entry.outcome
                    && accepted.passthrough
                    && accepted.message_state_label.is_none()
                {
                    let fallback = send.capture.as_ref().map_or(
                        "✓ delivered",
                        crate::board::row_detail::Capture::delivery_fallback,
                    );
                    if entry.feedback != fallback {
                        entry.feedback = fallback.into();
                        changed = true;
                    }
                }
                if let Some(Ok(accepted)) = &mut entry.outcome
                    && (accepted.message_state_label != send.message_state_label
                        || accepted.working != send.working)
                {
                    accepted.message_state_label = send.message_state_label.clone();
                    accepted.working = send.working;
                    entry.feedback_clock = None;
                    changed = true;
                }
            }
        }
        // Captured bodies can grow without changing the delivery label.
        // Admission enforces the same retained-byte bound in either case.
        self.trim_outgoing();
        if changed {
            self.refresh_outgoing();
        }
    }

    pub(super) fn refresh_outgoing(&mut self) {
        self.stale = true;
    }
}

impl App {
    pub(in crate::board) fn has_outgoing(&self, key: &(String, String)) -> bool {
        self.conversation
            .borrow()
            .sessions
            .get(&(Some(key.0.clone()), key.1.clone()))
            .is_some_and(|session| !session.outgoing.is_empty())
    }

    /// Receipt IDs arrive with the ordered send outcome. Until then, only the
    /// matching conversation's new sent history must wait; replies stay live.
    pub(in crate::board) fn awaits_send_receipt(
        &self,
        key: &crate::board::home_leads::SentKey,
    ) -> bool {
        self.conversation
            .borrow()
            .sessions
            .get(&(Some(key.sender.clone()), key.agent.clone()))
            .is_some_and(|session| session.outgoing.iter().any(Entry::pending))
    }

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

    fn loaded_reply_time(&self, key: &crate::board::home_leads::MessageKey) -> Option<u64> {
        self.home_leads
            .replies
            .iter()
            .chain(self.view.iter().flat_map(|view| &view.replies))
            .find(|reply| {
                reply["recipientId"].as_str() == Some(key.lead.as_str())
                    && reply["requestId"].as_str() == Some(key.request.as_str())
            })?["submittedAtMs"]
            .as_u64()
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

    /// Press the painted Send now chip, by click or key. One request per press: the chip is
    /// disabled until Core answers, and nothing is retried.
    pub(in crate::board) fn send_now_press(&mut self) -> Effect {
        let Some(hit) = self.conversation.borrow().send_now_hit.clone() else {
            return Effect::None;
        };
        let Some(me) = self.view.as_ref().and_then(|view| view.me.clone()) else {
            return Effect::None;
        };
        let mut state = self.conversation.borrow_mut();
        state.send_now_pending = Some(hit.request.clone());
        state.send_now_hit = None;
        Effect::Act(Request::SendNow {
            me,
            request: hit.request,
            to: hit.to,
        })
    }

    /// Core answered: the notice is its own sentence, whether it sent the message or refused.
    pub(in crate::board) fn finished_send_now(&mut self, done: super::SendNowDone) {
        let mut state = self.conversation.borrow_mut();
        if state.send_now_pending.as_deref() == Some(done.request.as_str()) {
            state.send_now_pending = None;
        }
        drop(state);
        self.finished(done.outcome);
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
