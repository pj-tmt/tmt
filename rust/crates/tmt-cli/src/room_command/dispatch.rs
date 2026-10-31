//! Queue a frozen room audience through the same composition owner as HTTP.

use crate::{
    identity_context,
    invocation::{OutputMode, RoomOperation},
    output::{Failure, after_cleanup},
};
use std::io::{self, Write};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    dispatch::encode_receipt,
    host::CallerEnvironment,
    request_runtime::wall_time_ms,
    storage::{DispatchError, Storage},
};
use tmt_core::{
    dispatch::{Acceptance, DispatchInput, DispatchReceipt, DispatchRoom, canonical_id},
    operation::new_operation_id,
    request::Originator,
};

fn failure(error: DispatchError, operation_id: &str) -> Failure {
    let message = match &error {
        DispatchError::Invalid => "Invalid message, operation ID or room audience.",
        DispatchError::IdempotencyConflict => {
            "This operation ID already belongs to a different sender, message or room audience."
        }
        DispatchError::RoomRosterChanged => {
            "Room membership changed before enqueue. Refresh the room before retrying."
        }
        DispatchError::RoomRecipientNotMember => {
            "The target is no longer a member of the selected room."
        }
        DispatchError::Request(_) | DispatchError::Storage(_) => {
            "Dispatch could not be confirmed. Do not create a new operation merely to retry."
        }
    };
    Failure::new(error.code(), message, 1)
        .suggestion(format!(
            "Retain operation ID {operation_id}; retry only the same composition."
        ))
        .caused_by(error)
}

fn run(operation: RoomOperation) -> Result<DispatchReceipt, Failure> {
    run_with(
        operation,
        || ConfigPaths::discover().map_err(Failure::from),
        || crate::caller_context::host(&CallerEnvironment::current()),
    )
}

fn run_with(
    operation: RoomOperation,
    paths: impl FnOnce() -> Result<ConfigPaths, Failure>,
    host: impl FnOnce() -> Result<tmt_adapters::host::Host, Failure>,
) -> Result<DispatchReceipt, Failure> {
    let RoomOperation::Dispatch {
        room,
        message,
        identity,
        operation_id,
        kind,
    } = operation
    else {
        unreachable!("room dispatch")
    };
    let operation_id = operation_id.unwrap_or_else(new_operation_id);
    if !canonical_id(&operation_id) {
        return Err(Failure::new(
            "DISPATCH_INVALID",
            "Operation ID must be a non-nil UUID.",
            1,
        ));
    }
    let paths = paths()?;
    let settings = ConfigFiles {
        paths: paths.clone(),
    }
    .load()
    .map_err(Failure::from)?
    .settings;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "Nothing was queued.",
            "STORAGE_UNAVAILABLE",
            "Dispatch could not be confirmed. Do not create a new operation merely to retry.",
        )
        .suggestion(format!(
            "Retain operation ID {operation_id}; retry only the same composition."
        ))
    })?;
    let pending = (|| {
        let room = super::resolve(&mut storage, &room)?;
        if room.member_ids.is_empty() {
            return Err(Failure::new(
                "ROOM_EMPTY",
                "Room has no active members; nothing was queued.",
                1,
            ));
        }
        let originator = identity_context::optional(&mut storage, host, identity.as_deref())?
            .map_or(Originator::Unknown, |sender| {
                if identity.is_some() {
                    Originator::Explicit(sender.id)
                } else {
                    Originator::Verified(sender.id)
                }
            });
        storage
            .dispatch_request(
                DispatchInput {
                    originator,
                    kind,
                    origin: tmt_core::request::RequestOrigin::Person,
                    operation_id: operation_id.clone(),
                    recipient_ids: room.member_ids,
                    message,
                    room: Some(DispatchRoom::Roster {
                        room_id: room.id,
                        revision: room.revision,
                    }),
                    mode: tmt_core::request::digest::DeliveryMode::Request,
                },
                settings.retention_days,
                wall_time_ms,
            )
            .map_err(|error| failure(error, &operation_id))
    })();
    after_cleanup(pending, || storage.close()).map_err(|error| {
        // A close failure can follow a committed enqueue. Keep the operation
        // recoverable even when no receipt reached stdout.
        error.suggestion(format!(
            "Retain operation ID {operation_id}; retry only the same composition."
        ))
    })
}

