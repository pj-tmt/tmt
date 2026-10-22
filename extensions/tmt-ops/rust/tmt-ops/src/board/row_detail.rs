//! One read-only entity detail projection and renderer for every board row.
//! Acquisition remains on the refresh worker; this module only consumes snapshots.
use super::{
    app::{App, Effect, RowTarget},
    home_leads::{FinalsKey, Kind, MessageKey, SentKey},
};
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use serde_json::{Value, json};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Row(RowTarget),
    Job(String),
}

/// How far Core has answered for the exchanges a conversation shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::board) enum ExchangesRead {
    Pending,
    Observed,
    Failed,
}

#[derive(Default)]
pub(super) struct State {
    pub expanded: Vec<Target>,
    /// Bodies belong to active rows and exact request identities, never row positions.
    bodies: Vec<(MessageKey, Result<String, String>)>,
    /// Each active agent's own finals, read once per activation. A failed read
    /// is kept as empty so it is not retried in a loop; the windows still show.
    agent_finals: Vec<(FinalsKey, Vec<Value>)>,
    /// The agents whose finals read failed, so the conversation can say so.
    finals_failed: Vec<FinalsKey>,
    /// The same agent's durable sends from this user, read after its finals.
    agent_sent: Vec<(SentKey, Vec<Value>)>,
    sent_stale: bool,
    pub reader: Option<Reader>,
}

impl State {
    pub fn toggle(&mut self, target: Target) {
        if let Some(index) = self.expanded.iter().position(|entry| entry == &target) {
            self.expanded.remove(index);
        } else {
            self.expanded.push(target);
        }
    }
    pub fn reconcile(&mut self, alive: &[Target], keys: &[MessageKey]) {
        self.expanded.retain(|target| alive.contains(target));
        if self
            .reader
            .as_ref()
            .is_some_and(|reader| reader.frozen.is_none() && !alive.contains(&reader.target))
        {
            self.reader = None;
        }
        self.bodies.retain(|(key, _)| keys.contains(key));
    }
    fn body_failed(&self, key: &MessageKey) -> bool {
        self.bodies
            .iter()
            .any(|(current, body)| current == key && body.is_err())
    }
    fn body(&self, key: &MessageKey) -> Option<&str> {
        self.bodies
            .iter()
            .find(|(current, _)| current == key)
            .map(|(_, body)| match body {
                Ok(text) | Err(text) => text.as_str(),
            })
    }
}

#[derive(Clone)]
pub(super) struct Field {
    pub label: String,
    pub text: String,
    pub role: Role,
}

#[derive(Clone)]
pub(super) struct Reply {
    pub from: String,
    /// The age when the detail was built; a header derives it again from `at_ms`.
    pub age: String,
    /// When the message settled (`submittedAtMs`) or was prepared, if known.
    pub at_ms: Option<u64>,
    pub body: String,
    pub key: Option<MessageKey>,
    pub acquired: bool,
    /// Only successful exact bodies may produce source-range attachments.
    pub source: bool,
}

#[derive(Clone, Default)]
pub(super) struct Detail {
    pub fields: Vec<Field>,
    pub reply: Option<Reply>,
    /// The newest settled finals to this user's requests, oldest first, read
    /// from storage by request ID. `reply` is the last of them.
    pub finals: Vec<Reply>,
    /// Whose finals these are, when they can be read from storage by agent.
    pub(super) agent: Option<FinalsKey>,
    /// This user's durable sends to the agent, oldest first (restored after a
    /// restart; sends made in this session are shown from the session itself).
    pub sent: Vec<Sent>,
    pub show_reply: bool,
}

/// One of the user's own sends, restored from the request record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Sent {
    pub request: String,
    pub at_ms: u64,
    pub text: String,
    /// Only the listed preview survives: the full prompt is no longer retained.
    pub truncated: bool,
    pub capture: Option<Capture>,
    pub message_state_label: Option<String>,
}

