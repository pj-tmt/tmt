//! Inert event captions. Identity, source and read receipts remain with the exchange owner.
use super::*;
use crate::board::{
    conversation_checklist::{Key, activity::Snapshot},
    inventory::Inventory,
};
use std::sync::Arc;

pub(super) struct Row {
    pub id: u64,
    pub at_ms: u64,
    pub text: String,
}

pub(super) struct Presentation {
    key: Key,
    facts: Arc<Snapshot>,
    inventory: Arc<()>,
    until_ms: u64,
    pub rows: Vec<Row>,
    pub captions: Vec<&'static str>,
}

impl PartialEq for Presentation {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}
impl Eq for Presentation {}

impl Presentation {
    pub fn get(app: &App, held: Option<&Arc<Self>>, member: &str) -> Option<Arc<Self>> {
        let key = app
            .conversation_checklist_key()
            .filter(|key| key.member() == Some(member))?;
        let facts = app.notes.activity(&key)?;
        let now = crate::status::now_ms();
        if let Some(held) = held.filter(|held| {
            held.key == key
                && Arc::ptr_eq(&held.facts, facts)
                && Arc::ptr_eq(&held.inventory, &app.references.generation)
                && now < held.until_ms
        }) {
            return Some(Arc::clone(held));
        }
        Some(Arc::new(Self::new(
            key,
            Arc::clone(facts),
            &app.references,
            now,
        )))
    }

    fn new(key: Key, facts: Arc<Snapshot>, inventory: &Inventory, now: u64) -> Self {
        let mut rows = Vec::new();
        let mut until_ms = u64::MAX;
        if let Some(user) = &facts.user {
            for event in facts.events.iter().filter(|event| event.actor_id != *user) {
                let verb = match event.action.as_str() {
                    "add" => "added",
                    "edit" => "edited",
                    "assign" => "assigned",
                    "unassign" => "unassigned",
                    "complete" => "marked done",
                    "reopen" => "reopened",
                    "archive" => "archived",
                    "restore" => "restored",
                    "delete" => "removed",
                    "reorder" => "reordered the checklist",
                    _ => continue,
                };
                let actor = inventory
                    .agents
                    .get(&event.actor_id)
                    .map(|agent| agent.name.as_str())
                    .or_else(|| facts.names.get(&event.actor_id).map(String::as_str));
                let actor = label(actor, "an agent");
                let title = label(
                    (event.action != "delete")
                        .then_some(event)
                        .and_then(|event| {
                            event
                                .item_id
                                .as_ref()
                                .and_then(|id| facts.titles.get(id))
                                .map(String::as_str)
                        }),
                    "an item",
                );
                let action = if event.action == "reorder" {
                    verb.into()
                } else {
                    format!("{verb} {title}")
                };
                let age = crate::requests::age(now, event.at_ms);
                let age = super::exchange::age_stamp(&age);
                rows.push(Row {
                    id: event.id,
                    at_ms: event.at_ms,
                    text: format!("≡ {actor} · checklist · {action} · {age}"),
                });
                let elapsed = now.saturating_sub(event.at_ms);
                let unit = match elapsed {
                    0..60_000 => 1000,
                    60_000..3_600_000 => 60_000,
                    3_600_000..86_400_000 => 3_600_000,
                    _ => 86_400_000,
                };
                until_ms = until_ms.min(event.at_ms.saturating_add((elapsed / unit + 1) * unit));
            }
        }
        let mut captions = Vec::new();
        if let Some(gap) = facts.gap {
            captions.push(match gap {
                crate::checklist::events::Gap::Legacy => "○ earlier checklist activity unknown",
                crate::checklist::events::Gap::Pruned => "○ older checklist activity not kept",
            });
        }
        if facts.failed {
            captions.push("✗ checklist activity unavailable");
        }
        if facts.user.is_none() {
            captions.push("○ checklist activity unavailable · user identity unresolved");
        }
        if facts.more {
            captions.push("◌ more checklist activity pending");
        }
        Self {
            key,
            facts,
            inventory: Arc::clone(&inventory.generation),
            until_ms,
            rows,
            captions,
        }
    }

    pub fn anchor(&self, index: usize) -> String {
        format!(
            "checklist:{}:{}:{}",
            self.key.room,
            self.key.member().unwrap_or_default(),
            self.rows[index].id
        )
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.captions.is_empty()
    }
}

fn label(text: Option<&str>, fallback: &str) -> String {
    text.map(crate::board::notes::single_line)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| fallback.into())
}

/// Each stream keeps its authoritative order; literal head times only choose between streams.
#[derive(Clone, Copy)]
pub(super) enum Source {
    Exchange(usize),
    Pane(usize),
    Checklist(usize),
}

pub(super) fn merge(
    times: &[u64],
    items: &[crate::board::recent_responses::Item],
    activity: Option<&Presentation>,
) -> Vec<Source> {
    let base = crate::board::recent_responses::merge(times, items);
    let rows = activity.map_or(&[][..], |activity| activity.rows.as_slice());
    let (mut index, mut event) = (0, 0);
    let mut order = Vec::with_capacity(base.len() + rows.len());
    while index < base.len() || event < rows.len() {
        let next = base.get(index).map(|source| match source {
            crate::board::recent_responses::Source::Exchange(i) => {
                (times[*i], Source::Exchange(*i))
            }
            crate::board::recent_responses::Source::Pane(i) => {
                (items[*i].ended_ms, Source::Pane(*i))
            }
        });
        if let Some((_, source)) =
            next.filter(|(time, _)| event == rows.len() || *time <= rows[event].at_ms)
        {
            order.push(source);
            index += 1;
        } else {
            order.push(Source::Checklist(event));
            event += 1;
        }
    }
    order
}

#[cfg(test)]
mod tests;
