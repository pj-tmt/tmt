//! Owner-only views of canonical requests; reads never acknowledge or resubmit work.

use super::{Request, require_json_origin, response};
use std::{io, net::TcpStream};
use tmt_adapters::{
    config::ConfigPaths,
    dispatch::decode_dispatch_lookup,
    office_service::ServiceReceipt,
    request_history::{decode_history_query, decode_history_request},
};

const LIST: &str = "/api/v1/local/requests/list";
const SHOW: &str = "/api/v1/local/requests/show";
const RECEIPT: &str = "/api/v1/local/dispatch/show";

pub(super) fn handles(path: &str) -> bool {
    matches!(path, LIST | SHOW | RECEIPT)
}

enum Inspection {
    List,
    Show,
    Receipt,
}

/// Admit only bodies the shared decoders accept, so malformed input never
/// reaches core.
fn inspection(request: &Request) -> Option<Inspection> {
    match request.path.as_str() {
        LIST => decode_history_query(&request.body).map(|_| Inspection::List),
        SHOW => decode_history_request(&request.body).map(|_| Inspection::Show),
        RECEIPT => decode_dispatch_lookup(&request.body).map(|_| Inspection::Receipt),
        _ => None,
    }
}

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
    let Some(inspection) = inspection(&request) else {
        return response(
            stream,
            400,
            "application/json",
            br#"{"error":"REQUEST_HISTORY_INVALID"}"#,
        );
    };
    let Ok(input) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
        return response(
            stream,
            400,
            "application/json",
            br#"{"error":"REQUEST_HISTORY_INVALID"}"#,
        );
    };
    let core = match super::core::open(paths) {
        Ok(core) => core,
        Err(_) => return failure(stream, 500, "STORAGE_UNAVAILABLE"),
    };
    let operation = match inspection {
        Inspection::List => "requests.list",
        Inspection::Show => "requests.show",
        Inspection::Receipt => "dispatch.show",
    };
    match core.api(operation, input, None) {
        Ok(body) => response(stream, 200, "application/json", &serde_json::to_vec(&body)?),
        Err(error) => {
            let (status, code) = status_for(&error.code);
            failure(stream, status, code)
        }
    }
}

fn failure(stream: &mut TcpStream, status: u16, code: &str) -> io::Result<()> {
    response(
        stream,
        status,
        "application/json",
        &serde_json::to_vec(&serde_json::json!({"error": code}))?,
    )
}

/// Every code core's request inspection can answer with has an explicit HTTP
/// status; any other code is reported as unavailable storage.
const HISTORY_STATUSES: &[(&str, u16, &str)] = &[
    ("API_INPUT_INVALID", 400, "REQUEST_HISTORY_INVALID"),
    ("REQUEST_NOT_FOUND", 404, "REQUEST_NOT_FOUND"),
    ("DISPATCH_NOT_FOUND", 404, "DISPATCH_NOT_FOUND"),
];

fn status_for(code: &str) -> (u16, &'static str) {
    HISTORY_STATUSES
        .iter()
        .find(|(known, ..)| *known == code)
        .map_or((500, "STORAGE_UNAVAILABLE"), |(_, status, code)| {
            (*status, *code)
        })
}

#[cfg(test)]
mod tests;
