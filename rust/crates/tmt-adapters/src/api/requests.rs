//! Request history operations reuse the existing inspection contract.

use super::{Fault, invalid};
use crate::{
    request_history,
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use tmt_core::request::{
    RequestService,
    history::{HistoryQuery, HistoryScope},
};

pub(super) fn decode_list(input: &[u8]) -> Result<HistoryQuery, Fault> {
    request_history::decode_history_query(input).ok_or_else(invalid)
}

pub(super) fn decode_show(input: &[u8]) -> Result<String, Fault> {
    request_history::decode_history_request(input).ok_or_else(invalid)
}

pub(super) fn list_history(storage: &mut Storage, query: HistoryQuery) -> Result<Vec<u8>, Fault> {
    let results = matches!(query.scope, HistoryScope::OriginatorResults(_));
    RequestService::new(storage, wall_time_ms)
        .request_history(query)
        .map(|value| request_history::encode_history_page(&value, results))
        .map_err(request_error)
}

pub(super) fn show_request(storage: &mut Storage, id: String) -> Result<Vec<u8>, Fault> {
    RequestService::new(storage, wall_time_ms)
        .request_detail(&id)
        .map(|value| request_history::encode_history_detail(&value))
        .map_err(request_error)
}

fn request_error(error: tmt_core::request::RequestError<StorageError>) -> Fault {
    match error {
        tmt_core::request::RequestError::Invalid(_) => invalid(),
        tmt_core::request::RequestError::NotFound => {
            Fault::new("REQUEST_NOT_FOUND", "Request was not found.")
        }
        _ => Fault::unavailable(),
    }
}
