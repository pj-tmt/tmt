//! Selected-agent provider finals: bounded public observations, never request results.
#[cfg(test)]
use crate::core::Core;
use serde_json::{Value, json};
use std::sync::Arc;

mod paging;
pub(super) use paging::AnswerRead;

pub(super) const MAX_ITEMS: usize = 50;
const MAX_TEXT_BYTES: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Item {
    pub text: String,
    pub provider: String,
    pub ended_ms: u64,
    pub truncated: bool,
    pub original_bytes: u64,
    pub session: Option<String>,
}

/// Core's coverage observation, never a client retention policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Coverage {
    pub older_dropped: bool,
    pub dropped_through_ms: Option<u64>,
}

/// One coherent loaded cursor chain. Items remain in capture order, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Page {
    pub items: Vec<Item>,
    pub coverage: Coverage,
    pub next_before: Option<u64>,
    pub capture: Option<bool>,
}

impl std::ops::Deref for Page {
    type Target = [Item];
    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl Page {
    pub fn as_slice(&self) -> &[Item] {
        &self.items
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Observation {
    #[default]
    Pending,
    Unavailable,
    /// Oldest capture first, preserving Core's sequence even across clock rollback.
    Observed(Arc<Page>),
}

impl Observation {
    #[cfg(test)]
    pub fn observed(mut items: Vec<Item>) -> Self {
        for item in &mut items {
            if item.original_bytes == 0 {
                item.original_bytes = item.text.len() as u64;
            }
        }
        Self::Observed(Arc::new(Page {
            items,
            ..Page::default()
        }))
    }
}

pub(super) fn canonical_identity(identity: &str) -> bool {
    crate::config::uuid_like(identity) && identity == identity.to_ascii_lowercase()
}

#[cfg(test)]
pub(super) fn read(core: &Core, identity: &str) -> Observation {
    read_with(identity, |operation, input| core.api(operation, input).ok())
}

#[cfg(test)]
fn read_with(identity: &str, call: impl FnMut(&str, Value) -> Option<Value>) -> Observation {
    AnswerRead {
        key: (None, identity.into()),
        before: None,
        held: None,
        rebuild: false,
    }
    .read_with(call)
}

fn page_with(
    identity: &str,
    before: Option<u64>,
    call: &mut impl FnMut(&str, Value) -> Option<Value>,
) -> Option<Page> {
    let mut input = json!({"identityId":identity,"limit":MAX_ITEMS});
    if let Some(cursor) = before {
        input["before"] = json!(cursor);
    }
    let page = call("responses.recent", input).and_then(|value| parse(identity, &value))?;
    if before
        .zip(page.next_before)
        .is_some_and(|(before, next)| next >= before)
        || page.next_before.is_some() && page.is_empty()
    {
        return None;
    }
    Some(page)
}

fn parse(identity: &str, value: &Value) -> Option<Page> {
    if value["identityId"].as_str()? != identity {
        return None;
    }
    let values = value["items"].as_array()?;
    if values.len() > MAX_ITEMS {
        return None;
    }
    let mut items = Vec::with_capacity(values.len());
    for value in values.iter().rev() {
        let text = value["text"].as_str()?;
        // Provider is Core-owned metadata, not an Ops driver dispatch key.
        let provider = value["provider"].as_str()?;
        if text.len() > MAX_TEXT_BYTES {
            return None;
        }
        if let Some(session) = value.get("sessionId") {
            session.as_str()?;
        }
        let original_bytes = value["originalBytes"].as_u64()?;
        if original_bytes < text.len() as u64 {
            return None;
        }
        items.push(Item {
            text: text.into(),
            provider: provider.into(),
            ended_ms: value["endedMs"].as_u64()?,
            truncated: value["truncated"].as_bool()?,
            original_bytes,
            session: value
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }
    let coverage = Coverage {
        older_dropped: value["coverage"]["olderDropped"].as_bool()?,
        dropped_through_ms: match value["coverage"].get("droppedThroughMs") {
            None => None,
            Some(value) => Some(value.as_u64()?),
        },
    };
    let next_before = match value.get("nextBefore") {
        None => None,
        Some(value) => Some(value.as_u64()?),
    };
    let capture = match value.get("capture") {
        None => None,
        Some(value) => Some(value["enabled"].as_bool()?),
    };
    Some(Page {
        items,
        coverage,
        next_before,
        capture,
    })
}

/// Both sources keep their own order. Literal head times decide only which head
/// is emitted next; exchanges precede pane output on ties. Never sort either input.
pub(super) fn merge(exchanges: &[u64], outputs: &[Item]) -> Vec<Source> {
    let (mut exchange, mut output) = (0, 0);
    let mut order = Vec::with_capacity(exchanges.len() + outputs.len());
    while exchange < exchanges.len() || output < outputs.len() {
        if exchange < exchanges.len()
            && (output == outputs.len() || exchanges[exchange] <= outputs[output].ended_ms)
        {
            order.push(Source::Exchange(exchange));
            exchange += 1;
        } else {
            order.push(Source::Pane(output));
            output += 1;
        }
    }
    order
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Exchange(usize),
    Pane(usize),
}

#[cfg(test)]
mod tests;
