//! Pane-output presentation shares the conversation viewport, never its request authority.
use super::*;
use crate::board::recent_responses::{Observation, Source};

/// R1 for the conversation's loading line: nothing for the first 400 ms.
const LOADING_DELAY_MS: u64 = 400;

/// What the transcript may say about exchanges it does not hold.
#[derive(PartialEq, Eq)]
pub(super) enum Exchanges {
    /// Core answered, so an empty history is the real state.
    Observed,
    /// Core answered with an error: the one case for "unavailable".
    Failed,
    /// Core has not answered yet; `loading` is set once the delay has passed.
    Pending { loading: bool },
}

impl Default for Exchanges {
    fn default() -> Self {
        Self::Pending { loading: false }
    }
}

#[derive(Default, PartialEq, Eq)]
pub(super) struct Context {
    pub enabled: bool,
    pub name: String,
    pub times: Vec<u64>,
    pub exchanges: Exchanges,
    pub output_ages: Vec<String>,
    pub final_ages: Vec<String>,
    pub sent_ages: Vec<String>,
    pub reader: String,
}

impl State {
    pub(in crate::board) fn refresh_pane_output(&mut self) {
        for session in self.sessions.values_mut() {
            session.pane_output_fresh = false;
        }
    }
}

impl App {
    pub(in crate::board) fn pane_output_enabled(&self) -> bool {
        self.workspace_snapshot().is_none()
            || self.shell.tab == crate::board::shell::OpsTab::Agents && !self.flat_agents()
    }

    pub(in crate::board) fn pane_output_read(&self) -> Option<ConversationKey> {
        if !self.pane_output_enabled() || !self.scrolls.visible(Pane::Replies) {
            return None;
        }
        let target = self.conversation_target()?;
        let id = self.target_row(&target)?["id"].as_str()?;
        if !crate::board::recent_responses::canonical_identity(id) {
            return None;
        }
        let key = (self.view.as_ref()?.me_id.clone(), id.to_owned());
        self.conversation
            .borrow()
            .sessions
            .get(&key)
            .is_none_or(|session| !session.pane_output_fresh)
            .then_some(key)
    }

    pub(in crate::board) fn apply_pane_output(
        &self,
        key: ConversationKey,
        observation: Observation,
    ) {
        if self.pane_output_read().as_ref() != Some(&key) {
            return;
        }
        let mut state = self.conversation.borrow_mut();
        let session = state.sessions.entry(key).or_default();
        if session.pane_output != observation {
            session.pane_output = observation;
            session.stale = true;
        }
        session.pane_output_fresh = true;
    }
}

