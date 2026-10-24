//! Request history operations reuse the existing inspection contract.

use super::{DispatchIdentity, Fault, identity_entry, invalid};
use crate::send_now::{SendNow, untyped_failure};
use crate::{
    request_history::{self, ListQuery},
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use tmt_core::{
    request::{
        RequestError, RequestService, attention::AttentionRejection, delivery::SendNowRejection,
        history::HistoryScope,
    },
    settings::Settings,
};

pub(super) fn decode_list(input: &[u8]) -> Result<ListQuery, Fault> {
    request_history::decode_list_query(input).ok_or_else(invalid)
}

pub(super) fn decode_show(input: &[u8]) -> Result<String, Fault> {
    request_history::decode_history_request(input).ok_or_else(invalid)
}

pub(super) fn list_history(storage: &mut Storage, query: ListQuery) -> Result<Vec<u8>, Fault> {
    let mut service = RequestService::new(storage, wall_time_ms);
    match query {
        ListQuery::History(query) => {
            let results = matches!(query.scope, HistoryScope::OriginatorResults { .. });
            service
                .request_history(query)
                .map(|value| request_history::encode_history_page(&value, results))
        }
        ListQuery::Answers(query) => service
            .request_answers(query)
            .map(|value| request_history::encode_answers_page(&value)),
    }
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
    let caller =
        caller_id(storage, selector)?.ok_or_else(|| rejection(SendNowRejection::NotAllowed))?;
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

/// The identity the envelope names; `None` is the anonymous originator.
fn caller_id(
    storage: &mut Storage,
    selector: Option<DispatchIdentity>,
) -> Result<Option<String>, Fault> {
    match selector {
        Some(DispatchIdentity::Explicit(selector)) => {
            Ok(Some(identity_entry(storage, &selector)?.id))
        }
        Some(DispatchIdentity::SavedId(id)) => storage
            .find_active_identity_by_id(&id)
            .map_err(|_| Fault::unavailable())?
            .map(|entry| Some(entry.id))
            .ok_or_else(|| Fault::new("NAME_NOT_FOUND", "Caller identity was not found.")),
        None => Ok(None),
    }
}

/// Most requests one `requests.ack` call acknowledges, and the input bytes that many take.
const ACK_LIMIT: usize = 50;
const ACK_INPUT_LIMIT: usize = 8 * 1024;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AckWire {
    items: Vec<AckItem>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AckItem {
    request_id: String,
    revision: u64,
}

/// `requests.ack` input: 1 to 50 distinct requests, each with the revision being acknowledged.
pub(super) fn decode_ack(input: &[u8]) -> Result<Vec<(String, u64)>, Fault> {
    if input.len() > ACK_INPUT_LIMIT {
        return Err(invalid());
    }
    let wire: AckWire = serde_json::from_slice(input).map_err(|_| invalid())?;
    let mut seen = std::collections::BTreeSet::new();
    if wire.items.is_empty()
        || wire.items.len() > ACK_LIMIT
        || !wire.items.iter().all(|item| {
            crate::request_runtime::valid_request_id(&item.request_id)
                && (1..=tmt_core::limits::MAX_JS_SAFE_INTEGER).contains(&item.revision)
                && seen.insert(item.request_id.as_str())
        })
    {
        return Err(invalid());
    }
    Ok(wire
        .items
        .into_iter()
        .map(|item| (item.request_id, item.revision))
        .collect())
}

/// `requests.ack`: the caller named by the envelope acknowledges what answered their messages,
/// the same acknowledgment `tmt x ack` makes, one request at its exact revision. Each item
/// settles on its own: `acknowledged`, `already` (nothing to change), `conflict` with the
/// request's `current` revision (a newer answer arrived, so it stays unread) or `notFound`.
pub(super) fn acknowledge(
    storage: &mut Storage,
    selector: Option<DispatchIdentity>,
    items: Vec<(String, u64)>,
) -> Result<Vec<u8>, Fault> {
    let caller = caller_id(storage, selector)?.ok_or_else(invalid)?;
    let mut results = Vec::with_capacity(items.len());
    for (request_id, revision) in items {
        let result = RequestService::new(&mut *storage, wall_time_ms).acknowledge_exchange(
            &caller,
            &request_id,
            revision,
        );
        results.push(match result {
            Ok(done) => serde_json::json!({
                "requestId": request_id, "revision": revision,
                "status": if done.changed { "acknowledged" } else { "already" },
            }),
            Err(RequestError::Attention(AttentionRejection::RevisionConflict {
                current, ..
            })) => serde_json::json!({
                "requestId": request_id, "revision": revision,
                "status": "conflict", "current": current,
            }),
            Err(RequestError::Attention(AttentionRejection::NotFound)) => serde_json::json!({
                "requestId": request_id, "revision": revision, "status": "notFound",
            }),
            Err(RequestError::Attention(AttentionRejection::Invalid(_))) => return Err(invalid()),
            Err(error) => return Err(request_error(error)),
        });
    }
    serde_json::to_vec(&serde_json::json!({ "items": results })).map_err(|_| Fault::unavailable())
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
