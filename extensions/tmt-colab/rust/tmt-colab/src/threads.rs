//! Inert discussion record admission. Envelopes authenticate writers; no DOM,
//! publication, signing or dispatch authority lives in this codec.
use crate::limits::{
    COMMENT_BODY_BYTES, COMMENT_CONTEXT_BYTES, COMMENT_CONTEXT_POINTS, MAX_SEQUENCE,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tmt_colab_model::{Invalid, Result, attachment, bounded::List, values};
pub mod status;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proposer {
    pub machine_id: String,
    pub agent_id: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proposal {
    pub proposal_id: String,
    pub title: String,
    pub body: String,
    pub proposer: Proposer,
}
impl Proposal {
    pub fn validate(&self) -> Result<()> {
        values::generated_id(&self.proposal_id)?;
        values::generated_id(&self.proposer.machine_id)?;
        values::core_id(&self.proposer.agent_id)?;
        require(
            !self.title.is_empty()
                && self.title.chars().count() <= crate::limits::PROPOSAL_TITLE_POINTS,
        )?;
        require(!self.body.is_empty() && self.body.len() <= crate::limits::PROPOSAL_BODY_BYTES)?;
        require(
            !self.proposer.label.is_empty()
                && self.proposer.label.chars().count() <= crate::limits::PROPOSAL_LABEL_POINTS,
        )?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuoteSelector {
    exact: String,
    prefix: String,
    suffix: String,
}
impl QuoteSelector {
    fn validate(&self) -> Result<()> {
        require(!self.exact.is_empty() && self.exact.len() <= COMMENT_BODY_BYTES)?;
        for context in [&self.prefix, &self.suffix] {
            require(
                context.len() <= COMMENT_CONTEXT_BYTES
                    && context.chars().count() <= COMMENT_CONTEXT_POINTS,
            )?;
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThreadRecord {
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
    thread_id: String,
    anchor: Option<QuoteSelector>,
    #[serde(rename = "resolved")]
    _resolved: bool,
    #[serde(default)]
    proposal: Option<Proposal>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadRef {
    writer: String,
    id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommentRecord {
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
    message_id: String,
    thread: ThreadRef,
    body: String,
    #[serde(default)]
    sequence: Option<String>,
    #[serde(default, deserialize_with = "comment_attachments")]
    attachments: Option<attachment::MessageAttachments>,
}
fn comment_attachments<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<attachment::MessageAttachments>, D::Error> {
    List::deserialize(d).map(Some)
}
fn scope(
    version: u8,
    space: &str,
    page: &str,
    epoch: &str,
    sender: &str,
    revision: &str,
    name: &str,
) -> Result<()> {
    require(version == 1 && name.len() <= 128)?;
    values::space_id(space)?;
    values::generated_id(page)?;
    values::generated_id(sender)?;
    values::decimal(epoch, false)?;
    values::decimal(revision, false)?;
    Ok(())
}
fn timestamp(at: &str) -> Result<()> {
    require(values::decimal(at, true)? <= 8_640_000_000_000_000)
}
fn require(valid: bool) -> Result<()> {
    if valid { Ok(()) } else { Err(Invalid) }
}
pub(crate) fn validate_record(root: &str, key: &str, value: &Value) -> Result<()> {
    match value.get("kind").and_then(Value::as_str) {
        Some("proposal-decision") => status::validate_decision(root, key, value)?,
        Some("thread-status" | "thread-notification") => status::validate(root, key, value)?,
        Some("thread") => {
            require(value.as_object().is_some_and(|v| {
                v.len() == 13 + usize::from(v.contains_key("proposal"))
                    && v.contains_key("anchor")
                    && (!v.contains_key("proposal") || v["proposal"].is_object())
            }))?;
            let v: ThreadRecord = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
            require(
                root == "threads"
                    && v.kind == "thread"
                    && key == format!("{}:{}", v.thread_id, v.revision),
            )?;
            values::generated_id(&v.thread_id)?;
            scope(
                v.version,
                &v.space_id,
                &v.page_id,
                &v.epoch,
                &v.sender_device,
                &v.revision,
                &v.device_name,
            )?;
            timestamp(&v.at)?;
            if let Some(proposal) = &v.proposal {
                proposal.validate()?;
                require(
                    proposal.proposal_id == v.thread_id
                        && proposal.proposal_id != v.sender_device
                        && v.anchor.is_none(),
                )?;
            }
            require(!v.deleted || v.anchor.is_none())?;
            if let Some(anchor) = v.anchor {
                anchor.validate()?;
            }
        }
        Some("comment") => {
            // An explicit null is not an absent field.
            require(value.get("sequence").is_none_or(Value::is_string))?;
            let v: CommentRecord = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
            require(
                root == "messages"
                    && v.kind == "comment"
                    && key == format!("{}:{}", v.message_id, v.revision),
            )?;
            values::generated_id(&v.message_id)?;
            values::generated_id(&v.thread.writer)?;
            values::generated_id(&v.thread.id)?;
            scope(
                v.version,
                &v.space_id,
                &v.page_id,
                &v.epoch,
                &v.sender_device,
                &v.revision,
                &v.device_name,
            )?;
            timestamp(&v.at)?;
            if let Some(sequence) = &v.sequence {
                require(values::decimal(sequence, false)? <= MAX_SEQUENCE)?;
            }
            require(v.body.len() <= COMMENT_BODY_BYTES)?;
            let attachments = v.attachments.as_ref().map_or(&[][..], List::as_slice);
            attachment::validate_attachment_list(
                attachments,
                attachment::MESSAGE_ATTACHMENTS,
                Some((&v.space_id, &v.page_id)),
            )?;
            require(if v.deleted {
                v.body.is_empty() && attachments.is_empty()
            } else {
                !v.body.is_empty() || !attachments.is_empty()
            })?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comment_sequence_is_a_canonical_decimal_up_to_two_to_the_53_minus_1() {
        let f: Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/discussion-v1.json"
        ))
        .unwrap();
        let comment = f["comment"].clone();
        let key = format!("{}:1", comment["messageId"].as_str().unwrap());
        validate_record("messages", &key, &comment).unwrap();
        let cases = &f["sequenceCases"];
        for valid in cases["valid"].as_array().unwrap() {
            let mut value = comment.clone();
            value["sequence"] = valid.clone();
            validate_record("messages", &key, &value).unwrap();
        }
        for invalid in cases["invalid"]
            .as_array()
            .unwrap()
            .iter()
            .chain(cases["invalidTypes"].as_array().unwrap())
        {
            let mut value = comment.clone();
            value["sequence"] = invalid.clone();
            assert!(
                validate_record("messages", &key, &value).is_err(),
                "{invalid}"
            );
        }
    }
    #[test]
    fn proposal_bounds_are_unicode_points_and_utf8_bytes() {
        let f: Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/discussion-v1.json"
        ))
        .unwrap();
        let mut value = f["proposal"].clone();
        let key = format!("{}:1", value["threadId"].as_str().unwrap());
        validate_record("threads", &key, &value).unwrap();
        value["proposal"]["title"] = Value::String("🐈".repeat(200));
        value["proposal"]["body"] = Value::String("é".repeat(2048));
        value["proposal"]["proposer"]["label"] = Value::String("🐈".repeat(64));
        validate_record("threads", &key, &value).unwrap();
        for (field, invalid) in [("title", "🐈".repeat(201)), ("body", "é".repeat(2049))] {
            let mut bad = value.clone();
            bad["proposal"][field] = Value::String(invalid);
            assert!(validate_record("threads", &key, &bad).is_err());
        }
        value["proposal"]["proposer"]["label"] = Value::String("🐈".repeat(65));
        assert!(validate_record("threads", &key, &value).is_err());
    }
}
