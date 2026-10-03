//! Local MCP wire admission. Command composition stays with the CLI owners.

mod stdio;
#[cfg(test)]
mod tests;
mod tools;

use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};
use std::io;

pub use tools::ToolCall;

/// Canonical exchange text can expand sixfold in JSON; envelopes add metadata.
pub const INPUT_LIMIT: usize = crate::api::INPUT_LIMIT + 65_536;
/// Structured content plus its escaped JSON text copy and the MCP envelope.
pub const OUTPUT_LIMIT: usize = 3 * crate::api::OUTPUT_LIMIT + 65_536;
pub const PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Initialize {
    protocol_version: String,
    capabilities: serde_json::Map<String, Value>,
    client_info: ClientInfo,
}
#[derive(Deserialize)]
struct ClientInfo {
    name: String,
    version: String,
}

#[derive(Default)]
pub(crate) struct Session {
    initialized: bool,
    ready: bool,
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}})
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "result":result})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {
    #[serde(rename = "_meta", default)]
    _meta: serde_json::Map<String, Value>,
}
fn empty(params: Option<&RawValue>) -> bool {
    params.is_none_or(|params| serde_json::from_str::<EmptyParams>(params.get()).is_ok())
}

impl Session {
    pub(crate) fn receive(
        &mut self,
        bytes: &[u8],
        call: &mut impl FnMut(ToolCall) -> Result<Value, Value>,
    ) -> Option<Value> {
        // The syntax check distinguishes JSON parse errors from bad RPC values.
        let value: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => return Some(error(Value::Null, -32700, "Invalid JSON")),
        };
        let message: Message = match serde_json::from_slice(bytes) {
            Ok(message) => message,
            Err(_) => return Some(error(Value::Null, -32600, "Invalid MCP request")),
        };
        let id = message.id;
        if message.jsonrpc != "2.0"
            || id.as_ref().is_some_and(|id| {
                !id.is_string()
                    && !id.as_number().is_some_and(|number| {
                        number.as_i64().is_some_and(|n| {
                            n.unsigned_abs() <= tmt_core::limits::MAX_JS_SAFE_INTEGER
                        })
                    })
            })
            || value.get("id").is_some_and(Value::is_null)
        {
            return Some(error(Value::Null, -32600, "Invalid MCP request"));
        }
        let null_params;
        let params = if value.get("params").is_some_and(Value::is_null) {
            null_params = RawValue::from_string("null".into()).expect("constant JSON");
            Some(null_params.as_ref())
        } else {
            message.params.as_deref()
        };
        let Some(id) = id else {
            // Notifications cannot call tools, initialize or cause core effects.
            if message.method == "notifications/initialized" && self.initialized && empty(params) {
                self.ready = true;
            }
            return None;
        };
        let result = match message.method.as_str() {
            "initialize" if !self.initialized => {
                let initialize =
                    params.and_then(|params| serde_json::from_str::<Initialize>(params.get()).ok());
                let Some(initialize) = initialize else {
                    return Some(error(id, -32602, "Invalid initialize parameters"));
                };
                // Metadata is descriptive only; it never selects the identity.
                let _ = (
                    initialize.capabilities,
                    initialize.client_info.name,
                    initialize.client_info.version,
                );
                let version = PROTOCOLS
                    .iter()
                    .copied()
                    .find(|version| *version == initialize.protocol_version)
                    .unwrap_or(PROTOCOLS[0]);
                self.initialized = true;
                json!({"protocolVersion":version,"capabilities":{"tools":{}},
                    "serverInfo":{"name":"tmt","version":env!("CARGO_PKG_VERSION")},
                    "instructions":"These are same-user local TMT reads. Pull requests with inbox and request. Reads never acknowledge or complete work; unavailable results do not mean cancellation. This server does not enroll a channel or wake an unloaded model."})
            }
            "initialize" => return Some(error(id, -32600, "Already initialized")),
            "ping" if empty(params) => json!({}),
            "tools/list" if self.ready && empty(params) => json!({"tools":tools::definitions()}),
            "tools/call" if self.ready => {
                let Some(tool) = params.and_then(tools::decode) else {
                    return Some(error(id, -32602, "Unknown tool or invalid arguments"));
                };
                let (resource, failed) = match call(tool) {
                    Ok(resource) => (resource, false),
                    Err(resource) => (resource, true),
                };
                let (resource, text, failed) = match stdio::encode(
                    &resource,
                    crate::api::OUTPUT_LIMIT,
                ) {
                    Ok(bytes) => (
                        resource,
                        String::from_utf8(bytes).expect("JSON is UTF-8"),
                        failed,
                    ),
                    Err(_) => {
                        let resource = json!({"error":{"code":"MCP_OUTPUT_TOO_LARGE","message":"Resource exceeds the local MCP output bound."}});
                        let text = resource.to_string();
                        (resource, text, true)
                    }
                };
                json!({"structuredContent":resource,"content":[{"type":"text","text":text}],"isError":failed})
            }
            "tools/list" | "tools/call" if !self.ready => {
                return Some(error(id, -32600, "Initialize before using tools"));
            }
            "ping" | "tools/list" => return Some(error(id, -32602, "Invalid parameters")),
            _ => return Some(error(id, -32601, "Method not found")),
        };
        Some(response(id, result))
    }
}

/// The invocation owns stdin/stdout. No threads, listeners or persistent handle.
pub fn serve(call: impl FnMut(ToolCall) -> Result<Value, Value>) -> io::Result<()> {
    stdio::serve(&io::stdin(), &io::stdout(), call)
}
