//! The exchange tool schemas and strict argument admission, without effects.

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
    Send(Box<tmt_core::dispatch::DispatchInput>),
    Answer {
        request_id: String,
        message: String,
    },
    Ack {
        request_id: String,
        revision: u64,
    },
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Send {
    operation_id: String,
    recipient_id: String,
    message: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Answer {
    request_id: String,
    message: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Ack {
    request_id: String,
    revision: u64,
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
        "tmt_send" => {
            let input: Send = serde_json::from_str(arguments).ok()?;
            let input = tmt_core::dispatch::DispatchInput {
                operation_id: input.operation_id,
                recipient_ids: vec![input.recipient_id],
                message: input.message,
                originator: tmt_core::request::Originator::Unknown,
                kind: tmt_core::request::RequestKind::Request,
                room: None,
            }
            .normalize()?;
            ToolCall::Send(Box::new(input))
        }
        "tmt_answer" => {
            let input: Answer = serde_json::from_str(arguments).ok()?;
            if !text(&input.request_id)
                || tmt_core::exact_text::validate_exact_text(input.message.as_bytes()).is_err()
            {
                return None;
            }
            ToolCall::Answer {
                request_id: input.request_id,
                message: input.message,
            }
        }
        "tmt_ack" => {
            let input: Ack = serde_json::from_str(arguments).ok()?;
            if !text(&input.request_id)
                || input.revision == 0
                || input.revision > tmt_core::limits::MAX_JS_SAFE_INTEGER
            {
                return None;
            }
            ToolCall::Ack {
                request_id: input.request_id,
                revision: input.revision,
            }
        }
        _ => return None,
    })
}

pub(super) fn definitions() -> Vec<Value> {
    let text = json!({"type":"string","minLength":1,"maxLength":256});
    let uuid = json!({"type":"string","format":"uuid"});
    let message =
        json!({"type":"string","maxLength":tmt_core::exact_text::MAX_EXCHANGE_TEXT_BYTES});
    [
        ("tmt_list", "List local non-retired identity records; presence is not reported.", json!({}), Vec::<&str>::new()),
        ("tmt_operation", "Recover an immutable dispatch acceptance by operation UUID; never sends or wakes again.", json!({"operationId":{"type":"string","format":"uuid"}}), vec!["operationId"]),
        ("tmt_inbox", "List open requests addressed to this server's saved identity, oldest first. Reading never acknowledges.", json!({"from":text,"limit":{"type":"integer","minimum":1,"maximum":200}}), vec![]),
        ("tmt_request", "Read this identity's incoming request and its local reply receipt. Reading does not acknowledge or complete it.", json!({"requestId":text}), vec!["requestId"]),
        ("tmt_result", "Read an exact retained final by request ID. Unavailable is not cancellation or permission to resend.", json!({"requestId":text}), vec!["requestId"]),
        ("tmt_send", "Dispatch one durable request. Returns acceptance, with an optional first advisory wake. Retain operationId for recovery; identical retries never dispatch or wake twice.", json!({"operationId":uuid,"recipientId":uuid,"message":message}), vec!["operationId","recipientId","message"]),
        ("tmt_answer", "Submit this saved identity's exact final for one incoming request. Identical retained retries are safe; changed finals conflict. Does not acknowledge attention.", json!({"requestId":text,"message":message}), vec!["requestId","message"]),
        ("tmt_ack", "Acknowledge one incoming request at its observed revision. A stale revision cannot suppress newer attention.", json!({"requestId":text,"revision":{"type":"integer","minimum":1,"maximum":tmt_core::limits::MAX_JS_SAFE_INTEGER}}), vec!["requestId","revision"]),
    ].into_iter().map(|(name, description, properties, required)| json!({
        "name":name,"description":description,
        "inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},
        "annotations":{"readOnlyHint":!matches!(name, "tmt_send" | "tmt_answer" | "tmt_ack"),"destructiveHint":false,"idempotentHint":true,"openWorldHint":false},
    })).collect()
}
