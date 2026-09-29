//! Read-only saved-identity notes, never caller-selected filesystem paths.

use super::{Request, response};
use serde_json::json;
use std::{io, net::TcpStream};
use tmt_adapters::config::ConfigPaths;

pub(super) const PREFIX: &str = "/api/v1/local/notebooks/";

pub(super) fn api(stream: &mut TcpStream, request: Request, paths: &ConfigPaths) -> io::Result<()> {
    let Some(id) = request
        .path
        .strip_prefix(PREFIX)
        .filter(|id| tmt_core::dispatch::canonical_id(id))
    else {
        return response(stream, 404, "application/json", br#"{"error":"NOT_FOUND"}"#);
    };
    if request.method != "GET" {
        return response(
            stream,
            405,
            "application/json",
            br#"{"error":"METHOD_NOT_ALLOWED"}"#,
        );
    }
    let (status, body) = match super::core::open(paths)
        .and_then(|core| core.api("notes.read", json!({"identityId": id}), None))
    {
        Ok(notebook) => (200, notebook),
        Err(error) => {
            let (status, code) = status_for(&error.code);
            (status, json!({ "error": code }))
        }
    };
    response(
        stream,
        status,
        "application/json",
        &serde_json::to_vec(&body)?,
    )
}

/// Every notebook code core can answer with has an explicit HTTP status; any
/// other code is reported as an unavailable notebook.
const NOTEBOOK_STATUSES: &[(&str, u16)] = &[
    ("NOTEBOOK_INVALID_IDENTITY", 400),
    ("API_INPUT_INVALID", 400),
    ("NOTEBOOK_IDENTITY_NOT_FOUND", 404),
    ("NOTEBOOK_NOT_FOUND", 404),
    ("NOTEBOOK_SAVED_IDENTITY_REQUIRED", 403),
    ("NOTEBOOK_TOO_LARGE", 413),
    ("NOTEBOOK_INVALID_TEXT", 422),
    ("NOTEBOOK_UNAVAILABLE", 500),
];

fn status_for(code: &str) -> (u16, &str) {
    NOTEBOOK_STATUSES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or((500, "NOTEBOOK_UNAVAILABLE"), |(known, status)| {
            (
                *status,
                if *known == "API_INPUT_INVALID" {
                    "NOTEBOOK_INVALID_IDENTITY"
                } else {
                    known
                },
            )
        })
}

#[cfg(test)]
mod tests;
