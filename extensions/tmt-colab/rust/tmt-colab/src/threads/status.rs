//! Immutable status actions. The caller supplies authenticated writer projections;
//! labels and wall clocks grant no authority and never determine conflict order.
use super::{require, scope, timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use tmt_colab_model::{Invalid, Result, values};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub writer: String,
    pub id: String,
}
impl Reference {
    fn validate(&self) -> Result<()> {
        values::generated_id(&self.writer)?;
        values::generated_id(&self.id)?;
        Ok(())
    }
    fn key(&self) -> String {
        format!("{}:{}", self.writer, self.id)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Recipient {
    pub machine: String,
    pub agent: String,
    pub agent_name: String,
    pub operation_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub version: u8,
    pub kind: String,
    pub space_id: String,
    pub page_id: String,
    pub epoch: String,
    pub sender_device: String,
    pub revision: String,
    pub deleted: bool,
    pub device_name: String,
    pub at: String,
    pub action_id: String,
    pub thread: Reference,
    pub previous: Option<Reference>,
    pub resolved: bool,
    pub actor: String,
    pub agent_name: Option<String>,
    pub recipients: Vec<Recipient>,
}
impl Action {
    fn validate(&self) -> Result<()> {
        scope(
            self.version,
            &self.space_id,
            &self.page_id,
            &self.epoch,
            &self.sender_device,
            &self.revision,
            &self.device_name,
        )?;
        timestamp(&self.at)?;
        values::generated_id(&self.action_id)?;
        self.thread.validate()?;
        if let Some(previous) = &self.previous {
            previous.validate()?;
        }
        require(self.kind == "thread-status" && self.revision == "1" && !self.deleted)?;
        require(matches!(self.actor.as_str(), "person" | "agent"))?;
        require(self.agent_name.as_ref().is_none_or(|name| {
            !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
        }))?;
        require(self.actor == "agent" || self.agent_name.is_none())?;
        require(
            self.recipients.len() <= crate::limits::STATUS_RECIPIENTS
                && ((self.actor == "person" && self.resolved) || self.recipients.is_empty()),
        )?;
        let mut agents = BTreeSet::new();
        let mut operations = BTreeSet::new();
        for recipient in &self.recipients {
            values::generated_id(&recipient.machine)?;
            values::core_id(&recipient.agent)?;
            values::generated_id(&recipient.operation_id)?;
            require(
                recipient.agent_name.len() <= 128
                    && agents.insert((&recipient.machine, &recipient.agent))
                    && operations.insert(&recipient.operation_id),
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Notification {
    version: u8,
    kind: String,
    space_id: String,
    page_id: String,
    epoch: String,
    sender_device: String,
    revision: String,
    deleted: bool,
    device_name: String,
    at: String,
    pub(crate) operation_id: String,
    pub(crate) status: Reference,
    pub(crate) reason: String,
}
pub(super) fn validate(root: &str, key: &str, value: &Value) -> Result<()> {
    require(root == "messages")?;
    if value["kind"] == "thread-status" {
        require(value.as_object().is_some_and(|v| {
            v.len() == 17 && v.contains_key("previous") && v.contains_key("agentName")
        }))?;
        let action: Action = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
        action.validate()?;
        require(key == format!("{}:thread-status", action.action_id))
    } else {
        let notification: Notification =
            serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
        scope(
            notification.version,
            &notification.space_id,
            &notification.page_id,
            &notification.epoch,
            &notification.sender_device,
            &notification.revision,
            &notification.device_name,
        )?;
        timestamp(&notification.at)?;
        notification.status.validate()?;
        values::generated_id(&notification.operation_id)?;
        require(
            notification.kind == "thread-notification"
                && notification.revision == "1"
                && !notification.deleted
                && notification.status.writer == notification.sender_device
                && matches!(
                    notification.reason.as_str(),
                    "RECIPIENT_UNAVAILABLE" | "PREPARATION_FAILED"
                )
                && key == format!("{}:thread-notification", notification.operation_id),
        )
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    #[serde(flatten)]
    pub action: Action,
    #[serde(rename = "ref")]
    pub reference: Reference,
    pub depth: usize,
}
/// The same causal fold as browser thread-status.ts, pinned by shared vectors.
/// Invalid parents/cycles are never reached from a root. Only the already verified
/// and scope-bound actions supplied by the caller can become presentation state.
pub fn fold(thread: &Reference, deleted: bool, chat: bool, actions: &[Action]) -> Option<Status> {
    if deleted || chat {
        return None;
    }
    let mut children: BTreeMap<String, Vec<&Action>> = BTreeMap::new();
    let mut queue = VecDeque::new();
    for action in actions.iter().filter(|a| a.thread == *thread) {
        if let Some(previous) = &action.previous {
            children.entry(previous.key()).or_default().push(action);
        } else {
            queue.push_back((action, 1));
        }
    }
    let mut winner: Option<Status> = None;
    while let Some((action, depth)) = queue.pop_front() {
        let reference = Reference {
            writer: action.sender_device.clone(),
            id: action.action_id.clone(),
        };
        if winner.as_ref().is_none_or(|old| {
            depth > old.depth || (depth == old.depth && reference.key() > old.reference.key())
        }) {
            winner = Some(Status {
                action: action.clone(),
                reference: reference.clone(),
                depth,
            });
        }
        for child in children.get(&reference.key()).into_iter().flatten() {
            queue.push_back((*child, depth + 1));
        }
    }
    winner
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_causal_vectors_match_browser_without_clock_or_input_order_authority() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../contracts/vectors/discussion-v1.json"
        ))
        .unwrap();
        for row in fixture["statusCases"].as_array().unwrap() {
            let thread = &row["thread"];
            let reference = Reference {
                writer: thread["senderDevice"].as_str().unwrap().into(),
                id: thread["threadId"].as_str().unwrap().into(),
            };
            let actions: Vec<Action> = serde_json::from_value(row["actions"].clone()).unwrap();
            for mut values in [actions.clone(), actions.iter().cloned().rev().collect()] {
                for action in &mut values {
                    action.at = "8640000000000000".into();
                    action.device_name = "Another label".into();
                }
                let status = fold(
                    &reference,
                    thread["deleted"] == true,
                    reference.id == reference.writer && thread["anchor"].is_null(),
                    &values,
                );
                assert_eq!(
                    status
                        .as_ref()
                        .map_or(thread["resolved"] == true, |s| s.action.resolved),
                    row["expected"]["resolved"].as_bool().unwrap(),
                    "{}",
                    row["name"]
                );
                let result = status.map(|s| serde_json::json!({"writer":s.reference.writer,"id":s.reference.id,"depth":s.depth})).unwrap_or(Value::Null);
                assert_eq!(result, row["expected"]["status"], "{}", row["name"]);
            }
        }
    }
}
