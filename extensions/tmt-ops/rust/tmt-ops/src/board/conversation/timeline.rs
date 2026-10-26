//! Rendering over Core's committed stream. Legacy observations keep their own order and identity.
use super::*;
use crate::board::{
    exchange_view, home_leads, recent_responses::timeline as model, unread::BodyKind,
};
use std::{collections::HashSet, ops::Range};

mod legacy;

pub(super) struct BodySpan {
    pub range: Range<usize>,
    /// None is a legacy source, resolved by the existing source-bound reader.
    /// Some(empty) is a Core body with no published acknowledgment keys.
    pub acknowledgements: Option<Vec<model::Acknowledgement>>,
    pub kind: BodyKind,
    pub source: Option<home_leads::MessageKey>,
    pub legacy: Option<(String, u64)>,
}

struct Paint<'a> {
    width: usize,
    look: Look,
    read_only: bool,
    inventory: &'a crate::board::inventory::Inventory,
    lines: Vec<Line<'static>>,
    mentions: Vec<Vec<crate::board::member_links::Link>>,
    anchors: Vec<String>,
    copy: copy::Map,
    notes: note_source::Map,
    bodies: Vec<BodySpan>,
    inputs: Vec<String>,
    feedback: Vec<(Range<usize>, u64)>,
    pane_start: Option<usize>,
    next_mention: usize,
}
impl<'a> Paint<'a> {
    fn new(width: usize, look: Look, inventory: &'a crate::board::inventory::Inventory) -> Self {
        Self {
            width,
            look,
            read_only: false,
            inventory,
            lines: Vec::new(),
            mentions: Vec::new(),
            anchors: Vec::new(),
            copy: Default::default(),
            notes: Default::default(),
            bodies: Vec::new(),
            inputs: Vec::new(),
            feedback: Vec::new(),
            pane_start: None,
            next_mention: 0,
        }
    }
    fn line(
        &mut self,
        line: Line<'static>,
        links: Vec<crate::board::member_links::Link>,
        at: Option<copy::At>,
        note: &note_source::Source,
        anchor: Option<String>,
    ) {
        let row = self.lines.len();
        let source = self.notes.add(note.clone());
        self.notes.row(row, source);
        self.copy
            .rows
            .push(copy::Row::new(at, line.to_string(), &links));
        self.anchors
            .push(anchor.unwrap_or_else(|| line.to_string()));
        self.lines.push(line);
        self.mentions.push(links);
    }
    fn caption(&mut self, text: &str) {
        for line in markdown::wrap_styled(
            Line::styled(text.to_owned(), self.look.role(Role::Muted)),
            self.width.saturating_sub(3).max(1),
        ) {
            self.line(
                exchange_view::body(line, self.look),
                Vec::new(),
                None,
                &note_source::Source::Unavailable,
                None,
            );
        }
    }
    fn notice(&mut self, text: &str) {
        for mut line in markdown::wrap_styled(
            Line::styled(text.to_owned(), self.look.role(Role::Muted)),
            self.width.saturating_sub(1).max(1),
        ) {
            line.spans.insert(0, Span::raw(" "));
            self.line(
                line,
                Vec::new(),
                None,
                &note_source::Source::Unavailable,
                None,
            );
        }
    }
    fn separate(&mut self) {
        if !self.lines.is_empty() {
            self.line(
                Line::default(),
                Vec::new(),
                None,
                &note_source::Source::Unavailable,
                None,
            );
        }
    }
    fn header(
        &mut self,
        sender: exchange_view::Sender<'_>,
        recipient: &str,
        stamp: &str,
        muted: bool,
        anchor: String,
    ) {
        let mut line = exchange_view::header(
            sender,
            recipient,
            stamp,
            self.read_only,
            self.width,
            self.look,
        );
        if muted {
            mute(&mut line, self.look);
        }
        self.line(
            line,
            Vec::new(),
            None,
            &note_source::Source::Unavailable,
            Some(anchor),
        );
    }
    fn body(&mut self, text: &str, muted: bool, note: note_source::Source) -> Range<usize> {
        let text = sanitize(text);
        let mapped = markdown::render_conversation_with_members(
            &text,
            self.width.saturating_sub(3).max(1),
            self.look,
            Some(self.inventory),
        );
        let message = self.copy.messages.len();
        self.copy.messages.push(text);
        let start = self.lines.len();
        let base = self.next_mention;
        self.next_mention += mapped
            .mentions
            .iter()
            .flatten()
            .map(|link| link.occurrence + 1)
            .max()
            .unwrap_or(0);
        for ((line, mut links), source) in mapped
            .lines
            .into_iter()
            .zip(mapped.mentions)
            .zip(mapped.sources)
        {
            let shown = line.to_string();
            let mut line = exchange_view::body(line, self.look);
            if muted {
                mute(&mut line, self.look);
            }
            for link in &mut links {
                link.column += 3;
                link.occurrence += base;
            }
            self.line(
                line,
                links,
                Some(copy::At {
                    message,
                    line: source,
                }),
                &note,
                None,
            );
            self.copy.rows.last_mut().unwrap().shown = shown;
        }
        start..self.lines.len()
    }
    fn truncated(&mut self, truncated: bool) {
        if truncated {
            self.caption("◌ answer truncated · full text in the agent's pane");
        }
    }
    fn finish(self) -> transcript::RenderedTranscript {
        let count = self.lines.len();
        transcript::RenderedTranscript {
            pane_start: self.pane_start,
            mapped: Mapped {
                lines: self.lines,
                sources: (0..count).collect(),
                mentions: self.mentions,
            },
            anchor_text: self.anchors.join("\n"),
            timeline_spans: self.bodies,
            timeline_inputs: self.inputs,
            feedback: self.feedback,
            copy: self.copy,
            note_sources: self.notes,
        }
    }
}
fn mute(line: &mut Line<'_>, look: Look) {
    line.style = line.style.patch(look.role(Role::Muted));
    for span in &mut line.spans {
        span.style = span.style.patch(look.role(Role::Muted));
    }
}

