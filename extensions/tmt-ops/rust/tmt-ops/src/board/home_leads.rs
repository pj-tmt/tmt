//! Latest exchanges are a deferred projection shared by conversation placements.
//! Submitted replies use the originator results view; HOME pending asks use one
//! bounded recipient-history page per distinct lead, never a room scan.

use crate::{
    core::{Core, SquadError},
    me::Me,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};

const LIMIT: usize = super::unread::RESULTS_LIMIT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Question,
    Asked,
    Reply,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LeadPreview {
    pub kind: Kind,
    pub request: Option<String>,
    pub since_ms: Option<u64>,
    /// Already safe display text, distinct from the exact retained body.
    pub preview: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub(super) struct Lead {
    pub squad: String,
    pub row: Value,
    pub exchange: Option<LeadPreview>,
    pub failure: Option<String>,
}

impl Lead {
    pub fn id(&self) -> &str {
        self.row["id"].as_str().unwrap_or_default()
    }
    pub fn name(&self) -> &str {
        self.row["name"].as_str().unwrap_or_default()
    }
}

/// One agent's own settled finals to one user, read from storage when the
/// bounded windows above may have scrolled them past.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalsKey {
    pub(super) sender: String,
    pub(super) agent: String,
}

impl FinalsKey {
    /// The newest settled finals to this user by this agent as the same
    /// normalized rows the windows produce. An older core rejects the
    /// recipient form (`API_INPUT_INVALID`); the caller treats any failure as
    /// "no more finals than the windows already show".
    pub(super) fn read(&self, core: &Core) -> Result<Vec<Value>, SquadError> {
        let document = core.api(
            "requests.list",
            json!({"originatorId":self.sender,"recipientId":self.agent,"view":"results","limit":super::row_detail::FINALS}),
        )?;
        let mut replies = Vec::new();
        collect_replies(&mut replies, &page(document)?, &self.sender);
        replies.retain(|reply| reply["recipientId"] == self.agent);
        Ok(replies)
    }
}

/// How many of the user's own sends to one agent are restored, newest first.
pub(super) const SENT: usize = 20;
/// Full prompts fetched per read, newest first; the rest keep their preview
/// until a later read reaches them.
const SENT_SHOWS: usize = 5;
/// The recipient-history page that holds those sends among other senders'.
/// `requests.list` refuses a larger limit, so an agent with more than this many
/// recent requests from anyone restores only the own sends inside that page.
const SENT_PAGE: usize = 50;
const SENT_CACHE_ENTRIES: usize = 256;
const SENT_CACHE_BYTES: usize = 2 * 1024 * 1024;

/// Prompt texts already read, by request ID: `Some` is the full text and
/// `None` a prompt that is gone. A prepared prompt never changes, so neither
/// is read twice.
#[derive(Default)]
pub(super) struct SentTexts {
    known: BTreeMap<String, Option<String>>,
    bytes: usize,
}

impl SentTexts {
    fn remember(&mut self, request: &str, text: Option<String>) {
        if self.known.len() >= SENT_CACHE_ENTRIES || self.bytes >= SENT_CACHE_BYTES {
            *self = Self::default();
        }
        self.bytes += text.as_ref().map_or(0, String::len);
        self.known.insert(request.to_owned(), text);
    }
}

/// One agent's durable requests from this user, read from storage so they
/// survive a restart and are not bounded by the session's own sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentKey {
    pub(super) sender: String,
    pub(super) agent: String,
}

