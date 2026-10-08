//! Admitted writes through the one backend algorithm. An invoked mutation may
//! have taken effect even when its result cannot be disclosed; nothing retries,
//! rolls back, releases charge or reconciles in response to that uncertainty.
use super::{
    dispatch::{self, ChannelEnd, Hub},
    original::original_id,
};
use crate::objects::{
    BackendError, BeginResult, BeginSpec, BlobKey, IntentId, IoBudget, Limit, NamespaceId,
    OpaqueKey, Receipt, Transfer, TransferState,
};
use std::time::{Duration, Instant};
use tmt_extension_objects::{
    AdmitInput, Bus, Bytes32, Call, Checkpoint, Context, Decision, Disclosure, ErrorCode,
    Operation, Outcome, PartAdmit, Policy, Request, Retained, Sha256Hex, State, Success,
    TransferAdmit,
};

/// Errors from an invoked mutating call, not an assertion that storage stayed
/// unchanged. Missing is conservatively unknown: not-found is read-only on wire.
fn mutation_error(error: BackendError) -> ErrorCode {
    match error {
        BackendError::Invalid => ErrorCode::Invalid,
        BackendError::Conflict => ErrorCode::Conflict,
        BackendError::Capacity(limit) => ErrorCode::Capacity(match limit {
            Limit::NamespaceBytes => tmt_extension_objects::Limit::NamespaceBytes,
            Limit::ExtensionBytes => tmt_extension_objects::Limit::ExtensionBytes,
            Limit::InstallationBytes => tmt_extension_objects::Limit::InstallationBytes,
            Limit::NamespaceEntries => tmt_extension_objects::Limit::NamespaceEntries,
            Limit::ExtensionEntries => tmt_extension_objects::Limit::ExtensionEntries,
            Limit::InstallationEntries => tmt_extension_objects::Limit::InstallationEntries,
            Limit::ActiveIntents => tmt_extension_objects::Limit::ActiveIntents,
            Limit::RetainedExtension => tmt_extension_objects::Limit::RetainedExtension,
            Limit::RetainedInstallation => tmt_extension_objects::Limit::RetainedInstallation,
        }),
        BackendError::Missing
        | BackendError::Unavailable
        | BackendError::Cancelled
        | BackendError::Deadline => ErrorCode::Unknown,
    }
}

