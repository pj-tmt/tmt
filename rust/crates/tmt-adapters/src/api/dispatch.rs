//! Dispatch receipt recovery and the existing one-shot wake composition.

use super::{Fault, identity, invalid};
use crate::{dispatch, request_runtime::wall_time_ms, storage::Storage};
use tmt_core::{dispatch::DispatchInput, request::Originator, settings::Settings};

pub(super) fn decode_show(input: &[u8]) -> Result<String, Fault> {
    dispatch::decode_dispatch_lookup(input).ok_or_else(invalid)
}

pub(super) fn decode_create(input: &[u8]) -> Result<DispatchInput, Fault> {
    dispatch::decode_input(input).ok_or_else(invalid)
}

pub(super) fn show_receipt(storage: &Storage, id: String) -> Result<Vec<u8>, Fault> {
    storage
        .dispatch_receipt(&id)
        .map_err(|_| Fault::unavailable())?
        .map(|value| dispatch::encode_receipt(&value))
        .ok_or_else(|| Fault::new("DISPATCH_NOT_FOUND", "Operation receipt was not found."))
}

pub(super) fn create_dispatch(
    storage: &mut Storage,
    selector: Option<String>,
    mut input: DispatchInput,
    settings: Option<Settings>,
) -> Result<Vec<u8>, Fault> {
    input.originator = match &selector {
        Some(selector) => Originator::Explicit(identity(storage, selector)?),
        None => Originator::Unknown,
    };
    let settings = settings.expect("dispatch settings");
    let direct = input.kind == tmt_core::request::RequestKind::Request
        && input.recipient_ids.len() == 1
        && !matches!(
            input.room.as_ref(),
            Some(tmt_core::dispatch::DispatchRoom::Roster { .. })
        );
    let (receipt, created) = storage
        .dispatch_request_with_creation(input, settings.retention_days, wall_time_ms)
        .map_err(|error| {
            Fault::new(
                error.code(),
                "Dispatch could not be confirmed; retain the operation ID and recover its receipt.",
            )
        })?;
    let wake = if direct
        && created
        && receipt
            .items
            .first()
            .is_some_and(|item| item.acceptance == tmt_core::dispatch::Acceptance::Queued)
    {
        let item = &receipt.items[0];
        let message = format!(
            "[tmt] request {} is queued: tmt x show {} --incoming --identity {} --json",
            item.request_id, item.request_id, item.recipient_id
        );
        Some(crate::delivery::wake_request(
            storage,
            &item.request_id,
            &item.recipient_id,
            &message,
            std::time::Duration::from_secs_f64(settings.paste_enter_delay_ms.min(500.0) / 1000.0),
        ))
    } else {
        None
    };
    Ok(dispatch::encode_receipt_with_wake(&receipt, wake))
}
