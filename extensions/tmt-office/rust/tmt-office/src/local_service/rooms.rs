//! Room definition and membership endpoints use the existing local owner authority.

use super::{Request, require_json_origin, response};
use serde_json::{Value, json};
use std::{io, net::TcpStream};
use tmt_adapters::{
    config::ConfigPaths,
    room::{decode_retire, decode_write},
};
use tmt_core::dispatch::canonical_id;
use tmt_office_service::ServiceReceipt;
use tmt_office_storage::core_client::WriteOriginator;

pub(super) const PATH: &str = "/api/v1/local/rooms";
fn room_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix(PATH)?.strip_prefix('/')?;
    canonical_id(id).then_some(id)
}
pub(super) fn input_limit(method: &str, path: &str) -> Option<usize> {
    ((method == "PUT" && room_id(path).is_some())
        || (method == "POST" && path.strip_suffix("/retire").and_then(room_id).is_some()))
    .then_some(tmt_adapters::room::INPUT_LIMIT)
}

enum Mutation<'a> {
    Save(&'a str, Value),
    Retire(&'a str, u64),
}

pub(super) fn api(
    stream: &mut TcpStream,
    request: Request,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
) -> io::Result<()> {
    // Admit paths, methods and write authority before touching storage.
    let write = if request.path == PATH && request.method == "GET" {
        None
    } else if let Some((id, retiring)) = request
        .path
        .strip_suffix("/retire")
        .and_then(room_id)
        .map(|id| (id, true))
        .or_else(|| room_id(&request.path).map(|id| (id, false)))
    {
        if request.method != if retiring { "POST" } else { "PUT" } {
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
        let mutation = if retiring {
            decode_retire(&request.body).map(|revision| Mutation::Retire(id, revision))
        } else {
            decode_write(&request.body)
                .and_then(|_| serde_json::from_slice(&request.body).ok())
                .map(|room| Mutation::Save(id, room))
        };
        let Some(mutation) = mutation else {
            return response(
                stream,
                400,
                "application/json",
                br#"{"error":"ROOM_INVALID"}"#,
            );
        };
        Some(mutation)
    } else {
        return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
    };
    let core = match super::core::open(paths) {
        Ok(core) => core,
        Err(_) => return storage_unavailable(stream),
    };
    // The browser is the local owner: writes are anonymous, like the CLI without
    // an identity, and carry the browser's expected revision.
    let result = match write {
        Some(Mutation::Save(id, input)) => core.api(
            "rooms.write",
            json!({"roomId": id, "room": input}),
            Some(WriteOriginator::Anonymous),
        ),
        Some(Mutation::Retire(id, revision)) => core.api(
            "rooms.retire",
            json!({"roomId": id, "expectedRevision": revision}),
            Some(WriteOriginator::Anonymous),
        ),
        None => core
            .command(&["room", "list"])
            .map(|value| value["rooms"].clone()),
    };
    match result {
        Ok(body) => response(stream, 200, "application/json", &serde_json::to_vec(&body)?),
        Err(error) => {
            let status = status_for(&error.code);
            response(
                stream,
                status,
                "application/json",
                &serde_json::to_vec(&json!({"error": error_code(&error.code)}))?,
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

/// Every room code core can answer with has an explicit HTTP status; anything
/// else is reported as unavailable storage rather than passed through.
const ROOM_STATUSES: &[(&str, u16)] = &[
    ("ROOM_INVALID", 400),
    ("ROOM_NOT_FOUND", 404),
    ("ROOM_REVISION_CONFLICT", 409),
    ("ROOM_IDENTITY_INACTIVE", 409),
    ("ROOM_RETIRED", 409),
    ("API_INPUT_INVALID", 400),
];

fn status_for(code: &str) -> u16 {
    ROOM_STATUSES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or(500, |(_, status)| *status)
}

fn error_code(code: &str) -> &str {
    if ROOM_STATUSES.iter().any(|(known, _)| *known == code) {
        if code == "API_INPUT_INVALID" {
            "ROOM_INVALID"
        } else {
            code
        }
    } else {
        "STORAGE_UNAVAILABLE"
    }
}

#[cfg(test)]
mod tests;
