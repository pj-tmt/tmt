//! Admitted observational operations. Original IDs are principal-scoped; status
//! never adopts or repairs. Reads interpret only raw backend metadata and bytes.
use super::{
    dispatch::{self, ChannelEnd, Hub},
    original::original_id,
};
use crate::objects::{
    BackendError, BlobKey, IntentId, IoBudget, NamespaceId, OpaqueKey, TransferState,
};
use std::time::Instant;
use tmt_extension_objects::{
    AdmitInput, Bus, Bytes32, Call, Checkpoint, Chunk, Context, Decision, Disclosure, ErrorCode,
    Operation, Outcome, Request, Sha256Hex, State, Success,
};

fn refusal(decision: Decision) -> Option<ErrorCode> {
    match decision {
        Decision::Allow => None,
        Decision::Deny => Some(ErrorCode::Denied),
        Decision::Unavailable => Some(ErrorCode::Unavailable),
    }
}
fn backend_error(error: BackendError) -> ErrorCode {
    match error {
        BackendError::Invalid => ErrorCode::Invalid,
        BackendError::Conflict => ErrorCode::Conflict,
        BackendError::Missing => ErrorCode::NotFound,
        BackendError::Capacity(_) => ErrorCode::Unavailable,
        BackendError::Unavailable | BackendError::Cancelled | BackendError::Deadline => {
            ErrorCode::Unavailable
        }
    }
}

/// Every controlled boundary uses the same captured context. A current but
/// altered binding is not permission to reclassify or disclose this request.
fn fence(
    hub: &Hub,
    request: &Request,
    context: Context,
    io: &IoBudget<'_>,
) -> Result<(), ErrorCode> {
    io.check().map_err(backend_error)?;
    if dispatch::stands(hub, request.origin) != Some(context) {
        return Err(ErrorCode::Denied);
    }
    Ok(())
}

pub(super) fn serve(
    hub: &Hub,
    bus: &Bus,
    request: &Request,
    deadline: Instant,
) -> Result<(), ChannelEnd> {
    let io = IoBudget {
        deadline,
        cancelled: &hub.stop,
    };
    let reply = |outcome| dispatch::send_until(hub, bus, request, outcome, deadline);
    let fail = |code| reply(Outcome::Failure(code));
    let Some(context) = dispatch::stands(hub, request.origin) else {
        return fail(ErrorCode::Denied);
    };
    let input = match &request.call {
        Call::Status(input) => AdmitInput::Status(input.clone()),
        Call::Read(input) => AdmitInput::Read(input.clone()),
        _ => return fail(ErrorCode::Invalid),
    };
    if let Err(code) = fence(hub, request, context, &io) {
        return fail(code);
    }
    let operation = |disclosure| Operation {
        input: input.clone(),
        disclosure,
    };
    let Some(acquire) = dispatch::ask(
        hub,
        bus,
        request,
        context,
        Checkpoint::Acquire,
        operation(None),
        deadline,
    )?
    else {
        return fail(ErrorCode::Unavailable);
    };
    if let Some(code) = refusal(acquire) {
        return fail(code);
    }
    if let Err(code) = fence(hub, request, context, &io) {
        return fail(code);
    }
    let observed = observe(hub, request, context, &io);
    dispatch::between(hub, deadline);
    if let Err(code) = fence(hub, request, context, &io) {
        return fail(code);
    }
    let (success, disclosure, proof) = match observed {
        Ok(value) => value,
        Err(code) => return fail(code),
    };
    let Some(disclose) = dispatch::ask(
        hub,
        bus,
        request,
        context,
        Checkpoint::Disclose,
        operation(Some(disclosure)),
        deadline,
    )?
    else {
        return fail(ErrorCode::Unavailable);
    };
    if let Some(code) = refusal(disclose) {
        return fail(code);
    }
    dispatch::before_result(hub, deadline);
    if let Err(code) = proof.current(hub, &io) {
        return fail(code);
    }
    if let Err(code) = fence(hub, request, context, &io) {
        return fail(code);
    }
    reply(Outcome::Success(success))
}

/// Recheck the exact observed metadata after disclosure admission. Namespace
/// removal or an original's transition/expiry cannot disclose a stale snapshot.
enum ObservationProof {
    Original(IntentId, crate::objects::Transfer),
    Payload(BlobKey, crate::objects::Receipt),
}
impl ObservationProof {
    fn current(&self, hub: &Hub, io: &IoBudget<'_>) -> Result<(), ErrorCode> {
        let same = match self {
            Self::Original(intent, observed) => {
                hub.backend.status(*intent, io).map_err(backend_error)? == *observed
            }
            Self::Payload(key, observed) => {
                hub.backend.stat(*key, io).map_err(backend_error)? == *observed
            }
        };
        if same {
            Ok(())
        } else {
            Err(ErrorCode::Unavailable)
        }
    }
}

