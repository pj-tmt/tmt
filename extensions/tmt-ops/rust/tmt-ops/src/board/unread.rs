//! Unread submitted results from one bounded originator page, never Core acks.
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const RESULTS_LIMIT: usize = 50;
const MAX_TIMESTAMP: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Count {
    pub total: usize,
    pub more: bool,
}
impl Count {
    /// An unobserved possibility is not evidence of an unread reply.
    pub fn label(self) -> Option<String> {
        (self.total > 0)
            .then(|| format!("{}{} unread", self.total, if self.more { "+" } else { "" }))
    }

    pub fn add(&mut self, other: Self) {
        self.total += other.total;
        self.more |= other.more;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Page {
    pub actor: String,
    replies: BTreeMap<String, Vec<(String, u64)>>,
    oldest: Option<u64>,
    has_older: bool,
}

impl Page {
    /// Missing or malformed optional observations cannot erase a held page.
    pub fn read(actor: &str, document: &Value) -> Option<Self> {
        let items = document["items"].as_array()?;
        if actor.is_empty() || items.len() > RESULTS_LIMIT {
            return None;
        }
        let has_older = match document.get("nextBefore")? {
            Value::Null => false,
            cursor => {
                timestamp(&cursor["submittedAtMs"])?;
                nonempty(&cursor["requestId"])?;
                if items.is_empty() {
                    return None;
                }
                true
            }
        };
        let mut requests = BTreeMap::new();
        let mut oldest = None;
        for item in items {
            if item["kind"] != "request"
                || item["sender"]["identityId"].as_str()? != actor
                || !matches!(
                    item["final"]["status"].as_str()?,
                    "retained" | "expired" | "unavailable"
                )
            {
                return None;
            }
            let request = nonempty(&item["requestId"])?;
            let member = nonempty(&item["recipientId"])?;
            let at = timestamp(&item["final"]["submittedAtMs"])?;
            oldest = Some(oldest.map_or(at, |previous: u64| previous.min(at)));
            if let Some(previous) = requests.insert(request, (member, at))
                && previous != (member, at)
            {
                return None;
            }
        }
        let mut replies: BTreeMap<String, Vec<(String, u64)>> = BTreeMap::new();
        for (request, (member, at)) in requests {
            replies
                .entry(member.into())
                .or_default()
                .push((request.into(), at));
        }
        Some(Self {
            actor: actor.into(),
            replies,
            oldest,
            has_older,
        })
    }

    pub fn newest(&self, member: &str) -> Option<u64> {
        self.replies.get(member)?.iter().map(|(_, at)| *at).max()
    }

    pub fn submitted(&self, member: &str, request: &str) -> Option<u64> {
        self.replies
            .get(member)?
            .iter()
            .find_map(|(id, at)| (id == request).then_some(*at))
    }

    pub fn count(&self, member: &str, seen_at: u64) -> Count {
        Count {
            total: self.replies.get(member).map_or(0, |times| {
                times.iter().filter(|(_, at)| *at > seen_at).count()
            }),
            // This boundary belongs to each member's marker, not to the actor
            // who happened to submit the last item on the shared page.
            more: self.has_older && self.oldest.is_some_and(|at| seen_at < at),
        }
    }
}

fn timestamp(value: &Value) -> Option<u64> {
    value.as_u64().filter(|at| (1..=MAX_TIMESTAMP).contains(at))
}

fn nonempty(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests;

mod controller;
pub(super) mod publish;
