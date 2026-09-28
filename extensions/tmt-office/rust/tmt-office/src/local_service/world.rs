//! Whole-candidate world edits use the existing authenticated loopback service.

use super::{Request, require_json_origin, response};
#[cfg(test)]
use serde_json::json;
use std::{io, net::TcpStream};
use tmt_adapters::{
    config::ConfigPaths,
    office_service::ServiceReceipt,
    office_world::{WORLD_ENVELOPE_LIMIT, WorldFailure, access, decode_save, snapshot_value},
    request_runtime::wall_time_ms,
};

pub(super) const PATH: &str = "/api/v1/local/world";

pub(super) fn input_limit(method: &str, path: &str) -> Option<usize> {
    (method == "PUT" && path == PATH).then_some(WORLD_ENVELOPE_LIMIT)
}

fn failure(stream: &mut TcpStream, error: WorldFailure) -> io::Result<()> {
    response(
        stream,
        error.code.status(),
        "application/json",
        &serde_json::to_vec(&error)?,
    )
}

pub(super) fn api(
    stream: &mut TcpStream,
    request: Request,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
) -> io::Result<()> {
    // Bearer/Host checks belong to the enclosing API. Reject writes before opening storage.
    let input = match request.method.as_str() {
        "GET" => None,
        "PUT" => {
            let origin = format!("http://127.0.0.1:{}", receipt.port);
            if let Err((status, body)) = require_json_origin(&request, &origin) {
                return response(stream, status, "application/json", body);
            }
            match decode_save(&request.body) {
                Ok(input) => Some(input),
                Err(error) => return failure(stream, error.into()),
            }
        }
        _ => {
            return response(
                stream,
                405,
                "application/json",
                br#"{"error":"METHOD_NOT_ALLOWED"}"#,
            );
        }
    };
    match access::run(&paths.database, input, wall_time_ms()) {
        Ok(snapshot) => response(
            stream,
            200,
            "application/json",
            &serde_json::to_vec(&snapshot_value(&snapshot))?,
        ),
        Err(error) => failure(stream, error),
    }
}

#[cfg(test)]
mod tests;