/// Core's captured turn, independent of ordinary replies and pane observations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Capture {
    Pending,
    Bound,
    Answered {
        text: String,
        truncated: bool,
        provider: String,
        ended_ms: u64,
    },
    NoAnswer,
}
impl Capture {
    fn read(send: &Value) -> Option<Self> {
        if send["mode"] != "passthrough" {
            return None;
        }
        let capture = &send["passthrough"];
        match capture["state"].as_str()? {
            "pending" => Some(Self::Pending),
            "bound" => Some(Self::Bound),
            "answered" => Some(Self::Answered {
                text: capture["answer"]["text"]
                    .as_str()
                    .filter(|text| text.len() <= 8192)?
                    .to_owned(),
                truncated: capture["answer"]["truncated"].as_bool()?,
                provider: capture["answer"]["provider"].as_str()?.to_owned(),
                ended_ms: capture["answer"]["endedMs"].as_u64()?,
            }),
            "no_answer" => Some(Self::NoAnswer),
            _ => None,
        }
    }
    pub(super) fn markdown(&self) -> String {
        match self {
            Self::Pending | Self::Bound => "⠹ working…".into(),
            Self::NoAnswer => "No turn output was captured for this message.".into(),
            Self::Answered {
                text, truncated, ..
            } => format!(
                "turn output{}\n\n{}",
                if *truncated { " · truncated" } else { "" },
                super::notes::sanitize(text).trim_end_matches('\n')
            ),
        }
    }
}

/// A collapsed field suppresses detail only if its complete text survives the
/// same cell budget and overflow policy that paint uses.
pub(super) fn uncut(text: &str, width: usize, flow: tmt_tui::style::TextFlow) -> bool {
    use tmt_tui::style::TextFlow;
    let text = tmt_cli_style::table::escape(text);
    let width = width.min(usize::from(u16::MAX));
    if width == 0 {
        return false;
    }
    match flow {
        TextFlow::Wrap | TextFlow::Clamp(_) => {
            let content = |text: &str| {
                text.chars()
                    .filter(|ch| !ch.is_whitespace())
                    .collect::<String>()
            };
            content(&tmt_tui::text::lines(&text, width as u16, flow).join("")) == content(&text)
        }
        _ => text.width() <= width,
    }
}

impl Reply {
    /// The age at `now`, from the settled time when it is known: a held or
    /// cached reply never keeps the age string it had when it was built.
    pub fn age_at(&self, now: u64) -> String {
        self.at_ms
            .filter(|at| *at > 0 && *at <= now)
            .map_or_else(|| self.age.clone(), |at| crate::requests::age(now, at))
    }
}

impl Detail {
    /// Every message this detail needs a body for: the finals, else the one reply.
    pub fn messages(&self) -> Vec<&Reply> {
        if self.finals.is_empty() {
            self.reply.iter().collect()
        } else {
            self.finals.iter().collect()
        }
    }

    fn add(&mut self, label: &str, text: Option<&str>, role: Role) {
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            self.fields.push(Field {
                label: label.to_owned(),
                text: super::notes::sanitize(text),
                role,
            });
        }
    }
    pub fn additional(mut self, visible: &[&str]) -> Self {
        self.fields
            .retain(|field| !visible.contains(&field.label.as_str()));
        self
    }
    /// Cache keys contain exactly the presentation inputs; request identities stay
    /// in application state, outside the renderer's data.
    pub fn value(&self) -> Value {
        json!({"fields": self.fields.iter().map(|field| json!({"label":field.label,"text":field.text,"role":field.role.name()})).collect::<Vec<_>>(),
            "reply": self.reply.as_ref().map(|reply| json!({"from":reply.from,"age":reply.age,"body":reply.body})),
            "show_reply": self.show_reply})
    }
}

/// How many settled finals a conversation holds per agent.
pub(super) const FINALS: usize = 8;

fn sent_key(key: &FinalsKey) -> SentKey {
    SentKey {
        sender: key.sender.clone(),
        agent: key.agent.clone(),
    }
}

fn age(at: Option<u64>, now: u64) -> String {
    at.filter(|at| *at > 0 && *at <= now)
        .map(|at| crate::requests::age(now, at))
        .unwrap_or_else(|| "–".into())
}

