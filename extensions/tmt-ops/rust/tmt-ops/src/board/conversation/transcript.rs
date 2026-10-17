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
}

impl State {
    pub(in crate::board) fn refresh_pane_output(&mut self) {
        for session in self.sessions.values_mut() {
            session.pane_output_fresh = false;
        }
    }
}

impl App {
    fn pane_output_enabled(&self) -> bool {
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
    ) -> RenderedTranscript {
        let Context {
            enabled,
            name,
            times,
            exchanges_observed,
            output_ages,
        } = &self.transcript_context;
        if !enabled
            || (matches!(&self.pane_output, Observation::Pending)
                || matches!(&self.pane_output, Observation::Observed(items) if items.is_empty()))
                && (*exchanges_observed || self.reply.is_some() || !self.outgoing.is_empty())
        {
            return RenderedTranscript {
                mapped,
                anchor_text: self.text.clone(),
                response_sources: None,
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
        let times: Vec<_> = (0..blocks.len())
            .map(|index| times.get(index).copied().unwrap_or(0))
            .collect();
        let items = match &self.pane_output {
            Observation::Observed(items) => items.as_slice(),
            _ => &[],
        };
        let mut lines = Vec::new();
        let mut sources = Vec::new();
        let mut mentions = Vec::new();
        let mut original = Vec::new();
        let mut anchor_text = String::new();
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
        };
        for source in crate::board::recent_responses::merge(&times, items) {
            match source {
                Source::Exchange(block) => {
                    for &index in &blocks[block] {
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
                    let body = markdown::render(
                        &sanitize(&item.text),
                        width.saturating_sub(3).max(1),
                        look,
                    );
                    for line in body {
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
        RenderedTranscript {
            mapped: Mapped {
                lines,
                sources,
                mentions,
            },
            anchor_text,
            response_sources: Some(original),
        }
    }
}

pub(super) struct RenderedTranscript {
    pub mapped: Mapped,
    pub anchor_text: String,
    pub response_sources: Option<Vec<Option<usize>>>,
}
