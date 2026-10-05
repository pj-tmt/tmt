//! Inert discussion record admission. Envelopes authenticate writers; no DOM,
//! publication, signing or dispatch authority lives in this codec.
use crate::limits::{COMMENT_BODY_BYTES, COMMENT_CONTEXT_BYTES, COMMENT_CONTEXT_POINTS};
use serde::Deserialize;
use serde_json::Value;
use tmt_colab_model::{Invalid, Result, values};
pub mod status;

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
        Some("thread-status" | "thread-notification") => status::validate(root, key, value)?,
        Some("thread") => {
            require(
                value
                    .as_object()
                    .is_some_and(|v| v.len() == 13 && v.contains_key("anchor")),
            )?;
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
            require(!v.deleted || v.anchor.is_none())?;
            if let Some(anchor) = v.anchor {
                anchor.validate()?;
            }
        }
        Some("comment") => {
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
            require(v.body.len() <= COMMENT_BODY_BYTES)?;
            require(if v.deleted {
                v.body.is_empty()
            } else {
                !v.body.is_empty()
            })?;
        }
        _ => {}
    }
    Ok(())
}
