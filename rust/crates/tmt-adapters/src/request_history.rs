//! Strict local-owner request inspection protocol; never carries reply credentials.

use crate::request_runtime::valid_request_id;
use serde::Deserialize;
use serde_json::{Value, json};
use tmt_core::{
    dispatch::canonical_id,
    limits::MAX_JS_SAFE_INTEGER,
    request::{RequestPrompt, attention::FinalState, history::*},
};

pub const HISTORY_INPUT_LIMIT: usize = 4096;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorWire {
    prepared_at_ms: Option<u64>,
    submitted_at_ms: Option<u64>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QueryWire {
    originator_id: Option<String>,
    view: Option<String>,
    recipient_id: Option<String>,
    room_id: Option<String>,
    limit: Option<u64>,
    before: Option<CursorWire>,
}

pub fn decode_history_query(bytes: &[u8]) -> Option<HistoryQuery> {
    if bytes.len() > HISTORY_INPUT_LIMIT {
        return None;
    }
    let input: QueryWire = serde_json::from_slice(bytes).ok()?;
    if input
        .recipient_id
        .as_deref()
        .is_some_and(|id| !canonical_id(id))
        || input.room_id.as_deref().is_some_and(|id| !canonical_id(id))
    {
        return None;
    }
    let scope = match (
        input.originator_id,
        input.view.as_deref(),
        input.recipient_id,
        input.room_id,
    ) {
        (Some(id), Some("results"), None, None) if canonical_id(&id) => {
            HistoryScope::OriginatorResults(id)
        }
        (None, None, Some(identity_id), room_id) => HistoryScope::Recipient {
            identity_id,
            room_id,
        },
        (None, None, None, Some(room)) => HistoryScope::Room(room),
        _ => return None,
    };
    let results = matches!(scope, HistoryScope::OriginatorResults(_));
    let limit = input.limit.unwrap_or(if results {
        RESULTS_LIMIT
    } else {
        HISTORY_LIMIT
    });
    if !(1..=HISTORY_MAX_LIMIT).contains(&limit) {
        return None;
    }
    let before = match input.before {
        None => None,
        Some(cursor) => {
            let timestamp = match (results, cursor.prepared_at_ms, cursor.submitted_at_ms) {
                (false, Some(time), None) | (true, None, Some(time)) => time,
                _ => return None,
            };
            if timestamp == 0
                || timestamp > MAX_JS_SAFE_INTEGER
                || !valid_request_id(&cursor.request_id)
            {
                return None;
            }
            Some(if results {
                HistoryCursor::Submitted {
                    submitted_at_ms: timestamp,
                    request_id: cursor.request_id,
                }
            } else {
                HistoryCursor::Prepared {
                    prepared_at_ms: timestamp,
                    request_id: cursor.request_id,
                }
            })
        }
    };
    Some(HistoryQuery {
        scope,
        limit,
        before,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequestWire {
    request_id: String,
}

pub fn decode_history_request(bytes: &[u8]) -> Option<String> {
    if bytes.len() > HISTORY_INPUT_LIMIT {
        return None;
    }
    let input: RequestWire = serde_json::from_slice(bytes).ok()?;
    valid_request_id(&input.request_id).then_some(input.request_id)
}

fn item_document<T>(item: &HistoryItem<T>, content: impl FnOnce(&T) -> Option<Value>) -> Value {
    let final_state = match &item.final_state {
        FinalState::Withdrawn(withdrawal) => {
            json!({"status":"withdrawn", "reason":withdrawal.reason, "withdrawnAtMs":withdrawal.withdrawn_at_ms})
        }
        FinalState::NotSubmitted => json!({"status":"not_submitted"}),
        FinalState::NotRequired => json!({"status":"not_required"}),
        FinalState::Retained {
            content: body,
            submitted_at_ms,
            body_bytes,
            expires_at_ms,
        } => {
            let mut value = json!({"status":"retained","submittedAtMs":submitted_at_ms,"bodyBytes":body_bytes,"expiresAtMs":expires_at_ms});
            if let Some(body) = content(body) {
                value["response"] = body;
            }
            value
        }
        FinalState::Expired {
            submitted_at_ms,
            expires_at_ms,
        } => {
            json!({"status":"expired","submittedAtMs":submitted_at_ms,"expiresAtMs":expires_at_ms})
        }
        FinalState::Unavailable {
            submitted_at_ms,
            expires_at_ms,
        } => {
            json!({"status":"unavailable","submittedAtMs":submitted_at_ms,"expiresAtMs":expires_at_ms})
        }
    };
    json!({"requestId":item.request_id,"roomId":item.room_id,"recipientId":item.recipient_identity_id,
        "sender":{"kind":item.originator.as_str(),"identityId":item.originator.identity_id()},
        "kind":item.kind.as_str(),"urgent":item.delivery_policy.urgent,"focusKind":item.delivery_policy.kind.as_str(),"preparedAtMs":item.prepared_at_ms,"delivery":item.delivery.as_str(),
        "recipientAcknowledged":item.recipient_acknowledged,"final":final_state})
}

pub fn encode_history_page(page: &HistoryPage, results: bool) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "items":page.items.iter().map(|row| {
            let mut value = item_document(&row.item, |_| None);
            value["preview"] = json!(row.preview);
            if results {
                value["responsePreview"] = json!(row.response_preview.as_ref().map(|preview| &preview.text));
                value["previewTruncated"] = json!(row.response_preview.as_ref().is_some_and(|preview| preview.truncated));
            }
            value
        }).collect::<Vec<_>>(),
        "nextBefore":page.next_before.as_ref().map(|cursor| match cursor {
            HistoryCursor::Prepared { prepared_at_ms, request_id } => json!({"preparedAtMs":prepared_at_ms,"requestId":request_id}),
            HistoryCursor::Submitted { submitted_at_ms, request_id } => json!({"submittedAtMs":submitted_at_ms,"requestId":request_id}),
        }),
    })).expect("serializable request history")
}

pub fn encode_history_detail(detail: &HistoryDetail) -> Vec<u8> {
    let mut value = item_document(&detail.item, |body| Some(json!(body)));
    value["prompt"] = match &detail.prompt {
        RequestPrompt::Unavailable => json!({"status":"unavailable"}),
        RequestPrompt::Expired { expires_at_ms } => {
            json!({"status":"expired","expiresAtMs":expires_at_ms})
        }
        RequestPrompt::Retained(prompt) => {
            json!({"status":"retained","message":prompt.message,"messageBytes":prompt.message_bytes,"expiresAtMs":prompt.expires_at_ms})
        }
    };
    serde_json::to_vec(&value).expect("serializable request detail")
}

#[cfg(test)]
mod tests;
