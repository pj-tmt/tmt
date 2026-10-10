//! Conversation export: the page's authorized threads, comments and Ask conversations as exact
//! JSON plus a plain Markdown reading of the same frozen data. The browser builds the same
//! bytes (`typescript/app/src/conversations.ts`); `vectors/export-v1.json` pins both.
//!
//! Inputs are the decoder-validated per-writer `own` projections and each writer's historical
//! signing key from its cut-admitted envelopes. A body can never select another writer's
//! stream: records join only inside the stream that carried them.
use crate::ask::SignedAsk;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const FORMAT: &str = "tmt-colab-conversations";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Head {
    pub revision: String,
    pub statement_hash: String,
}
#[derive(Serialize)]
pub struct Anchor {
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub writer: String,
    pub id: String,
    pub revision: String,
    pub deleted: bool,
    pub body: String,
    pub device_name: String,
    pub at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub writer: String,
    pub id: String,
    pub revision: String,
    pub anchor: Option<Anchor>,
    pub resolved: bool,
    pub deleted: bool,
    pub device_name: String,
    pub at: String,
    pub comments: Vec<Comment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<crate::threads::Proposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<crate::threads::status::Decision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::threads::status::Status>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notifications: Vec<crate::threads::status::Notification>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub request_id: String,
    pub agent_id: String,
    pub body: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub writer: String,
    pub operation_id: String,
    pub device_name: String,
    pub agent_name: String,
    pub agent: String,
    pub machine: String,
    pub thread: String,
    pub message_ids: Vec<String>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub message: String,
    pub state: String,
    pub reason: Option<String>,
    pub request_id: Option<String>,
    pub reply: Option<Reply>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversations {
    pub format: &'static str,
    pub version: u8,
    pub space_id: String,
    pub page_id: String,
    pub title: String,
    pub epoch: String,
    pub membership_head: Head,
    pub threads: Vec<Thread>,
    pub asks: Vec<Ask>,
}

/// The captured snapshot's identity; everything else comes from the writers' streams.
#[derive(Clone)]
pub struct Capture<'a> {
    pub space_id: &'a str,
    pub page_id: &'a str,
    pub title: &'a str,
    pub epoch: &'a str,
    pub head: Head,
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}
fn owned(value: &Value, key: &str) -> Option<String> {
    text(value, key).map(str::to_owned)
}
fn revision(value: &Value) -> Option<u128> {
    text(value, "revision")?.parse().ok()
}
fn pair(value: &Value) -> Option<(&str, &str)> {
    Some((text(value, "writer")?, text(value, "id")?))
}

/// The newest record of one logical thread or comment, or nothing when its revisions have a
/// gap, a changed thread reference, or a resurrection after a tombstone.
fn latest(mut records: Vec<&Value>) -> Option<&Value> {
    records.sort_by_key(|record| revision(record));
    let first = records.first()?;
    if text(first, "revision")? != "1" || first["deleted"] == true {
        return None;
    }
    let mut previous = *first;
    for record in &records[1..] {
        if revision(record)? != revision(previous)? + 1 || previous["deleted"] == true {
            return None;
        }
        if text(record, "kind") == Some("thread") && record.get("proposal") != first.get("proposal")
        {
            return None;
        }
        if text(record, "kind") == Some("comment")
            && text(previous, "kind") == Some("comment")
            && pair(&record["thread"]) != pair(&previous["thread"])
        {
            return None;
        }
        previous = record;
    }
    Some(previous)
}

