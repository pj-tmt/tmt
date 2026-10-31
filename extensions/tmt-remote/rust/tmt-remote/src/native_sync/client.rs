//! No SDK, Admin authority or hidden write replay. Each consumer retains one
//! bounded provider page; its serving owner shares request caps and read costs.
use super::NativeBudgetFile;
use crate::{
    declaration::Resource,
    deploy_plan::sha256_hex,
    firebase_auth::{Cancellation, Clock, Http, Jitter, Operation, Request, Time},
    firestore_budget::{
        Decision, ExhaustedEvidence, ExhaustedOutcome, Model, ProviderExhausted, READ_LOOKUPS,
        Refusal, charge_native, classify_provider_exhausted,
    },
    limits,
    objects::{BackendError, IoBudget},
    sealed_sync::{
        Document, DocumentAccess, Mutation, Page, Pending, Published, Query, SealedSyncPort,
        Subscription, SyncContext, SyncCursor, SyncError, SyncInterest, SyncQueryPage,
        SyncQueryRequest, Target, validate_document, validate_mutation, validate_query,
    },
    wire,
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

pub trait NativeBudgetPort: Send + Sync {
    fn usage(&self, project: &str) -> Result<crate::firestore_budget::NativeUsage, SyncError>;
    fn charge(
        &self,
        project: &str,
        model: &Model,
        maximum_reads: u64,
        time: Time,
    ) -> Result<Decision, SyncError>;
    fn flush(&self) -> Result<(), SyncError>;
    fn checkpoint(&self, _time: Time) -> Result<(), SyncError> {
        self.flush()
    }
}
impl NativeBudgetPort for Mutex<NativeBudgetFile> {
    fn usage(&self, project: &str) -> Result<crate::firestore_budget::NativeUsage, SyncError> {
        let file = self.lock().map_err(|_| SyncError::Unavailable)?;
        if file.project() != project {
            return Err(SyncError::Denied);
        }
        Ok(file.usage())
    }
    fn charge(
        &self,
        project: &str,
        model: &Model,
        maximum_reads: u64,
        time: Time,
    ) -> Result<Decision, SyncError> {
        let mut file = self.lock().map_err(|_| SyncError::Unavailable)?;
        if file.project() != project {
            return Err(SyncError::Denied);
        }
        file.authorize(model, maximum_reads, time.wall_ms, time.monotonic_ms)
            .map_err(|_| SyncError::Unavailable)
    }
    fn flush(&self) -> Result<(), SyncError> {
        self.lock()
            .map_err(|_| SyncError::Unavailable)?
            .flush()
            .map_err(|_| SyncError::Unavailable)
    }
    fn checkpoint(&self, time: Time) -> Result<(), SyncError> {
        self.lock()
            .map_err(|_| SyncError::Unavailable)?
            .checkpoint(time.monotonic_ms)
            .map_err(|_| SyncError::Unavailable)
    }
}
#[derive(Default)]
pub(super) struct Caps {
    pub(super) subscriptions: usize,
    writes: HashSet<WriteKey>,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct WriteKey {
    resource: String,
    stream: Option<String>,
}
pub struct NativeRuntime {
    http: Arc<dyn Http + Send + Sync>,
    pub(crate) clock: Arc<dyn Clock + Send + Sync>,
    pub(crate) jitter: Arc<dyn Jitter + Send + Sync>,
    budget: Arc<dyn NativeBudgetPort>,
    pub(super) caps: Mutex<Caps>,
    exhausted: Mutex<Option<ExhaustedEvidence>>,
    pub(super) stopped: AtomicBool,
}
impl NativeRuntime {
    pub fn new(
        http: Arc<dyn Http + Send + Sync>,
        clock: Arc<dyn Clock + Send + Sync>,
        jitter: Arc<dyn Jitter + Send + Sync>,
        budget: Arc<dyn NativeBudgetPort>,
    ) -> Self {
        Self {
            http,
            clock,
            jitter,
            budget,
            caps: Mutex::new(Caps::default()),
            exhausted: Mutex::new(None),
            stopped: AtomicBool::new(false),
        }
    }
    /// Fence every binding of this serving project before joining its workers.
    /// Persistence remains available until the owner releases the budget writer.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
    fn check(&self) -> Result<(), SyncError> {
        if self.stopped.load(Ordering::SeqCst) {
            Err(SyncError::Unavailable)
        } else {
            Ok(())
        }
    }
    fn provider_refusal(&self) -> Result<Option<Refusal>, SyncError> {
        let now = self.clock.now().wall_ms;
        Ok(self
            .exhausted
            .lock()
            .map_err(|_| SyncError::Unavailable)?
            .filter(|e| e.is_current(now))
            .map(|e| Refusal {
                reset_at_ms: e.resets_at_ms,
                retry_after_ms: e.resets_at_ms.saturating_sub(now),
            }))
    }
    /// The serving owner calls this before releasing its persistent budget writer.
    pub fn finish(&self) -> Result<(), SyncError> {
        self.budget.flush()
    }
    /// The serving owner schedules one shared minute tick, including while
    /// provider workers back off. This runtime creates no background work.
    pub fn checkpoint(&self) -> Result<(), SyncError> {
        self.budget.checkpoint(self.clock.now())
    }
}
pub(super) struct Consumer {
    pub(super) context: SyncContext,
    pub(super) runtime: Arc<NativeRuntime>,
    model: Model,
    pub(super) closed: AtomicBool,
    pub(super) subscriptions: Mutex<usize>,
    pub(super) activity: AtomicU64,
    pub(super) retry_after_ms: AtomicU64,
    pub(super) discovery_key: OnceLock<Result<[u8; 32], SyncError>>,
}
pub struct NativeClient(Arc<Consumer>);
impl NativeClient {
    pub fn new(context: SyncContext, runtime: Arc<NativeRuntime>, model: Model) -> Self {
        Self(Arc::new(Consumer {
            context,
            runtime,
            model,
            closed: AtomicBool::new(false),
            subscriptions: Mutex::new(0),
            activity: AtomicU64::new(0),
            retry_after_ms: AtomicU64::new(0),
            discovery_key: OnceLock::new(),
        }))
    }
    /// The serving UI reads this projection to show the existing 70-percent warning.
    /// It exposes no credentials, changes no allowance and performs no provider I/O.
    pub fn budget_state(&self) -> Result<Decision, SyncError> {
        self.0.runtime.check()?;
        if let Some(refusal) = self.0.runtime.provider_refusal()? {
            return Ok(Decision::Refuse(refusal));
        }
        crate::firestore_budget::native_budget_state(
            &self.0.model,
            self.0
                .runtime
                .budget
                .usage(&self.0.context.identity().project)?,
            self.0.runtime.clock.now().wall_ms,
        )
        .map_err(|_| SyncError::Unavailable)
    }
}
pub(super) struct CallCancellation<'a> {
    pub(super) caller: &'a AtomicBool,
    pub(super) closed: &'a AtomicBool,
    pub(super) project: &'a AtomicBool,
}
impl Cancellation for CallCancellation<'_> {
    fn cancelled(&self) -> bool {
        self.caller.load(Ordering::SeqCst)
            || self.closed.load(Ordering::SeqCst)
            || self.project.load(Ordering::SeqCst)
    }
}
fn check(io: &IoBudget<'_>) -> Result<(), SyncError> {
    io.check().map_err(|e| match e {
        BackendError::Cancelled => SyncError::Cancelled,
        BackendError::Deadline => SyncError::Deadline,
        _ => SyncError::Unavailable,
    })
}
impl Consumer {
    pub(super) fn check(&self, io: &IoBudget<'_>) -> Result<(), SyncError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SyncError::Cancelled);
        }
        self.runtime.check()?;
        check(io)
    }
    pub(super) fn resource(&self, name: &str) -> Result<&Resource, SyncError> {
        self.context
            .resources()
            .iter()
            .find(|r| r.name == name)
            .ok_or(SyncError::Invalid)
    }
    pub(super) fn root(&self) -> String {
        format!(
            "projects/{}/databases/(default)/documents",
            self.context.identity().project
        )
    }
    pub(super) fn limit(&self, resource: &Resource) -> Result<usize, SyncError> {
        let bytes =
            usize::try_from(resource.bounds.max_object_bytes).map_err(|_| SyncError::Invalid)?;
        if bytes == 0 || bytes > limits::NATIVE_SYNC_FIELDS_BYTES {
            return Err(SyncError::Invalid);
        }
        // One document fits the admitted single-document carrier metadata bound.
        // Packing smaller documents awaits the leaf's actual encoded-size validator.
        Ok(1)
    }
    pub(super) async fn send(
        &self,
        operation: Operation,
        url: String,
        body: Vec<u8>,
        cost: u64,
        io: &IoBudget<'_>,
    ) -> Result<Option<crate::firebase_auth::Response>, SyncError> {
        self.runtime
            .send(
                RestCall {
                    identity: self.context.identity(),
                    auth: self.context.auth(),
                    model: &self.model,
                    closed: &self.closed,
                    retry_after_ms: &self.retry_after_ms,
                    response_limit: limits::NATIVE_SYNC_PAGE_BYTES,
                    before_commit: None,
                    operation,
                    url,
                    body,
                    cost,
                },
                io,
            )
            .await
    }
    pub(super) fn parse_document(
        &self,
        value: &Value,
        resource: &str,
    ) -> Result<Document, SyncError> {
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .ok_or(SyncError::Unavailable)?;
        let prefix = format!(
            "{}/x/{}/{}/",
            self.root(),
            self.context.extension(),
            self.resource(resource)?.path
        );
        let id = name.strip_prefix(&prefix).ok_or(SyncError::Denied)?;
        let mut fields = value.get("fields").cloned().ok_or(SyncError::Unavailable)?;
        fields.sort_all_objects();
        let bytes = serde_json::to_vec(&fields).map_err(|_| SyncError::Unavailable)?;
        let document = Document {
            target: Target {
                resource: resource.into(),
                id: id.into(),
            },
            digest: sha256_hex(&bytes),
            bytes,
        };
        validate_document(&self.context, &document, DocumentAccess::Read)?;
        Ok(document)
    }
    pub(super) async fn query(&self, query: &Query, io: &IoBudget<'_>) -> Result<Page, SyncError> {
        self.check(io)?;
        validate_query(&self.context, query)?;
        let resource = self.resource(&query.resource)?;
        let spec = resource
            .sync
            .as_ref()
            .and_then(|s| s.query.as_ref())
            .ok_or(SyncError::Unavailable)?;
        let limit = self.limit(resource)?;
        let mut filters: Vec<Value> = query
            .scope
            .iter()
            .map(|(key, value)| {
                json!({"fieldFilter":{
            "field":{"fieldPath":key},"op":"EQUAL","value":value}})
            })
            .collect();
        let previous = query.cursor.as_ref().map_or(0, |c| c.seq);
        filters.push(
            json!({"fieldFilter":{"field":{"fieldPath":spec.seq_field},"op":"GREATER_THAN",
            "value":{"integerValue":previous.to_string()}}}),
        );
        let (parent, collection) = resource
            .path
            .rsplit_once('/')
            .unwrap_or(("", &resource.path));
        let path = format!(
            "{}/x/{}{}",
            self.root(),
            self.context.extension(),
            if parent.is_empty() {
                String::new()
            } else {
                format!("/{parent}")
            }
        );
        let body = json!({"structuredQuery":{
            "from":[{"collectionId":collection}],
            "where":{"compositeFilter":{"op":"AND","filters":filters}},
            "orderBy":[{"field":{"fieldPath":spec.seq_field},"direction":"ASCENDING"}],"limit":limit}});
        let response = self
            .send(
                Operation::Query,
                format!("https://firestore.googleapis.com/v1/{path}:runQuery"),
                body.to_string().into_bytes(),
                limit as u64 + READ_LOOKUPS,
                io,
            )
            .await?
            .ok_or(SyncError::Unavailable)?;
        status(response.status)?;
        if response.body.len() > limits::NATIVE_SYNC_PAGE_BYTES {
            return Err(SyncError::CatchUpRequired);
        }
        let reply = wire::strict_json(&response.body).ok_or(SyncError::Unavailable)?;
        let rows = reply.as_array().ok_or(SyncError::Unavailable)?;
        let mut documents = Vec::new();
        let mut seq = previous;
        let mut retained_bytes = 0usize;
        for row in rows {
            let Some(value) = row.get("document") else {
                if row.get("readTime").is_none() {
                    return Err(SyncError::Unavailable);
                }
                continue;
            };
            if documents.len() >= limit {
                return Err(SyncError::CatchUpRequired);
            }
            let document = self.parse_document(value, &query.resource)?;
            let fields = crate::sealed_sync::validate_fields(&document.bytes)?;
            if query
                .scope
                .iter()
                .any(|(key, value)| fields.get(key) != Some(value))
            {
                return Err(SyncError::Denied);
            }
            let next = fields
                .get(&spec.seq_field)
                .and_then(|v| v.get("integerValue"))
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or(SyncError::Unavailable)?;
            if seq.checked_add(1) != Some(next) {
                return Err(SyncError::CatchUpRequired);
            }
            seq = next;
            retained_bytes = retained_bytes.saturating_add(document.bytes.len());
            if retained_bytes > limits::NATIVE_SYNC_FIELDS_BYTES {
                return Err(SyncError::CatchUpRequired);
            }
            documents.push(document);
        }
        self.check(io)?;
        let cursor = if documents.is_empty() {
            query.cursor.clone()
        } else {
            Some(SyncCursor {
                stream: query.stream.clone(),
                seq,
            })
        };
        Ok(Page { documents, cursor })
    }
}
pub(super) fn status(status: u16) -> Result<(), SyncError> {
    match status {
        200 => Ok(()),
        403 => Err(SyncError::Denied),
        429 => Err(SyncError::RateLimited),
        _ => Err(SyncError::Unavailable),
    }
}
impl SealedSyncPort for NativeClient {
    fn context(&self) -> &SyncContext {
        &self.0.context
    }
    fn read<'a>(
        &'a self,
        target: &'a Target,
        io: &'a IoBudget<'a>,
    ) -> Pending<'a, Option<Document>> {
        Box::pin(async move {
            self.0.check(io)?;
            let target = target.clone();
            let path = self.0.context.document_path(&target)?;
            self.0
                .resource(&target.resource)?
                .sync
                .as_ref()
                .ok_or(SyncError::Unavailable)?;
            let response = self
                .0
                .send(
                    Operation::Read,
                    format!(
                        "https://firestore.googleapis.com/v1/{}/{path}",
                        self.0.root()
                    ),
                    Vec::new(),
                    1 + READ_LOOKUPS,
                    io,
                )
                .await?
                .ok_or(SyncError::Unavailable)?;
            if response.status == 404 {
                self.0.check(io)?;
                return Ok(None);
            }
            status(response.status)?;
            if response.body.len() > limits::NATIVE_SYNC_PAGE_BYTES {
                return Err(SyncError::Unavailable);
            }
            let reply = wire::strict_json(&response.body).ok_or(SyncError::Unavailable)?;
            let document = self.0.parse_document(&reply, &target.resource)?;
            if document.target != target {
                return Err(SyncError::Denied);
            }
            self.0.check(io)?;
            Ok(Some(document))
        })
    }
    fn query<'a>(
        &'a self,
        query: &'a SyncQueryRequest,
        io: &'a IoBudget<'a>,
    ) -> Pending<'a, SyncQueryPage> {
        Box::pin(async move {
            match query {
                SyncQueryRequest::Stream(query) => {
                    let page = self.0.query(query, io).await?;
                    Ok(SyncQueryPage::Stream(page))
                }
                SyncQueryRequest::Discovery(query) => super::discovery::query(&self.0, query, io)
                    .await
                    .map(SyncQueryPage::Discovery),
            }
        })
    }
    fn publish<'a>(
        &'a self,
        mutation: &'a Mutation,
        io: &'a IoBudget<'a>,
    ) -> Pending<'a, Published> {
        Box::pin(async move {
            self.0.check(io)?;
            let mutation = match mutation {
                Mutation::Create(document) => Mutation::Create(document.clone()),
                Mutation::Put(document) => Mutation::Put(document.clone()),
                Mutation::Delete(target) => Mutation::Delete(target.clone()),
            };
            validate_mutation(&self.0.context, &mutation)?;
            let (target, document) = match &mutation {
                Mutation::Create(document) | Mutation::Put(document) => {
                    (&document.target, Some(document))
                }
                Mutation::Delete(target) => (target, None),
            };
            let mapping = self
                .0
                .resource(&target.resource)?
                .sync
                .as_ref()
                .ok_or(SyncError::Unavailable)?;
            let fields = document
                .map(|d| crate::sealed_sync::validate_fields(&d.bytes))
                .transpose()?
                .unwrap_or_default();
            let stream = mapping
                .query
                .as_ref()
                .and_then(|s| fields.get(&s.stream_field))
                .and_then(|v| v.get("stringValue"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            // Delete has no fields and must not perform a preparation read or
            // infer a stream from an opaque ID. Reserve the resource instead.
            let key = WriteKey {
                resource: format!(
                    "{}/{}/{}/{}",
                    self.0.context.identity().project,
                    self.0.context.extension(),
                    self.0.context.space(),
                    target.resource
                ),
                stream,
            };
            let _permit = WritePermit::acquire(self.0.runtime.clone(), key)?;
            let name = format!(
                "{}/{}",
                self.0.root(),
                self.0.context.document_path(target)?
            );
            let write = match &mutation {
                Mutation::Create(_) => {
                    json!({"update":{"name":name,"fields":fields},"currentDocument":{"exists":false}})
                }
                Mutation::Put(_) => json!({"update":{"name":name,"fields":fields}}),
                Mutation::Delete(_) => json!({"delete":name}),
            };
            let body = json!({"writes":[write]});
            let response = self.0.send(Operation::Commit,
                format!("https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents:commit", self.0.context.identity().project),
                body.to_string().into_bytes(), self.0.model.reads_per_append(), io).await;
            let response = match response {
                Ok(Some(r)) => r,
                Ok(None) => return Ok(Published::Unknown),
                Err(error) => return Err(error),
            };
            if self.0.check(io).is_err() || response.body.len() > limits::NATIVE_SYNC_PAGE_BYTES {
                return Ok(Published::Unknown);
            }
            if response.status == 403 {
                return Err(SyncError::Denied);
            }
            if response.status == 409 && matches!(mutation, Mutation::Create(_)) {
                let existing = self.read(target, io).await?;
                return match existing {
                    Some(existing) if document.is_some_and(|desired| existing == *desired) => {
                        Ok(Published::AlreadyPresent)
                    }
                    Some(_) => Err(SyncError::Conflict),
                    None => Err(SyncError::Unavailable),
                };
            }
            if response.status != 200 {
                return Ok(Published::Unknown);
            }
            let reply = wire::strict_json(&response.body);
            if !reply
                .as_ref()
                .and_then(|v| v.get("writeResults"))
                .and_then(Value::as_array)
                .is_some_and(|r| {
                    r.len() == 1
                        && r[0].is_object()
                        && if matches!(mutation, Mutation::Delete(_)) {
                            reply
                                .as_ref()
                                .and_then(|v| v.get("commitTime"))
                                .and_then(Value::as_str)
                                .is_some()
                        } else {
                            r[0].get("updateTime").and_then(Value::as_str).is_some()
                        }
                })
            {
                return Ok(Published::Unknown);
            }
            if self.0.check(io).is_err() {
                return Ok(Published::Unknown);
            }
            self.0.activity.store(
                self.0.runtime.clock.now().monotonic_ms.saturating_add(1),
                Ordering::SeqCst,
            );
            Ok(Published::Confirmed)
        })
    }
    fn subscribe<'a>(
        &'a self,
        interest: &'a SyncInterest,
        io: &'a IoBudget<'a>,
    ) -> Pending<'a, Box<dyn Subscription>> {
        Box::pin(super::subscription::create_subscription(
            self.0.clone(),
            interest,
            io,
        ))
    }
    fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
        if let Ok(mut count) = self.0.subscriptions.lock()
            && let Ok(mut caps) = self.0.runtime.caps.lock()
        {
            caps.subscriptions = caps.subscriptions.saturating_sub(*count);
            *count = 0;
        }
    }
}
impl Drop for NativeClient {
    fn drop(&mut self) {
        self.close();
    }
}
struct WritePermit {
    runtime: Arc<NativeRuntime>,
    key: WriteKey,
}
impl WritePermit {
    fn acquire(runtime: Arc<NativeRuntime>, key: WriteKey) -> Result<Self, SyncError> {
        {
            let mut caps = runtime.caps.lock().map_err(|_| SyncError::Unavailable)?;
            if caps.writes.len() >= limits::NATIVE_SYNC_SERVE_WRITES
                || caps.writes.iter().any(|held| {
                    held.resource == key.resource
                        && (held.stream.is_none()
                            || key.stream.is_none()
                            || held.stream == key.stream)
                })
            {
                return Err(SyncError::RateLimited);
            }
            caps.writes.insert(key.clone());
        }
        Ok(Self { runtime, key })
    }
}
impl Drop for WritePermit {
    fn drop(&mut self) {
        if let Ok(mut caps) = self.runtime.caps.lock() {
            caps.writes.remove(&self.key);
        }
    }
}

