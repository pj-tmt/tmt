//! HOME's latest exchanges are a deferred projection of public request reads.
//! Submitted replies use the originator results view; pending asks use one
//! bounded recipient-history page per distinct lead, never a room scan.

use crate::{
    core::{Core, SquadError},
    me::Me,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};

const LIMIT: usize = 50;

/// Retained bodies preserve paragraph breaks; the shared fitter escapes each
/// logical line and wraps it to the band's recorded content width.
pub(super) fn message_lines(text: &str, width: u16) -> Vec<String> {
    text.split('\n')
        .flat_map(|line| tmt_tui::text::lines(line, width, tmt_tui::style::TextFlow::Wrap))
        .collect()
}

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
                .map(super::notes::sanitize)
                .ok_or_else(|| {
                    SquadError::new(
                        "SQUAD_CORE_UNAVAILABLE",
                        "The retained message body is unavailable.",
                    )
                }),
            Some("expired") => Ok("(message expired)".into()),
            Some("not_submitted") => Ok("(reply not submitted)".into()),
            _ => Ok("(message unavailable)".into()),
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
    pub(super) failure: Option<String>,
    pub(super) incomplete: bool,
    /// Measured subprocess cost, including the shared originator results page.
    pub(super) calls: usize,
    pub(super) elapsed_ms: u128,
}

#[derive(Default)]
pub(super) struct State {
    sender: Option<String>,
    pub leads: Vec<Lead>,
    pub failure: Option<String>,
    pub incomplete: bool,
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
            failure: None,
            incomplete: false,
            calls: 0,
            elapsed_ms: 0,
        };
        let Some(sender) = self.sender.filter(|_| !read.leads.is_empty()) else {
            return read;
        };
        read.calls += 1;
        let results = match list(json!({"originatorId":sender.id,"view":"results","limit":LIMIT}))
            .and_then(page)
        {
            Ok((items, incomplete)) => {
                read.incomplete |= incomplete;
                items
            }
            Err(error) => {
                read.failure = Some(error.to_string());
                Vec::new()
            }
        };
        let mut histories = BTreeMap::new();
        for lead in &mut read.leads {
            let history = histories.entry(lead.id().to_owned()).or_insert_with(|| {
                read.calls += 1;
                list(json!({"recipientId":lead.id(),"limit":LIMIT})).and_then(page)
            });
            let items = match history {
                Ok((items, incomplete)) => {
                    read.incomplete |= *incomplete;
                    items.as_slice()
                }
                Err(error) => {
                    lead.failure = Some(error.to_string());
                    &[]
                }
            };
            lead.exchange = latest(lead, &sender.id, &results, items, now);
        }
        sort(&mut read.leads);
        read.elapsed_ms = start.elapsed().as_millis();
        read
    }
}

impl State {
    /// Refreshes the roster while retaining observations only for the same user
    /// and lead occurrence. Replaced leads cannot inherit another lead's message.
    pub fn reconcile(&mut self, home: &super::home::Home, sender: Option<&str>) {
        let same_sender = self.sender.as_deref() == sender;
        let mut leads = roster(home);
        if same_sender {
            for lead in &mut leads {
                if let Some(previous) = self
                    .leads
                    .iter()
                    .find(|old| old.squad == lead.squad && old.id() == lead.id())
                {
                    lead.exchange = previous.exchange.clone();
                    lead.failure = previous.failure.clone();
                }
            }
        } else {
            self.failure = None;
            self.incomplete = false;
        }
        self.sender = sender.map(str::to_owned);
        sort(&mut leads);
        self.leads = leads;
    }

    pub fn replace(&mut self, mut read: Read) {
        if self.sender != read.sender {
            return;
        }
        read.leads.retain(|lead| {
            self.leads
                .iter()
                .any(|current| current.squad == lead.squad && current.id() == lead.id())
        });
        for lead in &mut read.leads {
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
        self.incomplete = read.incomplete;
    }
}

fn page(value: Value) -> Result<(Vec<Value>, bool), SquadError> {
    let items = value["items"].as_array().ok_or_else(|| {
        SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "requests.list returned no items array.",
        )
    })?;
    Ok((items.clone(), !value["nextBefore"].is_null()))
}

fn sort(leads: &mut [Lead]) {
    leads.sort_by(|a, b| {
        b.exchange
            .as_ref()
            .and_then(|e| e.since_ms)
            .cmp(&a.exchange.as_ref().and_then(|e| e.since_ms))
            .then_with(|| a.name().cmp(b.name()))
            .then_with(|| a.squad.cmp(&b.squad))
            .then_with(|| a.id().cmp(b.id()))
    });
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
