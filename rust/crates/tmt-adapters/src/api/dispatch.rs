//! Dispatch receipt recovery and the existing one-shot wake composition.

use super::{DispatchIdentity, Fault, identity_entry, invalid};
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
    selector: Option<DispatchIdentity>,
    mut input: DispatchInput,
    settings: Option<Settings>,
) -> Result<Vec<u8>, Fault> {
    let (originator, sender) = match selector {
        Some(DispatchIdentity::Explicit(selector)) => {
            let selected = identity_entry(storage, &selector)?;
            (Originator::Explicit(selected.id), selected.name)
        }
        Some(DispatchIdentity::SavedId(id)) => {
            let selected = storage
                .find_active_identity_by_id(&id)
                .map_err(|_| Fault::unavailable())?
                .ok_or_else(|| {
                    Fault::new("NAME_NOT_FOUND", "Saved originator identity was not found.")
                })?;
            if selected.lifetime != tmt_core::identity::Lifetime::Saved {
                return Err(Fault::new(
                    "MCP_SAVED_IDENTITY_REQUIRED",
                    "MCP requires an existing saved identity.",
                ));
            }
            (Originator::Explicit(selected.id), selected.name)
        }
        None => (Originator::Unknown, "anonymous".into()),
    };
    input.originator = originator;
    let settings = settings.expect("dispatch settings");
    let direct = input.kind == tmt_core::request::RequestKind::Request
        && input.recipient_ids.len() == 1
        && !matches!(
            input.room.as_ref(),
            Some(tmt_core::dispatch::DispatchRoom::Roster { .. })
        );
    let preview = direct.then(|| input.message.clone());
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
        let message = crate::delivery::queued_wake(
            &sender,
            preview.as_deref(),
            &item.request_id,
            &item.recipient_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::identity;
    use crate::test_support::TestDirectory;
    use tmt_core::{
        binding::BindingRepository,
        identity::{Lifetime, create_or_resolve},
    };

    #[test]
    fn saved_dispatch_selection_never_falls_back_to_a_retired_uuid_display_name() {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("fixture.db")).unwrap();
        let sender = create_or_resolve(&mut storage, "Sender", Lifetime::Saved)
            .unwrap()
            .identity;
        let recipient = create_or_resolve(&mut storage, "Receiver", Lifetime::Saved)
            .unwrap()
            .identity;
        let temporary = create_or_resolve(&mut storage, "Temporary", Lifetime::Temporary)
            .unwrap()
            .identity;
        storage
            .with_binding_transaction(|rows| rows.retire_identity(&sender, false))
            .unwrap();
        let replacement = create_or_resolve(&mut storage, &sender.id, Lifetime::Saved)
            .unwrap()
            .identity;
        // Ordinary JSON API name/UUID selection keeps its existing fallback.
        assert_eq!(identity(&mut storage, &sender.id).unwrap(), replacement.id);
        let operation = "6ceab579-6d31-45fc-bf61-f2ce90c96a1e";
        let input = DispatchInput {
            originator: Originator::Unknown,
            operation_id: operation.into(),
            recipient_ids: vec![recipient.id],
            message: "work".into(),
            kind: tmt_core::request::RequestKind::Request,
            room: None,
        };
        let error = create_dispatch(
            &mut storage,
            Some(DispatchIdentity::SavedId(sender.id)),
            input.clone(),
            Some(Settings::default()),
        )
        .unwrap_err();
        assert_eq!(error.code, "NAME_NOT_FOUND");
        let error = create_dispatch(
            &mut storage,
            Some(DispatchIdentity::SavedId(temporary.id)),
            input,
            Some(Settings::default()),
        )
        .unwrap_err();
        assert_eq!(error.code, "MCP_SAVED_IDENTITY_REQUIRED");
        assert!(storage.dispatch_receipt(operation).unwrap().is_none());
        storage.close().unwrap();
    }
}