impl App {
    pub(super) fn row_detail(&self, target: &RowTarget, visible: &[&str]) -> Option<Detail> {
        let row = match target {
            RowTarget::Workspace(_) => self.target_row(target)?,
            RowTarget::Member {
                tab,
                section,
                squad,
                id,
            } if self.shown_tab() == Some(tab) => self.view.as_ref()?.document["sections"]
                [*section]["rows"]
                .as_array()?
                .iter()
                .find(|row| row["id"] == *id && row["squad"].as_str().unwrap_or(tab) == squad)?,
            RowTarget::Lead { .. } => self.target_row(target)?,
            _ => return None,
        };
        let mut detail = Detail {
            show_reply: true,
            ..Detail::default()
        };
        if !visible.contains(&"task") {
            detail.add("task", row["fields"]["task"].as_str(), Role::Text);
        }
        if !visible.contains(&"pending") {
            let pending = super::view::waiting::text(row).map(|text| format!("◆ {text}"));
            detail.add("pending", pending.as_deref(), Role::Waiting);
        }
        if !visible.contains(&"note") {
            detail.add(
                "note",
                row["note"]
                    .as_str()
                    .or_else(|| row["fields"]["note"].as_str()),
                Role::Text,
            );
        }
        let links = row["fields"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| {
                (key.as_str() == "link" || key.ends_with("_link"))
                    && !visible.contains(&key.as_str())
            })
            .filter_map(|(_, value)| value.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("  ");
        if !visible.contains(&"links") {
            detail.add("links", Some(&links), Role::Link);
        }
        if !visible.contains(&"model") {
            detail.add("model", row["fields"]["model"].as_str(), Role::Text);
        }
        let now = crate::status::now_ms();
        if !visible.contains(&"chips") {
            for (label, text) in crate::board::row_chips::detail(&self.labels, row, now) {
                detail.add(&label, Some(&text), Role::Muted);
            }
        }
        let (squad, id) = match target {
            RowTarget::Workspace(super::workspace::OpsTarget::Member { identity_id, .. }) => (
                row["squad"].as_str().unwrap_or("Common pool"),
                identity_id.as_str(),
            ),
            RowTarget::Workspace(_) => return None,
            RowTarget::Member { squad, id, .. } | RowTarget::Lead { squad, id, .. } => {
                (squad.as_str(), id.as_str())
            }
        };
        let sender = self.view.as_ref().and_then(|view| view.me_id.clone());
        // The flat list discusses the acquired request the member is waiting on,
        // not an unrelated newer response to one of our outgoing messages.
        let flat = self.workspace_snapshot().is_some() && self.flat_agents();
        if flat && let Some(question) = self.pending_question(target, None) {
            detail.reply = Some(question);
            return Some(detail.additional(visible));
        }
        if flat {
            // A board decision without an observed request is not a transcript.
            return Some(detail.additional(visible));
        }
        detail.finals = self.finals(row, squad, id, sender.as_deref(), now);
        // An agent answers by sending the user a new request as often as by replying to one
        // of the user's, so every open request of the agent belongs in the thread at the time
        // it was sent; a second one must not hide behind the first.
        let mut open: Vec<Reply> = row["waitingOnYou"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| self.pending_question(target, item["requestId"].as_str()))
            .collect();
        open.sort_by_key(|question| question.at_ms);
        for question in open {
            let at = detail
                .finals
                .iter()
                .position(|held| held.at_ms > question.at_ms)
                .unwrap_or(detail.finals.len());
            detail.finals.insert(at, question);
        }
        detail.reply = detail.finals.last().cloned();
        detail.sent = self.sent(id, sender.as_deref());
        detail.agent = sender.map(|sender| FinalsKey {
            sender,
            agent: id.into(),
        });
        Some(detail.additional(visible))
    }

    /// The user's durable sends to this agent that storage returned.
    fn sent(&self, id: &str, sender: Option<&str>) -> Vec<Sent> {
        self.row_details
            .agent_sent
            .iter()
            .filter(|(key, _)| Some(key.sender.as_str()) == sender && key.agent == id)
            .flat_map(|(_, sends)| sends)
            .filter_map(|send| {
                Some(Sent {
                    request: send["requestId"].as_str()?.into(),
                    at_ms: send["preparedAtMs"].as_u64()?,
                    text: send["text"].as_str()?.into(),
                    truncated: send["truncated"].as_bool().unwrap_or(false),
                    capture: Capture::read(send),
                    message_state_label: crate::requests::message_state_label(send),
                })
            })
            .collect()
    }

    /// The newest settled finals to this user's requests, oldest first. They come
    /// from the bounded originator and room pages (storage, by request ID), so
    /// neither the originator notification nor pane output is involved.
    fn finals(
        &self,
        row: &Value,
        squad: &str,
        id: &str,
        sender: Option<&str>,
        now: u64,
    ) -> Vec<Reply> {
        let mut settled: Vec<&Value> = Vec::new();
        let own = self
            .row_details
            .agent_finals
            .iter()
            .filter(|(key, _)| Some(key.sender.as_str()) == sender && key.agent == id)
            .flat_map(|(_, replies)| replies);
        for reply in self
            .home_leads
            .replies
            .iter()
            .chain(self.view.iter().flat_map(|view| &view.replies))
            .chain(own)
            .filter(|reply| reply["recipientId"] == id)
        {
            match settled
                .iter_mut()
                .find(|held| held["requestId"] == reply["requestId"])
            {
                // The room page carries a body when it already read one.
                Some(held) if held["response"].is_null() && reply["response"].is_string() => {
                    *held = reply;
                }
                Some(_) => {}
                None => settled.push(reply),
            }
        }
        settled.sort_by_key(|reply| (reply["submittedAtMs"].as_u64(), reply["requestId"].as_str()));
        let skip = settled.len().saturating_sub(FINALS);
        settled
            .into_iter()
            .skip(skip)
            .map(|reply| {
                let key = sender
                    .zip(reply["requestId"].as_str())
                    .map(|(sender, request)| MessageKey {
                        sender: sender.into(),
                        lead: id.into(),
                        squad: squad.to_owned(),
                        request: request.into(),
                        kind: Kind::Reply,
                    });
                let body = reply["response"]
                    .as_str()
                    .or_else(|| key.as_ref().and_then(|key| self.row_details.body(key)));
                Reply {
                    from: row["name"].as_str().unwrap_or("–").into(),
                    age: age(reply["submittedAtMs"].as_u64(), now),
                    at_ms: reply["submittedAtMs"].as_u64(),
                    body: body.unwrap_or("(reading reply…)").into(),
                    source: body.is_some()
                        && (reply["response"].is_string()
                            || key
                                .as_ref()
                                .is_none_or(|key| !self.row_details.body_failed(key))),
                    key,
                    acquired: body.is_some(),
                }
            })
            .collect()
    }

    /// One held request projection for Overview and explicit answers, even if
    /// a compact editor has moved to the Agents placement.
    pub(in crate::board) fn pending_question(
        &self,
        target: &RowTarget,
        request: Option<&str>,
    ) -> Option<Reply> {
        let row = self.target_row(target)?;
        let question = if let Some(request) = request {
            row["waitingOnYou"]
                .as_array()?
                .iter()
                .find(|item| item["requestId"].as_str() == Some(request))?
        } else {
            super::view::waiting::oldest(row)?
        };
        let request = question["requestId"].as_str()?;
        let identity = row["id"].as_str()?;
        let key = self.view.as_ref()?.me_id.clone().map(|sender| MessageKey {
            sender,
            lead: identity.into(),
            squad: self.target_squad(target).unwrap_or_default(),
            request: request.into(),
            kind: Kind::Question,
        });
        let body = key.as_ref().and_then(|key| self.row_details.body(key));
        Some(Reply {
            from: row["name"].as_str().unwrap_or("unknown agent").into(),
            age: age(question["preparedAtMs"].as_u64(), crate::status::now_ms()),
            at_ms: question["preparedAtMs"].as_u64(),
            body: body
                .or_else(|| question["preview"].as_str())
                .unwrap_or("(question unavailable)")
                .into(),
            acquired: body.is_some(),
            source: body.is_some()
                && key
                    .as_ref()
                    .is_some_and(|key| !self.row_details.body_failed(key)),
            key,
        })
    }

    pub(super) fn selected_detail_target(&self) -> Option<Target> {
        if let Some(list) = &self.cron_list {
            return list.selected().map(Target::Job);
        }
        self.row_target(self.selected).map(Target::Row)
    }
    pub(super) fn detail(&self, target: &Target) -> Option<Detail> {
        match target {
            Target::Row(target) => self.row_detail(target, &[]),
            Target::Job(id) => self
                .cron
                .cron
                .as_ref()?
                .jobs
                .iter()
                .find(|job| super::cronboard::detail_id(job) == *id)
                .map(|job| super::cronboard::job_detail(job, self.cron.now_ms())),
        }
    }
    pub(super) fn toggle_row_detail(&mut self) -> Effect {
        if let Some(target) = self
            .selected_detail_target()
            .filter(|target| self.detail(target).is_some())
        {
            if matches!(target, Target::Row(_)) {
                self.focus = self
                    .effective_board()
                    .and_then(|board| {
                        board
                            .panes
                            .iter()
                            .position(|pane| *pane == crate::config::Pane::Rows)
                    })
                    .unwrap_or(0);
            }
            self.row_details.toggle(target);
            self.follow = true;
            self.reconcile_row_details();
        }
        Effect::None
    }
    fn active_detail_targets(&self) -> Vec<Target> {
        let mut targets = self.row_details.expanded.clone();
        if self.scrolls.visible(crate::config::Pane::Replies)
            && let Some(target) = self.conversation_target()
        {
            targets.push(Target::Row(target));
        }
        if let Some(reader) = &self.row_details.reader {
            targets.push(reader.target.clone());
        }
        targets
    }
    pub(super) fn reconcile_row_details(&mut self) {
        if self.row_details.reader.as_ref().is_some_and(|reader| {
            reader.frozen.is_none()
                && self
                    .detail(&reader.target)
                    .and_then(|detail| detail.reply)
                    .is_none()
        }) {
            self.row_details.reader = None;
        }
        let alive = self
            .active_detail_targets()
            .into_iter()
            .filter(|target| self.detail(target).is_some())
            .collect::<Vec<_>>();
        let mut keys = alive
            .iter()
            .filter_map(|target| self.detail(target))
            .flat_map(|detail| {
                detail
                    .messages()
                    .into_iter()
                    .filter_map(|reply| reply.key.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        if let Some(target) = self.input.as_ref().and_then(|input| input.target()) {
            // Keep already acquired question bodies through compact composer
            // placement and an in-flight send. No new read or body owner: retain
            // only cached requests still admitted for this sender and row.
            keys.extend(
                self.row_details
                    .bodies
                    .iter()
                    .filter(|(key, _)| {
                        key.kind == Kind::Question
                            && self
                                .pending_question(target, Some(&key.request))
                                .is_some_and(|reply| {
                                    reply
                                        .key
                                        .as_ref()
                                        .is_some_and(|current| current.same_source(key))
                                })
                    })
                    .map(|(key, _)| key.clone()),
            );
        }
        let owner = self.conversation_finals();
        self.row_details.reconcile(&alive, &keys);
        self.row_details
            .agent_finals
            .retain(|(key, _)| Some(key) == owner.as_ref());
        self.row_details
            .finals_failed
            .retain(|key| Some(key) == owner.as_ref());
        let sent_owner = owner.as_ref().map(sent_key);
        self.row_details
            .agent_sent
            .retain(|(key, _)| Some(key) == sent_owner.as_ref());
    }
    /// Whose finals the painted conversation shows. Only that conversation reads
    /// an agent's whole run of finals; other details show the newest alone.
    pub(in crate::board) fn conversation_finals(&self) -> Option<FinalsKey> {
        if self.loading()
            || !self.pane_output_enabled()
            || !self.scrolls.visible(crate::config::Pane::Replies)
        {
            return None;
        }
        self.detail(&Target::Row(self.conversation_target()?))?
            .agent
    }
    /// Source demand for an admitted conversation painter. Unlike acquisition,
    /// this does not consult Scrolls: the current frame is still registering it.
    pub(super) fn conversation_sources_pending(&self, target: &RowTarget) -> bool {
        let Some(detail) = self.detail(&Target::Row(target.clone())) else {
            return false;
        };
        let Some(owner) = detail.agent.as_ref() else {
            return false;
        };
        !self
            .row_details
            .agent_finals
            .iter()
            .any(|(key, _)| key == owner)
            || self.row_details.sent_stale
            || !self
                .row_details
                .agent_sent
                .iter()
                .any(|(key, _)| key == &sent_key(owner))
            || detail.messages().iter().any(|message| !message.acquired)
    }

    /// The conversation's agent, until its own finals have been read once.
    pub(super) fn finals_read(&self) -> Option<FinalsKey> {
        self.conversation_finals().filter(|key| {
            !self
                .row_details
                .agent_finals
                .iter()
                .any(|(read, _)| read == key)
        })
    }
    /// The conversation's agent, once its finals are in and its sends are not.
    pub(super) fn sent_read(&self) -> Option<SentKey> {
        let key = sent_key(&self.conversation_finals()?);
        let finals_in = self
            .row_details
            .agent_finals
            .iter()
            .any(|(read, _)| sent_key(read) == key);
        (finals_in
            && (self.row_details.sent_stale
                || !self
                    .row_details
                    .agent_sent
                    .iter()
                    .any(|(read, _)| read == &key)))
        .then_some(key)
    }
    pub(super) fn refresh_sent(&mut self) {
        self.row_details.sent_stale = true;
    }

    pub(super) fn apply_sent(&mut self, key: SentKey, result: Result<Vec<Value>, String>) {
        if self.conversation_finals().as_ref().map(sent_key).as_ref() != Some(&key) {
            return;
        }
        self.row_details.sent_stale = false;
        if result.is_err()
            && self
                .row_details
                .agent_sent
                .iter()
                .any(|(current, _)| current == &key)
        {
            return; // A failed refresh cannot erase an already acquired capture.
        }
        self.row_details
            .agent_sent
            .retain(|(current, _)| current != &key);
        self.row_details
            .agent_sent
            .push((key, result.unwrap_or_default()));
    }
    pub(super) fn apply_finals(&mut self, key: FinalsKey, result: Result<Vec<Value>, String>) {
        if self.conversation_finals().as_ref() != Some(&key) {
            return;
        }
        self.row_details
            .agent_finals
            .retain(|(current, _)| current != &key);
        self.row_details
            .finals_failed
            .retain(|current| current != &key);
        if result.is_err() {
            self.row_details.finals_failed.push(key.clone());
        }
        self.row_details
            .agent_finals
            .push((key, result.unwrap_or_default()));
    }
    /// Whether the conversation has heard from Core about its exchanges. An
    /// agent's own finals read answers for that agent; without one (a squad or
    /// lead view) the shared originator page answers.
    pub(in crate::board) fn exchanges_read(&self, agent: Option<&FinalsKey>) -> ExchangesRead {
        let me = self.view.as_ref().and_then(|view| view.me_id.as_deref());
        match agent {
            Some(key) if self.row_details.finals_failed.contains(key) => ExchangesRead::Failed,
            Some(key)
                if self
                    .row_details
                    .agent_finals
                    .iter()
                    .any(|(read, _)| read == key) =>
            {
                ExchangesRead::Observed
            }
            Some(_) => ExchangesRead::Pending,
            None if self.home_leads.results_observed(me) => ExchangesRead::Observed,
            None if self.home_leads.results_failed(me) => ExchangesRead::Failed,
            None => ExchangesRead::Pending,
        }
    }
    pub(super) fn detail_read(&self) -> Option<MessageKey> {
        if self.loading() {
            return None;
        }
        self.active_detail_targets()
            .iter()
            .filter_map(|target| self.detail(target))
            // The newest message first, so the one in view arrives before history.
            .flat_map(|detail| {
                detail
                    .messages()
                    .into_iter()
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .find(|reply| !reply.acquired)
            .and_then(|reply| reply.key)
    }
    pub(super) fn apply_message(&mut self, key: &MessageKey, body: Result<String, String>) {
        if self.loading()
            || !self.active_detail_targets().iter().any(|target| {
                self.detail(target).is_some_and(|detail| {
                    detail
                        .messages()
                        .iter()
                        .any(|reply| reply.key.as_ref() == Some(key))
                })
            })
        {
            return;
        }
        self.row_details
            .bodies
            .retain(|(current, _)| current != key);
        self.row_details.bodies.push((
            key.clone(),
            body.map_err(|error| {
                format!(
                    "({} unavailable: {})",
                    if key.kind == Kind::Question {
                        "question"
                    } else {
                        "reply"
                    },
                    super::notes::sanitize(&error)
                )
            }),
        ));
    }
    fn selected_reply_target(&self) -> Option<Target> {
        self.selected_detail_target().filter(|target| {
            self.detail(target)
                .is_some_and(|detail| detail.reply.is_some())
        })
    }
    pub(super) fn view_row_reply(&mut self) -> Effect {
        if let Some(target) = self.selected_reply_target() {
            self.row_details.reader = Some(Reader::new(
                target,
                self.conversation_reply().filter(|reply| reply.acquired),
            ));
        }
        Effect::None
    }
    pub(super) fn reply_hint(&self) -> bool {
        self.selected_reply_target().is_some()
    }
}

/// Shared typed field/reply preview renderer; viewport and input belong to callers.
pub(super) fn render(
    data: &Value,
    width: u16,
    indent: u16,
    look: Look,
    selected: bool,
) -> Vec<Line<'static>> {
    if data.is_null() {
        return Vec::new();
    }
    let fields = data["fields"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let labels = if width < super::home::COMPACT {
        5
    } else {
        fields
            .iter()
            .filter_map(|field| field["label"].as_str())
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(5)
            .clamp(5, 10)
    };
    let indent = usize::from(indent.min(width.saturating_sub(1)));
    let value_width = usize::from(width)
        .saturating_sub(indent + labels + 2)
        .max(1);
    let muted = look.role(Role::Muted);
    let mut output = Vec::new();
    let mut line = |label: &str, value: Line<'static>| {
        let mut spans = vec![
            Span::raw(" ".repeat(indent.saturating_sub(1))),
            Span::styled(
                "│",
                look.role(if selected { Role::Accent } else { Role::Dim }),
            ),
            Span::styled(
                if fields.is_empty() && data["show_reply"] != true {
                    String::new()
                } else {
                    format!("{:<labels$}  ", super::view::fit(label, labels).trim_end())
                },
                muted,
            ),
        ];
        spans.extend(value.spans.into_iter().map(|span| {
            let style = value.style.patch(span.style);
            span.style(style)
        }));
        output.push(Line::from(spans));
    };
    for field in fields {
        let label = field["label"].as_str().unwrap_or_default();
        let role = field["role"]
            .as_str()
            .and_then(crate::look::role)
            .unwrap_or(Role::Text);
        let text = super::notes::sanitize(field["text"].as_str().unwrap_or_default());
        let wrapped =
            tmt_tui::text::lines(&text, value_width as u16, tmt_tui::style::TextFlow::Wrap);
        for (index, text) in wrapped.iter().take(3).enumerate() {
            let text = if index == 2 && wrapped.len() > 3 {
                format!(
                    "{}…",
                    super::view::fit(text, value_width.saturating_sub(1)).trim_end()
                )
            } else {
                text.clone()
            };
            line(
                if index == 0 { label } else { "" },
                Line::styled(text, look.role(role)),
            );
        }
    }
    if data["show_reply"] == true {
        if data["reply"].is_null() {
            line(
                "reply",
                Line::styled(
                    super::view::fit("no reply yet", value_width)
                        .trim_end()
                        .to_owned(),
                    muted,
                ),
            );
        } else {
            let reply = &data["reply"];
            line(
                "reply",
                Line::styled(
                    super::view::fit(
                        &format!(
                            "from {} · {}",
                            super::notes::sanitize(reply["from"].as_str().unwrap_or("–")),
                            reply["age"].as_str().unwrap_or("–")
                        ),
                        value_width,
                    )
                    .trim_end()
                    .to_owned(),
                    muted,
                ),
            );
            let lines = super::markdown::render(
                &super::notes::sanitize(reply["body"].as_str().unwrap_or_default()),
                value_width,
                look,
            );
            for text in lines.iter().take(6) {
                line("", text.clone().style(look.role(Role::Text)));
            }
            if lines.len() > 6 {
                line(
                    "",
                    Line::styled(
                        super::view::fit(
                            &format!("… {} more lines · v view", lines.len() - 6),
                            value_width,
                        )
                        .trim_end()
                        .to_owned(),
                        muted,
                    ),
                );
            }
        }
    } else if fields.is_empty() {
        line("", Line::styled("no details yet", muted));
    }
    for line in &mut output {
        line.spans.push(Span::raw(
            " ".repeat(usize::from(width).saturating_sub(line.width())),
        ));
    }
    output
}

/// Paint list-reserved lines using the same renderer, leaving scrolling and hit
/// geometry entirely with the List component.
pub(super) fn paint_list(
    buffer: &mut Buffer,
    map: &tmt_tui::components::ListFrame,
    rows: &[Value],
    look: Look,
    selected: Option<&str>,
    indent: u16,
) {
    for row in &map.geometry {
        let Some(data) = rows.iter().find(|data| data["id"] == row.id) else {
            continue;
        };
        let lines = render(
            &data["expanded_detail"],
            map.viewport.width,
            indent,
            look,
            selected == Some(row.id.as_str()),
        );
        for (index, line) in lines.into_iter().enumerate() {
            let at = row.lines.start + 1 + index;
            if at < map.offset || at >= map.offset + usize::from(map.viewport.height) {
                continue;
            }
            tmt_tui::components::strip::paint_left(
                buffer,
                Rect::new(
                    map.viewport.x,
                    map.viewport.y + (at - map.offset) as u16,
                    map.viewport.width,
                    1,
                ),
                line,
            );
        }
    }
}

pub(super) struct Reader {
    /// A paused conversation opens exactly the response it currently displays.
    pub frozen: Option<Reply>,
    pub selection: Option<super::response::Selection>,
    pub source_area: std::cell::Cell<Option<Rect>>,
    pub target: Target,
    pub scroll: std::cell::RefCell<tmt_tui::components::ScrollState>,
    rendered: std::cell::RefCell<Option<ReaderRender>>,
}

struct ReaderRender {
    source: String,
    width: u16,
    look: Look,
    lines: Vec<Line<'static>>,
}

impl Reader {
    pub fn new(target: Target, frozen: Option<Reply>) -> Self {
        Self {
            target,
            frozen,
            selection: None,
            source_area: Default::default(),
            scroll: Default::default(),
            rendered: Default::default(),
        }
    }

    fn prepare(&self, reply: &Reply, width: u16, look: Look) {
        let mut rendered = self.rendered.borrow_mut();
        if rendered.as_ref().is_some_and(|cached| {
            cached.source == reply.body && cached.width == width && cached.look == look
        }) {
            return;
        }
        *rendered = Some(ReaderRender {
            source: reply.body.clone(),
            width,
            look,
            lines: super::markdown::render(
                &super::notes::sanitize(&reply.body),
                usize::from(width),
                look,
            ),
        });
    }
}

impl App {
    pub(super) fn reader_reply(&self) -> Option<Reply> {
        let reader = self.row_details.reader.as_ref()?;
        reader
            .frozen
            .clone()
            .or_else(|| self.detail(&reader.target)?.reply)
    }
}

pub(super) fn render_reader(frame: &mut ratatui::Frame, app: &App, body: Rect) {
    use tmt_tui::components::{Modal, Placement, strip};
    let Some(reader) = &app.row_details.reader else {
        return;
    };
    let Some(reply) = app.reader_reply() else {
        return;
    };
    let look = app.look();
    let modal = Modal {
        title: format!("reply from {}", reply.from),
        placement: Placement::Body,
    };
    let areas = modal.areas(body, [body.width, body.height], true, false);
    modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
    reader.source_area.set(Some(areas.content));
    let mut scroll = reader.scroll.borrow_mut();
    if let Some(selection) = &reader.selection {
        let layout = selection.source.layout(areas.content.width);
        scroll.update(areas.content, layout.lines.len(), None);
        if let Some(caret) = layout.paint(
            frame.buffer_mut(),
            areas.content,
            scroll.offset(),
            selection.source.selection(),
            look.role(Role::Text),
            look.role(Role::Text)
                .add_modifier(ratatui::style::Modifier::REVERSED),
        ) {
            frame.set_cursor_position(caret);
        }
    } else {
        reader.prepare(&reply, areas.content.width, look);
        let rendered = reader.rendered.borrow();
        let lines = &rendered.as_ref().unwrap().lines;
        scroll.update(areas.content, lines.len(), None);
        for (index, line) in lines
            .iter()
            .skip(scroll.offset())
            .take(usize::from(areas.content.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + index as u16,
                    areas.content.width,
                    1,
                ),
                line.clone().style(look.role(Role::Text)),
            );
        }
    }
    strip::paint_left(
        frame.buffer_mut(),
        areas.position,
        Line::styled(
            reader.selection.as_ref().map_or_else(
                || scroll.position(),
                |selection| format!("source {} · {}", selection.key.request, scroll.position()),
            ),
            look.role(Role::Muted),
        ),
    );
    strip::paint_left(
        frame.buffer_mut(),
        areas.footer,
        super::view::footer::hint_line(
            if reader.selection.is_some() {
                "Shift-arrows / drag select · y copy · a annotate · Esc close"
            } else {
                "↑↓ scroll · PgUp/PgDn page · s select source · Esc close"
            },
            look,
        ),
    );
}

#[cfg(test)]
mod tests;