struct UploadOriginal {
    intent: IntentId,
    input: AdmitInput,
    spec: BeginSpec,
    /// Only incomplete staging has a remaining staging budget. The persisted
    /// deadline is never renewed; committed receipts remain observable later.
    expires_at_ms: Option<u64>,
}
impl UploadOriginal {
    fn lookup(
        hub: &Hub,
        request: &Request,
        context: Context,
        io: &IoBudget<'_>,
    ) -> Result<Self, ErrorCode> {
        let transfer_id = match &request.call {
            Call::Begin(x) => x.transfer_id,
            Call::Part(x) => x.transfer_id,
            Call::Commit(x) | Call::Discard(x) => x.transfer_id,
            _ => return Err(ErrorCode::Invalid),
        };
        let intent = original_id(hub.backend.extension(), context, transfer_id);
        let known = hub
            .backend
            .status(intent, io)
            .map_err(|_| ErrorCode::Unavailable)?;
        let expires_at_ms = matches!(known.state, TransferState::Pending(_))
            .then(|| known.original.as_ref().map(|x| x.expires_at_ms))
            .flatten();
        let (spec, input) = if let Call::Begin(x) = &request.call {
            let spec = BeginSpec {
                intent,
                key: BlobKey {
                    namespace: NamespaceId(*x.namespace.as_bytes()),
                    object: OpaqueKey(*x.opaque_key.as_bytes()),
                },
                payload_sha256: *x.payload_sha256.as_bytes(),
                payload_bytes: x.payload_bytes,
                binding: x.policy.as_bytes().to_vec(),
            };
            if known
                .original
                .as_ref()
                .is_some_and(|original| original.spec != spec)
            {
                return Err(ErrorCode::Conflict);
            }
            (spec, AdmitInput::Begin(x.clone()))
        } else {
            let spec = known.original.ok_or(ErrorCode::Unavailable)?.spec;
            if spec.intent != intent {
                return Err(ErrorCode::Denied);
            }
            let retained = Retained {
                namespace: Bytes32::from_bytes(spec.key.namespace.0),
                opaque_key: Bytes32::from_bytes(spec.key.object.0),
                policy: Policy::new(spec.binding.clone()).map_err(|_| ErrorCode::Unavailable)?,
                payload_sha256: Sha256Hex::from_bytes(spec.payload_sha256),
                payload_bytes: spec.payload_bytes,
            };
            let input = match &request.call {
                Call::Part(x) => AdmitInput::Part(PartAdmit {
                    transfer_id,
                    index: x.index,
                    length: u32::try_from(x.bytes.as_bytes().len())
                        .map_err(|_| ErrorCode::Invalid)?,
                    retained,
                }),
                Call::Commit(_) => AdmitInput::Commit(TransferAdmit {
                    transfer_id,
                    retained,
                }),
                Call::Discard(_) => AdmitInput::Discard(TransferAdmit {
                    transfer_id,
                    retained,
                }),
                _ => return Err(ErrorCode::Invalid),
            };
            (spec, input)
        };
        Ok(Self {
            intent,
            input,
            spec,
            expires_at_ms,
        })
    }
    fn deadline(&self, hub: &Hub, request: Instant) -> Result<Instant, ErrorCode> {
        let Some(expiry) = self.expires_at_ms else {
            return Ok(request);
        };
        let left = expiry
            .checked_sub(hub.writer.now_ms())
            .filter(|left| *left > 0)
            .ok_or(ErrorCode::Unavailable)?;
        Ok(Instant::now()
            .checked_add(Duration::from_millis(left))
            .map_or(request, |end| request.min(end)))
    }
    fn fence(
        &self,
        hub: &Hub,
        request: &Request,
        context: Context,
        io: &IoBudget<'_>,
        pending: bool,
    ) -> Result<(), ErrorCode> {
        io.check().map_err(|_| ErrorCode::Unavailable)?;
        if dispatch::stands(hub, request.origin) != Some(context) {
            return Err(ErrorCode::Denied);
        }
        if self
            .expires_at_ms
            .is_some_and(|expiry| hub.writer.now_ms() >= expiry)
        {
            return Err(ErrorCode::Unavailable);
        }
        // The original is immutable, but it can be removed or expire while a
        // callback runs. Never turn a stale scoped lookup into a fresh write.
        let current = hub
            .backend
            .status(self.intent, io)
            .map_err(|_| ErrorCode::Unavailable)?;
        if current
            .original
            .as_ref()
            .is_some_and(|x| x.spec != self.spec)
        {
            return Err(ErrorCode::Denied);
        }
        if pending
            && self.expires_at_ms.is_some()
            && !matches!(current.state, TransferState::Pending(_))
        {
            return Err(ErrorCode::Unavailable);
        }
        Ok(())
    }
    fn perform(
        &self,
        hub: &Hub,
        call: &Call,
        io: &IoBudget<'_>,
    ) -> Result<(Success, Disclosure), BackendError> {
        match call {
            Call::Begin(_) => match hub.writer.begin(&self.spec, io)? {
                BeginResult::Pending(x) => Ok((
                    Success::Pending {
                        next_index: x.next_index,
                        received: x.received,
                        expires_at_ms: None,
                    },
                    Disclosure::Status {
                        next_index: x.next_index,
                        received: x.received,
                        expires_at_ms: None,
                    },
                )),
                BeginResult::Committed(x) => Ok(receipt(x)),
                BeginResult::Terminal(x) => {
                    let state = match x {
                        TransferState::Expired => State::Expired,
                        TransferState::Discarded => State::Discarded,
                        TransferState::Unavailable => State::Unavailable,
                        TransferState::Unknown => State::Unknown,
                        TransferState::NotObserved => State::NotObserved,
                        _ => return Err(BackendError::Unavailable),
                    };
                    Ok((Success::State(state), Disclosure::Terminal { state }))
                }
            },
            Call::Part(x) => {
                let progress = hub
                    .writer
                    .append(self.intent, x.index, x.bytes.as_bytes(), io)?;
                Ok((
                    Success::Progress {
                        next_index: progress.next_index,
                        received: progress.received,
                    },
                    Disclosure::Progress {
                        next_index: progress.next_index,
                        received: progress.received,
                    },
                ))
            }
            Call::Commit(_) => Ok(receipt(hub.writer.commit(self.intent, io)?)),
            Call::Discard(_) => {
                hub.writer.discard(self.intent, io)?;
                // Backend discard can close an expired original; the wire only
                // allows discarded here, so never mislabel that terminal state.
                if hub.backend.status(self.intent, io)?.state != TransferState::Discarded {
                    return Err(BackendError::Conflict);
                }
                Ok((
                    Success::State(State::Discarded),
                    Disclosure::Terminal {
                        state: State::Discarded,
                    },
                ))
            }
            _ => Err(BackendError::Invalid),
        }
    }
}
fn receipt(x: Receipt) -> (Success, Disclosure) {
    let opaque_key = Bytes32::from_bytes(x.key.object.0);
    let payload_sha256 = Sha256Hex::from_bytes(x.payload_sha256);
    let payload_bytes = x.payload_bytes;
    (
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
    )
}

