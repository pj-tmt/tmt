//! One actor-fenced page of Core answers and their acknowledgment revisions.
//! Immutable finals and captured turns share the same unread projection.
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const ANSWERS_LIMIT: usize = 50;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AnswerKind {
    Final,
    Turn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Answer {
    pub request: String,
    pub member: String,
    pub answered_at_ms: u64,
    pub kind: Option<AnswerKind>,
    pub revision: u64,
    pub acknowledged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Page {
    pub actor: String,
    replies: BTreeMap<String, Vec<Answer>>,
    pub next_before: Option<Value>,
}

impl Page {
    /// Missing or malformed observations cannot erase a held page.
    pub fn read(actor: &str, document: &Value) -> Option<Self> {
        let items = document["items"].as_array()?;
        if actor.is_empty() || items.len() > ANSWERS_LIMIT {
            return None;
        }
        let next_before = match document.get("nextBefore")? {
            Value::Null => None,
            cursor => {
                timestamp(&cursor["answeredAtMs"])?;
                nonempty(&cursor["requestId"])?;
                if items.is_empty() {
                    return None;
                }
                Some(cursor.clone())
            }
        };
        let mut requests = BTreeMap::new();
        for item in items {
            if !matches!(item["kind"].as_str()?, "request" | "announcement")
                || item["sender"]["identityId"].as_str()? != actor
            {
                return None;
            }
            let answer = Answer {
                request: nonempty(&item["requestId"])?.into(),
                member: nonempty(&item["recipientId"])?.into(),
                answered_at_ms: timestamp(&item["answeredAtMs"])?,
                kind: Some(match item["answer"].as_str()? {
                    "final" => AnswerKind::Final,
                    "turn" => AnswerKind::Turn,
                    _ => return None,
                }),
                revision: timestamp(&item["revision"])?,
                acknowledged: item["acknowledged"].as_bool()?,
            };
            if let Some(previous) = requests.insert(answer.request.clone(), answer.clone())
                && previous != answer
            {
                return None;
            }
        }
        let mut replies: BTreeMap<String, Vec<Answer>> = BTreeMap::new();
        for answer in requests.into_values() {
            replies
                .entry(answer.member.clone())
                .or_default()
                .push(answer);
        }
        Some(Self {
            actor: actor.into(),
            replies,
            next_before,
        })
    }

    pub fn answers(&self, member: &str) -> impl Iterator<Item = &Answer> {
        self.replies.get(member).into_iter().flatten()
    }

    pub fn answer(&self, member: &str, request: &str) -> Option<&Answer> {
        self.answers(member)
            .find(|answer| answer.request == request)
    }

    /// An ack may complete after a newer answer/page; only the exact revision settles.
    pub fn acknowledged(&mut self, completed: &publish::Completed) {
        if self.actor != completed.actor {
            return;
        }
        for (sent, outcome) in &completed.items {
            let Some(answers) = self.replies.get_mut(&sent.member) else {
                continue;
            };
            if matches!(outcome, publish::Outcome::NotFound) {
                answers.retain(|answer| {
                    answer.request != sent.request || answer.revision != sent.revision
                });
                continue;
            }
            let Some(answer) = answers
                .iter_mut()
                .find(|answer| answer.request == sent.request && answer.revision == sent.revision)
            else {
                continue;
            };
            match outcome {
                publish::Outcome::Acknowledged => answer.acknowledged = true,
                publish::Outcome::Conflict(current) => {
                    answer.revision = *current;
                    answer.acknowledged = false;
                    // No source observation for this newer revision has been acquired yet.
                    answer.kind = None;
                }
                publish::Outcome::NotFound => unreachable!(),
            }
        }
    }

    /// A pre-ack read arriving late cannot turn the same revision unread again.
    pub fn retain_acknowledged(&mut self, held: &Self) {
        if self.actor != held.actor {
            return;
        }
        for answers in self.replies.values_mut() {
            for answer in answers {
                if let Some(old) = held.answer(&answer.member, &answer.request) {
                    if old.revision > answer.revision {
                        *answer = old.clone();
                    } else if old.revision == answer.revision && old.acknowledged {
                        answer.acknowledged = true;
                    }
                }
            }
        }
    }

    pub fn count(&self, member: &str) -> Count {
        Count {
            total: self
                .answers(member)
                .filter(|answer| !answer.acknowledged)
                .count(),
            more: self.next_before.is_some(),
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
mod test_support;
#[cfg(test)]
mod tests;

mod controller;
pub(super) mod publish;
pub(super) mod walk;
