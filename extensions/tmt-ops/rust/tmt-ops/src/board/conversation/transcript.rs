//! Pane output and checklist activity share the conversation viewport, never request authority.
use super::*;
use crate::board::recent_responses::Observation;

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
    pub activity: Option<std::sync::Arc<super::activity::Presentation>>,
    pub enabled: bool,
    pub read_only: bool,
    pub name: String,
    pub exchanges: Exchanges,
    pub output_ages: Vec<String>,
    pub final_ages: Vec<String>,
    pub sent_ages: Vec<String>,
    pub reader: String,
    pub viewer: Option<String>,
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

    /// Share the existing admitted timeline with a selected read; never copy its bodies.
    pub(in crate::board) fn held_conversation_timeline(
        &self,
        key: &ConversationKey,
    ) -> Option<std::sync::Arc<crate::board::recent_responses::Page>> {
        let state = self.conversation.borrow();
        match &state.sessions.get(key)?.pane_output {
            Observation::Observed(page) if page.timeline.is_some() => {
                Some(std::sync::Arc::clone(page))
            }
            _ => None,
        }
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
            .and_then(|_| held.as_ref()?.cursor());
        Some(crate::board::recent_responses::AnswerRead {
            key,
            before,
            held,
            rebuild: session.is_some_and(|session| session.pane_rebuild),
        })
    }

    #[cfg(test)]
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
            && matches!(&observation, Observation::Observed(page) if page.cursor().is_some())
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
            || !matches!(&session.pane_output, Observation::Observed(page) if page.cursor().is_some())
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
        let output_ages = match &self.pane_output {
            Observation::Observed(page) => {
                let mut items = page.items.iter().collect::<Vec<_>>();
                if let Some(timeline) = &page.timeline {
                    items.extend(timeline.entries.iter().filter_map(|entry| {
                        if let super::super::recent_responses::timeline::Kind::Final {
                            response_id,
                            ..
                        } = &entry.kind
                        {
                            timeline.responses.get(response_id)
                        } else {
                            None
                        }
                    }));
                }
                items
                    .into_iter()
                    .map(|item| {
                        super::timeline::entry_stamp(
                            "",
                            (item.ended_ms > 0).then_some(item.ended_ms),
                        )
                    })
                    .collect()
            }
            _ => vec![],
        };
        let activity =
            super::activity::Presentation::get(app, self.transcript_context.activity.as_ref(), id);
        let context = Context {
            activity,
            enabled,
            read_only: app
                .composer_projection()
                .is_none_or(|projection| projection.key.is_none())
                || self.change != Change::Current,
            name,
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
                .sent
                .iter()
                .map(|send| {
                    let capture = match &send.capture {
                        Some(crate::board::row_detail::Capture::Answered { ended_ms, .. }) => {
                            super::timeline::entry_stamp("", Some(*ended_ms))
                        }
                        _ => String::new(),
                    };
                    format!(
                        "{} / {capture}",
                        super::timeline::entry_stamp("", Some(send.at_ms))
                    )
                })
                .chain(
                    self.outgoing
                        .iter()
                        .map(|entry| super::timeline::entry_stamp("", Some(entry.started_ms))),
                )
                .collect(),
            viewer: app.view.as_ref().and_then(|view| view.me_id.clone()),
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
        self.transcript_context
            .activity
            .as_ref()
            .is_some_and(|activity| !activity.is_empty())
            || self.reply.is_some()
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
}

pub(super) struct RenderedTranscript {
    pub pane_start: Option<usize>,
    pub mapped: Mapped,
    pub anchor_text: String,
    pub timeline_spans: Vec<super::timeline::BodySpan>,
    pub timeline_inputs: Vec<String>,
    pub feedback: Vec<(std::ops::Range<usize>, u64)>,
    pub copy: super::copy::Map,
    pub note_sources: super::note_source::Map,
}
