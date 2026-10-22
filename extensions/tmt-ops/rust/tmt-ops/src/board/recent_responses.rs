//! Selected-agent provider finals: bounded public observations, never request results.
use crate::core::Core;
use serde_json::{Value, json};

pub(super) const MAX_ITEMS: usize = 10;
const MAX_TEXT_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub text: String,
    pub provider: String,
    pub ended_ms: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Observation {
    #[default]
    Pending,
    Unavailable,
    /// Oldest capture first, preserving Core's sequence even across clock rollback.
    Observed(Vec<Item>),
}

pub(super) fn canonical_identity(identity: &str) -> bool {
    crate::config::uuid_like(identity) && identity == identity.to_ascii_lowercase()
}

pub(super) fn read(core: &Core, identity: &str) -> Observation {
    read_with(identity, |operation, input| core.api(operation, input).ok())
}

fn read_with(identity: &str, mut call: impl FnMut(&str, Value) -> Option<Value>) -> Observation {
    if !canonical_identity(identity) {
        return Observation::Unavailable;
    }
    let Some(capabilities) = call("capabilities", json!({})) else {
        return Observation::Unavailable;
    };
    if !capabilities["operations"]
        .as_array()
        .is_some_and(|operations| {
            operations
                .iter()
                .any(|operation| operation == "responses.recent")
        })
    {
        return Observation::Unavailable;
    }
    call(
        "responses.recent",
        json!({"identityId":identity,"limit":MAX_ITEMS}),
    )
    .and_then(|value| parse(identity, &value))
    .map_or(Observation::Unavailable, Observation::Observed)
}

fn parse(identity: &str, value: &Value) -> Option<Vec<Item>> {
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
        items.push(Item {
            text: text.into(),
            provider: provider.into(),
            ended_ms: value["endedMs"].as_u64()?,
            truncated: value["truncated"].as_bool()?,
        });
    }
    Some(items)
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