fn sender_name(sender: &model::Sender) -> &str {
    sender.name.as_deref().unwrap_or("unknown sender")
}

/// A partial key list cannot prove that the viewer is absent. Empty/partial
/// evidence falls back only to the explicit preceding input group in this batch.
fn attribution(
    entry: &model::Entry,
    inputs: &[model::Entry],
    viewer: Option<&str>,
    inventory: &crate::board::inventory::Inventory,
) -> Option<String> {
    let viewer = viewer?;
    if entry
        .acknowledgements
        .iter()
        .any(|key| key.identity == viewer)
    {
        return None;
    }
    if !entry.acknowledgements.is_empty() && !entry.acknowledgements_truncated {
        let mut ids = HashSet::new();
        let names = entry
            .acknowledgements
            .iter()
            .filter(|key| ids.insert(key.identity.as_str()))
            .map(|key| {
                inventory
                    .agents
                    .get(&key.identity)
                    .map(|agent| agent.name.as_str())
                    .or_else(|| {
                        inputs.iter().find_map(|entry| match &entry.kind {
                            model::Kind::Input(input)
                                if input.sender.identity.as_deref()
                                    == Some(key.identity.as_str()) =>
                            {
                                input.sender.name.as_deref()
                            }
                            _ => None,
                        })
                    })
                    .unwrap_or("unknown sender")
            })
            .collect::<Vec<_>>();
        return Some(format!("from {}", names.join(", ")));
    }
    if entry.batch.is_none() || inputs.is_empty() || inputs[0].batch != entry.batch {
        return None;
    }
    let senders = inputs
        .iter()
        .filter_map(|entry| match &entry.kind {
            model::Kind::Input(input) => Some(&input.sender),
            _ => None,
        })
        .collect::<Vec<_>>();
    if senders
        .iter()
        .any(|sender| sender.identity.as_deref() == Some(viewer))
    {
        return None;
    }
    let mut names = Vec::new();
    for sender in senders {
        let name = sender_name(sender);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    (!names.is_empty()).then(|| format!("from {}", names.join(", ")))
}

impl Session {
    pub(super) fn timeline_transcript(
        &self,
        width: usize,
        look: Look,
        inventory: &crate::board::inventory::Inventory,
    ) -> transcript::RenderedTranscript {
        let empty = match &self.pane_output {
            crate::board::recent_responses::Observation::Observed(page) => model::Page {
                coverage: page.coverage.clone(),
                capture: page.capture,
                ..Default::default()
            },
            _ => model::Page::default(),
        };
        let page = match &self.pane_output {
            crate::board::recent_responses::Observation::Observed(page) => {
                page.timeline.as_ref().unwrap_or(&empty)
            }
            _ => &empty,
        };
        let context = &self.transcript_context;
        let mut paint = Paint::new(width, look, inventory);
        paint.read_only = context.read_only;
        if page.coverage.older_dropped
            && matches!(&self.pane_output, crate::board::recent_responses::Observation::Observed(held) if held.cursor().is_none())
        {
            paint.notice(&context.dropped_age.as_ref().map_or_else(
                || "○ older answers not kept".into(),
                |age| format!("○ older answers not kept · at or before {age}"),
            ));
        }
        if page.capture == Some(false) {
            paint.notice("◆ answers are not captured for this agent: run tmt setup");
        }
        if self.pane_failure == Some(AnswerFailure::Older) {
            paint.notice("✗ older answers unavailable · scroll up to retry");
        } else if context.enabled
            && (self.pane_failure.is_some()
                || matches!(
                    self.pane_output,
                    crate::board::recent_responses::Observation::Unavailable
                ))
        {
            paint.notice("Recent pane output unavailable");
        }
        // The send worker owns this bounded receipt window. Core may have
        // committed the input before its receipt returns; until then no identity
        // can safely link it to an optimistic local body.
        let pending = self.outgoing.iter().any(outgoing::Entry::pending);
        let mut local_pending_painted = false;
        let mut history = legacy::History::new(self, page);
        if let Some(activity) = &context.activity {
            for caption in &activity.captions {
                paint.notice(caption);
            }
        }
        let mut inputs: &[model::Entry] = &[];
        let mut painted_inputs = HashSet::new();
        let blocks = committed::blocks(&page.entries);
        for (index, block) in blocks.iter().enumerate() {
            let at = blocks[index..]
                .iter()
                .find_map(|block| self.committed_time(block, page))
                .unwrap_or(u64::MAX);
            history.through(self, &mut paint, at, &painted_inputs);
            history.boundary();
            paint.separate();
            paint.pane_start.get_or_insert(paint.lines.len());
            match block {
                committed::Block::Inputs(range) => {
                    inputs = &page.entries[range.clone()];
                    let mut previous_sender: Option<&model::Sender> = None;
                    let mut previous_body: Option<&str> = None;
                    for entry in inputs {
                        let model::Kind::Input(input) = &entry.kind else {
                            unreachable!()
                        };
                        let own = input
                            .sender
                            .identity
                            .as_deref()
                            .is_some_and(|sender| Some(sender) == context.viewer.as_deref());
                        let known = self.sent.iter().any(|send| send.request == input.request)
                            || self
                                .outgoing
                                .iter()
                                .any(|send| send.request() == Some(input.request.as_str()))
                            || self.rendered.as_ref().is_some_and(|frame| {
                                frame.timeline_inputs.contains(&input.request)
                            });
                        if own && pending && !known {
                            if !local_pending_painted {
                                for send in self.outgoing.iter().filter(|send| send.pending()) {
                                    self.timeline_local(&mut paint, send);
                                }
                                local_pending_painted = true;
                            }
                            continue;
                        }
                        painted_inputs.insert(input.request.as_str());
                        paint.inputs.push(input.request.clone());
                        let sender = if own {
                            context.reader.clone()
                        } else {
                            format!("from {}", sender_name(&input.sender))
                        };
                        let same_sender = previous_sender.is_some_and(|previous| {
                            previous.identity.is_some()
                                && previous.identity == input.sender.identity
                        });
                        if !same_sender {
                            let who = if own {
                                exchange_view::Sender::You(&sender)
                            } else {
                                exchange_view::Sender::Agent(&sender)
                            };
                            paint.header(
                                who,
                                &context.name,
                                "",
                                !own,
                                format!("input {}", entry.id),
                            );
                        }
                        previous_sender = Some(&input.sender);
                        if let Some(body) = &input.body {
                            if same_sender
                                && previous_body.is_some_and(|previous| {
                                    previous.trim().contains("\n\n")
                                        || body.text.trim().contains("\n\n")
                                })
                            {
                                paint.separate();
                            }
                            previous_body = Some(&body.text);
                            paint.body(
                                &body.text,
                                !own,
                                note_source::Source::Message {
                                    author: sender_name(&input.sender).into(),
                                    kind: "message",
                                    at_ms: None,
                                    reference: format!("request {}", input.request),
                                },
                            );
                            if body.truncated {
                                paint.caption("◌ input truncated · source not fully retained");
                            }
                        } else {
                            paint.caption("○ input text unavailable");
                        }
                        if input.uncertain {
                            paint.caption("◌ delivery uncertain");
                        }
                    }
                    if let Some(last) = inputs.iter().rev().find_map(|entry| match &entry.kind {
                        model::Kind::Input(input) => Some(input),
                        _ => None,
                    }) {
                        if let Some(entry) = self
                            .outgoing
                            .iter()
                            .find(|entry| entry.request() == Some(last.request.as_str()))
                        {
                            self.timeline_status(&mut paint, entry);
                        } else if let Some(send) =
                            self.sent.iter().find(|send| send.request == last.request)
                        {
                            if let Some(label) = &send.message_state_label {
                                paint.caption(label);
                            }
                            if matches!(
                                send.capture,
                                Some(crate::board::row_detail::Capture::NoAnswer)
                            ) {
                                paint.caption("No turn output was captured for this message.");
                            }
                        }
                    }
                }
                committed::Block::Final(index) | committed::Block::Reply(index) => {
                    let entry = &page.entries[*index];
                    let attributed =
                        attribution(entry, inputs, context.viewer.as_deref(), inventory);
                    let muted = attributed.is_some();
                    let stamp = attributed.as_ref().map_or_else(
                        || "in turn".to_owned(),
                        |sender| format!("in turn · {sender}"),
                    );
                    paint.header(
                        exchange_view::Sender::Agent(&context.name),
                        &context.reader,
                        &stamp,
                        muted,
                        format!("reply {}", entry.id),
                    );
                    let (text, truncated, at_ms, kind, request) = match &entry.kind {
                        model::Kind::Final {
                            response_id,
                            available,
                        } => {
                            if let Some(item) =
                                page.responses.get(response_id).filter(|_| *available)
                            {
                                (
                                    Some(item.text.as_str()),
                                    item.truncated,
                                    Some(item.ended_ms),
                                    BodyKind::Turn,
                                    None,
                                )
                            } else if let Some(capture) = self.timeline_capture(response_id) {
                                let crate::board::row_detail::Capture::Answered {
                                    text,
                                    truncated,
                                    ended_ms,
                                    ..
                                } = capture
                                else {
                                    unreachable!()
                                };
                                (
                                    Some(text.as_str()),
                                    *truncated,
                                    Some(*ended_ms),
                                    BodyKind::Turn,
                                    None,
                                )
                            } else {
                                (None, false, None, BodyKind::Turn, None)
                            }
                        }
                        model::Kind::Reply { request, body } => (
                            body.as_ref().map(|body| body.text.as_str()),
                            body.as_ref().is_some_and(|body| body.truncated),
                            None,
                            BodyKind::Final,
                            Some(request),
                        ),
                        _ => unreachable!(),
                    };
                    if let Some(text) = text {
                        let source = request.filter(|_| !truncated).and_then(|request| {
                            self.finals
                                .iter()
                                .chain(self.local_finals())
                                .find(|reply| {
                                    reply.acquired
                                        && reply.source
                                        && reply.key.as_ref().is_some_and(|key| {
                                            key.request == *request
                                                && Some(key.sender.as_str())
                                                    == context.viewer.as_deref()
                                                && key.kind == home_leads::Kind::Reply
                                        })
                                })
                                .and_then(|reply| reply.key.clone())
                        });
                        let range = paint.body(
                            text,
                            muted,
                            note_source::Source::Message {
                                author: context.name.clone(),
                                kind: "reply",
                                at_ms,
                                reference: match &entry.kind {
                                    model::Kind::Final { response_id, .. } => {
                                        format!("response {response_id}")
                                    }
                                    _ => format!("request {}", request.unwrap()),
                                },
                            },
                        );
                        paint.bodies.push(BodySpan {
                            range,
                            acknowledgements: Some(entry.acknowledgements.clone()),
                            kind,
                            source,
                            legacy: None,
                        });
                        paint.truncated(truncated);
                    } else {
                        paint.caption("○ reply text unavailable");
                    }
                }
                committed::Block::Failure(_) => {
                    paint.caption("✗ reply not captured · see the agent's pane")
                }
            }
        }
        history.finish(self, &mut paint);
        // Sends not yet observed in the committed stream keep immediate local
        // feedback. A timeline input with the same request ID owns that body.
        let requests = page
            .entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                model::Kind::Input(input) => Some(input.request.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        for entry in self.outgoing.iter().filter(|entry| entry.pending()) {
            if local_pending_painted && entry.pending() {
                continue;
            }
            if entry
                .request()
                .is_some_and(|request| requests.contains(request))
            {
                continue;
            }
            paint.separate();
            self.timeline_local(&mut paint, entry);
        }
        if paint.lines.is_empty() {
            match context.exchanges {
                transcript::Exchanges::Observed => paint.notice("(no response in loaded history)"),
                transcript::Exchanges::Failed => paint.notice("Exchange history unavailable"),
                transcript::Exchanges::Pending { loading: true } => {
                    paint.notice("Loading exchanges…")
                }
                transcript::Exchanges::Pending { loading: false } => paint.notice(""),
            }
        }
        paint.finish()
    }
    fn committed_time(&self, block: &committed::Block, page: &model::Page) -> Option<u64> {
        match block {
            committed::Block::Inputs(range) => {
                page.entries[range.clone()].iter().find_map(|entry| {
                    let model::Kind::Input(input) = &entry.kind else {
                        return None;
                    };
                    self.sent
                        .iter()
                        .find(|send| send.request == input.request)
                        .map(|send| send.at_ms)
                        .or_else(|| {
                            self.outgoing
                                .iter()
                                .find(|send| send.request() == Some(input.request.as_str()))
                                .map(|send| send.started_ms)
                        })
                })
            }
            committed::Block::Final(index) => match &page.entries[*index].kind {
                model::Kind::Final { response_id, .. } => {
                    page.responses.get(response_id).map(|item| item.ended_ms)
                }
                _ => None,
            },
            committed::Block::Reply(index) => match &page.entries[*index].kind {
                model::Kind::Reply { request, .. } => self
                    .finals
                    .iter()
                    .find(|reply| {
                        reply
                            .key
                            .as_ref()
                            .is_some_and(|key| key.request == *request)
                    })
                    .and_then(|reply| reply.at_ms),
                _ => None,
            },
            committed::Block::Failure(_) => None,
        }
    }
    fn timeline_local(&self, paint: &mut Paint<'_>, entry: &outgoing::Entry) {
        let context = &self.transcript_context;
        paint.header(
            exchange_view::Sender::You(&context.reader),
            &context.name,
            "",
            false,
            format!("local send {}", entry.sequence()),
        );
        paint.body(
            entry.text(),
            false,
            note_source::Source::Message {
                author: context.reader.clone(),
                kind: "message",
                at_ms: Some(entry.started_ms),
                reference: entry.request().map_or_else(
                    || "local send".into(),
                    |request| format!("request {request}"),
                ),
            },
        );
        self.timeline_status(paint, entry);
        if self.sent.iter().any(|send| {
            entry.request() == Some(send.request.as_str())
                && matches!(
                    send.capture,
                    Some(crate::board::row_detail::Capture::NoAnswer)
                )
        }) {
            paint.caption("No turn output was captured for this message.");
        }
    }
    fn timeline_status(&self, paint: &mut Paint<'_>, entry: &outgoing::Entry) {
        if !entry.has_core_state()
            && entry.request().is_some_and(|request| {
                self.finals.iter().any(|reply| {
                    reply.acquired
                        && reply.source
                        && reply.key.as_ref().is_some_and(|key| {
                            key.kind == home_leads::Kind::Reply && key.request == request
                        })
                })
            })
        {
            return;
        }
        let text = entry.status();
        let start = paint.lines.len();
        let lines = markdown::wrap_styled(
            exchange_view::feedback_content(&text, paint.look),
            paint.width.saturating_sub(3).max(1),
        );
        for line in lines {
            paint.line(
                exchange_view::feedback(line, paint.look),
                Vec::new(),
                None,
                &note_source::Source::Unavailable,
                None,
            );
        }
        if paint.lines.len() == start {
            paint.line(
                Line::default(),
                Vec::new(),
                None,
                &note_source::Source::Unavailable,
                None,
            );
        }
        paint
            .feedback
            .push((start..paint.lines.len(), entry.sequence()));
    }
    fn timeline_capture(&self, response: &str) -> Option<&crate::board::row_detail::Capture> {
        self.sent
            .iter()
            .filter_map(|send| send.capture.as_ref())
            .find(|capture| capture.response_id() == Some(response))
    }
}

#[cfg(test)]
pub(in crate::board) mod tests;