impl SentKey {
    /// The newest requests this user sent to the agent as `{requestId,
    /// preparedAtMs, text, truncated}`, oldest first. One `requests.list`, then
    /// `requests.show` for at most `SENT_SHOWS` full prompts the cache does not
    /// hold yet, newest first. A prompt not read (yet) or no longer retained
    /// keeps the listed preview, marked truncated. A failed list is an error; a
    /// failed show only keeps the preview.
    pub(super) fn read(
        &self,
        core: &Core,
        texts: &mut SentTexts,
    ) -> Result<Vec<Value>, SquadError> {
        let document = core.api(
            "requests.list",
            json!({"recipientId":self.agent,"limit":SENT_PAGE}),
        )?;
        let mut own: Vec<Value> = page(document)?
            .into_iter()
            .filter(|item| {
                item["kind"] == "request"
                    && item["sender"]["identityId"].as_str() == Some(self.sender.as_str())
                    && item["recipientId"].as_str() == Some(self.agent.as_str())
                    && item["preparedAtMs"].is_u64()
            })
            .collect();
        own.sort_by_key(|item| item["preparedAtMs"].as_u64());
        let skip = own.len().saturating_sub(SENT);
        let kept: Vec<Value> = own.into_iter().skip(skip).collect();
        let mut budget = SENT_SHOWS;
        for item in kept.iter().rev() {
            let request = item["requestId"].as_str().unwrap_or_default();
            if budget == 0 {
                break;
            }
            if texts.known.contains_key(request) {
                continue;
            }
            budget -= 1;
            if let Ok(detail) = crate::requests::show_request(core, request) {
                let full = (detail["prompt"]["status"] == "retained")
                    .then(|| detail["prompt"]["message"].as_str().map(str::to_owned))
                    .flatten();
                texts.remember(request, full);
            }
        }
        Ok(kept
            .into_iter()
            .map(|item| {
                let request = item["requestId"].as_str().unwrap_or_default();
                let full = texts.known.get(request).cloned().flatten();
                json!({
                    "requestId": request,
                    "preparedAtMs": item["preparedAtMs"],
                    "truncated": full.is_none(),
                    "text": full.unwrap_or_else(|| item["preview"].as_str().unwrap_or_default().to_owned()),
                })
            })
            .collect())
    }
}

/// A full-body read is anchored to the exact user, lead and latest exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageKey {
    pub(super) sender: String,
    pub(super) lead: String,
    pub(super) squad: String,
    pub(super) request: String,
    pub(super) kind: Kind,
}

impl MessageKey {
    /// A squad is presentation/dispatch context, not response provenance.
    pub(super) fn same_source(&self, other: &Self) -> bool {
        self.sender == other.sender
            && self.lead == other.lead
            && self.request == other.request
            && self.kind == other.kind
    }

    pub(super) fn read(&self, core: &Core) -> Result<String, SquadError> {
        self.body(crate::requests::show_request(core, &self.request)?)
    }

    fn body(&self, detail: Value) -> Result<String, SquadError> {
        let (from, to) = match self.kind {
            Kind::Question => (&self.lead, &self.sender),
            Kind::Asked | Kind::Reply => (&self.sender, &self.lead),
        };
        if detail["requestId"].as_str() != Some(&self.request)
            || detail["sender"]["identityId"].as_str() != Some(from)
            || detail["recipientId"].as_str() != Some(to)
            || detail["kind"] != "request"
        {
            return Err(SquadError::new(
                "SQUAD_CORE_UNAVAILABLE",
                "The request no longer matches this lead exchange.",
            ));
        }
        let part = if self.kind == Kind::Reply {
            &detail["final"]
        } else {
            &detail["prompt"]
        };
        let field = if self.kind == Kind::Reply {
            "response"
        } else {
            "message"
        };
        match part["status"].as_str() {
            Some("retained") => part[field]
                .as_str()
                // Acquisition keeps exact bytes for source-range annotations.
                // Every painter escapes independently; a display string cannot
                // stand in for the response's source offsets.
                .map(str::to_owned)
                .ok_or_else(|| {
                    SquadError::new(
                        "SQUAD_CORE_UNAVAILABLE",
                        "The retained message body is unavailable.",
                    )
                }),
            status => Err(SquadError::new(
                "SQUAD_RESPONSE_UNAVAILABLE",
                match status {
                    Some("expired") => "The message expired.",
                    Some("not_submitted") => "The reply has not been submitted.",
                    _ => "The message is unavailable.",
                },
            )),
        }
    }
}

pub(super) struct Fetch {
    sender: Option<Me>,
    leads: Vec<Lead>,
}

pub struct Read {
    sender: Option<String>,
    pub(super) leads: Vec<Lead>,
    pub(super) replies: Vec<Value>,
    unread: Option<super::unread::Page>,
    pub(super) failure: Option<String>,
    /// Measured subprocess cost, including the shared originator results page.
    pub(super) calls: usize,
    pub(super) elapsed_ms: u128,
}

#[derive(Default)]
pub(super) struct State {
    sender: Option<String>,
    results_observed: bool,
    /// Current roster occurrences, including those still awaiting their first read.
    lead_observed: BTreeMap<(String, String), bool>,
    pub leads: Vec<Lead>,
    /// Bounded result metadata from the same originator read, for HOME members.
    pub replies: Vec<Value>,
    unread: Option<super::unread::Page>,
    pub failure: Option<String>,
}