pub(crate) fn threads(
    scope: &Capture,
    own: &BTreeMap<String, Value>,
    keys: &BTreeMap<String, [u8; 32]>,
    status_writers: &BTreeSet<String>,
) -> Vec<Thread> {
    let mut threads: BTreeMap<String, Thread> = BTreeMap::new();
    let mut comments = Vec::new();
    let mut actions = Vec::new();
    let mut decisions = Vec::new();
    let mut notifications = Vec::new();
    for (writer, roots) in own {
        if !keys.contains_key(writer) {
            continue;
        }
        let mut groups: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
        for root in ["threads", "messages"] {
            let Some(entries) = roots.get(root).and_then(Value::as_object) else {
                continue;
            };
            for (key, value) in entries {
                let kind = text(value, "kind");
                if !matches!(
                    kind,
                    Some(
                        "thread"
                            | "comment"
                            | "thread-status"
                            | "thread-notification"
                            | "proposal-decision"
                    )
                ) {
                    continue;
                }
                if crate::threads::validate_record(root, key, value).is_err()
                    || text(value, "senderDevice") != Some(writer)
                    || text(value, "spaceId") != Some(scope.space_id)
                    || text(value, "pageId") != Some(scope.page_id)
                    || text(value, "epoch") != Some(scope.epoch)
                {
                    continue;
                }
                if kind == Some("thread-notification") {
                    if let Ok(notification) = serde_json::from_value::<
                        crate::threads::status::Notification,
                    >(value.clone())
                    {
                        notifications.push(notification);
                    }
                    continue;
                }
                if kind == Some("thread-status") {
                    if !status_writers.contains(writer) {
                        continue;
                    }
                    if let Ok(action) =
                        serde_json::from_value::<crate::threads::status::Action>(value.clone())
                    {
                        actions.push(action);
                    }
                    continue;
                }
                if kind == Some("proposal-decision") {
                    if status_writers.contains(writer)
                        && let Ok(action) = serde_json::from_value::<crate::threads::status::Decision>(
                            value.clone(),
                        )
                    {
                        decisions.push(action);
                    }
                    continue;
                }
                let id = text(
                    value,
                    if kind == Some("thread") {
                        "threadId"
                    } else {
                        "messageId"
                    },
                );
                let (Some(kind), Some(id)) = (kind, id) else {
                    continue;
                };
                groups
                    .entry(format!("{kind}:{id}"))
                    .or_default()
                    .push(value);
            }
        }
        for records in groups.into_values() {
            let Some(record) = latest(records) else {
                continue;
            };
            let (Some(device_name), Some(at), Some(rev)) = (
                owned(record, "deviceName"),
                owned(record, "at"),
                owned(record, "revision"),
            ) else {
                continue;
            };
            if text(record, "kind") == Some("thread") {
                let Some(id) = owned(record, "threadId") else {
                    continue;
                };
                let anchor = record.get("anchor").filter(|a| !a.is_null()).and_then(|a| {
                    Some(Anchor {
                        exact: owned(a, "exact")?,
                        prefix: owned(a, "prefix")?,
                        suffix: owned(a, "suffix")?,
                    })
                });
                threads.insert(
                    format!("{writer}:{id}"),
                    Thread {
                        writer: writer.clone(),
                        proposal: record
                            .get("proposal")
                            .and_then(|value| serde_json::from_value(value.clone()).ok()),
                        decision: None,
                        id,
                        revision: rev,
                        anchor,
                        resolved: record["resolved"] == true,
                        deleted: record["deleted"] == true,
                        device_name,
                        at,
                        comments: Vec::new(),
                        status: None,
                        notifications: Vec::new(),
                    },
                );
            } else {
                let (Some(id), Some((tw, ti))) =
                    (owned(record, "messageId"), pair(&record["thread"]))
                else {
                    continue;
                };
                comments.push((
                    format!("{tw}:{ti}"),
                    Comment {
                        writer: writer.clone(),
                        id,
                        revision: rev,
                        deleted: record["deleted"] == true,
                        body: owned(record, "body").unwrap_or_default(),
                        device_name,
                        at,
                    },
                ));
            }
        }
    }
    for (target, comment) in comments {
        if let Some(thread) = threads.get_mut(&target) {
            thread.comments.push(comment);
        }
    }
    let mut out: Vec<Thread> = threads.into_values().collect();
    out.sort_by(|a, b| (&a.writer, &a.id).cmp(&(&b.writer, &b.id)));
    for thread in &mut out {
        if thread.proposal.is_some() && !thread.deleted {
            thread.decision = crate::threads::status::fold_decision(
                &crate::threads::status::Reference {
                    writer: thread.writer.clone(),
                    id: thread.id.clone(),
                },
                &decisions,
            );
        }
        thread.status = crate::threads::status::fold(
            &crate::threads::status::Reference {
                writer: thread.writer.clone(),
                id: thread.id.clone(),
            },
            thread.deleted,
            thread.id == thread.writer && thread.anchor.is_none(),
            &actions,
        );
        if let Some(status) = &thread.status {
            thread.resolved = status.action.resolved;
            thread.notifications =
                notifications
                    .iter()
                    .filter(|notification| {
                        notification.status == status.reference
                            && status.action.recipients.iter().any(|recipient| {
                                recipient.operation_id == notification.operation_id
                            })
                    })
                    .cloned()
                    .collect();
            thread
                .notifications
                .sort_by(|a, b| a.operation_id.cmp(&b.operation_id));
        }
        thread
            .comments
            .sort_by(|a, b| (&a.writer, &a.id).cmp(&(&b.writer, &b.id)));
    }
    out
}

