//! Sealed-document carrier adapter. The injected provider owns Auth, I/O and polling.
use crate::{
    objects::IoBudget,
    sealed_sync::{self as client, SealedSyncPort, SyncError},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tmt_extension_objects::{Uuid4, sync as wire};

pub struct Adapter {
    port: Arc<dyn SealedSyncPort>,
    generation: Uuid4,
    binding: Uuid4,
    bound: AtomicBool,
    interests: Arc<interests::Interests>,
    closed: AtomicBool,
}
impl Adapter {
    pub fn new(port: Arc<dyn SealedSyncPort>, generation: Uuid4, binding: Uuid4) -> Self {
        Self::with_signal(
            port,
            generation,
            binding,
            Arc::new(interests::Signal::new()),
        )
    }
    fn with_signal(
        port: Arc<dyn SealedSyncPort>,
        generation: Uuid4,
        binding: Uuid4,
        signal: Arc<interests::Signal>,
    ) -> Self {
        Self {
            port,
            generation,
            binding,
            bound: AtomicBool::new(false),
            interests: Arc::new(interests::Interests::with_signal(signal)),
            closed: AtomicBool::new(false),
        }
    }
    /// The bus admitted the request's grammar and generation before dispatch.
    pub async fn execute(
        &self,
        request: &wire::Frame,
        io: &IoBudget<'_>,
    ) -> Result<wire::Frame, SyncError> {
        wire::encoded_size(request).map_err(|_| SyncError::Invalid)?;
        let wire::Frame::Request {
            request_id, call, ..
        } = request
        else {
            return Err(SyncError::Invalid);
        };
        let result = if request.generation().map_err(|_| SyncError::Invalid)? != self.generation {
            Err(SyncError::Denied)
        } else {
            self.call(call, request_id, io).await
        };
        let mut reply = wire::Frame::Result {
            version: 1,
            generation: self.generation.to_string(),
            request_id: request_id.clone(),
            method: call.method(),
            outcome: result.unwrap_or_else(failure),
        };
        // A provider result is one page, never a retained multi-frame batch. An invalid
        // or oversized page must not disclose rows or advance the consumer's position.
        if wire::encoded_size(&reply).is_err()
            && let wire::Frame::Result { outcome, .. } = &mut reply
        {
            *outcome = failure(SyncError::CatchUpRequired)
        }
        Ok(reply)
    }
    async fn call(
        &self,
        call: &wire::Call,
        request_id: &str,
        io: &IoBudget<'_>,
    ) -> Result<wire::Outcome, SyncError> {
        check_io(io)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(SyncError::Cancelled);
        }
        self.identity()?;
        let context = self.port.context();
        if let wire::Call::Bind {
            project,
            extension,
            declaration_digest,
            space,
        } = call
        {
            if project != &context.identity().project
                || extension != context.extension()
                || declaration_digest != context.declaration_digest()
                || space != context.space()
            {
                return Err(SyncError::Denied);
            }
            self.bound.store(true, Ordering::Release);
            return Ok(wire::Outcome::Binding {
                context: wire::BoundContext {
                    id: self.binding.to_string(),
                    project: project.clone(),
                    extension: extension.clone(),
                    declaration_digest: declaration_digest.clone(),
                    space: space.clone(),
                    uid: context.identity().uid.clone(),
                },
            });
        }
        let binding = match call {
            wire::Call::Read { binding, .. }
            | wire::Call::Query { binding, .. }
            | wire::Call::Subscribe { binding, .. }
            | wire::Call::Cancel { binding, .. }
            | wire::Call::Publish { binding, .. } => binding,
            _ => unreachable!(),
        };
        if binding != &self.binding.to_string() || !self.bound.load(Ordering::Acquire) {
            return Err(SyncError::Denied);
        }
        let outcome = match call {
            wire::Call::Read { target, .. } => {
                let target = client::Target {
                    resource: target.resource.clone(),
                    id: target.id.clone(),
                };
                context.document_path(&target)?;
                let document = self.port.read(&target, io).await?;
                if document.as_ref().is_some_and(|doc| doc.target != target) {
                    return Err(SyncError::Invalid);
                }
                wire::Outcome::Read {
                    document: document
                        .as_ref()
                        .map(|doc| to_document(context, doc))
                        .transpose()?,
                }
            }
            wire::Call::Query { query, .. } => {
                let query = to_query(query);
                match &query {
                    client::SyncQueryRequest::Stream(q) => client::validate_query(context, q)?,
                    client::SyncQueryRequest::Discovery(q) => {
                        client::validate_discovery(context, q)?
                    }
                }
                let page = self.port.query(&query, io).await?;
                // Providers own pagination; the carrier does not omit rows behind
                // an end cursor or manufacture a cursor for a discovery result.
                match (&query, &page) {
                    (
                        client::SyncQueryRequest::Stream(query),
                        client::SyncQueryPage::Stream(page),
                    ) => validate_tail(context, query, page)?,
                    (
                        client::SyncQueryRequest::Discovery(query),
                        client::SyncQueryPage::Discovery(page),
                    ) => validate_page_scope(
                        context,
                        &query.resource,
                        &query.scope,
                        &page.documents,
                    )?,
                    _ => return Err(SyncError::Invalid),
                }
                page_outcome(context, &page)?
            }
            wire::Call::Subscribe { interest, .. } => {
                let subscription_id = request_id;
                let interest = client::SyncInterest {
                    resource: interest.resource.clone(),
                    scope: interest.scope.clone().into_iter().collect(),
                };
                client::validate_interest(context, &interest)?;
                let entry = self.interests.reserve(subscription_id)?;
                let subscription = self.port.subscribe(&interest, io).await?;
                if let Err(error) = check_io(io).and_then(|()| self.identity()) {
                    let mut subscription = subscription;
                    subscription.close();
                    return Err(error);
                }
                self.interests
                    .start(entry.entry.clone(), subscription, self.port.clone())?;
                entry.commit();
                wire::Outcome::Subscribed {
                    id: subscription_id.to_string(),
                }
            }
            wire::Call::Cancel {
                subscription_id, ..
            } => {
                self.interests.cancel(subscription_id)?;
                wire::Outcome::Cancelled {
                    id: subscription_id.to_string(),
                }
            }
            wire::Call::Publish { mutation, .. } => {
                let mutation = to_mutation(mutation)?;
                client::validate_mutation(context, &mutation)?;
                let published = self.port.publish(&mutation, io).await?;
                // A cancellation, deadline or identity fence after invoking a write
                // cannot truthfully claim that nothing was sent.
                let status = if check_io(io).and_then(|()| self.identity()).is_err() {
                    wire::Published::Unknown
                } else {
                    match published {
                        client::Published::Confirmed => wire::Published::Confirmed,
                        client::Published::AlreadyPresent => wire::Published::AlreadyPresent,
                        client::Published::Unknown => wire::Published::Unknown,
                    }
                };
                return Ok(wire::Outcome::Published { status });
            }
            wire::Call::Bind { .. } => unreachable!(),
        };
        if let Err(error) = check_io(io).and_then(|()| self.identity()) {
            if let wire::Outcome::Subscribed { id } = &outcome {
                let _ = self.interests.cancel(id);
            }
            return Err(error);
        }
        Ok(outcome)
    }
    /// Called only after the successful register result was admitted to the wire.
    fn cancel_registration(&self, id: &str) {
        let _ = self.interests.cancel(id);
    }
    pub fn registered(&self, id: &str) {
        self.interests.enable(id)
    }
    fn take_notice(&self) -> Option<wire::Frame> {
        let (subscription_id, sequence, notice) = self.interests.take()?;
        Some(wire::Frame::Notice {
            version: 1,
            generation: self.generation.to_string(),
            subscription_id,
            sequence: sequence.to_string(),
            notice,
        })
    }
    pub fn next_notice(&self, deadline: std::time::Instant) -> Option<wire::Frame> {
        let (subscription_id, sequence, notice) = self.interests.next(deadline)?;
        Some(wire::Frame::Notice {
            version: 1,
            generation: self.generation.to_string(),
            subscription_id,
            sequence: sequence.to_string(),
            notice,
        })
    }
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let entries = self.interests.stop();
        self.port.close();
        for entry in entries {
            entry.join()
        }
    }
    fn identity(&self) -> Result<(), SyncError> {
        let context = self.port.context();
        if context.auth().identity() != *context.identity() {
            Err(SyncError::Denied)
        } else {
            Ok(())
        }
    }
}
impl Drop for Adapter {
    fn drop(&mut self) {
        self.close();
    }
}
fn check_io(io: &IoBudget<'_>) -> Result<(), SyncError> {
    io.check().map_err(|error| match error {
        crate::objects::BackendError::Cancelled => SyncError::Cancelled,
        _ => SyncError::Deadline,
    })
}
fn to_mutation(mutation: &wire::Mutation) -> Result<client::Mutation, SyncError> {
    let document = |document: &wire::Document| -> Result<client::Document, SyncError> {
        let mut fields = serde_json::Value::Object(document.fields.clone());
        fields.sort_all_objects();
        Ok(client::Document {
            target: client::Target {
                resource: document.target.resource.clone(),
                id: document.target.id.clone(),
            },
            bytes: serde_json::to_vec(&fields).map_err(|_| SyncError::Invalid)?,
            digest: document.digest.clone(),
        })
    };
    Ok(match mutation {
        wire::Mutation::Create { document: doc } => client::Mutation::Create(document(doc)?),
        wire::Mutation::Put { document: doc } => client::Mutation::Put(document(doc)?),
        wire::Mutation::Delete { target } => client::Mutation::Delete(client::Target {
            resource: target.resource.clone(),
            id: target.id.clone(),
        }),
    })
}
fn to_query(query: &wire::Query) -> client::SyncQueryRequest {
    match query {
        wire::Query::Stream {
            resource,
            scope,
            stream,
            cursor,
        } => client::SyncQueryRequest::Stream(client::Query {
            resource: resource.clone(),
            scope: scope.clone().into_iter().collect(),
            stream: stream.clone(),
            cursor: cursor.as_ref().map(|c| client::SyncCursor {
                stream: c.stream.clone(),
                seq: c.seq,
            }),
        }),
        wire::Query::Discovery {
            resource,
            name,
            scope,
            continuation,
        } => client::SyncQueryRequest::Discovery(client::DiscoveryQuery {
            resource: resource.clone(),
            shape: name.clone(),
            scope: scope.clone().into_iter().collect(),
            continuation: continuation.clone().map(client::DiscoveryContinuation),
        }),
    }
}
fn to_document(
    context: &client::SyncContext,
    document: &client::Document,
) -> Result<wire::Document, SyncError> {
    client::validate_document(context, document, client::DocumentAccess::Read)?;
    Ok(wire::Document {
        target: wire::Target {
            resource: document.target.resource.clone(),
            id: document.target.id.clone(),
        },
        fields: client::validate_fields(&document.bytes)?,
        digest: document.digest.clone(),
    })
}
fn to_documents(
    context: &client::SyncContext,
    documents: &[client::Document],
) -> Result<Vec<wire::Document>, SyncError> {
    check_page_documents(documents)?;
    documents
        .iter()
        .map(|doc| to_document(context, doc))
        .collect()
}
/// Reject a provider's oversized page before parsing or duplicating its fields.
/// The final encoder also counts targets, cursors and other frame metadata.
fn check_page_documents(documents: &[client::Document]) -> Result<(), SyncError> {
    if documents.len() > tmt_extension_objects::limits::SYNC_PAGE_DOCUMENTS {
        return Err(SyncError::CatchUpRequired);
    }
    let mut bytes = 0usize;
    for document in documents {
        if document.bytes.len() > tmt_extension_objects::limits::SYNC_DOCUMENT_BYTES {
            return Err(SyncError::CatchUpRequired);
        }
        bytes = bytes
            .checked_add(document.bytes.len())
            .ok_or(SyncError::CatchUpRequired)?;
        if bytes > tmt_extension_objects::limits::V3_FRAME_BYTES {
            return Err(SyncError::CatchUpRequired);
        }
    }
    Ok(())
}
fn page_outcome(
    context: &client::SyncContext,
    page: &client::SyncQueryPage,
) -> Result<wire::Outcome, SyncError> {
    Ok(match page {
        client::SyncQueryPage::Stream(page) => wire::Outcome::Stream {
            documents: to_documents(context, &page.documents)?,
            cursor: page.cursor.as_ref().map(|c| wire::StreamPosition {
                stream: c.stream.clone(),
                seq: c.seq,
            }),
        },
        client::SyncQueryPage::Discovery(page) => wire::Outcome::Discovery {
            documents: to_documents(context, &page.documents)?,
            continuation: page.continuation.as_ref().map(|c| c.0.clone()),
        },
    })
}
/// Encoded page bound for provider composition. Reserves the longest request ID;
/// no row is omitted and no provider end cursor is rewritten to make a page fit.
/// Direct leaf imports stay confined to the carrier owner.
pub fn query_page_bytes(
    context: &client::SyncContext,
    page: &client::SyncQueryPage,
) -> Result<usize, SyncError> {
    let frame = wire::Frame::Result {
        version: 1,
        generation: "00000000-0000-4000-8000-000000000000".into(),
        request_id: u64::MAX.to_string(),
        method: wire::Method::Query,
        outcome: page_outcome(context, page)?,
    };
    wire::encoded_size(&frame).map_err(|_| SyncError::CatchUpRequired)
}
fn validate_page_scope(
    context: &client::SyncContext,
    resource: &str,
    scope: &std::collections::BTreeMap<String, serde_json::Value>,
    documents: &[client::Document],
) -> Result<(), SyncError> {
    check_page_documents(documents)?;
    for document in documents {
        let row = to_document(context, document)?;
        if row.target.resource != resource
            || !scope
                .iter()
                .all(|(key, value)| row.fields.get(key) == Some(value))
        {
            return Err(SyncError::Denied);
        }
    }
    Ok(())
}
fn validate_tail(
    context: &client::SyncContext,
    query: &client::Query,
    page: &client::Page,
) -> Result<(), SyncError> {
    if page.documents.len() > tmt_extension_objects::limits::SYNC_PAGE_DOCUMENTS {
        return Err(SyncError::CatchUpRequired);
    }
    validate_page_scope(context, &query.resource, &query.scope, &page.documents)?;
    let mapping = context
        .resources()
        .iter()
        .find(|resource| resource.name == query.resource)
        .and_then(|resource| resource.sync.as_ref())
        .and_then(|mapping| mapping.query.as_ref())
        .ok_or(SyncError::Invalid)?;
    let mut position = query.cursor.clone();
    for document in &page.documents {
        let fields = client::validate_fields(&document.bytes)?;
        let seq = fields
            .get(&mapping.seq_field)
            .and_then(|value| value.get("integerValue"))
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(SyncError::Invalid)?;
        if position.as_ref().is_some_and(|cursor| seq <= cursor.seq) {
            return Err(SyncError::CatchUpRequired);
        }
        position = Some(client::SyncCursor {
            stream: query.stream.clone(),
            seq,
        });
    }
    if page.cursor != position {
        return Err(SyncError::CatchUpRequired);
    }
    Ok(())
}
pub(in crate::object_service) fn failure(error: SyncError) -> wire::Outcome {
    use wire::{Code, State};
    let (code, state) = match error {
        SyncError::Invalid => (Code::Invalid, None),
        SyncError::Denied => (Code::Denied, None),
        SyncError::Unavailable => (Code::Unavailable, None),
        SyncError::BudgetExhausted(_) => (Code::Budget, None),
        SyncError::RateLimited => (Code::Rate, None),
        SyncError::Cancelled => (Code::Unavailable, Some(State::Cancelled)),
        SyncError::Deadline => (Code::Unavailable, Some(State::Deadline)),
        SyncError::CatchUpRequired => (Code::Unavailable, Some(State::CatchUpRequired)),
        SyncError::Conflict => (Code::Unavailable, Some(State::Conflict)),
        SyncError::Unsupported => (Code::Unavailable, Some(State::Unsupported)),
    };
    let (reset_at_ms, retry_after_ms) = match error {
        SyncError::BudgetExhausted(refusal) => {
            (Some(refusal.reset_at_ms), Some(refusal.retry_after_ms))
        }
        _ => (None, None),
    };
    wire::Outcome::Error {
        error: wire::Error {
            code,
            state,
            reset_at_ms,
            retry_after_ms,
        },
    }
}
#[cfg(test)]
pub(in crate::object_service) mod tests;

mod interests;

mod factory;
pub use factory::{BindInput, ClientFactory, Registration};

mod bindings;
pub(super) use bindings::Bindings;

pub(in crate::object_service) use interests::complete;