/// One HTTP/Auth/budget owner for extension documents and native machine carriers.
pub(crate) struct RestCall<'a> {
    pub identity: &'a crate::sealed_sync::AuthIdentity,
    pub auth: &'a dyn crate::sealed_sync::AuthPort,
    pub model: &'a Model,
    pub closed: &'a AtomicBool,
    pub retry_after_ms: &'a AtomicU64,
    pub before_commit: Option<Box<dyn FnOnce() -> Result<bool, SyncError> + Send + 'a>>,
    pub response_limit: usize,
    pub operation: Operation,
    pub url: String,
    pub body: Vec<u8>,
    pub cost: u64,
}
impl NativeRuntime {
    pub(crate) async fn send(
        &self,
        call: RestCall<'_>,
        io: &IoBudget<'_>,
    ) -> Result<Option<crate::firebase_auth::Response>, SyncError> {
        let RestCall {
            identity,
            auth,
            model,
            closed,
            retry_after_ms,
            response_limit,
            before_commit,
            operation,
            url,
            body,
            cost,
        } = call;
        let check_call = || {
            if closed.load(Ordering::SeqCst) {
                return Err(SyncError::Cancelled);
            }
            self.check()?;
            check(io)
        };
        check_call()?;
        let mut projected = self.budget.usage(&identity.project)?;
        if let Some(refusal) = self.provider_refusal()? {
            return Err(SyncError::BudgetExhausted(refusal));
        }
        // A cold token request can itself refresh over HTTP. Refuse it with the
        // same cost policy on a read-only usage copy first. The authoritative
        // charge below still rechecks concurrent usage after token validation;
        // an Auth wait cancelled before then consumes no Firestore cost.
        if let Decision::Refuse(refusal) =
            charge_native(model, &mut projected, cost, self.clock.now().wall_ms)
                .map_err(|_| SyncError::Unavailable)?
        {
            return Err(SyncError::BudgetExhausted(refusal));
        }
        let token = auth.token(io).await.map_err(|_| SyncError::Unavailable)?;
        check_call()?;
        if token.identity != *identity {
            return Err(SyncError::Denied);
        }
        if token.expires_at_ms <= self.clock.now().wall_ms || token.token.is_empty() {
            return Err(SyncError::Unavailable);
        }
        match self
            .budget
            .charge(&identity.project, model, cost, self.clock.now())?
        {
            Decision::Refuse(r) => return Err(SyncError::BudgetExhausted(r)),
            Decision::Allow | Decision::Warn => {}
        }
        check_call()?;
        let remaining = io
            .deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        let deadline = self
            .clock
            .now()
            .monotonic_ms
            .saturating_add(u64::try_from(remaining).unwrap_or(u64::MAX));
        let request = Request::firestore(operation, url, body, token.token, response_limit);
        let cancel = CallCancellation {
            caller: io.cancelled,
            closed,
            project: &self.stopped,
        };
        if let Some(claim) = before_commit
            && !claim()?
        {
            return Ok(None);
        }
        let response = self.http.send(&request, deadline, &cancel);
        if let Ok(response) = &response {
            if response.status == 429 || response.status >= 500 {
                retry_after_ms.store(
                    self.clock
                        .now()
                        .monotonic_ms
                        .saturating_add(response.retry_after_ms.unwrap_or(0)),
                    Ordering::SeqCst,
                );
            }
            if response.status == 429
                && response.body.len() <= response_limit
                && wire::strict_json(&response.body)
                    .is_some_and(|v| v["error"]["status"] == "RESOURCE_EXHAUSTED")
            {
                let (outcome, evidence) = classify_provider_exhausted(
                    ProviderExhausted {
                        mutation: operation == Operation::Commit,
                    },
                    self.clock.now().wall_ms,
                );
                *self.exhausted.lock().map_err(|_| SyncError::Unavailable)? = Some(evidence);
                return match outcome {
                    ExhaustedOutcome::Unknown => Ok(None),
                    ExhaustedOutcome::Refused(r) => Err(SyncError::BudgetExhausted(r)),
                };
            }
        }
        // After invoking a write, every lost/cancelled/malformed reply may hide an effect.
        let late = self.clock.now().monotonic_ms >= deadline;
        if operation == Operation::Commit {
            if late || check_call().is_err() {
                return Ok(None);
            }
            return Ok(response.ok());
        }
        check_call()?;
        if late {
            return Err(SyncError::Deadline);
        }
        response.map(Some).map_err(|_| SyncError::Unavailable)
    }
}