fn roster(home: &super::home::Home) -> Vec<Lead> {
    home.squads
        .iter()
        .filter_map(|squad| {
            let row = squad.lead.as_ref()?;
            row["id"].as_str()?;
            row["name"].as_str()?;
            Some(Lead {
                squad: squad.squad.clone(),
                row: row.clone(),
                exchange: None,
                failure: None,
            })
        })
        .collect()
}

impl Fetch {
    /// The same originator result page outside HOME, without lead-history reads.
    pub fn results(sender: Option<Me>) -> Self {
        Self {
            sender,
            leads: Vec::new(),
        }
    }

    pub fn new(home: &super::home::Home, sender: Option<Me>) -> Self {
        Self {
            sender,
            leads: roster(home),
        }
    }

    pub fn complete(self, core: &Core) -> Read {
        self.read(
            |input| core.api("requests.list", input),
            crate::status::now_ms(),
        )
    }

    fn read(self, mut list: impl FnMut(Value) -> Result<Value, SquadError>, now: u64) -> Read {
        let start = Instant::now();
        let mut read = Read {
            sender: self.sender.as_ref().map(|me| me.id.clone()),
            leads: self.leads,
            replies: Vec::new(),
            unread: None,
            failure: None,
            calls: 0,
            elapsed_ms: 0,
        };
        let Some(sender) = self.sender else {
            return read;
        };
        read.calls += 1;
        let results = match list(json!({"originatorId":sender.id,"view":"results","limit":LIMIT}))
            .and_then(|document| {
                read.unread = super::unread::Page::read(&sender.id, &document);
                page(document)
            }) {
            Ok(items) => items,
            Err(error) => {
                read.failure = Some(error.to_string());
                Vec::new()
            }
        };
        collect_replies(&mut read.replies, &results, &sender.id);
        let mut histories = BTreeMap::new();
        for lead in &mut read.leads {
            let history = histories.entry(lead.id().to_owned()).or_insert_with(|| {
                read.calls += 1;
                list(json!({"recipientId":lead.id(),"limit":LIMIT})).and_then(page)
            });
            let items = match history {
                Ok(items) => items.as_slice(),
                Err(error) => {
                    lead.failure = Some(error.to_string());
                    &[]
                }
            };
            collect_replies(&mut read.replies, items, &sender.id);
            lead.exchange = latest(lead, &sender.id, &results, items, now);
        }
        read.replies
            .sort_by_key(|reply| std::cmp::Reverse(reply["submittedAtMs"].as_u64()));
        sort(&mut read.leads);
        read.elapsed_ms = start.elapsed().as_millis();
        read
    }
}

impl State {
    /// Only the originator page for this exact actor supplies unread evidence.
    /// Recipient history and pane output never extend its bounded count.
    pub(super) fn unread(&self, actor: &str) -> Option<&super::unread::Page> {
        (self.sender.as_deref() == Some(actor))
            .then_some(self.unread.as_ref())
            .flatten()
            .filter(|page| page.actor == actor)
    }

    pub(super) fn results_observed(&self, sender: Option<&str>) -> bool {
        sender.is_some()
            && self.sender.as_deref() == sender
            && self.results_observed
            && self.failure.is_none()
    }

    pub fn set_sender(&mut self, sender: Option<&str>) {
        if self.sender.as_deref() != sender {
            self.sender = sender.map(str::to_owned);
            self.results_observed = false;
            self.unread = None;
            self.lead_observed.clear();
            self.replies.clear();
            self.leads.clear();
            self.failure = None;
        }
    }

    /// Refreshes the roster while retaining observations only for the same user
    /// and lead occurrence. Replaced leads cannot inherit another lead's message.
    pub fn reconcile(&mut self, home: &super::home::Home, sender: Option<&str>) {
        let same_sender = self.sender.as_deref() == sender;
        let mut leads = roster(home);
        let mut observed = BTreeMap::new();
        for lead in &leads {
            let key = (lead.squad.clone(), lead.id().to_owned());
            observed.insert(key, false);
        }
        if same_sender {
            for lead in &mut leads {
                if let Some(previous) = self
                    .leads
                    .iter()
                    .find(|old| old.squad == lead.squad && old.id() == lead.id())
                {
                    lead.exchange = previous.exchange.clone();
                    lead.failure = previous.failure.clone();
                    let key = (lead.squad.clone(), lead.id().to_owned());
                    observed.insert(key.clone(), self.lead_observed.get(&key) == Some(&true));
                }
            }
        } else {
            self.results_observed = false;
            self.unread = None;
            self.replies.clear();
            self.failure = None;
        }
        self.sender = sender.map(str::to_owned);
        self.lead_observed = observed;
        sort(&mut leads);
        self.leads = leads;
    }

