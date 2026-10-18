//! Pane-output presentation shares the conversation viewport, never its request authority.
use super::*;
use crate::board::recent_responses::{Observation, Source};

#[derive(Default, PartialEq, Eq)]
pub(super) struct Context {
    pub enabled: bool,
    pub name: String,
    pub times: Vec<u64>,
    pub exchanges_observed: bool,
    pub output_ages: Vec<String>,
    pub final_ages: Vec<String>,
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
        self.workspace_snapshot().is_none() || self.shell.tab == crate::board::shell::OpsTab::Agents
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
            session.rendered = None;
        }
        session.pane_output_fresh = true;
    }
}

impl Session {
    /// Metadata is already acquired. Unknown times remain unknown (zero for
    /// merge placement), rather than fabricating an observation from frame time.
    pub(super) fn transcript_context(&mut self, app: &App, row: &serde_json::Value) {
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
            times.push(entry.started_ms);
            if entry.reply.is_some() {
                times.push(time(entry.reply.as_ref()));
            }
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
            reader: app
                .view
                .as_ref()
                .and_then(|view| view.me.clone())
                .unwrap_or_else(|| "you".into()),
            exchanges_observed: app
                .home_leads
                .results_observed(app.view.as_ref().and_then(|view| view.me_id.as_deref())),
        };
        if self.transcript_context != context {
            self.transcript_context = context;
            self.rendered = None;
        }
    }

    pub(super) fn merge_pane_output(
        &self,
        mapped: Mapped,
        width: usize,
        look: Look,
        inventory: &super::super::inventory::Inventory,
    ) -> RenderedTranscript {
        let Context {
            enabled,
            name,
            times,
            exchanges_observed,
            output_ages,
            final_ages,
            reader,
        } = &self.transcript_context;
        if self.finals.is_empty()
            && (!enabled
                || (matches!(&self.pane_output, Observation::Pending)
                    || matches!(&self.pane_output, Observation::Observed(items) if items.is_empty()))
                    && (*exchanges_observed || self.reply.is_some() || !self.outgoing.is_empty()))
        {
            return RenderedTranscript {
                mapped,
                anchor_text: self.text.clone(),
                response_sources: None,
                final_spans: Vec::new(),
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
            anchor_text.push_str(&line.to_string());
            sources.push(index);
            mentions.push(links);
            original.push(source);
            lines.push(line);
            index
        };
        let mut first = true;
        for source in crate::board::recent_responses::merge(&times, items) {
            if !first {
                append(Line::default(), vec![], None);
            }
            first = false;
            match source {
                Source::Exchange(block) => {
                    if let super::history::Block::Final(index) = messages[block].1 {
                        let reply = &self.finals[index];
                        let header = append(
                            super::super::exchange_view::header(
                                &reply.from,
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
                        }
                        next_mention += count;
                        let mut end = header + 1;
                        for (line, links) in body.lines.into_iter().zip(body.mentions) {
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
                                &format!("{name} · pane output · {stamp}"),
                                width,
                            ),
                            look.role(Role::Muted),
                        ),
                        vec![],
                        None,
                    );
                    // Pane text is untrusted display data; no request source or member-action hits.
                    let body = markdown::render_conversation_with_members(
                        &sanitize(&item.text),
                        width.saturating_sub(3).max(1),
                        look,
                        None,
                    );
                    for line in body.lines {
                        append(super::super::exchange_view::body(line, look), vec![], None);
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
        if !has_exchange && !exchanges_observed {
            append(
                Line::styled("Exchange history unavailable", look.role(Role::Muted)),
                vec![],
                None,
            );
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
        }
    }
}

pub(super) struct RenderedTranscript {
    pub mapped: Mapped,
    pub anchor_text: String,
    pub response_sources: Option<Vec<Option<usize>>>,
    pub final_spans: Vec<(std::ops::Range<usize>, usize)>,
}
