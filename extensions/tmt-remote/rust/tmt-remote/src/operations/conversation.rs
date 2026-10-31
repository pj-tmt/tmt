//! Bounded conversation input for Core's public, viewer-resolved read.
//! This module does not resolve a person, open storage or acknowledge a request.
use crate::{canonical, error::RemoteError, limits};
use serde_json::{Value, json};

/// Validate only the browser-selectable conversation scope. Core alone validates opaque
/// positions and binds them to its resolved viewer/member; Remote never interprets them.
pub(super) fn request(input: &Value) -> Result<Value, RemoteError> {
    let invalid = super::invalid;
    let fields = input.as_object().ok_or_else(invalid)?;
    if fields
        .keys()
        .any(|key| !matches!(key.as_str(), "memberId" | "limit" | "before" | "since"))
    {
        return Err(invalid());
    }
    let member = fields
        .get("memberId")
        .and_then(Value::as_str)
        .filter(|id| canonical::is_core_id(id))
        .ok_or_else(invalid)?;
    if fields.get("since").is_some_and(|value| !position(value)) {
        return Err(invalid());
    }
    let limit = match fields.get("limit") {
        None => limits::CONVERSATION_PAGE_DEFAULT,
        Some(value) => value
            .as_u64()
            .filter(|limit| (1..=limits::CONVERSATION_PAGE_MAX).contains(limit))
            .ok_or_else(invalid)?,
    };
    if let Some(before) = fields.get("before") {
        let cursor = before.as_object().ok_or_else(invalid)?;
        if cursor.len() != 2
            || !cursor.contains_key("preparedAtMs")
            || !cursor.contains_key("requestId")
        {
            return Err(invalid());
        }
        cursor["preparedAtMs"]
            .as_u64()
            .filter(|ms| *ms <= 9_007_199_254_740_991)
            .ok_or_else(invalid)?;
        super::request_reference(cursor["requestId"].as_str().ok_or_else(invalid)?)?;
    }
    let mut query = json!({"view":"conversation", "memberId":member, "limit":limit});
    for key in ["before", "since"] {
        if let Some(value) = fields.get(key) {
            query[key] = value.clone();
        }
    }
    Ok(json!({"version":1,"operation":"requests.list","input":query}))
}

fn position(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        !text.is_empty()
            && text.len() <= limits::CONVERSATION_POSITION_BYTES
            && text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}
fn millis(value: &Value) -> bool {
    value
        .as_u64()
        .is_some_and(|value| value <= 9_007_199_254_740_991)
}
fn preview(value: &Value) -> bool {
    value.is_null()
        || value.as_str().is_some_and(|text| {
            text.len() <= limits::CONVERSATION_PREVIEW_BYTES
                && text.chars().count() <= limits::CONVERSATION_PREVIEW_SCALARS
        })
}
fn id(value: &Value) -> bool {
    value.as_str().is_some_and(canonical::is_core_id)
}
fn request_id(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|id| super::request_reference(id).is_ok())
}
fn word(value: &Value, words: &[&str]) -> bool {
    value.as_str().is_some_and(|text| words.contains(&text))
}
fn require(valid: bool) -> Result<(), RemoteError> {
    if valid { Ok(()) } else { Err(super::invalid()) }
}