    pub fn replace(&mut self, mut read: Read) {
        if self.sender != read.sender {
            return;
        }
        let admitted = self.sender.is_some() && read.calls > 0;
        if admitted && read.failure.is_none() {
            self.results_observed = true;
        }
        if let Some(unread) = read.unread.take() {
            self.unread = Some(unread);
        }
        if read.failure.is_none() {
            self.replies = std::mem::take(&mut read.replies);
        }
        read.leads.retain(|lead| {
            self.leads
                .iter()
                .any(|current| current.squad == lead.squad && current.id() == lead.id())
        });
        for lead in &mut read.leads {
            if admitted && lead.failure.is_none() {
                self.lead_observed
                    .insert((lead.squad.clone(), lead.id().to_owned()), true);
            }
            if (read.failure.is_some() || lead.failure.is_some())
                && let Some(old) = self
                    .leads
                    .iter()
                    .find(|old| old.squad == lead.squad && old.id() == lead.id())
            {
                // Missing pages cannot erase a newer submitted reply. Questions
                // are transient roster evidence and may have been answered.
                let missing = lead.exchange.is_none();
                let newer_reply = old.exchange.as_ref().is_some_and(|prior| {
                    prior.kind == Kind::Reply
                        && prior.since_ms > lead.exchange.as_ref().and_then(|next| next.since_ms)
                });
                if missing || newer_reply {
                    lead.exchange = old.exchange.clone();
                }
            }
        }
        sort(&mut read.leads);
        self.leads = read.leads;
        self.failure = read.failure;
    }
}

/// Bounded pages already acquired for HOME supply reply identities, not bodies.
fn collect_replies(replies: &mut Vec<Value>, items: &[Value], sender: &str) {
    for item in items {
        if item["kind"] != "request"
            || item["sender"]["identityId"] != sender
            || !matches!(
                item["final"]["status"].as_str(),
                Some("retained" | "expired" | "unavailable")
            )
            || replies
                .iter()
                .any(|reply| reply["requestId"] == item["requestId"])
        {
            continue;
        }
        replies.push(json!({"requestId":item["requestId"],"recipientId":item["recipientId"],"submittedAtMs":item["final"]["submittedAtMs"],"status":item["final"]["status"]}));
    }
}

fn page(value: Value) -> Result<Vec<Value>, SquadError> {
    let items = value["items"].as_array().ok_or_else(|| {
        SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "requests.list returned no items array.",
        )
    })?;
    Ok(items.clone())
}

fn sort(leads: &mut [Lead]) {
    leads.sort_by(compare);
}

