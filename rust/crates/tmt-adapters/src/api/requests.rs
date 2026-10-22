//! Request history operations reuse the existing inspection contract.

use super::{DispatchIdentity, Fault, identity_entry, invalid};
use crate::send_now::{SendNow, untyped_failure};
use crate::{
    request_history,
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use tmt_core::{
    request::{
        RequestError, RequestService,
        delivery::SendNowRejection,
        history::{HistoryQuery, HistoryScope},
    },
    settings::Settings,
};

pub(super) fn decode_list(input: &[u8]) -> Result<HistoryQuery, Fault> {
    request_history::decode_history_query(input).ok_or_else(invalid)
}

pub(super) fn decode_show(input: &[u8]) -> Result<String, Fault> {
    request_history::decode_history_request(input).ok_or_else(invalid)
}

pub(super) fn list_history(storage: &mut Storage, query: HistoryQuery) -> Result<Vec<u8>, Fault> {
    let results = matches!(query.scope, HistoryScope::OriginatorResults { .. });
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

/// `requests.sendNow`: the caller named by the envelope sends one message now. The reply is
/// `{status: "sent"|"already", requestId, state}`; a refusal carries the `SEND_NOW_*` or
/// `DELIVERY_*` code and types nothing it was not asked to.
pub(super) fn send_now(
    storage: &mut Storage,
    selector: Option<DispatchIdentity>,
    request_id: &str,
    settings: Option<Settings>,
) -> Result<Vec<u8>, Fault> {
    let caller = match selector {
        Some(DispatchIdentity::Explicit(selector)) => identity_entry(storage, &selector)?.id,
        Some(DispatchIdentity::SavedId(id)) => {
            storage
                .find_active_identity_by_id(&id)
                .map_err(|_| Fault::unavailable())?
                .ok_or_else(|| Fault::new("NAME_NOT_FOUND", "Caller identity was not found."))?
                .id
        }
        None => return Err(rejection(SendNowRejection::NotAllowed)),
    };
    let settings = settings.expect("send-now settings");
    let delay =
        std::time::Duration::from_secs_f64(settings.paste_enter_delay_ms.min(500.0) / 1000.0);
    let (status, state) = match crate::send_now::send_now(storage, request_id, &caller, delay) {
        Ok(SendNow::Typed) => ("sent", "delivered"),
        Ok(SendNow::Already(state)) => ("already", state.as_str()),
        Ok(SendNow::Untyped(delivery)) => {
            let (code, message) = untyped_failure(&delivery);
            return Err(Fault::new(code, message));
        }
        Ok(SendNow::Closed) => return Err(rejection(SendNowRejection::Closed)),
        Err(RequestError::SendNow(reason)) => return Err(rejection(reason)),
        Err(error) => return Err(request_error(error)),
    };
    serde_json::to_vec(
        &serde_json::json!({"status": status, "requestId": request_id, "state": state}),
    )
    .map_err(|_| Fault::unavailable())
}

fn rejection(reason: SendNowRejection) -> Fault {
    Fault::detailed(reason.code(), reason.message())
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
