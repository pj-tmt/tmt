//! The read-only tool schemas and strict argument admission, without effects.

use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};

#[derive(Debug)]
pub enum ToolCall {
    List,
    Operation(String),
    Inbox {
        from: Option<String>,
        limit: Option<u64>,
    },
    Request(String),
    Result(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    name: String,
    #[serde(default)]
    arguments: Option<Box<RawValue>>,
    #[serde(rename = "_meta", default)]
    _meta: serde_json::Map<String, Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Operation {
    operation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inbox {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
}
fn text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
}

pub(super) fn decode(params: &RawValue) -> Option<ToolCall> {
    let envelope: serde_json::Map<String, Value> = serde_json::from_str(params.get()).ok()?;
    if envelope.get("arguments").is_some_and(Value::is_null) {
        return None;
    }
    let call: Call = serde_json::from_str(params.get()).ok()?;
    let arguments = call.arguments.as_deref().map_or("{}", RawValue::get);
    // Optional properties may be absent, but JSON null is not their schema type.
    let object: serde_json::Map<String, Value> = serde_json::from_str(arguments).ok()?;
    if object.values().any(Value::is_null) {
        return None;
    }
    Some(match call.name.as_str() {
        "tmt_list" => {
            serde_json::from_str::<Nothing>(arguments).ok()?;
            ToolCall::List
        }
        "tmt_operation" => {
            let input: Operation = serde_json::from_str(arguments).ok()?;
            if !tmt_core::dispatch::canonical_id(&input.operation_id) {
                return None;
            }
            ToolCall::Operation(input.operation_id)
        }
        "tmt_inbox" => {
            let input: Inbox = serde_json::from_str(arguments).ok()?;
            if input
                .limit
                .is_some_and(|n| !(1..=tmt_core::request::inbox::INBOX_MAX_LIMIT).contains(&n))
                || input.from.as_ref().is_some_and(|s| !text(s))
            {
                return None;
            }
            ToolCall::Inbox {
                from: input.from,
                limit: input.limit,
            }
        }
        name @ ("tmt_request" | "tmt_result") => {
            let input: Request = serde_json::from_str(arguments).ok()?;
            if !text(&input.request_id) {
                return None;
            }
            if name == "tmt_request" {
                ToolCall::Request(input.request_id)
            } else {
                ToolCall::Result(input.request_id)
            }
        }
        _ => return None,
    })
}

pub(super) fn definitions() -> Vec<Value> {
    let text = json!({"type":"string","minLength":1,"maxLength":256});
    [
        ("tmt_list", "List local non-retired identity records; presence is not reported.", json!({}), Vec::<&str>::new()),
        ("tmt_operation", "Recover an immutable dispatch acceptance by operation UUID; never sends or wakes again.", json!({"operationId":{"type":"string","format":"uuid"}}), vec!["operationId"]),
        ("tmt_inbox", "List open requests addressed to this server's saved identity, oldest first. Reading never acknowledges.", json!({"from":text,"limit":{"type":"integer","minimum":1,"maximum":200}}), vec![]),
        ("tmt_request", "Read this identity's incoming request and its local reply receipt. Reading does not acknowledge or complete it.", json!({"requestId":text}), vec!["requestId"]),
        ("tmt_result", "Read an exact retained final by request ID. Unavailable is not cancellation or permission to resend.", json!({"requestId":text}), vec!["requestId"]),
    ].into_iter().map(|(name, description, properties, required)| json!({
        "name":name,"description":description,
        "inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false},
    })).collect()
}
