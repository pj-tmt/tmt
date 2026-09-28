//! Authenticated owner request composition; never a shell/CLI execution endpoint.

use super::{Request, require_json_origin, response};
use std::{io, net::TcpStream, time::Duration};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    delivery,
    dispatch::{decode_input, encode_receipt_with_wake},
    office_service::ServiceReceipt,
    request_runtime::{valid_request_id, wall_time_ms},
    storage::{DispatchError, Storage},
};
use tmt_core::{
    dispatch::{Acceptance, DispatchReceipt, DispatchRoom, canonical_id},
    request::{RequestKind, WakeState},
    settings::Settings,
};

pub(super) const PATH: &str = "/api/v1/local/dispatch";

/// A wake is advisory to the already-queued request. In particular, a claimed
/// wake is never retried after process loss because pane input may have occurred.
fn wake_direct_request(
    storage: &mut Storage,
    accepted: &DispatchReceipt,
    settings: &Settings,
) -> Option<WakeState> {
    let [item] = accepted.items.as_slice() else {
        return None;
    };
    if item.acceptance != Acceptance::Queued
        || !canonical_id(&item.recipient_id)
        || !valid_request_id(&item.request_id)
    {
        return None;
    }
    let notification = format!(
        "Office request {} is queued. Read it with: tmt x show {} --incoming --identity {} --json",
        item.request_id, item.request_id, item.recipient_id
    );
    let delay = Duration::from_secs_f64(settings.paste_enter_delay_ms.min(500.0) / 1000.0);
    Some(delivery::wake_request(
        storage,
        &item.request_id,
        &item.recipient_id,
        &notification,
        delay,
    ))
}

#[cfg(test)]
mod tests;

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
    let Some(input) = decode_input(&request.body) else {
        return response(
            stream,
            400,
            "application/json",
            br#"{"error":"DISPATCH_INVALID"}"#,
        );
    };
    let settings = match (ConfigFiles {
        paths: paths.clone(),
    })
    .load()
    {
        Ok(value) => value.settings,
        Err(_) => {
            return response(
                stream,
                500,
                "application/json",
                br#"{"error":"CONFIG_ERROR"}"#,
            );
        }
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
    let direct_request = input.kind == RequestKind::Request
        && input.recipient_ids.len() == 1
        && !matches!(input.room.as_ref(), Some(DispatchRoom::Roster { .. }));
    let result =
        storage.dispatch_request_with_creation(input, settings.retention_days, wall_time_ms);
    let wake = result.as_ref().ok().and_then(|(accepted, created)| {
        (direct_request && *created)
            .then(|| wake_direct_request(&mut storage, accepted, &settings))
            .flatten()
    });
    let closed = storage.close();
    match result {
        Ok((receipt, _)) if closed.is_ok() => response(
            stream,
            200,
            "application/json",
            &encode_receipt_with_wake(&receipt, wake),
        ),
        Ok(_) => response(
            stream,
            500,
            "application/json",
            br#"{"error":"STORAGE_UNAVAILABLE"}"#,
        ),
        Err(error) => {
            let status = match error {
                DispatchError::Invalid => 400,
                DispatchError::IdempotencyConflict
                | DispatchError::RoomRosterChanged
                | DispatchError::RoomRecipientNotMember => 409,
                DispatchError::Request(_) | DispatchError::Storage(_) => 500,
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