fn observe(
    hub: &Hub,
    request: &Request,
    context: Context,
    io: &IoBudget<'_>,
) -> Result<(Success, Disclosure, ObservationProof), ErrorCode> {
    match &request.call {
        Call::Status(input) => {
            let intent = original_id(hub.backend.extension(), context, input.transfer_id);
            let transfer = hub.backend.status(intent, io).map_err(backend_error)?;
            if let Some(original) = &transfer.original
                && (original.spec.intent != intent
                    || original.spec.key.namespace.0 != *input.namespace.as_bytes()
                    || original.spec.binding != input.policy.as_bytes())
            {
                return Err(ErrorCode::Denied);
            }
            let proof = ObservationProof::Original(intent, transfer.clone());
            let (success, disclosure) = match transfer.state {
                TransferState::Pending(progress) => {
                    let original = transfer.original.ok_or(ErrorCode::Unavailable)?;
                    let expires_at_ms = Some(original.expires_at_ms);
                    Ok((
                        Success::Pending {
                            next_index: progress.next_index,
                            received: progress.received,
                            expires_at_ms,
                        },
                        Disclosure::Status {
                            next_index: progress.next_index,
                            received: progress.received,
                            expires_at_ms,
                        },
                    ))
                }
                TransferState::Committed(receipt) => {
                    let opaque_key = Bytes32::from_bytes(receipt.key.object.0);
                    let payload_sha256 = Sha256Hex::from_bytes(receipt.payload_sha256);
                    let payload_bytes = receipt.payload_bytes;
                    Ok((
                        Success::Committed {
                            opaque_key,
                            payload_sha256,
                            payload_bytes,
                        },
                        Disclosure::Receipt {
                            opaque_key,
                            payload_sha256,
                            payload_bytes,
                        },
                    ))
                }
                state => {
                    let state = match state {
                        TransferState::Expired => State::Expired,
                        TransferState::Discarded => State::Discarded,
                        TransferState::Unavailable => State::Unavailable,
                        TransferState::Unknown => State::Unknown,
                        TransferState::NotObserved => State::NotObserved,
                        _ => return Err(ErrorCode::Unavailable),
                    };
                    Ok((Success::State(state), Disclosure::Terminal { state }))
                }
            }?;
            Ok((success, disclosure, proof))
        }
        Call::Read(input) => {
            if input.count == 0
                || input.count > 32_768
                || input.offset > input.payload_bytes
                || (input.offset == input.payload_bytes && input.payload_bytes != 0)
            {
                return Err(ErrorCode::Invalid);
            }
            let key = BlobKey {
                namespace: NamespaceId(*input.namespace.as_bytes()),
                object: OpaqueKey(*input.opaque_key.as_bytes()),
            };
            let receipt = hub.backend.stat(key, io).map_err(backend_error)?;
            if receipt.key != key
                || receipt.payload_sha256 != *input.payload_sha256.as_bytes()
                || receipt.payload_bytes != input.payload_bytes
            {
                return Err(ErrorCode::Denied);
            }
            // Stat and read each return through the backend budget check; authority
            // is also fresh between the two actual backend calls.
            fence(hub, request, context, io)?;
            let part = hub
                .backend
                .read(key, input.offset, input.count, io)
                .map_err(backend_error)?;
            let expected = u64::from(input.count).min(input.payload_bytes - input.offset);
            if part.offset != input.offset
                || part.total_bytes != input.payload_bytes
                || part.bytes.len() as u64 != expected
            {
                return Err(ErrorCode::Unavailable);
            }
            let length = u32::try_from(part.bytes.len()).map_err(|_| ErrorCode::Unavailable)?;
            let bytes = Chunk::new(part.bytes).map_err(|_| ErrorCode::Unavailable)?;
            Ok((
                Success::Read {
                    offset: part.offset,
                    total_bytes: part.total_bytes,
                    bytes,
                },
                Disclosure::Bytes {
                    offset: part.offset,
                    length,
                },
                ObservationProof::Payload(key, receipt),
            ))
        }
        _ => Err(ErrorCode::Invalid),
    }
}