pub(super) fn compare(a: &Lead, b: &Lead) -> std::cmp::Ordering {
    match (&a.exchange, &b.exchange) {
        (Some(a_exchange), Some(b_exchange)) => {
            // Question is also the HOME heading's ◆ source. The row attention
            // predicate may still be true when the selected exchange is a reply.
            match (a_exchange.kind, b_exchange.kind) {
                (Kind::Question, Kind::Question) => {
                    // Longest waits first; missing times cannot establish age.
                    (a_exchange.since_ms.is_none(), a_exchange.since_ms)
                        .cmp(&(b_exchange.since_ms.is_none(), b_exchange.since_ms))
                }
                (Kind::Question, _) => std::cmp::Ordering::Less,
                (_, Kind::Question) => std::cmp::Ordering::Greater,
                _ => b_exchange.since_ms.cmp(&a_exchange.since_ms),
            }
            .then_with(|| a.id().cmp(b.id()))
            .then_with(|| a.squad.cmp(&b.squad))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a
            .name()
            .cmp(b.name())
            .then_with(|| a.squad.cmp(&b.squad))
            .then_with(|| a.id().cmp(b.id())),
    }
}

/// Named boards reuse their acquired bounded room history and the HOME exchange
/// projection. This performs no read and does not alter public row documents.
pub(super) fn members(
    document: &Value,
    sent: Option<&crate::requests::Sent>,
    now: u64,
) -> Vec<Lead> {
    let mut rows = Vec::new();
    let mut document = document.clone();
    crate::display_rows::each_row(&mut document, |row| {
        if row["id"].is_string() && !rows.iter().any(|lead: &Lead| lead.row["id"] == row["id"]) {
            rows.push(Lead {
                squad: String::new(),
                row: row.clone(),
                exchange: None,
                failure: None,
            });
        }
    });
    let name = document["squad"]["name"].as_str().unwrap_or_default();
    for member in &mut rows {
        member.squad = name.into();
        if let Some(sent) = sent {
            let (sender, history) = sent.history();
            member.exchange = latest(member, sender, &[], history, now);
        }
        // The compact squad row marks the actual decision, independently of
        // a newer reply. Feed that same question into the shared ordering.
        if crate::attention::waits_on_you(&member.row) {
            let question = member.row["waitingOnYou"]
                .as_array()
                .into_iter()
                .flatten()
                .min_by_key(|item| item["preparedAtMs"].as_u64().unwrap_or(u64::MAX));
            member.exchange = Some(LeadPreview {
                kind: Kind::Question,
                request: question
                    .and_then(|item| item["requestId"].as_str())
                    .map(str::to_owned),
                since_ms: question
                    .and_then(|item| item["preparedAtMs"].as_u64())
                    .filter(|at| *at > 0 && *at <= now),
                preview: safe_preview(
                    member.row["pending"]
                        .as_str()
                        .or_else(|| question.and_then(|item| item["preview"].as_str()))
                        .unwrap_or_default(),
                ),
                status: "retained".into(),
            });
        }
    }
    rows
}

fn safe_preview(text: &str) -> String {
    super::notes::sanitize(text)
        .split(['\n', '\r', '\u{2028}', '\u{2029}'])
        .next()
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect()
}

fn latest(
    lead: &Lead,
    sender: &str,
    results: &[Value],
    history: &[Value],
    now: u64,
) -> Option<LeadPreview> {
    let mut exchanges = Vec::new();
    for item in lead.row["waitingOnYou"].as_array().into_iter().flatten() {
        exchanges.push((
            LeadPreview {
                kind: Kind::Question,
                request: item["requestId"].as_str().map(str::to_owned),
                since_ms: item["preparedAtMs"].as_u64().filter(|at| *at <= now),
                preview: safe_preview(item["preview"].as_str().unwrap_or_default()),
                status: "retained".into(),
            },
            3,
        ));
    }
    if exchanges.is_empty()
        && let Some(pending) = lead.row["pending"].as_str().filter(|text| !text.is_empty())
    {
        exchanges.push((
            LeadPreview {
                kind: Kind::Question,
                request: None,
                since_ms: None,
                preview: safe_preview(pending),
                status: "retained".into(),
            },
            3,
        ));
    }
    for (item, source) in results
        .iter()
        .map(|item| (item, 2))
        .chain(history.iter().map(|item| (item, 1)))
    {
        if item["kind"] != "request"
            || item["recipientId"].as_str() != Some(lead.id())
            || item["sender"]["identityId"].as_str() != Some(sender)
        {
            continue;
        }
        let status = item["final"]["status"].as_str().unwrap_or("unavailable");
        let (kind, time, preview) = if status == "not_submitted" {
            (
                Kind::Asked,
                &item["preparedAtMs"],
                safe_preview(item["preview"].as_str().unwrap_or_default()),
            )
        } else if let Some(at) = item["final"]["submittedAtMs"].as_u64() {
            let preview = item["responsePreview"]
                .as_str()
                .map(safe_preview)
                .unwrap_or_else(|| match status {
                    "expired" => "(reply expired)".into(),
                    _ => "(reply preview unavailable)".into(),
                });
            exchanges.push((
                LeadPreview {
                    kind: Kind::Reply,
                    request: item["requestId"].as_str().map(str::to_owned),
                    since_ms: (at <= now).then_some(at),
                    preview,
                    status: status.into(),
                },
                source,
            ));
            continue;
        } else {
            continue;
        };
        exchanges.push((
            LeadPreview {
                kind,
                request: item["requestId"].as_str().map(str::to_owned),
                since_ms: time.as_u64().filter(|at| *at <= now),
                preview,
                status: status.into(),
            },
            source,
        ));
    }
    // Equal request/time duplicates prefer the results preview over history's
    // metadata-only final. Core never classifies a reply body as success/failure.
    exchanges
        .into_iter()
        .max_by(|(a, a_source), (b, b_source)| {
            a.since_ms
                .cmp(&b.since_ms)
                .then_with(|| a.request.cmp(&b.request))
                .then_with(|| a_source.cmp(b_source))
        })
        .map(|(exchange, _)| exchange)
}

#[cfg(test)]
mod tests;
