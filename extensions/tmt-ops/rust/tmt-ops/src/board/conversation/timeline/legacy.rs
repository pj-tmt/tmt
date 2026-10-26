//! Borrowed, unlinked observations. IDs choose bodies; clocks only merge independent streams.
use super::*;
use crate::board::{
    recent_responses::Item,
    row_detail::{Capture, Sent},
};

enum LegacyEntry<'a> {
    Input(&'a Sent),
    Local(&'a outgoing::Entry),
    Capture {
        request: &'a str,
        capture: &'a Capture,
    },
    Reply {
        reply: &'a Reply,
        placement: Option<(u64, u64, u8)>,
    },
    Recent {
        item: &'a Item,
        anchor: String,
        index: usize,
    },
    Checklist(&'a activity::Presentation, usize),
}
impl LegacyEntry<'_> {
    fn time(&self) -> u64 {
        match self {
            Self::Input(send) => send.at_ms,
            Self::Local(send) => send.started_ms,
            Self::Capture {
                capture: Capture::Answered { ended_ms, .. },
                ..
            } => *ended_ms,
            Self::Reply { reply, placement } => {
                placement.map_or_else(|| reply.at_ms.unwrap_or(0), |key| key.0)
            }
            Self::Recent { item, .. } => item.ended_ms,
            Self::Checklist(activity, index) => activity.rows[*index].at_ms,
            Self::Capture { .. } => 0,
        }
    }
    fn follows(&self) -> Option<&str> {
        match self {
            Self::Capture { request, .. } => Some(request),
            Self::Reply { reply, .. } => reply
                .key
                .as_ref()
                .filter(|key| key.kind == home_leads::Kind::Reply)
                .map(|key| key.request.as_str()),
            _ => None,
        }
    }
}

pub(super) struct History<'a> {
    events: std::collections::VecDeque<LegacyEntry<'a>>,
    committed_inputs: HashSet<&'a str>,
    previous_checklist: bool,
}
impl<'a> History<'a> {
    pub fn new(session: &'a Session, page: &'a model::Page) -> Self {
        let inputs = page
            .entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                model::Kind::Input(input) => Some(input.request.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let replies = page
            .entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                model::Kind::Reply { request, .. } => Some(request.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut finals = page
            .entries
            .iter()
            .filter_map(|entry| match &entry.kind {
                model::Kind::Final { response_id, .. } => Some(response_id.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut events = Vec::new();
        for send in session
            .restored_sends()
            .filter(|send| !inputs.contains(send.request.as_str()))
        {
            events.push(LegacyEntry::Input(send));
        }
        for entry in session.outgoing.iter().filter(|entry| {
            !entry.pending()
                && entry
                    .request()
                    .is_none_or(|request| !inputs.contains(request))
        }) {
            events.push(LegacyEntry::Local(entry));
        }
        // A capture is an independent answer, never a child placed by a turn's
        // settlement window. Its request copy survives canonical-body pruning.
        for send in &session.sent {
            if let Some(capture @ Capture::Answered { response_id, .. }) = &send.capture
                && response_id.as_deref().is_none_or(|id| finals.insert(id))
            {
                events.push(LegacyEntry::Capture {
                    request: &send.request,
                    capture,
                });
            }
        }
        let mut seen = Vec::<&Reply>::new();
        for reply in session
            .finals
            .iter()
            .chain(session.local_finals())
            .chain(session.before_send.iter())
            .chain(session.tail.iter())
            .chain(session.reply.iter())
            .chain(
                session
                    .outgoing
                    .iter()
                    .filter_map(|entry| entry.lead_in.as_ref()),
            )
        {
            let linked = reply.key.as_ref().is_some_and(|key| match key.kind {
                home_leads::Kind::Reply => replies.contains(key.request.as_str()),
                home_leads::Kind::Question | home_leads::Kind::Asked => {
                    inputs.contains(key.request.as_str())
                }
            });
            if linked || seen.iter().any(|held| same_source(held, reply)) {
                continue;
            }
            seen.push(reply);
            // Unknown clocks stay unknown in the header and acknowledgment
            // projection. Existing arrival boundaries only place a live
            // question relative to the send that preceded/followed it.
            let placement = if reply.at_ms.is_none() {
                session
                    .outgoing
                    .iter()
                    .find(|entry| {
                        entry
                            .lead_in
                            .as_ref()
                            .is_some_and(|question| same_source(question, reply))
                    })
                    .map(|entry| (entry.started_ms, entry.sequence(), 0))
                    .or_else(|| {
                        session
                            .tail
                            .as_ref()
                            .filter(|tail| same_source(tail, reply))
                            .and_then(|_| session.outgoing.back())
                            .map(|entry| (entry.started_ms, entry.sequence(), 2))
                    })
                    .or_else(|| {
                        reply
                            .key
                            .as_ref()
                            .filter(|key| key.kind == home_leads::Kind::Reply)
                            .and_then(|key| {
                                session
                                    .outgoing
                                    .iter()
                                    .find(|entry| entry.request() == Some(key.request.as_str()))
                            })
                            .map(|entry| (entry.started_ms, entry.sequence(), 2))
                    })
            } else {
                None
            };
            events.push(LegacyEntry::Reply { reply, placement });
        }
        events.sort_by_key(|event| match event {
            LegacyEntry::Local(entry) => (entry.started_ms, entry.sequence(), 1),
            LegacyEntry::Reply {
                placement: Some(key),
                ..
            } => *key,
            _ => (event.time(), 0, 1),
        });
        let items = match &session.pane_output {
            super::super::super::recent_responses::Observation::Observed(page)
                if page.timeline.is_none() =>
            {
                page.items.as_slice()
            }
            _ => &[],
        };
        // The existing merge preserves capture order and checklist event order,
        // including clock rollback; it compares only the literal stream heads.
        let anchors = {
            use std::hash::{Hash, Hasher};
            let mut occurrences = std::collections::HashMap::new();
            let mut anchors = items
                .iter()
                .rev()
                .map(|item| {
                    let mut hash = std::hash::DefaultHasher::new();
                    item.hash(&mut hash);
                    let id = hash.finish();
                    let n = occurrences.entry(id).or_insert(0usize);
                    *n += 1;
                    format!("legacy-answer:{id:016x}:{n}")
                })
                .collect::<Vec<_>>();
            anchors.reverse();
            anchors
        };
        let times = events.iter().map(LegacyEntry::time).collect::<Vec<_>>();
        let activity = session.transcript_context.activity.as_deref();
        let order = activity::merge(&times, items, activity);
        let mut ordered = std::collections::VecDeque::new();
        let mut events = events.into_iter().map(Some).collect::<Vec<_>>();
        for source in order {
            match source {
                activity::Source::Exchange(index) => {
                    ordered.push_back(events[index].take().unwrap())
                }
                activity::Source::Pane(index) => {
                    let item = &items[index];
                    if item
                        .response_id
                        .as_deref()
                        .is_none_or(|id| finals.insert(id))
                    {
                        ordered.push_back(LegacyEntry::Recent {
                            item,
                            anchor: anchors[index].clone(),
                            index,
                        });
                    }
                }
                activity::Source::Checklist(index) => {
                    ordered.push_back(LegacyEntry::Checklist(activity.unwrap(), index))
                }
            }
        }
        Self {
            events: ordered,
            committed_inputs: inputs,
            previous_checklist: false,
        }
    }
    pub fn boundary(&mut self) {
        self.previous_checklist = false;
    }
    pub fn through(
        &mut self,
        session: &Session,
        paint: &mut Paint<'_>,
        through: u64,
        painted: &HashSet<&str>,
    ) {
        while self.events.front().is_some_and(|event| {
            event.time() <= through
                && event.follows().is_none_or(|request| {
                    !self.committed_inputs.contains(request) || painted.contains(request)
                })
        }) {
            self.paint_next(session, paint);
        }
    }
    pub fn finish(mut self, session: &Session, paint: &mut Paint<'_>) {
        while !self.events.is_empty() {
            self.paint_next(session, paint);
        }
    }
    fn paint_next(&mut self, session: &Session, paint: &mut Paint<'_>) {
        let context = &session.transcript_context;
        let event = self.events.pop_front().unwrap();
        let checklist = matches!(event, LegacyEntry::Checklist(..));
        if !checklist || !self.previous_checklist {
            paint.separate();
        }
        self.previous_checklist = checklist;
        match event {
            LegacyEntry::Input(send) => {
                paint.header(
                    exchange_view::Sender::You(&context.reader),
                    &context.name,
                    &exchange::age_stamp(&crate::requests::age(
                        crate::status::now_ms(),
                        send.at_ms,
                    )),
                    false,
                    format!("request {}", send.request),
                );
                paint.inputs.push(send.request.clone());
                paint.body(
                    &send.text,
                    false,
                    note_source::Source::request(&context.reader, &send.request, send.at_ms),
                );
                if send.truncated {
                    paint.caption("◌ input truncated · source not fully retained");
                }
                if let Some(label) = &send.message_state_label {
                    paint.caption(label);
                }
                if matches!(send.capture, Some(Capture::NoAnswer)) {
                    paint.caption("No turn output was captured for this message.");
                }
            }
            LegacyEntry::Local(entry) => session.timeline_local(paint, entry),
            LegacyEntry::Capture { request, capture } => {
                let Capture::Answered {
                    text,
                    truncated,
                    settled_ms,
                    ..
                } = capture
                else {
                    unreachable!()
                };
                paint.header(
                    exchange_view::Sender::Agent(&context.name),
                    &context.reader,
                    "in turn",
                    false,
                    format!("captured request {request}"),
                );
                let range = paint.body(
                    text,
                    false,
                    note_source::Source::capture(&context.name, request, capture),
                );
                paint.bodies.push(BodySpan {
                    range,
                    acknowledgements: None,
                    kind: BodyKind::Turn,
                    source: None,
                    legacy: settled_ms.map(|at| (request.into(), at)),
                });
                paint.truncated(*truncated);
            }
            LegacyEntry::Reply { reply, .. } => {
                let asked = reply
                    .key
                    .as_ref()
                    .is_some_and(|key| key.kind == home_leads::Kind::Asked);
                let answer = reply
                    .key
                    .as_ref()
                    .is_some_and(|key| key.kind == home_leads::Kind::Reply);
                paint.header(
                    if asked {
                        exchange_view::Sender::You(&context.reader)
                    } else {
                        exchange_view::Sender::Agent(&reply.from)
                    },
                    if asked {
                        &context.name
                    } else {
                        &context.reader
                    },
                    &exchange::age_stamp(&reply.age_at(crate::status::now_ms())),
                    false,
                    reply.key.as_ref().map_or_else(
                        || "unlinked message".into(),
                        |key| format!("request {}", key.request),
                    ),
                );
                let acquired = reply.acquired && reply.source;
                let range = paint.body(
                    &reply.body,
                    false,
                    if acquired {
                        note_source::Source::reply(reply)
                    } else {
                        note_source::Source::Unavailable
                    },
                );
                paint.bodies.push(BodySpan {
                    range,
                    acknowledgements: if acquired { None } else { Some(Vec::new()) },
                    kind: BodyKind::Final,
                    source: acquired.then(|| reply.key.clone()).flatten(),
                    legacy: (answer && acquired)
                        .then(|| {
                            reply
                                .key
                                .as_ref()
                                .zip(reply.at_ms)
                                .map(|(key, at)| (key.request.clone(), at))
                        })
                        .flatten(),
                });
            }
            LegacyEntry::Recent {
                item,
                anchor,
                index,
            } => {
                paint.pane_start.get_or_insert(paint.lines.len());
                let age = exchange::age_stamp(&crate::requests::age(
                    crate::status::now_ms(),
                    item.ended_ms,
                ));
                paint.header(
                    exchange_view::Sender::Agent(&context.name),
                    &context.reader,
                    &format!("in turn · {age}"),
                    false,
                    anchor,
                );
                let range = paint.body(
                    &item.text,
                    false,
                    note_source::Source::Message {
                        author: context.name.clone(),
                        kind: "reply",
                        at_ms: Some(item.ended_ms),
                        reference: format!(
                            "member {} · provider {} · {}",
                            context.member,
                            item.provider,
                            item.response_id.as_ref().map_or_else(
                                || format!("observation {}", index + 1),
                                |id| format!("response {id}")
                            )
                        ),
                    },
                );
                paint.bodies.push(BodySpan {
                    range,
                    acknowledgements: Some(Vec::new()),
                    kind: BodyKind::Turn,
                    source: None,
                    legacy: None,
                });
                paint.truncated(item.truncated);
            }
            LegacyEntry::Checklist(activity, index) => {
                for line in markdown::wrap_styled(
                    Line::styled(
                        activity.rows[index].text.clone(),
                        paint.look.role(Role::Muted),
                    ),
                    paint.width.saturating_sub(1).max(1),
                ) {
                    paint.line(
                        line,
                        Vec::new(),
                        None,
                        &note_source::Source::Unavailable,
                        Some(activity.anchor(index)),
                    );
                }
            }
        }
    }
}
