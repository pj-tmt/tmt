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
    host::{CallerEnvironment, Host},
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
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let settings = ConfigFiles {
        paths: paths.clone(),
    }
    .load()
    .map_err(Failure::from)?
    .settings;
    let mut storage =
        Storage::open(paths.database).map_err(|error| failure(error.into(), &operation_id))?;
    let pending = (|| {
        let room = super::resolve(&mut storage, &room)?;
        if room.member_ids.is_empty() {
            return Err(Failure::new(
                "ROOM_EMPTY",
                "Room has no active members; nothing was queued.",
                1,
            ));
        }
        let originator = identity_context::optional(
            &mut storage,
            &Host::for_caller(&CallerEnvironment::current()),
            identity.as_deref(),
        )?
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
                    operation_id: operation_id.clone(),
                    recipient_ids: room.member_ids,
                    message,
                    room: Some(DispatchRoom::Roster {
                        room_id: room.id,
                        revision: room.revision,
                    }),
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
