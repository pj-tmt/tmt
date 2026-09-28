//! Authenticated document, immutable capture and PNG resource endpoints.

use super::{Request, require_content_origin, require_json_origin, response};
use std::{io, net::TcpStream};
use tmt_adapters::{
    config::ConfigPaths,
    office_service::ServiceReceipt,
    office_whiteboard::{
        document::{SAVE_INPUT_LIMIT, decode_save, encode_document, encode_receipt},
        image::{SNAPSHOT_PNG_LIMIT, ValidatedSnapshotImage, decode_snapshot_image},
        snapshot::{CAPTURE_INPUT_LIMIT, decode_capture, encode_snapshot},
    },
    request_runtime::wall_time_ms,
    storage::{Storage, WhiteboardStoreError},
};
use tmt_core::office_whiteboard::{
    document::{DocumentError, SaveDocument, SaveReceipt, WhiteboardDocument, valid_document_id},
    snapshot::{CaptureWhiteboard, WhiteboardSnapshot, valid_snapshot_id},
};

#[derive(Debug, PartialEq, Eq)]
enum Route<'a> {
    Document(&'a str),
    Capture(&'a str),
    Snapshot(&'a str),
    Image(&'a str),
}

/// Resource dispatch and admission budgets share exact path parsing.
fn route(path: &str) -> Option<Route<'_>> {
    if let Some(tail) = path.strip_prefix("/api/v1/local/whiteboards/") {
        if let Some(id) = tail.strip_suffix("/snapshots") {
            return valid_document_id(id).then_some(Route::Capture(id));
        }
        return valid_document_id(tail).then_some(Route::Document(tail));
    }
    let tail = path.strip_prefix("/api/v1/local/whiteboard-snapshots/")?;
    if let Some(id) = tail.strip_suffix("/image") {
        return valid_snapshot_id(id).then_some(Route::Image(id));
    }
    valid_snapshot_id(tail).then_some(Route::Snapshot(tail))
}

pub(super) fn input_limit(method: &str, path: &str) -> Option<usize> {
    match (method, route(path)?) {
        ("PUT", Route::Document(_)) => Some(SAVE_INPUT_LIMIT),
        ("POST", Route::Capture(_)) => Some(CAPTURE_INPUT_LIMIT),
        ("PUT", Route::Image(_)) => Some(SNAPSHOT_PNG_LIMIT),
        _ => None,
    }
}

enum Operation<'a> {
    ReadDocument(&'a str),
    Save(SaveDocument),
    Capture(CaptureWhiteboard),
    ReadSnapshot(&'a str),
    AttachImage(&'a str, ValidatedSnapshotImage),
    ReadImage(&'a str),
}

fn prepare<'a>(request: &'a Request, origin: &str) -> Result<Operation<'a>, (u16, &'static [u8])> {
    let invalid = (400, br#"{"error":"WHITEBOARD_INVALID"}"#.as_slice());
    let target = route(&request.path).ok_or((404, br#"{"error":"NOT_FOUND"}"#.as_slice()))?;
    match (request.method.as_str(), target) {
        ("GET", Route::Document(id)) => Ok(Operation::ReadDocument(id)),
        ("GET", Route::Snapshot(id)) => Ok(Operation::ReadSnapshot(id)),
        ("GET", Route::Image(id)) => Ok(Operation::ReadImage(id)),
        ("PUT", Route::Document(id)) => {
            require_json_origin(request, origin)?;
            decode_save(id, &request.body)
                .map(Operation::Save)
                .map_err(|_| invalid)
        }
        ("POST", Route::Capture(id)) => {
            require_json_origin(request, origin)?;
            decode_capture(id, &request.body)
                .map(Operation::Capture)
                .map_err(|_| invalid)
        }
        ("PUT", Route::Image(id)) => {
            require_content_origin(request, origin, "image/png")?;
            decode_snapshot_image(&request.body)
                .map(|image| Operation::AttachImage(id, image))
                .map_err(|_| invalid)
        }
        _ => Err((405, br#"{"error":"METHOD_NOT_ALLOWED"}"#)),
    }
}

enum Resource {
    Document(WhiteboardDocument),
    Receipt(SaveReceipt),
    Snapshot(WhiteboardSnapshot),
    Image(ValidatedSnapshotImage),
}

impl Resource {
    fn encode(self) -> io::Result<(&'static str, Vec<u8>)> {
        let body = match self {
            Self::Document(value) => encode_document(&value).map_err(io::Error::other)?,
            Self::Receipt(value) => encode_receipt(&value).map_err(io::Error::other)?,
            Self::Snapshot(value) => encode_snapshot(&value).map_err(io::Error::other)?,
            Self::Image(value) => return Ok(("image/png", value.into_bytes())),
        };
        Ok(("application/json", body))
    }
}

fn execute(
    storage: &mut Storage,
    operation: Operation<'_>,
) -> Result<Resource, WhiteboardStoreError> {
    match operation {
        Operation::ReadDocument(id) => storage.show_whiteboard(id).map(Resource::Document),
        Operation::Save(input) => storage
            .save_whiteboard(&input, wall_time_ms())
            .map(Resource::Receipt),
        Operation::Capture(input) => storage
            .capture_whiteboard(&input, wall_time_ms())
            .map(Resource::Snapshot),
        Operation::ReadSnapshot(id) => storage.show_whiteboard_snapshot(id).map(Resource::Snapshot),
        Operation::AttachImage(id, image) => storage
            .attach_whiteboard_snapshot_image(id, &image)
            .map(Resource::Image),
        Operation::ReadImage(id) => storage
            .show_whiteboard_snapshot_image(id)
            .map(Resource::Image),
    }
}

pub(super) fn api(
    stream: &mut TcpStream,
    request: Request,
    paths: &ConfigPaths,
    receipt: &ServiceReceipt,
) -> io::Result<()> {
    // The enclosing API owns bearer authentication. Admission precedes storage;
    // no operation lets the browser select an agent actor or dispatch work.
    let origin = format!("http://127.0.0.1:{}", receipt.port);
    let operation = match prepare(&request, &origin) {
        Ok(operation) => operation,
        Err((status, body)) => return response(stream, status, "application/json", body),
    };
    let unavailable = br#"{"error":"STORAGE_UNAVAILABLE"}"#;
    let mut storage = match Storage::open(&paths.database) {
        Ok(storage) => storage,
        Err(_) => return response(stream, 500, "application/json", unavailable),
    };
    let result = execute(&mut storage, operation);
    let close = storage.close();
    match result {
        Ok(resource) => {
            if close.is_err() {
                return response(stream, 500, "application/json", unavailable);
            }
            let (content_type, body) = resource.encode()?;
            response(stream, 200, content_type, &body)
        }
        Err(error) => {
            let status = match &error {
                WhiteboardStoreError::Policy(DocumentError::Invalid) => 400,
                WhiteboardStoreError::Policy(DocumentError::NotFound) => 404,
                WhiteboardStoreError::Policy(
                    DocumentError::RevisionConflict
                    | DocumentError::RevisionExhausted
                    | DocumentError::IdempotencyConflict,
                ) => 409,
                WhiteboardStoreError::Storage(_) => 500,
            };
            let body = serde_json::to_vec(&serde_json::json!({"error":error.code()}))?;
            response(stream, status, "application/json", &body)
        }
    }
}

#[cfg(test)]
mod tests;