fn can_transition(from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    match from {
        "dispatching" => matches!(
            to,
            "held" | "accepted" | "uncertain" | "failed" | "refused" | "cancelled" | "expired"
        ),
        "held" => matches!(to, "accepted" | "uncertain" | "refused" | "cancelled"),
        "uncertain" => matches!(
            to,
            "held" | "accepted" | "refused" | "cancelled" | "abandoned"
        ),
        _ => false,
    }
}

/// One verified Ask ledger, or nothing when any of its records disagree.
fn ask(
    scope: &Capture,
    writer: &str,
    key: &[u8; 32],
    roots: &Value,
    operation: &str,
    value: &Value,
) -> Option<Ask> {
    if text(value, "kind") != Some("ask") {
        return None;
    }
    let signed: SignedAsk = serde_json::from_value(value.get("signed")?.clone()).ok()?;
    let intent = signed.verify(key).ok()?;
    if intent.operation_id != operation
        || intent.sender_device != writer
        || intent.space != scope.space_id
        || intent.page != scope.page_id
    {
        return None;
    }
    let mut view = Ask {
        writer: writer.to_owned(),
        operation_id: operation.to_owned(),
        device_name: owned(value, "deviceName")?,
        agent_name: owned(value, "agentName")?,
        agent: intent.agent.clone(),
        machine: intent.machine,
        thread: intent.thread,
        message_ids: intent.message_ids,
        issued_at: intent.issued_at,
        expires_at: intent.expires_at,
        message: intent.message,
        state: "uncertain".into(),
        reason: None,
        request_id: None,
        reply: None,
    };
    let mut states: Vec<&Value> = roots
        .get("messages")?
        .as_object()?
        .values()
        .filter(|item| {
            text(item, "kind") == Some("ask-state") && text(item, "operationId") == Some(operation)
        })
        .collect();
    states.sort_by_key(|state| revision(state));
    let mut started = false;
    for state in states {
        let next = text(state, "state")?;
        if started && !can_transition(&view.state, next) {
            return None;
        }
        let request = state.get("requestId").and_then(Value::as_str);
        if view.request_id.is_some() && request != view.request_id.as_deref() {
            return None;
        }
        view.state = next.to_owned();
        view.reason = state
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        view.request_id = request.map(str::to_owned);
        started = true;
    }
    if let Some(reply) = roots
        .get("replies")
        .and_then(|replies| replies.get(operation))
    {
        if view.state != "accepted"
            || text(reply, "operationId") != Some(operation)
            || text(reply, "agentId") != Some(intent.agent.as_str())
            || text(reply, "requestId") != view.request_id.as_deref()
        {
            return None;
        }
        view.reply = Some(Reply {
            request_id: owned(reply, "requestId")?,
            agent_id: owned(reply, "agentId")?,
            body: owned(reply, "body")?,
        });
    }
    Some(view)
}

fn asks(
    scope: &Capture,
    own: &BTreeMap<String, Value>,
    keys: &BTreeMap<String, [u8; 32]>,
) -> Vec<Ask> {
    let mut out = Vec::new();
    for (writer, roots) in own {
        let Some(key) = keys.get(writer) else {
            continue;
        };
        let Some(intents) = roots.get("intents").and_then(Value::as_object) else {
            continue;
        };
        for (operation, value) in intents {
            out.extend(ask(scope, writer, key, roots, operation, value));
        }
    }
    out.sort_by(|a, b| (&a.writer, &a.operation_id).cmp(&(&b.writer, &b.operation_id)));
    out
}

