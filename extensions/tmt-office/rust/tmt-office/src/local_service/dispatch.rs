//! Authenticated owner request composition; never a shell/CLI execution endpoint.

use super::{Request, require_json_origin, response};
use std::{io, net::TcpStream};
use tmt_adapters::{config::ConfigPaths, dispatch::decode_input, office_service::ServiceReceipt};
use tmt_office_storage::core_client::Originator;

pub(super) const PATH: &str = "/api/v1/local/dispatch";

#[cfg(test)]
mod tests;

pub(super) fn api(
    stream: &mut TcpStream,
    request: Request,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
) -> io::Result<()> {
    if request.method != "POST" {
        return response(
            stream,
            405,
            "application/json",
            br#"{"error":"METHOD_NOT_ALLOWED"}"#,
        );
    }
    if let Err((status, body)) =
        require_json_origin(&request, &format!("http://127.0.0.1:{}", receipt.port))
    {
        return response(stream, status, "application/json", body);
    }
    if decode_input(&request.body).is_none() {
        return response(
            stream,
            400,
            "application/json",
            br#"{"error":"DISPATCH_INVALID"}"#,
        );
    }
    let core = match super::core::open(paths) {
        Ok(core) => core,
        Err(_) => return storage_unavailable(stream),
    };
    let Ok(body) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
        return response(
            stream,
            400,
            "application/json",
            br#"{"error":"DISPATCH_INVALID"}"#,
        );
    };
    // Core composes acceptance, the advisory wake and settings; the browser is
    // the local owner, so the originator is anonymous like the CLI without one.
    match core.api("dispatch.create", body, Some(Originator::Anonymous)) {
        Ok(receipt) => response(
            stream,
            200,
            "application/json",
            &serde_json::to_vec(&receipt)?,
        ),
        Err(error) => {
            let (status, code) = failure(&error.code);
            response(
                stream,
                status,
                "application/json",
                &serde_json::to_vec(&serde_json::json!({"error": code}))?,
            )
        }
    }
}

fn storage_unavailable(stream: &mut TcpStream) -> io::Result<()> {
    response(
        stream,
        500,
        "application/json",
        br#"{"error":"STORAGE_UNAVAILABLE"}"#,
    )
}

/// Every code core's dispatch can answer with has an explicit HTTP status; any
/// other code is reported as unavailable storage.
const DISPATCH_STATUSES: &[(&str, u16, &str)] = &[
    ("DISPATCH_INVALID", 400, "DISPATCH_INVALID"),
    ("API_INPUT_INVALID", 400, "DISPATCH_INVALID"),
    (
        "DISPATCH_IDEMPOTENCY_CONFLICT",
        409,
        "DISPATCH_IDEMPOTENCY_CONFLICT",
    ),
    ("ROOM_ROSTER_CHANGED", 409, "ROOM_ROSTER_CHANGED"),
    (
        "ROOM_RECIPIENT_NOT_MEMBER",
        409,
        "ROOM_RECIPIENT_NOT_MEMBER",
    ),
    ("CONFIG_ERROR", 500, "CONFIG_ERROR"),
];

fn failure(code: &str) -> (u16, &'static str) {
    DISPATCH_STATUSES
        .iter()
        .find(|(known, ..)| *known == code)
        .map_or((500, "STORAGE_UNAVAILABLE"), |(_, status, code)| {
            (*status, *code)
        })
}