pub(super) fn execute(operation: RoomOperation, mode: OutputMode) -> io::Result<u8> {
    let receipt = match run(operation) {
        Ok(receipt) => receipt,
        Err(error) => return error.publish(mode),
    };
    let mut out = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        out.write_all(&encode_receipt(&receipt))?;
        writeln!(out)?;
    } else {
        use tmt_cli_style::{
            Token,
            list::{self, Section},
            table::{Cell, Column, Table},
            value,
        };
        let terminal = out.terminal();
        let queued = receipt
            .items
            .iter()
            .filter(|item| matches!(item.acceptance, Acceptance::Queued))
            .count();
        tmt_cli_style::message::success(
            &mut out,
            terminal,
            &format!(
                "Queued for {queued} of {} recipients (operation {})",
                receipt.items.len(),
                value::short_id(&receipt.operation_id)
            ),
        )?;
        // Request ids stay whole: `tmt result` takes them.
        let mut rows = Table::new(&[Column::Name, Column::Fixed, Column::Fixed]);
        for item in &receipt.items {
            let acceptance = match item.acceptance {
                Acceptance::Queued => Cell::from("queued"),
                Acceptance::RecipientUnavailable => Cell::styled("unavailable", Token::Warn),
                Acceptance::RouteRestricted => Cell::styled("not sent · route", Token::Warn),
                Acceptance::HeldForPerson => Cell::styled("held · needs a person", Token::Warn),
            };
            rows.row([
                Cell::from(value::short_id(&item.recipient_id)),
                acceptance,
                Cell::from(&item.request_id),
            ]);
        }
        list::write(
            &mut out,
            terminal,
            &[Section {
                title: "recipients",
                count: Some(receipt.items.len()),
                rows,
                note: None,
                hint: Some("queued is not completed; inspect each with tmt result <request-id>"),
            }],
        )?;
    }
    Ok(0)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "queued is not completed; inspect each with tmt result <request-id>",
        &[""],
        &[],
    )];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caller_context::test_support::{World, cleanup_failure};
    use tmt_core::{
        identity::{Lifetime, create_or_resolve},
        request::RequestKind,
        room::{self, MembershipChange, RoomRepository},
    };

    #[test]
    fn explicit_room_dispatch_bypasses_failed_caller_selection_but_implicit_dispatch_does_not() {
        for explicit in [true, false] {
            let world = World::new();
            let mut storage = world.storage();
            let sender = create_or_resolve(&mut storage, "Sender", Lifetime::Saved)
                .unwrap()
                .identity;
            let target = create_or_resolve(&mut storage, "Recipient", Lifetime::Saved)
                .unwrap()
                .identity;
            let room = room::create(&mut storage, "Team".into()).unwrap();
            storage
                .change_meeting_membership(&room.id, &target.id, MembershipChange::Join)
                .unwrap();
            storage.close().unwrap();
            let operation_id = tmt_core::operation::new_operation_id();
            let result = run_with(
                RoomOperation::Dispatch {
                    room: room.id,
                    message: "stored audience".into(),
                    identity: explicit.then(|| "Sender".into()),
                    operation_id: Some(operation_id.clone()),
                    kind: RequestKind::Request,
                },
                || Ok(world.paths.clone()),
                || Err(cleanup_failure()),
            );
            if explicit {
                let receipt = result.unwrap_or_else(|error| panic!("{}", error.message));
                assert_eq!(receipt.items.len(), 1);
                assert_eq!(receipt.items[0].recipient_id, target.id);
                let mut storage = world.storage();
                let detail = tmt_core::request::RequestService::new(&mut storage, wall_time_ms)
                    .request_detail(&receipt.items[0].request_id)
                    .unwrap();
                assert_eq!(detail.item.originator, Originator::Explicit(sender.id));
                assert_eq!(
                    detail.item.recipient_identity_id.as_deref(),
                    Some(target.id.as_str())
                );
                storage.close().unwrap();
            } else {
                let error = result.expect_err("implicit caller selection fails");
                let expected = cleanup_failure().suggestion(format!(
                    "Retain operation ID {operation_id}; retry only the same composition."
                ));
                assert_eq!(error.document(), expected.document());
                assert_eq!(error.status, 1);
            }
        }
    }
}