impl Session {
    /// Metadata is already acquired. Unknown times remain unknown (zero for
    /// merge placement), rather than fabricating an observation from frame time.
    pub(super) fn transcript_context(
        &mut self,
        app: &App,
        row: &serde_json::Value,
        agent: Option<&crate::board::home_leads::FinalsKey>,
    ) {
        let name =
            super::super::notes::single_line(row["name"].as_str().unwrap_or("unknown agent"));
        let id = row["id"].as_str().unwrap_or_default();
        let enabled =
            app.pane_output_enabled() && crate::board::recent_responses::canonical_identity(id);
        let time = |reply: Option<&Reply>| {
            let Some(key) = reply.and_then(|reply| reply.key.as_ref()) else {
                return 0;
            };
            app.home_leads
                .replies
                .iter()
                .chain(app.view.iter().flat_map(|view| &view.replies))
                .find(|reply| {
                    reply["requestId"].as_str() == Some(key.request.as_str())
                        && reply["recipientId"].as_str() == Some(key.lead.as_str())
                })
                .and_then(|reply| reply["submittedAtMs"].as_u64())
                .or_else(|| {
                    row["waitingOnYou"].as_array()?.iter().find(|question| {
                        question["requestId"].as_str() == Some(key.request.as_str())
                    })?["preparedAtMs"]
                        .as_u64()
                })
                .unwrap_or(0)
        };
        let base = if self.outgoing.is_empty() {
            self.reply.as_ref()
        } else {
            self.before_send.as_ref()
        };
        let mut times = base
            .map(|reply| vec![time(Some(reply))])
            .unwrap_or_default();
        for entry in &self.outgoing {
            if entry.lead_in.is_some() {
                times.push(time(entry.lead_in.as_ref()));
            }
            times.push(entry.started_ms);
            if entry.reply.is_some() {
                times.push(time(entry.reply.as_ref()));
            }
        }
        if self.tail.is_some() {
            times.push(time(self.tail.as_ref()));
        }
        let output_ages = match &self.pane_output {
            Observation::Observed(items) => items
                .iter()
                .map(|item| {
                    if item.ended_ms == 0 {
                        "—".into()
                    } else {
                        super::exchange::age_stamp(&crate::requests::age(
                            crate::status::now_ms(),
                            item.ended_ms,
                        ))
                    }
                })
                .collect(),
            _ => vec![],
        };
        let context = Context {
            enabled,
            name,
            times,
            output_ages,
            final_ages: self
                .finals
                .iter()
                .map(|reply| {
                    let stamp = super::exchange::age_stamp(&reply.age_at(crate::status::now_ms()));
                    if self.change != Change::Current
                        && self
                            .reply
                            .as_ref()
                            .is_some_and(|held| same_source(held, reply))
                    {
                        format!(
                            "{stamp} · {}",
                            if self.change == Change::NewOutput {
                                "new output"
                            } else {
                                "unavailable"
                            }
                        )
                    } else {
                        stamp
                    }
                })
                .collect(),
            sent_ages: self
                .restored_sends()
                .map(|send| {
                    super::exchange::age_stamp(&crate::requests::age(
                        crate::status::now_ms(),
                        send.at_ms,
                    ))
                })
                .collect(),
            reader: app
                .view
                .as_ref()
                .and_then(|view| view.me.clone())
                .unwrap_or_else(|| "you".into()),
            exchanges: self.exchanges(app, agent),
        };
        if self.transcript_context != context {
            self.transcript_context = context;
            self.stale = true;
        }
    }

    /// Whether anything is held that the placeholder would stand in for.
    fn holds_content(&self) -> bool {
        self.reply.is_some()
            || !self.outgoing.is_empty()
            || !self.finals.is_empty()
            || self.restored_sends().next().is_some()
            || matches!(&self.pane_output, Observation::Observed(items) if !items.is_empty())
    }

    /// Pending history shows nothing for 400 ms, then the loading line; the gate
    /// wakes the board to repaint at that instant.
    fn exchanges(
        &mut self,
        app: &App,
        agent: Option<&crate::board::home_leads::FinalsKey>,
    ) -> Exchanges {
        match app.exchanges_read(agent) {
            crate::board::row_detail::ExchangesRead::Observed => {
                self.pending_since_ms = None;
                Exchanges::Observed
            }
            crate::board::row_detail::ExchangesRead::Failed => {
                self.pending_since_ms = None;
                Exchanges::Failed
            }
            // Held messages stand in for the wait: no placeholder, so no gate.
            crate::board::row_detail::ExchangesRead::Pending if self.holds_content() => {
                self.pending_since_ms = None;
                Exchanges::Pending { loading: false }
            }
            crate::board::row_detail::ExchangesRead::Pending => {
                let now = crate::status::now_ms();
                let since = *self.pending_since_ms.get_or_insert(now);
                let until_ms = since + LOADING_DELAY_MS;
                let loading = now >= until_ms;
                if !loading {
                    crate::board::view::ClockSurface::admit_delay_gate(app, until_ms);
                }
                Exchanges::Pending { loading }
            }
        }
    }