/// Match what was returned to the exact observed original before admitting
/// disclosure. A concurrent worker's transition is not this request's result.
fn matches_result(success: &Success, transfer: &Transfer) -> bool {
    match (success, &transfer.state) {
        (
            Success::Pending {
                next_index,
                received,
                ..
            }
            | Success::Progress {
                next_index,
                received,
            },
            TransferState::Pending(x),
        ) => *next_index == x.next_index && *received == x.received,
        (
            Success::Committed {
                opaque_key,
                payload_sha256,
                payload_bytes,
            },
            TransferState::Committed(x),
        ) => {
            *opaque_key.as_bytes() == x.key.object.0
                && *payload_sha256.as_bytes() == x.payload_sha256
                && *payload_bytes == x.payload_bytes
        }
        (Success::State(State::Expired), TransferState::Expired)
        | (Success::State(State::Discarded), TransferState::Discarded)
        | (Success::State(State::Unavailable), TransferState::Unavailable)
        | (Success::State(State::Unknown), TransferState::Unknown)
        | (Success::State(State::NotObserved), TransferState::NotObserved) => true,
        _ => false,
    }
}

pub(super) fn serve(
    hub: &Hub,
    bus: &Bus,
    request: &Request,
    deadline: Instant,
) -> Result<(), ChannelEnd> {
    let fail = |code| dispatch::send_until(hub, bus, request, Outcome::Failure(code), deadline);
    let unknown = || dispatch::send_result(hub, bus, request, Outcome::Failure(ErrorCode::Unknown));
    let io = IoBudget {
        deadline,
        cancelled: &hub.stop,
    };
    if io.check().is_err() {
        return fail(ErrorCode::Unavailable);
    }
    let Some(context) = dispatch::stands(hub, request.origin) else {
        return fail(ErrorCode::Denied);
    };
    let original = match UploadOriginal::lookup(hub, request, context, &io) {
        Ok(x) => x,
        Err(code) => return fail(code),
    };
    let deadline = match original.deadline(hub, deadline) {
        Ok(x) => x,
        Err(code) => return fail(code),
    };
    let io = IoBudget {
        deadline,
        cancelled: &hub.stop,
    };
    let operation = |disclosure| Operation {
        input: original.input.clone(),
        disclosure,
    };
    for checkpoint in [Checkpoint::Acquire, Checkpoint::Effect] {
        if let Err(code) = original.fence(hub, request, context, &io, true) {
            return fail(code);
        }
        let Some(decision) = dispatch::ask(
            hub,
            bus,
            request,
            context,
            checkpoint,
            operation(None),
            deadline,
        )?
        else {
            return fail(ErrorCode::Unavailable);
        };
        if io.check().is_err() {
            return fail(ErrorCode::Unavailable);
        }
        match decision {
            Decision::Allow => {}
            Decision::Deny => return fail(ErrorCode::Denied),
            Decision::Unavailable => return fail(ErrorCode::Unavailable),
        }
    }
    if let Err(code) = original.fence(hub, request, context, &io, true) {
        return fail(code);
    }
    // Effect boundary: after invoking this call every loss of the publication
    // budget or authority is unknown, even if the backend reports an error.
    let performed = original.perform(hub, &request.call, &io);
    dispatch::between(hub, deadline);
    if original.fence(hub, request, context, &io, false).is_err() {
        return unknown();
    }
    let (success, disclosure) = match performed {
        Ok(x) => x,
        Err(error) => {
            return dispatch::send_until_or(
                hub,
                bus,
                request,
                Outcome::Failure(mutation_error(error)),
                deadline,
                ErrorCode::Unknown,
            );
        }
    };
    let observed = match hub.backend.status(original.intent, &io) {
        Ok(x) if matches_result(&success, &x) => x,
        _ => return unknown(),
    };
    let Some(decision) = dispatch::ask(
        hub,
        bus,
        request,
        context,
        Checkpoint::Disclose,
        operation(Some(disclosure)),
        deadline,
    )?
    else {
        return unknown();
    };
    if decision != Decision::Allow {
        return unknown();
    }
    dispatch::before_result(hub, deadline);
    // A completed commit/discard need not remain staging. The original expiry
    // was still enforced through its call, and the publication budget stays clipped.
    if original.fence(hub, request, context, &io, false).is_err() {
        return unknown();
    }
    if hub.backend.status(original.intent, &io).ok().as_ref() != Some(&observed) {
        return unknown();
    }
    dispatch::send_until_or(
        hub,
        bus,
        request,
        Outcome::Success(success),
        deadline,
        ErrorCode::Unknown,
    )
}
