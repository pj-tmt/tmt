//! Pane-output presentation shares the conversation viewport, never its request authority.
use super::*;
use crate::board::recent_responses::{Observation, Source};

/// R1 for the conversation's loading line: nothing for the first 400 ms.
const LOADING_DELAY_MS: u64 = 400;

fn history_caption(text: &str, width: usize, look: Look) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(1);
    if inner == 0 {
        return Vec::new();
    }
    markdown::wrap_styled(Line::styled(text.to_owned(), look.role(Role::Muted)), inner)
        .into_iter()
        .map(|mut line| {
            line.spans.insert(0, Span::raw(" "));
            line
        })
        .collect()
}

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
    pub member: String,
    pub dropped_age: Option<String>,
}

impl State {
    pub(in crate::board) fn rebuild_pane_output(&mut self) {
        if let Some(session) = self
            .active
            .as_ref()
            .and_then(|key| self.sessions.get_mut(key))
        {
            session.pane_rebuild = true;
            session.pane_output_fresh = false;
        }
    }

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
            .is_none_or(|session| !session.pane_output_fresh || session.older_demand)
            .then_some(key)
    }

    pub(in crate::board) fn pane_output_plan(
        &self,
    ) -> Option<crate::board::recent_responses::AnswerRead> {
        let key = self.pane_output_read()?;
        let state = self.conversation.borrow();
        let session = state.sessions.get(&key);
        let held = session.and_then(|session| match &session.pane_output {
            Observation::Observed(page) => Some(std::sync::Arc::clone(page)),
            _ => None,
        });
        let before = session
            .filter(|session| session.pane_output_fresh && session.older_demand)
            .and_then(|_| held.as_ref()?.next_before);
        Some(crate::board::recent_responses::AnswerRead {
            key,
            before,
            held,
            rebuild: session.is_some_and(|session| session.pane_rebuild),
        })
    }

    pub(in crate::board) fn apply_pane_output(
        &self,
        key: ConversationKey,
        observation: Observation,
    ) {
        let Some(read) = self.pane_output_plan().filter(|read| read.key == key) else {
            return;
        };
        self.apply_answer_read(&read, observation);
    }

    pub(in crate::board) fn apply_answer_read(
        &self,
        read: &crate::board::recent_responses::AnswerRead,
        observation: Observation,
    ) {
        if self.pane_output_plan().as_ref() != Some(read) {
            return;
        }
        let mut state = self.conversation.borrow_mut();
        let session = state.sessions.entry(read.key.clone()).or_default();
        let failed = matches!(observation, Observation::Unavailable);
        let same = match (&session.pane_output, &observation) {
            (Observation::Observed(a), Observation::Observed(b)) => std::sync::Arc::ptr_eq(a, b),
            (a, b) => a == b,
        };
        let failure = if failed {
            Some(if read.before.is_some() {
                AnswerFailure::Older
            } else {
                AnswerFailure::Latest
            })
        } else if read.before.is_none()
            && !read.rebuild
            && session.pane_failure == Some(AnswerFailure::Older)
            && matches!(&observation, Observation::Observed(page) if page.next_before.is_some())
        {
            // Reading new answers did not retry the failed older cursor.
            Some(AnswerFailure::Older)
        } else {
            None
        };
        let changed = !same || session.pane_failure != failure;
        if !failed || !matches!(session.pane_output, Observation::Observed(_)) {
            session.pane_output = observation;
        }
        session.pane_failure = failure;
        session.pane_rebuild = false;
        if read.before.is_some()
            || failed
            || !matches!(&session.pane_output, Observation::Observed(page) if page.next_before.is_some())
        {
            session.older_demand = false;
        }
        session.stale |= changed;
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
        let places = self.answer_places();
        for (index, entry) in self.outgoing.iter().enumerate() {
            // A stacked body belongs to its block's first send: one block, one time.
            if !places[index].header() {
                continue;
            }
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
            member: id.into(),
            dropped_age: match &self.pane_output {
                Observation::Observed(page) => page.coverage.dropped_through_ms.map(|time| {
                    super::exchange::age_stamp(&crate::requests::age(crate::status::now_ms(), time))
                }),
                _ => None,
            },
        };
        if self.transcript_context != context {
            self.transcript_context = context;
            self.stale = true;
        }
    }

    /// The retained-content spinner: nothing for the spinner delay, then the shell's
    /// glyph; the gate wakes the board to repaint at that instant. With `motion` the
    /// glyph turns on the shared tick, otherwise it holds still.
    pub(super) fn refresh_spinner(
        &mut self,
        app: &App,
        retained: bool,
        motion: bool,
    ) -> Option<&'static str> {
        if !retained {
            self.refreshing_since_ms = None;
            return None;
        }
        let now = crate::status::now_ms();
        let since = *self.refreshing_since_ms.get_or_insert(now);
        let until_ms = since + crate::board::view::SPINNER_DELAY.as_millis() as u64;
        if now < until_ms {
            crate::board::view::ClockSurface::admit_delay_gate(app, until_ms);
            return None;
        }
        if motion {
            crate::board::view::ClockSurface::admit_refresh_spinner(app);
        }
        Some(crate::board::view::feedback_glyph(now, motion))
    }

    #[cfg(test)]
    pub(super) fn set_refreshing_since(&mut self, ms: u64) {
        self.refreshing_since_ms = Some(ms);
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
        let original_notes = self.original_note_sources();
        let Context {
            enabled,
            name,
            times,
            exchanges,
            output_ages,
            final_ages,
            sent_ages,
            reader,
            member: _,
            dropped_age,
        } = &self.transcript_context;
        let sends: Vec<_> = self.restored_sends().collect();
        if self.finals.is_empty()
            && sends.is_empty()
            && !(*enabled && self.pane_failure.is_some())
            && !matches!(&self.pane_output, Observation::Observed(page)
                if *enabled && (page.capture == Some(false) || page.coverage.older_dropped && page.next_before.is_none()))
            && (!enabled
                || (matches!(&self.pane_output, Observation::Pending)
                    || matches!(&self.pane_output, Observation::Observed(items) if items.is_empty()))
                    && (matches!(exchanges, Exchanges::Observed)
                        || self.reply.is_some()
                        || !self.outgoing.is_empty()))
        {
            let copy = self.copy_of_text(&mapped);
            let note_sources = original_notes.project(mapped.sources.iter().copied());
            return RenderedTranscript {
                mapped,
                anchor_text: self.text.clone(),
                response_sources: None,
                final_spans: Vec::new(),
                copy,
                note_sources,
                pane_start: None,
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
        // Displayed headers can be identical (same name and age). Anchor their source
        // identity separately, just as request headers below do, so an older prepend
        // cannot move a reader onto another answer with the same caption.
        let pane_anchors = {
            use std::hash::{Hash, Hasher};
            let mut occurrences = std::collections::BTreeMap::new();
            let mut anchors = items
                .iter()
                .rev()
                .map(|item| {
                    let mut hash = std::hash::DefaultHasher::new();
                    item.hash(&mut hash);
                    let key = hash.finish();
                    let occurrence = occurrences.entry(key).or_insert(0usize);
                    *occurrence += 1;
                    format!("pane-answer:{key:016x}:{}", *occurrence)
                })
                .collect::<Vec<_>>();
            anchors.reverse();
            anchors
        };
        let note_sources = std::cell::RefCell::new(super::note_source::Map::default());
        let unavailable = note_sources
            .borrow_mut()
            .add(super::note_source::Source::Unavailable);
        let next_note = std::cell::Cell::new(unavailable);
        let mut lines = Vec::new();
        let mut sources = Vec::new();
        let mut mentions = Vec::new();
        let mut original = Vec::new();
        let mut anchor_text = String::new();
        let mut final_spans = Vec::new();
        let mut copy_messages = vec![self.text.clone()];
        let mut copy_rows: Vec<super::copy::Row> = Vec::new();
        let next_copy = std::cell::RefCell::new(None);
        let mut header_anchors = std::collections::BTreeMap::new();
        let mut next_mention = mapped
            .mentions
            .iter()
            .flatten()
            .map(|link| link.occurrence + 1)
            .max()
            .unwrap_or(0);
        let mut append = |line: Line<'static>,
                          links: Vec<super::super::member_links::Link>,
                          source: Option<usize>| {
            let index = lines.len();
            note_sources.borrow_mut().row(index, next_note.get());
            if index > 0 {
                anchor_text.push('\n');
            }
            for span in &line.spans {
                anchor_text.push_str(&span.content);
            }
            copy_rows.push(
                next_copy
                    .borrow_mut()
                    .take()
                    .unwrap_or_else(|| super::copy::Row::new(None, line.to_string(), &links)),
            );
            sources.push(index);
            mentions.push(links);
            original.push(source);
            lines.push(line);
            index
        };
        let captured_outputs = self.captured_outputs(items);
        let mut pane_start = None;
        if *enabled && self.pane_failure == Some(AnswerFailure::Older) {
            for line in history_caption(
                "✗ older answers unavailable · scroll up to retry",
                width,
                look,
            ) {
                append(line, vec![], None);
            }
        }
        if *enabled && let Observation::Observed(page) = &self.pane_output {
            if page.next_before.is_none() && page.coverage.older_dropped {
                let text = dropped_age.as_ref().map_or_else(
                    || "○ older answers not kept".into(),
                    |age| format!("○ older answers not kept · at or before {age}"),
                );
                for line in history_caption(&text, width, look) {
                    append(line, vec![], None);
                }
            }
            if page.capture == Some(false) {
                for line in history_caption(
                    "◆ answers are not captured for this agent: run tmt setup",
                    width,
                    look,
                ) {
                    append(line, vec![], None);
                }
            }
        }
        let order = crate::board::recent_responses::merge(&times, items);
        // Sends that follow each other in the transcript, with no final, pane output or exchange
        // between them, may form one message block.
        let mut places = vec![super::answer_order::Place::default(); sends.len()];
        let mut run: Vec<usize> = Vec::new();
        let group = |run: &mut Vec<usize>, places: &mut Vec<super::answer_order::Place>| {
            let placed = super::answer_order::place(
                &run.iter()
                    .map(|index| super::answer_order::Probe {
                        at_ms: sends[*index].at_ms,
                        capture: sends[*index].capture.as_ref(),
                        direct: sends[*index].capture.is_some(),
                    })
                    .collect::<Vec<_>>(),
            );
            for (member, mut place) in run.iter().zip(placed) {
                place.answers = place.answers.iter().map(|answer| run[*answer]).collect();
                places[*member] = place;
            }
            run.clear();
        };
        for source in &order {
            match source {
                Source::Pane(index) if captured_outputs.contains(index) => {}
                Source::Exchange(block) => match messages[*block].1 {
                    super::history::Block::Sent(index) => run.push(index),
                    _ => group(&mut run, &mut places),
                },
                Source::Pane(_) => group(&mut run, &mut places),
            }
        }
        group(&mut run, &mut places);
        let mut first = true;
        let mut previous: Option<&str> = None;
        for source in order {
            if matches!(source, Source::Pane(index) if captured_outputs.contains(&index)) {
                continue;
            }
            // A stacked body follows the previous message of its block without a blank line.
            let stacked = matches!(
                source,
                Source::Exchange(block) if matches!(
                    messages[block].1,
                    super::history::Block::Sent(index)
                        if matches!(
                            places[index].role,
                            super::answer_order::Role::Member | super::answer_order::Role::Last
                        )
                )
            );
            let current = match source {
                Source::Exchange(block) => match messages[block].1 {
                    super::history::Block::Sent(index) => Some(sends[index].text.as_str()),
                    _ => None,
                },
                Source::Pane(_) => None,
            };
            let spaced = stacked
                && matches!((previous, current),
                    (Some(previous), Some(current)) if super::answer_order::spaced(previous, current));
            if !first && (!stacked || spaced) {
                append(Line::default(), vec![], None);
            }
            previous = current;
            first = false;
            match source {
                Source::Exchange(block) => {
                    if let super::history::Block::Sent(index) = messages[block].1 {
                        let send = sends[index];
                        next_note.set(note_sources.borrow_mut().add(
                            super::note_source::Source::request(reader, &send.request, send.at_ms),
                        ));
                        let place = &places[index];
                        if place.header() {
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
                        }
                        let body = markdown::render_conversation_with_members(
                            &sanitize(&send.text),
                            width.saturating_sub(3).max(1),
                            look,
                            None,
                        );
                        let message = copy_messages.len();
                        copy_messages.push(sanitize(&send.text));
                        let mut file_occurrences = std::collections::BTreeMap::new();
                        for ((line, mut links), source) in
                            body.lines.into_iter().zip(body.mentions).zip(body.sources)
                        {
                            next_copy.replace(Some(super::copy::Row::new(
                                (line.width() > 0).then_some(super::copy::At {
                                    message,
                                    line: source,
                                }),
                                line.to_string(),
                                &links,
                            )));
                            links.retain(|link| {
                                matches!(&link.target, super::super::member_links::Target::File(_))
                            });
                            for link in &mut links {
                                link.occurrence =
                                    *file_occurrences.entry(link.occurrence).or_insert_with(|| {
                                        let occurrence = next_mention;
                                        next_mention += 1;
                                        occurrence
                                    });
                                link.column += 3;
                            }
                            append(super::super::exchange_view::body(line, look), links, None);
                        }
                        // The block's one status line sits under its last body.
                        let label = if place.role == super::answer_order::Role::Last {
                            let first = (0..=index)
                                .rev()
                                .find(|member| {
                                    places[*member].role == super::answer_order::Role::First
                                })
                                .unwrap_or(index);
                            let labels = sends[first..=index]
                                .iter()
                                .filter_map(|member| member.message_state_label.as_deref())
                                .collect::<Vec<_>>();
                            (!labels.is_empty())
                                .then(|| super::answer_order::block_status(labels.into_iter()))
                        } else {
                            send.message_state_label.clone()
                        };
                        if place.status()
                            && let Some(text) = &label
                        {
                            for line in markdown::wrap_styled(
                                super::super::exchange_view::feedback_content(text, look),
                                width.saturating_sub(3).max(1),
                            ) {
                                append(super::super::exchange_view::body(line, look), vec![], None);
                            }
                        }
                        // A lone send keeps its own answer; a block's answer follows its last send.
                        let own = send
                            .capture
                            .as_ref()
                            .filter(|capture| {
                                place.role == super::answer_order::Role::Single
                                    && (send.message_state_label.is_none()
                                        || !matches!(
                                            capture,
                                            super::super::row_detail::Capture::Pending
                                                | super::super::row_detail::Capture::Bound
                                        ))
                            })
                            .map(|capture| (send.request.as_str(), capture));
                        let held = place.answers.iter().filter_map(|answered| {
                            let send = sends[*answered];
                            Some((send.request.as_str(), send.capture.as_ref()?))
                        });
                        for (request, capture) in own.into_iter().chain(held) {
                            next_note.set(
                                note_sources
                                    .borrow_mut()
                                    .add(super::note_source::Source::capture(
                                        name, request, capture,
                                    )),
                            );
                            let content = markdown::render_output_with_members(
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
                        next_note.set(
                            note_sources
                                .borrow_mut()
                                .add(super::note_source::Source::reply(reply)),
                        );
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
                        let occurrence_base = next_mention;
                        next_mention += count;
                        let mut end = header + 1;
                        let message = copy_messages.len();
                        copy_messages.push(sanitize(&reply.body));
                        let body_sources = std::mem::take(&mut body.sources);
                        for ((line, mut links), source) in
                            body.lines.into_iter().zip(body.mentions).zip(body_sources)
                        {
                            next_copy.replace(Some(super::copy::Row::new(
                                (line.width() > 0).then_some(super::copy::At {
                                    message,
                                    line: source,
                                }),
                                line.to_string(),
                                &links,
                            )));
                            for link in &mut links {
                                link.occurrence += occurrence_base;
                                link.column += gutter;
                            }
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
                        next_note.set(
                            note_sources
                                .borrow_mut()
                                .import(&original_notes, mapped.sources[index]),
                        );
                        next_copy.replace(Some(super::copy::Row::new(
                            (mapped.lines[index].width() > 0
                                && !self.authored_lines.contains_key(&mapped.sources[index]))
                            .then_some(super::copy::At {
                                message: 0,
                                line: mapped.sources[index],
                            }),
                            mapped.lines[index].to_string(),
                            &mapped.mentions[index],
                        )));
                        append(
                            mapped.lines[index].clone(),
                            mapped.mentions[index].clone(),
                            Some(mapped.sources[index]),
                        );
                    }
                }
                Source::Pane(index) => {
                    let item = &items[index];
                    next_note.set(note_sources.borrow_mut().add(
                        super::note_source::Source::Message {
                            author: name.clone(),
                            kind: "pane output",
                            at_ms: Some(item.ended_ms),
                            reference: format!(
                                "member {} · provider {} · observation {}",
                                self.transcript_context.member,
                                item.provider,
                                index + 1
                            ),
                        },
                    ));
                    let stamp = output_ages.get(index).map_or("—", String::as_str);
                    let header = append(
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
                    pane_start.get_or_insert(header);
                    header_anchors.insert(header, pane_anchors[index].clone());
                    // Display-only member links never give pane text request-source authority.
                    let body = markdown::render_output_with_members(
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
                        for line in markdown::wrap_styled(
                            Line::styled(
                                "◌ answer truncated · full text in the agent's pane",
                                look.role(Role::Muted),
                            ),
                            width.saturating_sub(3).max(1),
                        ) {
                            append(super::super::exchange_view::body(line, look), vec![], None);
                        }
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
                next_note.set(unavailable);
                append(Line::styled(note, look.role(Role::Muted)), vec![], None);
            }
        }
        if *enabled
            && self.pane_failure != Some(AnswerFailure::Older)
            && (self.pane_failure.is_some() || matches!(self.pane_output, Observation::Unavailable))
        {
            next_note.set(unavailable);
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
            note_sources: note_sources.into_inner(),
            copy: super::copy::Map {
                messages: copy_messages,
                rows: copy_rows,
            },
            pane_start,
        }
    }

    /// The source of the unmerged transcript: the session text, but for its authored lines.
    fn copy_of_text(&self, mapped: &Mapped) -> super::copy::Map {
        super::copy::Map {
            messages: vec![self.text.clone()],
            rows: mapped
                .lines
                .iter()
                .zip(&mapped.sources)
                .zip(&mapped.mentions)
                .map(|((line, source), links)| {
                    super::copy::Row::new(
                        (line.width() > 0 && !self.authored_lines.contains_key(source)).then_some(
                            super::copy::At {
                                message: 0,
                                line: *source,
                            },
                        ),
                        line.to_string(),
                        links,
                    )
                })
                .collect(),
        }
    }
}

pub(super) struct RenderedTranscript {
    pub pane_start: Option<usize>,
    pub mapped: Mapped,
    pub anchor_text: String,
    pub response_sources: Option<Vec<Option<usize>>>,
    pub final_spans: Vec<(std::ops::Range<usize>, usize)>,
    pub copy: super::copy::Map,
    pub note_sources: super::note_source::Map,
}