    pub(super) fn merge_pane_output(
        &self,
        mapped: Mapped,
        width: usize,
        gutter: usize,
        look: Look,
        inventory: &super::super::inventory::Inventory,
    ) -> RenderedTranscript {
        let Context {
            enabled,
            name,
            times,
            exchanges,
            output_ages,
            final_ages,
            sent_ages,
            reader,
        } = &self.transcript_context;
        let sends: Vec<_> = self.restored_sends().collect();
        if self.finals.is_empty()
            && sends.is_empty()
            && (!enabled
                || (matches!(&self.pane_output, Observation::Pending)
                    || matches!(&self.pane_output, Observation::Observed(items) if items.is_empty()))
                    && (matches!(exchanges, Exchanges::Observed)
                        || self.reply.is_some()
                        || !self.outgoing.is_empty()))
        {
            let copy = self.copy_of_text(&mapped);
            return RenderedTranscript {
                mapped,
                anchor_text: self.text.clone(),
                response_sources: None,
                final_spans: Vec::new(),
                copy,
            };
        }
        let mut starts = vec![0];
        starts.extend(
            self.authored_lines
                .iter()
                .filter_map(|(line, kind)| {
                    matches!(
                        kind,
                        AuthoredLine::Outgoing { .. } | AuthoredLine::ReplyHeading
                    )
                    .then_some(*line)
                })
                .filter(|line| *line > 0),
        );
        let has_exchange = self.reply.is_some() || !self.outgoing.is_empty();
        let mut blocks: Vec<Vec<usize>> = if has_exchange {
            vec![vec![]; starts.len()]
        } else {
            vec![]
        };
        for (index, source) in mapped.sources.iter().enumerate() {
            if has_exchange {
                let block = starts
                    .partition_point(|start| start <= source)
                    .saturating_sub(1);
                blocks[block].push(index);
            }
        }
        let mut messages = Vec::new();
        let mut represented = std::collections::BTreeSet::new();
        let mut previous_time = 0;
        for (index, block) in blocks.iter().enumerate() {
            let time = times.get(index).copied().unwrap_or(0);
            if let Some(final_index) = self.block_final(block, &mapped) {
                if represented.insert(final_index) {
                    messages.push((
                        self.finals[final_index].at_ms.unwrap_or(previous_time),
                        super::history::Block::Final(final_index),
                    ));
                }
            } else {
                messages.push((time, super::history::Block::Existing(index)));
            }
            if time > 0 {
                previous_time = time;
            }
        }
        messages.extend(
            self.finals
                .iter()
                .enumerate()
                .filter(|(index, _)| !represented.contains(index))
                .map(|(index, reply)| {
                    (
                        reply.at_ms.unwrap_or(0),
                        super::history::Block::Final(index),
                    )
                }),
        );
        messages.extend(
            sends
                .iter()
                .enumerate()
                .map(|(index, send)| (send.at_ms, super::history::Block::Sent(index))),
        );
        messages.sort_by_key(|(time, _)| *time);
        let times: Vec<_> = messages.iter().map(|(time, _)| *time).collect();
        let items = match &self.pane_output {
            Observation::Observed(items) if *enabled => items.as_slice(),
            _ => &[],
        };
        let mut lines = Vec::new();
        let mut sources = Vec::new();
        let mut mentions = Vec::new();
        let mut original = Vec::new();
        let mut anchor_text = String::new();
        let mut final_spans = Vec::new();
        let mut copy_messages = vec![self.text.clone()];
        let mut copy_lines: Vec<Option<super::copy::At>> = Vec::new();
        let mut copy_rows: Vec<String> = Vec::new();
        let next_at = std::cell::Cell::new(None);
        let mut header_anchors = std::collections::BTreeMap::new();
        let mut next_mention = mapped
            .mentions
            .iter()
            .flatten()
            .map(|link| link.occurrence + 1)
            .max()
            .unwrap_or(0);
        let mut append = |line: Line<'static>, links, source: Option<usize>| {
            let index = lines.len();
            if index > 0 {
                anchor_text.push('\n');
            }
            let shown = line.to_string();
            anchor_text.push_str(&shown);
            copy_rows.push(shown);
            sources.push(index);
            mentions.push(links);
            original.push(source);
            copy_lines.push(next_at.take());
            lines.push(line);
            index
        };
        let mut first = true;
        for source in crate::board::recent_responses::merge(&times, items) {
            if matches!(source, Source::Pane(index) if self.captures_output(&items[index])) {
                continue;
            }
            if !first {
                append(Line::default(), vec![], None);
            }
            first = false;
            match source {
                Source::Exchange(block) => {
                    if let super::history::Block::Sent(index) = messages[block].1 {
                        let send = sends[index];
                        let header = append(
                            super::super::exchange_view::header(
                                super::super::exchange_view::Sender::You(reader),
                                name,
                                sent_ages.get(index).map_or("—", String::as_str),
                                false,
                                width,
                                look,
                            ),
                            vec![],
                            None,
                        );
                        header_anchors.insert(header, format!("sent request {}", send.request));
                        let body = markdown::render_conversation_with_members(
                            &sanitize(&send.text),
                            width.saturating_sub(3).max(1),
                            look,
                            None,
                        );
                        let message = copy_messages.len();
                        copy_messages.push(sanitize(&send.text));
                        for (line, source) in body.lines.into_iter().zip(body.sources) {
                            next_at.set((line.width() > 0).then_some(super::copy::At {
                                message,
                                line: source,
                            }));
                            append(super::super::exchange_view::body(line, look), vec![], None);
                        }
                        if let Some(text) = &send.message_state_label {
                            for line in markdown::wrap_styled(
                                super::super::exchange_view::feedback_content(text, look),
                                width.saturating_sub(3).max(1),
                            ) {
                                append(super::super::exchange_view::body(line, look), vec![], None);
                            }
                        }
                        if let Some(capture) = &send.capture
                            && (send.message_state_label.is_none()
                                || !matches!(
                                    capture,
                                    super::super::row_detail::Capture::Pending
                                        | super::super::row_detail::Capture::Bound
                                ))
                        {
                            let content = markdown::render_conversation_with_members(
                                &capture.markdown(),
                                width.saturating_sub(3).max(1),
                                look,
                                None,
                            );
                            for line in content.lines {
                                append(super::super::exchange_view::body(line, look), vec![], None);
                            }
                        }
                        if send.truncated {
                            append(
                                Line::styled(
                                    super::super::view::fit(
                                        &format!("… truncated · tmt x show {}", send.request),
                                        width,
                                    ),
                                    look.role(Role::Muted),
                                ),
                                vec![],
                                None,
                            );
                        }
                        continue;
                    }
                    if let super::history::Block::Final(index) = messages[block].1 {
                        let reply = &self.finals[index];
                        let header = append(
                            super::super::exchange_view::header(
                                super::super::exchange_view::Sender::Agent(&reply.from),
                                reader,
                                final_ages.get(index).map_or("—", String::as_str),
                                false,
                                width,
                                look,
                            ),
                            vec![],
                            None,
                        );
                        // A changing age/cue must not make a held header match
                        // a different message from the same author.
                        header_anchors.insert(
                            header,
                            reply.key.as_ref().map_or_else(
                                || format!("final {:?} {index}", reply.at_ms),
                                |key| format!("final request {}", key.request),
                            ),
                        );
                        let mut body = markdown::render_conversation_with_members(
                            &sanitize(&reply.body),
                            width.saturating_sub(3).max(1),
                            look,
                            Some(inventory),
                        );
                        let count = body
                            .mentions
                            .iter()
                            .flatten()
                            .map(|link| link.occurrence + 1)
                            .max()
                            .unwrap_or(0);
                        for link in body.mentions.iter_mut().flatten() {
                            link.occurrence += next_mention;
                            // The grouped painter indents a final's lines by the gutter.
                            link.column += gutter;
                        }
                        next_mention += count;
                        let mut end = header + 1;
                        let message = copy_messages.len();
                        copy_messages.push(sanitize(&reply.body));
                        let body_sources = std::mem::take(&mut body.sources);
                        for ((line, links), source) in
                            body.lines.into_iter().zip(body.mentions).zip(body_sources)
                        {
                            next_at.set((line.width() > 0).then_some(super::copy::At {
                                message,
                                line: source,
                            }));
                            end = append(line, links, None) + 1;
                        }
                        final_spans.push((header + 1..end, index));
                        continue;
                    }
                    let super::history::Block::Existing(block) = messages[block].1 else {
                        unreachable!()
                    };
                    let rows = &blocks[block];
                    let first = rows
                        .iter()
                        .position(|index| mapped.lines[*index].width() > 0)
                        .unwrap_or(rows.len());
                    let end = rows
                        .iter()
                        .rposition(|index| mapped.lines[*index].width() > 0)
                        .map_or(first, |index| index + 1);
                    for &index in &rows[first..end] {
                        next_at.set(
                            (mapped.lines[index].width() > 0
                                && !self.authored_lines.contains_key(&mapped.sources[index]))
                            .then_some(super::copy::At {
                                message: 0,
                                line: mapped.sources[index],
                            }),
                        );
                        append(
                            mapped.lines[index].clone(),
                            mapped.mentions[index].clone(),
                            Some(mapped.sources[index]),
                        );
                    }
                }
                Source::Pane(index) => {
                    let item = &items[index];
                    let stamp = output_ages.get(index).map_or("—", String::as_str);
                    append(
                        Line::styled(
                            super::super::view::fit(
                                &format!(" {name} · pane output · {stamp}"),
                                width,
                            ),
                            look.role(Role::Muted),
                        ),
                        vec![],
                        None,
                    );
                    // Display-only member links never give pane text request-source authority.
                    let body = markdown::render_conversation_with_members(
                        &sanitize(&item.text),
                        width.saturating_sub(3).max(1),
                        look,
                        Some(inventory),
                    );
                    for (line, mut links) in body.lines.into_iter().zip(body.mentions) {
                        for link in &mut links {
                            link.column += 3;
                        }
                        append(super::super::exchange_view::body(line, look), links, None);
                    }
                    if item.truncated {
                        append(
                            Line::styled("… truncated", look.role(Role::Muted)),
                            vec![],
                            None,
                        );
                    }
                }
            }
        }
        if !self.holds_content() {
            let note = match exchanges {
                Exchanges::Observed => None,
                // One blank row, not none: an empty transcript collapses the pane,
                // and with it the reads that would fill it.
                Exchanges::Pending { loading: false } => Some(""),
                Exchanges::Failed => Some("Exchange history unavailable"),
                Exchanges::Pending { loading: true } => Some("Loading exchanges…"),
            };
            if let Some(note) = note {
                append(Line::styled(note, look.role(Role::Muted)), vec![], None);
            }
        }
        if matches!(self.pane_output, Observation::Unavailable) {
            append(
                Line::styled("Recent pane output unavailable", look.role(Role::Muted)),
                vec![],
                None,
            );
        }
        let anchor_text = anchor_text
            .split('\n')
            .enumerate()
            .map(|(index, line)| header_anchors.get(&index).map_or(line, String::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        RenderedTranscript {
            mapped: Mapped {
                lines,
                sources,
                mentions,
            },
            anchor_text,
            response_sources: Some(original),
            final_spans,
            copy: super::copy::Map {
                messages: copy_messages,
                lines: copy_lines,
                rows: copy_rows,
            },
        }
    }

    /// The source of the unmerged transcript: the session text, but for its authored lines.
    fn copy_of_text(&self, mapped: &Mapped) -> super::copy::Map {
        super::copy::Map {
            rows: mapped.lines.iter().map(ToString::to_string).collect(),
            messages: vec![self.text.clone()],
            lines: mapped
                .lines
                .iter()
                .zip(&mapped.sources)
                .map(|(line, source)| {
                    (line.width() > 0 && !self.authored_lines.contains_key(source)).then_some(
                        super::copy::At {
                            message: 0,
                            line: *source,
                        },
                    )
                })
                .collect(),
        }
    }
}

pub(super) struct RenderedTranscript {
    pub mapped: Mapped,
    pub anchor_text: String,
    pub response_sources: Option<Vec<Option<usize>>>,
    pub final_spans: Vec<(std::ops::Range<usize>, usize)>,
    pub copy: super::copy::Map,
}
