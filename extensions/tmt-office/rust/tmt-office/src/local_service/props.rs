//! Browser artwork authoring reuses native admission and the existing catalog CAS.

use super::{Request, require_json_origin, response};
use serde::Deserialize;
use serde_json::json;
use std::{io, net::TcpStream};
use tmt_adapters::{
    config::ConfigPaths,
    office_prop::{self, ValidatedPropPack},
    office_service::ServiceReceipt,
    storage::{LocalPropCatalogError, Storage},
};

const LIST: &str = "/api/v1/local/props/list";
const INSTALL: &str = "/api/v1/local/props/install";
const INSTALL_ENVELOPE_LIMIT: usize = office_prop::PACK_INPUT_LIMIT * 6 + 512;
const LIST_INPUT_LIMIT: usize = 4096;

pub(super) fn handles(path: &str) -> bool {
    matches!(path, LIST | INSTALL)
}

pub(super) fn input_limit(method: &str, path: &str) -> Option<usize> {
    if method != "POST" {
        return None;
    }
    match path {
        INSTALL => Some(INSTALL_ENVELOPE_LIMIT),
        LIST => Some(LIST_INPUT_LIMIT),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListInput {
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallInput {
    expected_revision: u64,
    document: String,
}

enum Operation {
    List(ListInput),
    Install {
        revision: u64,
        candidate: ValidatedPropPack,
    },
}

fn decode(request: &Request) -> Option<Operation> {
    if request.body.len() > input_limit(&request.method, &request.path)? {
        return None;
    }
    if request.path == LIST {
        let input: ListInput = serde_json::from_slice(&request.body).ok()?;
        if input
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > office_prop::CATALOG_CURSOR_MAX_BYTES)
        {
            return None;
        }
        Some(Operation::List(input))
    } else {
        let input: InstallInput = serde_json::from_slice(&request.body).ok()?;
        if input.expected_revision > tmt_core::office_profile::MAX_REVISION {
            return None;
        }
        Some(Operation::Install {
            revision: input.expected_revision,
            candidate: office_prop::validate_pack(input.document.as_bytes()).ok()?,
        })
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
    let Some(operation) = decode(&request) else {
        return response(
            stream,
            400,
            "application/json",
            br#"{"error":"OFFICE_PROP_INVALID"}"#,
        );
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
    let result = match operation {
        Operation::List(input) => storage.list_local_prop_packs(20, input.cursor.as_deref()).map(|page| json!({
            "revision": page.catalog_revision,
            "entries": page.packs.iter().map(|entry| json!({"digest":entry.pack.digest(),"label":entry.pack.pack().label})).collect::<Vec<_>>(),
            "excluded": page.excluded.iter().map(|entry| json!({"digest":entry.digest,"reason":entry.reason.code()})).collect::<Vec<_>>(),
            "nextCursor": page.next_cursor,
        })),
        Operation::Install { revision, candidate } => storage.install_local_prop_pack(revision, &candidate).map(|receipt| json!({
            "revision":receipt.catalog_revision,"digest":receipt.digest,"changed":receipt.changed,
        })),
    };
    let closed = storage.close();
    match result {
        Ok(value) if closed.is_ok() => response(
            stream,
            200,
            "application/json",
            &serde_json::to_vec(&value)?,
        ),
        Ok(_) => response(
            stream,
            500,
            "application/json",
            br#"{"error":"STORAGE_UNAVAILABLE"}"#,
        ),
        Err(error) => {
            let status = match &error {
                LocalPropCatalogError::Invalid | LocalPropCatalogError::CursorInvalid => 400,
                LocalPropCatalogError::NotFound => 404,
                LocalPropCatalogError::Storage(_) | LocalPropCatalogError::Corrupt => 500,
                _ => 409,
            };
            response(
                stream,
                status,
                "application/json",
                &serde_json::to_vec(&json!({"error":office_prop::catalog_error(error).code()}))?,
            )
        }
    }
}

#[cfg(test)]
mod tests;
