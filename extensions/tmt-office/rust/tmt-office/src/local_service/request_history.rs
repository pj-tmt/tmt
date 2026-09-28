//! Owner-only views of canonical requests; reads never acknowledge or resubmit work.

use super::{Request, require_json_origin, response};
use std::{io, net::TcpStream};
use tmt_adapters::{
    config::ConfigPaths,
    dispatch::{decode_dispatch_lookup, encode_receipt},
    office_service::ServiceReceipt,
    request_history::{
        decode_history_query, decode_history_request, encode_history_detail, encode_history_page,
    },
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use tmt_core::request::{RequestError, RequestService, history::HistoryQuery};

const LIST: &str = "/api/v1/local/requests/list";
const SHOW: &str = "/api/v1/local/requests/show";
const RECEIPT: &str = "/api/v1/local/dispatch/show";

pub(super) fn handles(path: &str) -> bool {
    matches!(path, LIST | SHOW | RECEIPT)
}

enum Inspection {
    List(HistoryQuery),
    Show(String),
    Receipt(String),
}

fn inspection(request: &Request) -> Option<Inspection> {
    match request.path.as_str() {
        LIST => decode_history_query(&request.body).map(Inspection::List),
        SHOW => decode_history_request(&request.body).map(Inspection::Show),
        RECEIPT => decode_dispatch_lookup(&request.body).map(Inspection::Receipt),
        _ => None,
    }
}

fn request_failure(error: RequestError<StorageError>) -> (u16, &'static str) {
    match error {
        RequestError::Invalid(_) => (400, "REQUEST_HISTORY_INVALID"),
        RequestError::NotFound => (404, "REQUEST_NOT_FOUND"),
        _ => (500, "STORAGE_UNAVAILABLE"),
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
    let result = match inspection {
        Inspection::List(query) => RequestService::new(&mut storage, wall_time_ms)
            .request_history(query)
            .map(|page| encode_history_page(&page))
            .map_err(request_failure),
        Inspection::Show(id) => RequestService::new(&mut storage, wall_time_ms)
            .request_detail(&id)
            .map(|detail| encode_history_detail(&detail))
            .map_err(request_failure),
        Inspection::Receipt(id) => storage
            .dispatch_receipt(&id)
            .map_err(|_| (500, "STORAGE_UNAVAILABLE"))
            .and_then(|receipt| {
                receipt
                    .map(|receipt| encode_receipt(&receipt))
                    .ok_or((404, "DISPATCH_NOT_FOUND"))
            }),
    };
    let closed = storage.close();
    match result {
        Ok(body) if closed.is_ok() => response(stream, 200, "application/json", &body),
        result => {
            let (status, error) = result.err().unwrap_or((500, "STORAGE_UNAVAILABLE"));
            response(
                stream,
                status,
                "application/json",
                &serde_json::to_vec(&serde_json::json!({"error":error}))?,
            )
        }
    }
}

#[cfg(test)]
mod tests;