impl Conversations {
    pub fn project(
        scope: Capture,
        own: &BTreeMap<String, Value>,
        keys: &BTreeMap<String, [u8; 32]>,
        status_writers: &BTreeSet<String>,
    ) -> Self {
        Self {
            format: FORMAT,
            version: 1,
            threads: threads(&scope, own, keys, status_writers),
            asks: asks(&scope, own, keys),
            space_id: scope.space_id.to_owned(),
            page_id: scope.page_id.to_owned(),
            title: scope.title.to_owned(),
            epoch: scope.epoch.to_owned(),
            membership_head: scope.head,
        }
    }

    /// Compact JSON in the declared field order.
    pub fn json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("conversations serialize")
    }

    /// A plain-text reading of the same frozen data. Names and times are labels.
    pub fn markdown(&self) -> String {
        let mut lines: Vec<String> = vec![
            "# Conversations".into(),
            String::new(),
            format!("- Page: {}", code_span(&self.title)),
            format!("- Page ID: {}", self.page_id),
            format!("- Space: {}", self.space_id),
            format!("- Epoch: {}", self.epoch),
            format!(
                "- Membership head: revision {}, {}",
                self.membership_head.revision, self.membership_head.statement_hash
            ),
            String::new(),
            "Names and times are labels asserted by each writer. They are not verified identities or clocks.".into(),
            String::new(),
            format!("## Threads ({})", self.threads.len()),
            String::new(),
        ];
        if self.threads.is_empty() {
            lines.extend(["No threads.".into(), String::new()]);
        }
        let mut threads: Vec<&Thread> = self.threads.iter().collect();
        threads.sort_by_key(|t| (millis(&t.at), format!("{}:{}", t.writer, t.id)));
        for (index, thread) in threads.iter().enumerate() {
            let status = if thread.deleted {
                "deleted"
            } else if thread.resolved {
                "resolved"
            } else {
                "open"
            };
            lines.extend([
                format!("### Thread {}", index + 1),
                String::new(),
                format!("- Thread ID: {}:{}", thread.writer, thread.id),
                format!("- Status: {status}"),
                format!(
                    "- Started by: {} at {}",
                    code_span(&thread.device_name),
                    when(millis(&thread.at))
                ),
                String::new(),
            ]);
            if let Some(status) = &thread.status {
                let actor = if status.action.actor == "agent" {
                    status.action.agent_name.as_deref().unwrap_or("Agent")
                } else {
                    &status.action.device_name
                };
                lines.extend([
                    format!(
                        "- {} by: {} at {}",
                        if status.action.resolved {
                            "Resolved"
                        } else {
                            "Reopened"
                        },
                        code_span(actor),
                        when(millis(&status.action.at))
                    ),
                    String::new(),
                ]);
                for notification in &thread.notifications {
                    lines.extend([
                        format!(
                            "- Notification {}: {}",
                            notification.operation_id, notification.reason
                        ),
                        String::new(),
                    ]);
                }
            }
            if let Some(proposal) = &thread.proposal {
                lines.extend([
                    format!("- Proposal ID: {}", proposal.proposal_id),
                    format!("- Proposal title: {}", code_span(&proposal.title)),
                    format!(
                        "- Proposer: {} ({}:{})",
                        code_span(&proposal.proposer.label),
                        proposal.proposer.machine_id,
                        proposal.proposer.agent_id
                    ),
                    format!(
                        "- Decision: {}",
                        thread
                            .decision
                            .as_ref()
                            .map_or("open", |d| d.decision.as_str())
                    ),
                    String::new(),
                    fence(&proposal.body),
                    String::new(),
                ]);
            }
            match &thread.anchor {
                Some(anchor) => lines.extend([
                    "Quoted text:".into(),
                    String::new(),
                    fence(&anchor.exact),
                    String::new(),
                ]),
                None => lines.extend(["Quoted text: none".into(), String::new()]),
            }
            let mut comments: Vec<&Comment> = thread.comments.iter().collect();
            comments.sort_by_key(|c| (millis(&c.at), format!("{}:{}", c.writer, c.id)));
            for (position, comment) in comments.iter().enumerate() {
                lines.extend([
                    format!("#### Comment {}", position + 1),
                    String::new(),
                    format!("- Comment ID: {}:{}", comment.writer, comment.id),
                    format!(
                        "- By: {} at {} (writer {})",
                        code_span(&comment.device_name),
                        when(millis(&comment.at)),
                        comment.writer
                    ),
                    format!("- Revision: {}", comment.revision),
                    String::new(),
                    if comment.deleted {
                        "Deleted.".into()
                    } else {
                        fence(&comment.body)
                    },
                    String::new(),
                ]);
            }
        }
        lines.extend([format!("## Asks ({})", self.asks.len()), String::new()]);
        if self.asks.is_empty() {
            lines.extend(["No asks.".into(), String::new()]);
        }
        let mut asks: Vec<&Ask> = self.asks.iter().collect();
        asks.sort_by_key(|a| (a.issued_at, format!("{}:{}", a.writer, a.operation_id)));
        for (index, ask) in asks.iter().enumerate() {
            let reason = ask
                .reason
                .as_deref()
                .filter(|r| !r.is_empty())
                .map_or(String::new(), |r| format!(" ({r})"));
            lines.extend([
                format!("### Ask {}", index + 1),
                String::new(),
                format!("- Operation ID: {}", ask.operation_id),
                format!(
                    "- Asked by: {} at {} (writer {})",
                    code_span(&ask.device_name),
                    when(ask.issued_at),
                    ask.writer
                ),
                format!(
                    "- Agent: {} ({}) on machine {}",
                    code_span(&ask.agent_name),
                    ask.agent,
                    ask.machine
                ),
                format!("- Thread: {}", ask.thread),
                format!("- State: {}{reason}", ask.state),
                format!(
                    "- Request ID: {}",
                    ask.request_id.as_deref().unwrap_or("none")
                ),
                String::new(),
                "Message sent:".into(),
                String::new(),
                fence(&ask.message),
                String::new(),
            ]);
            match &ask.reply {
                Some(reply) => lines.extend([
                    "Reply:".into(),
                    String::new(),
                    fence(&reply.body),
                    String::new(),
                ]),
                None => lines.extend(["No reply recorded.".into(), String::new()]),
            }
        }
        lines.join("\n")
    }
}