/// Allowlist a Core response before signing it for the owner's browser. No nested response
/// text, receipts, pane/provider metadata or unknown Core fields cross this boundary.
pub(super) fn response(value: &Value, query: &Value) -> Result<Value, RemoteError> {
    project(value, query)
        .map_err(|_| RemoteError::new("REMOTE_CORE_UNAVAILABLE", "Conversation is unavailable."))
}
fn project(value: &Value, query: &Value) -> Result<Value, RemoteError> {
    require(
        serde_json::to_vec(value)
            .map_err(|_| super::invalid())?
            .len()
            <= limits::CONVERSATION_RESPONSE_BYTES,
    )?;
    require(
        id(&value["viewerId"])
            && position(&value["changePosition"])
            && word(&value["member"], &["active", "unknown", "retired"]),
    )?;
    let viewer = &value["viewerId"];
    let member = &query["input"]["memberId"];
    let limit = query["input"]["limit"]
        .as_u64()
        .ok_or_else(super::invalid)?;
    let rows = value["items"].as_array().ok_or_else(super::invalid)?;
    require(rows.len() as u64 <= limit && (value["member"] == "active" || rows.is_empty()))?;
    let mut items = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for row in rows {
        require(
            request_id(&row["requestId"])
                && ids.insert(row["requestId"].as_str().unwrap())
                && id(&row["recipientId"])
                && id(&row["sender"]["identityId"])
                && word(&row["sender"]["kind"], &["explicit", "verified"])
                && millis(&row["preparedAtMs"])
                && position(&row["changePosition"])
                && word(&row["kind"], &["request", "announcement"])
                && word(&row["origin"], &["person", "automation", "unknown"])
                && word(
                    &row["delivery"],
                    &[
                        "prepared",
                        "sending",
                        "sent",
                        "queued",
                        "uncertain",
                        "definitely_failed",
                    ],
                )
                && row.get("preview").is_some_and(preview)
                && row.get("responsePreview").is_some_and(preview)
                && row["previewTruncated"].is_boolean()
                && row.get("roomId").is_none_or(Value::is_null),
        )?;
        let in_pair = match row["direction"].as_str() {
            Some("toMember") => {
                &row["recipientId"] == member && &row["sender"]["identityId"] == viewer
            }
            Some("fromMember") => {
                &row["recipientId"] == viewer && &row["sender"]["identityId"] == member
            }
            _ => false,
        };
        require(in_pair)?;
        let message = super::message_state(row, true).ok_or_else(super::invalid)?;
        let final_value = final_state(&row["final"])?;
        if row["final"]["status"] != "retained" {
            require(row["responsePreview"].is_null() && row["previewTruncated"] == false)?;
        }
        let mut item = json!({"requestId":row["requestId"],"recipientId":row["recipientId"],
            "sender":{"kind":row["sender"]["kind"],"identityId":row["sender"]["identityId"]},
            "kind":row["kind"],"origin":row["origin"],"preparedAtMs":row["preparedAtMs"],
            "delivery":row["delivery"],"messageState":message,"final":final_value,
            "preview":row["preview"],"responsePreview":row["responsePreview"],"previewTruncated":row["previewTruncated"],
            "direction":row["direction"],"changePosition":row["changePosition"]});
        if let Some(response_id) = row.get("responseId") {
            require(response_id.as_str().is_some_and(|id| {
                !id.is_empty()
                    && id.len() <= limits::CONVERSATION_RESPONSE_ID_BYTES
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            }))?;
            item["responseId"] = response_id.clone();
        }
        items.push(item);
    }
    let cursor = value.get("nextBefore").ok_or_else(super::invalid)?;
    let next = if cursor.is_null() {
        Value::Null
    } else {
        require(millis(&cursor["preparedAtMs"]) && request_id(&cursor["requestId"]))?;
        json!({"preparedAtMs":cursor["preparedAtMs"],"requestId":cursor["requestId"]})
    };
    let mut result = json!({"viewerId":viewer,"member":value["member"],"items":items,
        "nextBefore":next,"changePosition":value["changePosition"]});
    if query["input"].get("since").is_some() {
        let changed = value.get("changedSince").ok_or_else(super::invalid)?;
        result["changedSince"] = if changed == "unknown" {
            json!("unknown")
        } else {
            let count = changed["count"]
                .as_u64()
                .filter(|count| *count <= limits::CONVERSATION_PAGE_MAX)
                .ok_or_else(super::invalid)?;
            let more = changed["more"].as_bool().ok_or_else(super::invalid)?;
            require(!more || count == limits::CONVERSATION_PAGE_MAX)?;
            json!({"count":count,"more":more})
        };
    } else {
        require(value.get("changedSince").is_none())?;
    }
    require(
        serde_json::to_vec(&result)
            .map_err(|_| super::invalid())?
            .len()
            <= limits::CONVERSATION_RESPONSE_BYTES,
    )?;
    Ok(result)
}
fn final_state(value: &Value) -> Result<Value, RemoteError> {
    let status = value["status"].as_str().ok_or_else(super::invalid)?;
    let mut result = json!({"status":status});
    let fields: &[&str] = match status {
        "not_submitted" | "not_required" => &[],
        "withdrawn" => {
            require(
                value["reason"]
                    .as_str()
                    .is_some_and(|text| text.len() <= limits::CONVERSATION_WITHDRAWAL_BYTES),
            )?;
            result["reason"] = value["reason"].clone();
            &["withdrawnAtMs"]
        }
        "retained" => &["submittedAtMs", "bodyBytes", "expiresAtMs"],
        "expired" | "unavailable" => &["submittedAtMs", "expiresAtMs"],
        _ => return Err(super::invalid()),
    };
    for field in fields {
        require(millis(&value[*field]))?;
        result[*field] = value[*field].clone();
    }
    Ok(result)
}

#[cfg(test)]
#[path = "../../tests/operations/conversation.rs"]
mod tests;
