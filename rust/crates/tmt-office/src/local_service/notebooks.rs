//! Read-only saved-identity notes, never caller-selected filesystem paths.

use super::{Request, response};
use serde_json::json;
use std::{io, net::TcpStream};
use tmt_adapters::{
    config::ConfigPaths,
    notes::{self, NotebookError},
};

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
    let (status, body) = match notes::read(paths, id) {
        Ok(notebook) => (200, notes::value(&notebook)),
        Err(error) => {
            let status = match error {
                NotebookError::InvalidIdentity => 400,
                NotebookError::IdentityNotFound | NotebookError::Missing => 404,
                NotebookError::SavedIdentityRequired => 403,
                NotebookError::TooLarge => 413,
                NotebookError::InvalidText => 422,
                NotebookError::Unavailable => 500,
            };
            (status, json!({ "error": error.code() }))
        }
    };
    response(
        stream,
        status,
        "application/json",
        &serde_json::to_vec(&body)?,
    )
}

#[cfg(test)]
mod tests;
