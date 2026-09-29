//! Bounded wire decoding only. Lookup and proof validation belong to the
//! request service transaction; this adapter never consults storage or tmux.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Map, Value};
use std::{error::Error, fmt};
use tmt_core::{
    endpoint::ServerEvidence,
    host::HostKind,
    limits::MAX_JS_SAFE_INTEGER,
    request::{RequestEndpoint, RequestRoute, ResponseProof, correlation::response_token},
};

pub const MAX_REPLY_RECEIPT_LENGTH: usize = 8192;
pub const COMPACT_REPLY_RECEIPT_LENGTH: usize = 25;
const COMPACT_PREFIX: &str = "v2_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyReceiptError {
    Invalid,
    Mismatch,
}

impl ReplyReceiptError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "RESPONSE_RECEIPT_INVALID",
            Self::Mismatch => "RESPONSE_RECEIPT_MISMATCH",
        }
    }
}

impl fmt::Display for ReplyReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "Response receipt is invalid.",
            Self::Mismatch => "Response receipt does not match the requested ID.",
        })
    }
}
impl Error for ReplyReceiptError {}

/// Call with an immutable prepared association, never the current live pane.
pub fn encode_short_receipt(
    request_id: &str,
    attempt_id: &str,
    endpoint: &RequestEndpoint,
) -> String {
    encode_route_receipt(
        request_id,
        attempt_id,
        &RequestRoute::Pane(endpoint.clone()),
    )
}

pub fn encode_route_receipt(request_id: &str, attempt_id: &str, route: &RequestRoute) -> String {
    format!(
        "{COMPACT_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(response_token(request_id, attempt_id, route))
    )
}

pub fn decode_reply_receipt(
    encoded: &str,
    request_id: &str,
) -> Result<ResponseProof, ReplyReceiptError> {
    if encoded.is_empty() || encoded.len() > MAX_REPLY_RECEIPT_LENGTH {
        return Err(ReplyReceiptError::Invalid);
    }
    if let Some(payload) = encoded.strip_prefix(COMPACT_PREFIX) {
        if encoded.len() != COMPACT_REPLY_RECEIPT_LENGTH {
            return Err(ReplyReceiptError::Invalid);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| ReplyReceiptError::Invalid)?;
        return Ok(ResponseProof::Compact(
            bytes.try_into().map_err(|_| ReplyReceiptError::Invalid)?,
        ));
    }
    // v1 is decode-only. Canonical BASE64 spelling does not require canonical
    // JSON order or whitespace: the previous decoder accepted both.
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| ReplyReceiptError::Invalid)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ReplyReceiptError::Invalid)?;
    let value: Value = serde_json::from_str(text).map_err(|_| ReplyReceiptError::Invalid)?;
    decode_v1(&value, request_id)
}

fn decode_v1(value: &Value, request_id: &str) -> Result<ResponseProof, ReplyReceiptError> {
    let record = object(value, &["version", "requestId", "attemptId", "endpoint"])?;
    if record["version"].as_f64() != Some(1.0) {
        return Err(ReplyReceiptError::Invalid);
    }
    let recorded_request = bounded_string(&record["requestId"], 256)?;
    let attempt_id = bounded_string(&record["attemptId"], 4096)?;
    let endpoint = object(
        &record["endpoint"],
        &[
            "serverId",
            "socketPath",
            "serverPid",
            "serverStartTime",
            "paneId",
            "panePid",
        ],
    )?;
    // v1 is decode-only and predates every host but tmux.
    let pane_id = bounded_string(&endpoint["paneId"], 4096)?;
    if !HostKind::Tmux.is_pane_id(pane_id) {
        return Err(ReplyReceiptError::Invalid);
    }
    let endpoint = RequestEndpoint {
        server: ServerEvidence {
            host: HostKind::Tmux,
            server_id: bounded_string(&endpoint["serverId"], 4096)?.into(),
            socket_path: bounded_string(&endpoint["socketPath"], 4096)?.into(),
            server_pid: pid(&endpoint["serverPid"])?,
            server_start_time: bounded_string(&endpoint["serverStartTime"], 4096)?.into(),
        },
        pane_id: pane_id.into(),
        pane_pid: pid(&endpoint["panePid"])?,
    };
    if recorded_request != request_id {
        return Err(ReplyReceiptError::Mismatch);
    }
    Ok(ResponseProof::Recorded {
        attempt_id: attempt_id.into(),
        endpoint,
    })
}

fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, ReplyReceiptError> {
    value
        .as_object()
        .filter(|map| map.len() == keys.len() && keys.iter().all(|key| map.contains_key(*key)))
        .ok_or(ReplyReceiptError::Invalid)
}

fn bounded_string(value: &Value, max: usize) -> Result<&str, ReplyReceiptError> {
    value
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= max)
        .ok_or(ReplyReceiptError::Invalid)
}

fn pid(value: &Value) -> Result<u64, ReplyReceiptError> {
    // JSON.parse used IEEE-754 numbers, including integer-valued 1.0 / 1e0.
    let number = value.as_f64().ok_or(ReplyReceiptError::Invalid)?;
    if !number.is_finite()
        || number < 1.0
        || number > MAX_JS_SAFE_INTEGER as f64
        || number.fract() != 0.0
    {
        return Err(ReplyReceiptError::Invalid);
    }
    Ok(number as u64)
}

#[cfg(test)]
mod tests;
