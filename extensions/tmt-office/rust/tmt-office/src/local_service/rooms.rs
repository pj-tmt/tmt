//! Room definition and membership endpoints use the existing local owner authority.

use super::{Request, require_json_origin, response};
use std::{io, net::TcpStream};
use tmt_adapters::{
    config::ConfigPaths,
    office_service::ServiceReceipt,
    room::{decode_retire, decode_write, encode_room, encode_rooms},
    storage::{RoomStoreError, Storage},
};
use tmt_core::dispatch::canonical_id;
use tmt_core::room::{RoomRepository, RoomWrite};

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
    Save(&'a str, RoomWrite),
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
            decode_write(&request.body).map(|input| Mutation::Save(id, input))
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
    let mut storage = match Storage::open(&paths.database) {
        Ok(storage) => storage,
        Err(_) => {
            return response(
                stream,
                500,
                "application/json",
                br#"{"error":"STORAGE_UNAVAILABLE"}"#,
            );
        }
    };
    let result = match write {
        Some(Mutation::Save(id, input)) => storage
            .save_meeting_room(id, input)
            .map(|room| encode_room(&room)),
        Some(Mutation::Retire(id, revision)) => storage
            .retire_meeting_room(id, revision)
            .map(|room| encode_room(&room)),
        None => storage
            .list_meeting_rooms()
            .map(|rooms| encode_rooms(&rooms)),
    };
    let closed = storage.close();
    match result {
        Ok(body) if closed.is_ok() => response(stream, 200, "application/json", &body),
        Ok(_) => response(
            stream,
            500,
            "application/json",
            br#"{"error":"STORAGE_UNAVAILABLE"}"#,
        ),
        Err(error) => {
            let status = match error {
                RoomStoreError::Invalid => 400,
                RoomStoreError::NotFound => 404,
                RoomStoreError::RevisionConflict
                | RoomStoreError::IdentityInactive
                | RoomStoreError::Retired => 409,
                RoomStoreError::Storage(_) => 500,
            };
            response(
                stream,
                status,
                "application/json",
                &serde_json::to_vec(&serde_json::json!({"error":error.code()}))?,
            )
        }
    }
}

#[cfg(test)]
mod tests;
