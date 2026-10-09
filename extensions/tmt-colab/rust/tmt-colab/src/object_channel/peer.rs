//! One finite request worker owned by the actual upgraded sync peer. Socket
//! polling never waits on callbacks or a decoder, and close joins this worker.
use super::{
    CallbackOwner,
    admission::{CapturedTarget, PeerIdentity, RequestAdmission, RequestCapture},
    binding::FrozenUpload,
};
use crate::{
    attachments, page,
    sync::{Admission, SyncScope},
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex, atomic::Ordering, mpsc},
    thread::{self, JoinHandle},
    time::Instant,
};
use tmt_colab_model::{
    attachment::{AttachmentSelector, Descriptor},
    values,
};
use tmt_extension_objects::{
    AdmitInput, BeginInput, Bytes32, Call, ConfigInput, Counter, ErrorCode, Frame, Outcome, Policy,
    ReadInput, ResultFrame, Sha256Hex,
};

#[derive(Debug, Deserialize)]
#[serde(
    tag = "method",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectRequest {
    Config {},
    History {
        history_id: String,
        epoch: String,
    },
    HistoryNext {
        history_id: String,
    },
    HistoryCancel {
        history_id: String,
    },
    Begin {
        transfer_id: String,
        descriptor: Descriptor,
        base: String,
    },
    Part {
        transfer_id: String,
        index: u32,
        bytes: String,
    },
    Commit {
        transfer_id: String,
    },
    Status {
        transfer_id: String,
        descriptor: Descriptor,
        base: String,
    },
    Discard {
        transfer_id: String,
    },
    Verify {
        transfer_id: String,
        descriptor: Descriptor,
        base: String,
        offset: u64,
        count: u32,
    },
    Read {
        selector: AttachmentSelector,
        offset: u64,
        count: u32,
    },
}
struct Job {
    scope: SyncScope,
    id: String,
    request: ObjectRequest,
    deadline: Instant,
}
pub(crate) struct ObjectReply {
    pub scope: SyncScope,
    pub id: String,
    pub value: serde_json::Value,
    pub fence: Option<Arc<RequestCapture>>,
}
pub(crate) struct PeerObjects {
    peer: Arc<PeerIdentity>,
    requests: Option<mpsc::SyncSender<Job>>,
    pub(super) replies: mpsc::Receiver<ObjectReply>,
    worker: Option<JoinHandle<()>>,
    pending: Mutex<HashSet<String>>,
}
/// The wire code a failed request reports. `denied` is authority that ended or never existed,
/// `not-found` is a read or verify of a reference the page does not hold (never a transient
/// failure, so a retry cannot help), `conflict` is a disclosure that changed while it ran and
/// `unavailable` is everything else, including the backend and the channel.
pub(crate) fn error_code(
    error: &(dyn std::error::Error + 'static),
    request: &ObjectRequest,
) -> &'static str {
    match error.downcast_ref::<page::Fault>() {
        Some(page::Fault::Denied | page::Fault::Inactive) => "denied",
        Some(page::Fault::Invalid) => "invalid",
        Some(page::Fault::StaleBase) => "conflict",
        Some(page::Fault::Missing)
            if matches!(
                request,
                ObjectRequest::Read { .. } | ObjectRequest::Verify { .. }
            ) =>
        {
            "not-found"
        }
        _ if matches!(
            error.downcast_ref::<crate::sync::Code>(),
            Some(crate::sync::Code::Denied | crate::sync::Code::Expired)
        ) || matches!(
            error.downcast_ref::<crate::registration::Code>(),
            Some(crate::registration::Code::Denied | crate::registration::Code::Expired)
        ) =>
        {
            "denied"
        }
        _ => "unavailable",
    }
}
impl PeerObjects {
    pub(crate) fn new(peer: Arc<PeerIdentity>) -> std::io::Result<Self> {
        let (requests, incoming) = mpsc::sync_channel::<Job>(8);
        let (outgoing, replies) = mpsc::sync_channel(8);
        let identity = Arc::clone(&peer);
        let worker = thread::Builder::new()
            .name("colab-object-peer".into())
            .spawn(move || {
                let mut retained = BTreeMap::new();
                let mut history: Option<super::history::History> = None;
                loop {
                    let job = if let Some(active) = &history {
                        match incoming.recv_timeout(
                            active
                                .capture
                                .deadline
                                .saturating_duration_since(Instant::now()),
                        ) {
                            Ok(job) => job,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                history.take();
                                continue;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    } else {
                        match incoming.recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        }
                    };

                    if identity.closed.load(Ordering::Acquire) {
                        break;
                    }
                    let (value, fence) = match execute(&identity, &mut retained, &mut history, &job)
                    {
                        Ok(pair) => pair,
                        Err(error) => {
                            let code = error_code(error.as_ref(), &job.request);
                            (serde_json::json!({"error":{"code":code}}), None)
                        }
                    };
                    if identity.closed.load(Ordering::Acquire) {
                        break;
                    }
                    // A full output queue closes this worker; it never blocks peer shutdown.
                    if outgoing
                        .try_send(ObjectReply {
                            scope: job.scope,
                            id: job.id,
                            value,
                            fence,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            peer,
            requests: Some(requests),
            replies,
            worker: Some(worker),
            pending: Mutex::new(HashSet::new()),
        })
    }
    pub(crate) fn submit(
        &self,
        scope: SyncScope,
        id: String,
        request: ObjectRequest,
        deadline: Instant,
    ) -> Result<(), crate::sync::Code> {
        values::generated_id(&id)?;
        let mut pending = super::locked(&self.pending);
        if pending.contains(&id) {
            return Err(crate::sync::Code::Invalid);
        }
        if pending.len() >= 8 {
            return Err(crate::sync::Code::Capacity);
        }
        let sender = self.requests.as_ref().ok_or(crate::sync::Code::Denied)?;
        pending.insert(id.clone());
        if sender
            .try_send(Job {
                scope,
                id: id.clone(),
                request,
                deadline,
            })
            .is_err()
        {
            pending.remove(&id);
            return Err(crate::sync::Code::Capacity);
        }
        Ok(())
    }
    pub(crate) fn take_reply(&self) -> Option<ObjectReply> {
        let reply = self.replies.try_recv().ok()?;
        super::locked(&self.pending).remove(&reply.id);
        Some(reply)
    }
    pub(crate) fn close(&mut self) {
        self.peer.closed.store(true, Ordering::Release);
        self.peer.client.cancel_origin(self.peer.origin);
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for PeerObjects {
    fn drop(&mut self) {
        self.close();
    }
}
fn original(frozen: &FrozenUpload) -> BeginInput {
    let Call::Begin(input) = frozen.begin() else {
        unreachable!("frozen begin")
    };
    input
}
fn execute(
    peer: &Arc<PeerIdentity>,
    retained: &mut BTreeMap<String, FrozenUpload>,
    history: &mut Option<super::history::History>,
    job: &Job,
) -> crate::Result<(serde_json::Value, Option<Arc<RequestCapture>>)> {
    if Instant::now() >= job.deadline
        || !peer
            .client
            .wait_origin(peer.origin, &peer.closed, job.deadline)
    {
        return Ok((serde_json::json!({"error":{"code":"unavailable"}}), None));
    }
    match &job.request {
        ObjectRequest::History { history_id, epoch } => {
            if history.is_some() {
                return Err(page::Fault::Capacity.into());
            }
            *history = Some(super::history::History::new(
                Arc::clone(peer),
                job.scope.clone(),
                history_id.clone(),
                epoch,
                job.deadline,
            )?);
        }
        ObjectRequest::HistoryNext { history_id } => {
            let active = history.as_ref().ok_or(page::Fault::Invalid)?;
            if active.id != *history_id || active.capture.scope != job.scope {
                return Err(page::Fault::Denied.into());
            }
        }
        ObjectRequest::HistoryCancel { history_id } => {
            if history
                .as_ref()
                .is_some_and(|active| active.id == *history_id && active.capture.scope == job.scope)
            {
                history.take();
            }
            return Ok((serde_json::json!({"ok":{"result":"history-end"}}), None));
        }
        _ => {}
    }
    if matches!(
        job.request,
        ObjectRequest::History { .. } | ObjectRequest::HistoryNext { .. }
    ) {
        let active = history.as_mut().ok_or(page::Fault::Invalid)?;
        let capture = Arc::clone(&active.capture);
        let next = active.next();
        match next {
            Ok((value, finished)) => {
                if finished {
                    history.take();
                }
                return Ok((value, Some(capture)));
            }
            Err(error) => {
                history.take();
                return Err(error);
            }
        }
    }
    let write = matches!(
        job.request,
        ObjectRequest::Begin { .. }
            | ObjectRequest::Part { .. }
            | ObjectRequest::Commit { .. }
            | ObjectRequest::Discard { .. }
            | ObjectRequest::Verify { .. }
    );
    let context = peer.capture_scope(&job.scope, write)?;
    let source = peer
        .admission
        .save_source()
        .ok_or(page::Fault::Unavailable)?;
    let mut view = source()?;
    let base = page::revision(&view.store, &view.keyring, &job.scope.page)?;
    let mut target = CapturedTarget::Scope { base };
    let mut frozen_original = None;
    let call = match &job.request {
        ObjectRequest::History { .. }
        | ObjectRequest::HistoryNext { .. }
        | ObjectRequest::HistoryCancel { .. } => return Err(page::Fault::Invalid.into()),
        ObjectRequest::Config {} => {
            let policy = Policy::new(serde_json::to_vec(&serde_json::json!({"version":1,"space":job.scope.space,"page":job.scope.page,"epoch":job.scope.epoch}))?).map_err(|_| page::Fault::Capacity)?;
            Call::Config(ConfigInput {
                namespace: Bytes32::from_bytes(attachments::namespace(
                    &job.scope.space,
                    &job.scope.page,
                )?),
                policy,
            })
        }
        ObjectRequest::Begin {
            transfer_id,
            descriptor,
            base,
        }
        | ObjectRequest::Status {
            transfer_id,
            descriptor,
            base,
        } => {
            if descriptor.space != job.scope.space
                || descriptor.page != job.scope.page
                || (matches!(job.request, ObjectRequest::Begin { .. })
                    && descriptor.epoch != job.scope.epoch)
                || descriptor.author_device != peer.principal
                || peer.reader
            {
                return Err(page::Fault::Denied.into());
            }
            if matches!(job.request, ObjectRequest::Status { .. }) {
                peer.admission.attachment_context(
                    &peer.principal,
                    &job.scope,
                    values::decimal(&descriptor.epoch, false)?,
                )?;
            }
            let frozen = FrozenUpload::metadata(descriptor.clone(), base.clone(), transfer_id)?;
            if let Some(old) = retained.get(transfer_id)
                && (old.begin() != frozen.begin() || old.base() != frozen.base())
            {
                return Err(page::Fault::Invalid.into());
            }
            frozen_original = Some(original(&frozen));
            if matches!(job.request, ObjectRequest::Begin { .. }) {
                attachments::check_upload_target(
                    &mut view,
                    &peer.principal,
                    descriptor,
                    base,
                    job.deadline,
                )?;
                target = CapturedTarget::Upload {
                    descriptor: Box::new(descriptor.clone()),
                    base: base.clone(),
                };
                if !retained.contains_key(transfer_id) && retained.len() >= 8 {
                    return Err(page::Fault::Capacity.into());
                }
                let call = frozen.begin();
                retained.insert(transfer_id.clone(), frozen);
                call
            } else {
                if !retained.contains_key(transfer_id) && retained.len() >= 8 {
                    return Err(page::Fault::Capacity.into());
                }
                let call = frozen.status();
                retained.insert(transfer_id.clone(), frozen);
                call
            }
        }
        ObjectRequest::Part {
            transfer_id,
            index,
            bytes,
        } => {
            let frozen = retained.get(transfer_id).ok_or(page::Fault::Denied)?;
            target = CapturedTarget::Upload {
                descriptor: Box::new(frozen.descriptor().clone()),
                base: frozen.base().into(),
            };
            frozen_original = Some(original(frozen));
            frozen.streamed_part(*index, values::binary(bytes, 32_768)?)?
        }
        ObjectRequest::Commit { transfer_id } | ObjectRequest::Discard { transfer_id } => {
            let frozen = retained.get(transfer_id).ok_or(page::Fault::Denied)?;
            target = CapturedTarget::Upload {
                descriptor: Box::new(frozen.descriptor().clone()),
                base: frozen.base().into(),
            };
            frozen_original = Some(original(frozen));
            if matches!(job.request, ObjectRequest::Commit { .. }) {
                frozen.commit()
            } else {
                frozen.discard()
            }
        }
        ObjectRequest::Verify {
            transfer_id,
            descriptor,
            base,
            offset,
            count,
        } => {
            if descriptor.space != job.scope.space
                || descriptor.page != job.scope.page
                || descriptor.epoch != job.scope.epoch
                || descriptor.author_device != peer.principal
                || peer.reader
            {
                return Err(page::Fault::Denied.into());
            }
            let frozen = FrozenUpload::metadata(descriptor.clone(), base.clone(), transfer_id)?;
            attachments::check_upload_target(
                &mut view,
                &peer.principal,
                descriptor,
                base,
                job.deadline,
            )?;
            let original = original(&frozen);
            if !(1..=32_768).contains(count)
                || *offset >= original.payload_bytes
                || u64::from(*count) > original.payload_bytes - offset
            {
                return Err(page::Fault::Invalid.into());
            }
            target = CapturedTarget::Upload {
                descriptor: Box::new(frozen.descriptor().clone()),
                base: frozen.base().into(),
            };
            Call::Read(ReadInput {
                namespace: original.namespace,
                opaque_key: original.opaque_key,
                policy: original.policy,
                payload_sha256: original.payload_sha256,
                payload_bytes: original.payload_bytes,
                offset: *offset,
                count: *count,
            })
        }
        ObjectRequest::Read {
            selector,
            offset,
            count,
        } => {
            let owner = peer
                .admission
                .attachment_read_owner(&peer.principal, &job.scope)?;
            let capture = Arc::new(attachments::capture_session(
                &view.store,
                &view.keyring,
                &owner,
                selector,
                &mut view.decoder,
                job.deadline,
            )?);
            let descriptor = capture.descriptor();
            let bytes = values::decimal(&descriptor.payload_bytes, false)?;
            if !(1..=32_768).contains(count)
                || *offset >= bytes
                || u64::from(*count) > bytes - offset
            {
                return Err(page::Fault::Invalid.into());
            }
            let call = Call::Read(ReadInput {
                namespace: Bytes32::from_bytes(attachments::namespace(
                    &descriptor.space,
                    &descriptor.page,
                )?),
                opaque_key: Bytes32::from_bytes(
                    *Sha256Hex::parse(&descriptor.object_id)
                        .map_err(|_| page::Fault::Invalid)?
                        .as_bytes(),
                ),
                policy: super::binding::read_policy(descriptor, &job.scope.epoch, selector)?,
                payload_sha256: Sha256Hex::parse(&descriptor.payload_sha256)
                    .map_err(|_| page::Fault::Invalid)?,
                payload_bytes: bytes,
                offset: *offset,
                count: *count,
            });
            target = CapturedTarget::Read(capture);
            call
        }
    };
    drop(view);
    let input = match &call {
        Call::Config(input) => AdmitInput::Config(input.clone()),
        Call::Read(input) => AdmitInput::Read(input.clone()),
        Call::Begin(input) => AdmitInput::Begin(input.clone()),
        Call::Status(input) => AdmitInput::Status(input.clone()),
        Call::Part(input) => retained
            .get(&input.transfer_id.to_string())
            .ok_or(page::Fault::Denied)?
            .admission_input(&call)?,
        Call::Commit(input) | Call::Discard(input) => retained
            .get(&input.transfer_id.to_string())
            .ok_or(page::Fault::Denied)?
            .admission_input(&call)?,
    };
    let mutation = matches!(
        call,
        Call::Begin(_) | Call::Part(_) | Call::Commit(_) | Call::Discard(_)
    );
    let capture = Arc::new(RequestCapture {
        peer: Arc::clone(peer),
        scope: job.scope.clone(),
        context,
        target,
        source,
        write,
        mutation,
        deadline: job.deadline,
    });
    let admission = Arc::new(RequestAdmission {
        capture: Arc::clone(&capture),
        input,
        original: frozen_original,
    });
    // No late call crosses the effect boundary. Callback acquisition/effect and
    // disclosure each recheck the same captured owner and input independently.
    capture.current()?;
    if Instant::now() >= job.deadline {
        return Ok((serde_json::json!({"error":{"code":"unavailable"}}), None));
    }
    let owner: Arc<dyn CallbackOwner> = admission.clone();
    let outcome = peer
        .client
        .request(peer.origin, call.clone(), owner, job.deadline);
    let outcome = match outcome {
        Ok(outcome) if Instant::now() < job.deadline && capture.current().is_ok() => outcome,
        _ => Outcome::Failure(if mutation {
            ErrorCode::Unknown
        } else {
            ErrorCode::Unavailable
        }),
    };
    let transfer_id = match &call {
        Call::Begin(input) => Some(input.transfer_id),
        Call::Status(input) => Some(input.transfer_id),
        Call::Part(input) => Some(input.transfer_id),
        Call::Commit(input) | Call::Discard(input) => Some(input.transfer_id),
        _ => None,
    };
    if let Some(id) = transfer_id
        && matches!(
            &outcome,
            Outcome::Success(
                tmt_extension_objects::Success::Committed { .. }
                    | tmt_extension_objects::Success::State(
                        tmt_extension_objects::State::Expired
                            | tmt_extension_objects::State::Discarded
                    )
            )
        )
    {
        retained.remove(&id.to_string());
    }
    let projected = project_result(ResultFrame {
        generation: peer.client.generation(),
        request_id: Counter::new(1).map_err(|_| page::Fault::Invalid)?,
        method: call.method(),
        transfer_id,
        outcome,
    });
    // The call already crossed into Remote. Even a local projection failure
    // cannot describe a mutating original as having had no effect.
    Ok((
        projected.unwrap_or_else(
            |_| serde_json::json!({"error":{"code":if mutation {"unknown"} else {"unavailable"}}}),
        ),
        Some(capture),
    ))
}
fn project_result(result: ResultFrame) -> crate::Result<serde_json::Value> {
    let bytes =
        tmt_extension_objects::encode(&Frame::Result(result)).map_err(|_| page::Fault::Invalid)?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes[tmt_extension_objects::limits::PREFIX_BYTES..])?;
    let object = value.as_object_mut().ok_or(page::Fault::Invalid)?;
    Ok(if let Some(ok) = object.remove("ok") {
        serde_json::json!({"ok":ok})
    } else {
        serde_json::json!({"error":object.remove("error").ok_or(page::Fault::Invalid)?})
    })
}

#[cfg(test)]
mod error_code_tests {
    use super::*;
    use tmt_colab_model::attachment::AttachmentSelector;

    const ID: &str = "00000000-0000-4000-8000-000000000001";
    fn read() -> ObjectRequest {
        ObjectRequest::Read {
            selector: AttachmentSelector::Message {
                writer_id: ID.into(),
                message_id: ID.into(),
                message_revision: "1".into(),
                attachment_id: ID.into(),
                descriptor_hash: "00".repeat(32),
            },
            offset: 0,
            count: 1,
        }
    }
    fn code(error: impl std::error::Error + 'static, request: &ObjectRequest) -> &'static str {
        error_code(&error, request)
    }

    #[test]
    fn a_reference_the_page_does_not_hold_is_not_found_only_for_reads_never_a_transient_failure() {
        assert_eq!(code(page::Fault::Missing, &read()), "not-found");
        // The wire allows `not-found` only for read and verify; anything else stays unavailable.
        assert_eq!(
            code(
                page::Fault::Missing,
                &ObjectRequest::Commit {
                    transfer_id: ID.into()
                }
            ),
            "unavailable"
        );
    }

    #[test]
    fn authority_that_ended_is_denied_a_moved_disclosure_conflicts_and_the_rest_is_unavailable() {
        for fault in [page::Fault::Denied, page::Fault::Inactive] {
            assert_eq!(code(fault, &read()), "denied");
        }
        assert_eq!(code(crate::sync::Code::Expired, &read()), "denied");
        assert_eq!(code(page::Fault::StaleBase, &read()), "conflict");
        assert_eq!(code(page::Fault::Invalid, &read()), "invalid");
        for fault in [page::Fault::Unavailable, page::Fault::Capacity] {
            assert_eq!(code(fault, &read()), "unavailable");
        }
    }
}