fn millis(at: &str) -> u64 {
    at.parse().unwrap_or(0)
}

// Display escapes: controls, bidi/format marks and line separators become \u{hex}.
fn escapable(point: u32, keep_layout: bool) -> bool {
    if keep_layout && (point == 0x0a || point == 0x09) {
        return false;
    }
    point < 0x20
        || (0x7f..=0x9f).contains(&point)
        || matches!(point, 0x2028 | 0x2029 | 0x200e | 0x200f | 0xfeff)
        || (0x202a..=0x202e).contains(&point)
        || (0x2066..=0x2069).contains(&point)
}
fn display(value: &str, keep_layout: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if escapable(ch as u32, keep_layout) {
            out.push_str(&format!("\\u{{{:x}}}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    out
}
fn longest_run(value: &str) -> usize {
    let (mut longest, mut run) = (0, 0);
    for ch in value.chars() {
        run = if ch == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}
/// An inline code span that no label can close or extend.
fn code_span(value: &str) -> String {
    let shown = display(value, false);
    if shown.is_empty() {
        return "(none)".into();
    }
    let ticks = "`".repeat(longest_run(&shown) + 1);
    let edge = |c: char| c == '`' || c == ' ';
    let pad = if shown.starts_with(edge) || shown.ends_with(edge) {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{shown}{pad}{ticks}")
}
/// A fenced block longer than any backtick run inside, so a body cannot end it.
fn fence(value: &str) -> String {
    let shown = display(value, true);
    let ticks = "`".repeat(3.max(longest_run(&shown) + 1));
    format!("{ticks}\n{shown}\n{ticks}")
}

const YEAR_10000_MS: u64 = 253_402_300_800_000;
fn when(ms: u64) -> String {
    if ms >= YEAR_10000_MS {
        return format!("unix ms {ms}");
    }
    let days = (ms / 86_400_000) as i64;
    let seconds = (ms / 1000) % 86_400;
    // Civil date from days since 1970-01-01 (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}
