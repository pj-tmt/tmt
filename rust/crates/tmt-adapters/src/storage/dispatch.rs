//! One operation receipt and canonical inbox writes share the caller transaction.

use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::{
    dispatch::{
        Acceptance, DispatchInput, DispatchItem, DispatchReceipt, DispatchRoom, canonical_id,
    },
    request::{PrepareRequest, RequestError, RequestRoute, RequestService, digest::DeliveryPolicy},
    retention::{REQUEST_MIN_EXPIRY_MS, checked_deadline, valid_retention_days},
};

use super::{
    Storage, StorageError, StorageErrorCode, errors::classify,
    identities::with_immediate_transaction, requests::TransactionRequests,
};
use crate::{
    dispatch::{decode_receipt, encode_receipt, intent_digest},
    request_runtime::request_ids,
};

#[derive(Debug)]
pub enum DispatchError {
    Invalid,
    IdempotencyConflict,
    RoomRosterChanged,
    RoomRecipientNotMember,
    Request(RequestError<StorageError>),
    Storage(StorageError),
}

impl DispatchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "DISPATCH_INVALID",
            Self::IdempotencyConflict => "DISPATCH_IDEMPOTENCY_CONFLICT",
            Self::RoomRosterChanged => "ROOM_ROSTER_CHANGED",
            Self::RoomRecipientNotMember => "ROOM_RECIPIENT_NOT_MEMBER",
            Self::Request(_) | Self::Storage(_) => "STORAGE_UNAVAILABLE",
        }
    }
}
impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for DispatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Request(e) => Some(e),
            Self::Storage(e) => Some(e),
            _ => None,
        }
    }
}
impl From<StorageError> for DispatchError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}
impl From<RequestError<StorageError>> for DispatchError {
    fn from(value: RequestError<StorageError>) -> Self {
        match value {
            RequestError::RoomRecipientNotMember => Self::RoomRecipientNotMember,
            other => Self::Request(other),
        }
    }
}

impl Storage {
    /// Recover an accepted operation without resubmitting or consulting a changed roster.
    pub fn dispatch_receipt(
        &self,
        operation_id: &str,
    ) -> Result<Option<DispatchReceipt>, DispatchError> {
        if !canonical_id(operation_id) {
            return Err(DispatchError::Invalid);
        }
        Ok(read_dispatch_operation(self.connection()?, operation_id)?.map(|(_, receipt)| receipt))
    }

    pub fn dispatch_request(
        &mut self,
        input: DispatchInput,
        retention_days: u64,
        clock: impl Fn() -> u64,
    ) -> Result<DispatchReceipt, DispatchError> {
        self.dispatch_request_with_creation(input, retention_days, clock)
            .map(|(receipt, _)| receipt)
    }

    /// Whether this call made the immutable receipt. Replays must not create a
    /// new side effect after a prior response or an expired inbox attempt.
    pub fn dispatch_request_with_creation(
        &mut self,
        input: DispatchInput,
        retention_days: u64,
        clock: impl Fn() -> u64,
    ) -> Result<(DispatchReceipt, bool), DispatchError> {
        let input = input.normalize().ok_or(DispatchError::Invalid)?;
        if !valid_retention_days(retention_days) {
            return Err(DispatchError::Invalid);
        }
        let digest = intent_digest(&input);
        let calendar = super::orchestration::ReceivingCalendar::load(self.connection()?)?;
        with_immediate_transaction(self, "Request dispatch", |transaction| {
            if let Some((stored_digest, receipt)) =
                read_dispatch_operation(transaction, &input.operation_id)?
            {
                if stored_digest != digest {
                    return Err(DispatchError::IdempotencyConflict);
                }
                if !receipt
                    .items
                    .iter()
                    .map(|item| &item.recipient_id)
                    .eq(input.recipient_ids.iter())
                {
                    return Err(StorageError::new(
                        StorageErrorCode::Corrupt,
                        "Invalid dispatch receipt audience",
                    )
                    .into());
                }
                return Ok((receipt, false));
            }
            if let Some(DispatchRoom::Roster { room_id, revision }) = &input.room {
                let room = super::room::read_room(transaction, room_id)?;
                if !room.is_some_and(|room| {
                    room.revision == *revision && room.member_ids == input.recipient_ids
                }) {
                    return Err(DispatchError::RoomRosterChanged);
                }
            }
            let now = clock();
            let expires_at_ms =
                checked_deadline(now, REQUEST_MIN_EXPIRY_MS).ok_or(DispatchError::Invalid)?;
            let mut repository = TransactionRequests(transaction, &calendar);
            let mut service = RequestService::new(&mut repository, || now);
            let mut items = Vec::with_capacity(input.recipient_ids.len());
            for recipient_id in &input.recipient_ids {
                let (request_id, attempt_id) = request_ids();
                let result = service.enqueue_delivery(
                    PrepareRequest {
                        room_id: input.room.as_ref().map(|room| room.room_id().to_owned()),
                        kind: input.kind,
                        origin: input.origin,
                        request_id: request_id.clone(),
                        message: input.message.clone(),
                        route: RequestRoute::Inbox {
                            recipient_identity_id: recipient_id.clone(),
                        },
                        wait: false,
                        expires_at_ms,
                        originator: input.originator.clone(),
                        recipient_identity_id: Some(recipient_id.clone()),
                        preamble: None,
                    },
                    attempt_id,
                    retention_days,
                    DeliveryPolicy {
                        mode: input.mode,
                        ..DeliveryPolicy::default()
                    },
                );
                let acceptance = match result {
                    Ok(prepared) if prepared.held_for_person => Acceptance::HeldForPerson,
                    Ok(_) => Acceptance::Queued,
                    Err(RequestError::NotFound) => Acceptance::RecipientUnavailable,
                    // Nothing was stored for this recipient; the operation goes on and the
                    // refusal is journaled in the same transaction (#4892).
                    Err(RequestError::RouteRestricted(refusal)) => {
                        super::orchestration::append_route_refusal(transaction, &refusal, now)?;
                        Acceptance::RouteRestricted
                    }
                    Err(error) => return Err(error.into()),
                };
                items.push(DispatchItem {
                    recipient_id: recipient_id.clone(),
                    request_id,
                    acceptance,
                });
            }
            let receipt = DispatchReceipt {
                operation_id: input.operation_id,
                created_at_ms: now,
                items,
            };
            let encoded = String::from_utf8(encode_receipt(&receipt)).expect("JSON is UTF-8");
            transaction.execute(
                "INSERT INTO office_dispatch_operations (operation_id,intent_digest,receipt) VALUES (?,?,?)",
                params![receipt.operation_id,digest,encoded],
            ).map_err(|error| classify(error, "Store dispatch receipt"))?;
            Ok((receipt, true))
        })
    }
}

fn read_dispatch_operation(
    connection: &Connection,
    operation_id: &str,
) -> Result<Option<(String, DispatchReceipt)>, StorageError> {
    let stored = connection
        .query_row(
            "SELECT intent_digest, receipt FROM office_dispatch_operations WHERE operation_id=?",
            [operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| classify(error, "Read dispatch operation receipt"))?;
    stored
        .map(|(digest, encoded)| {
            let receipt = decode_receipt(encoded.as_bytes())
                .filter(|receipt| receipt.operation_id == operation_id)
                .ok_or_else(|| {
                    StorageError::new(
                        StorageErrorCode::Corrupt,
                        "Invalid dispatch operation receipt",
                    )
                })?;
            Ok((digest, receipt))
        })
        .transpose()
}

#[cfg(test)]
mod tests;
