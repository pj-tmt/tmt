//! Dispatch receipt recovery and the existing one-shot wake composition.

use super::{DispatchIdentity, Fault, identity_entry, invalid};
use crate::{dispatch, request_runtime::wall_time_ms, storage::Storage};
use tmt_core::{
    dispatch::DispatchInput,
    request::{Originator, digest::DeliveryMode},
    settings::Settings,
};

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
    let passthrough = input.mode == DeliveryMode::Passthrough;
    if passthrough
        && storage
            .dispatch_receipt(&input.operation_id)
            .map_err(|_| Fault::unavailable())?
            .is_none()
    {
        gate_passthrough(storage, &input)?;
    }
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
        let delay =
            std::time::Duration::from_secs_f64(settings.paste_enter_delay_ms.min(500.0) / 1000.0);
        let text = preview.as_deref().expect("a direct request keeps its text");
        Some(if passthrough {
            // The text is the delivery, typed as written: there is no wake line.
            crate::delivery::deliver_request(
                storage,
                &item.request_id,
                &item.recipient_id,
                text,
                delay,
            )
        } else {
            let message = crate::delivery::queued_wake(
                &sender,
                Some(text),
                &item.request_id,
                &item.recipient_id,
            );
            crate::delivery::wake_request(
                storage,
                &item.request_id,
                &item.recipient_id,
                &message,
                delay,
            )
        })
    } else {
        None
    };
    Ok(dispatch::encode_receipt_with_wake(&receipt, wake))
}

/// The rules of `tmt_adapters::passthrough`, before anything is stored.
fn gate_passthrough(storage: &mut Storage, input: &DispatchInput) -> Result<(), Fault> {
    use crate::passthrough::{Can, Reason, can_identity, gate};
    let person = match input.originator.identity_id() {
        Some(id) => tmt_core::request::RequestService::new(&mut *storage, wall_time_ms)
            .identity_is_user(id)
            .map_err(|_| Fault::unavailable())?,
        None => false,
    };
    let recipient = &input.recipient_ids[0];
    let can = can_identity(storage, recipient)
        .map_err(|_| Fault::unavailable())?
        .unwrap_or_else(|| Can::no(Reason::NoPane));
    gate(person, can).map_err(|refusal| {
        let target = storage
            .find_identity_by_id(recipient)
            .ok()
            .flatten()
            .map_or_else(|| recipient.clone(), |identity| identity.name);
        Fault::detailed(refusal.code(), refusal.message(&target))
    })
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

    fn passthrough_input(
        operation: &str,
        sender: &str,
        recipient: &str,
        mode: tmt_core::request::digest::DeliveryMode,
    ) -> DispatchInput {
        DispatchInput {
            origin: tmt_core::request::RequestOrigin::Person,
            originator: Originator::Unknown,
            operation_id: operation.into(),
            recipient_ids: vec![recipient.into()],
            message: format!("typed by {sender}"),
            kind: tmt_core::request::RequestKind::Request,
            room: None,
            mode,
        }
    }

    #[test]
    fn a_passthrough_dispatch_is_refused_before_anything_is_stored() {
        use tmt_core::request::digest::DeliveryMode;
        let directory = TestDirectory::new();
        let database = directory.path.join("fixture.db");
        let mut storage = Storage::open(&database).unwrap();
        let sender = create_or_resolve(&mut storage, "Sender", Lifetime::Saved)
            .unwrap()
            .identity;
        let recipient = create_or_resolve(&mut storage, "Receiver", Lifetime::Saved)
            .unwrap()
            .identity;
        let run = |storage: &mut Storage, operation: &str, mode| {
            create_dispatch(
                storage,
                Some(DispatchIdentity::Explicit(sender.id.clone())),
                passthrough_input(operation, &sender.name, &recipient.id, mode),
                Some(Settings::default()),
            )
        };
        let first = "6ceab579-6d31-45fc-bf61-f2ce90c96a01";
        let second = "6ceab579-6d31-45fc-bf61-f2ce90c96a02";
        let third = "6ceab579-6d31-45fc-bf61-f2ce90c96a03";

        // Not a person: refused, and no receipt exists to replay.
        let error = run(&mut storage, first, DeliveryMode::Passthrough).unwrap_err();
        assert_eq!(error.code, "PASSTHROUGH_NOT_ALLOWED");
        assert!(storage.dispatch_receipt(first).unwrap().is_none());

        // A person, but the recipient has no pane: the same code the CLI uses.
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute(
                "INSERT INTO identity_metadata(identity_id, key, value) VALUES(?1, 'user', 'true')",
                [&sender.id],
            )
            .unwrap();
        let error = run(&mut storage, second, DeliveryMode::Passthrough).unwrap_err();
        assert_eq!(error.code, "PASSTHROUGH_UNAVAILABLE");
        assert!(error.message.contains("no live pane"), "{}", error.message);
        assert!(storage.dispatch_receipt(second).unwrap().is_none());

        // Request mode is unchanged: it still queues for an identity without a pane.
        let receipt = run(&mut storage, third, DeliveryMode::Request).unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
        assert_eq!(receipt["items"][0]["acceptance"], "queued");
        storage.close().unwrap();
    }

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
            origin: tmt_core::request::RequestOrigin::Person,
            originator: Originator::Unknown,
            operation_id: operation.into(),
            recipient_ids: vec![recipient.id],
            message: "work".into(),
            kind: tmt_core::request::RequestKind::Request,
            room: None,
            mode: tmt_core::request::digest::DeliveryMode::Request,
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
