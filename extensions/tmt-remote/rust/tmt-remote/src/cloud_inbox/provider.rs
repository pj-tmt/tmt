//! Ordinary-user Firestore calls through the serving project's existing Auth/HTTP/budget owner.
use super::document::{self, Document};
use crate::{
    error::{CloudProviderDiagnostic, CloudProviderOperation, RemoteError},
    firebase_auth::Operation,
    firestore_budget::{Model, READ_LOOKUPS, WRITE_LOOKUPS},
    native_sync::{NativeRuntime, client::RestCall},
    objects::IoBudget,
    sealed_sync::{AuthIdentity, AuthPort, SyncError},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64},
};

pub(crate) struct Page {
    pub documents: Vec<Document>,
    pub read_ms: u64,
    pub full: bool,
}
pub struct Provider {
    auth: Arc<dyn AuthPort>,
    identity: AuthIdentity,
    runtime: Arc<NativeRuntime>,
    model: Model,
    machine: String,
    root: String,
    closed: AtomicBool,
    retry_after_ms: AtomicU64,
}
impl Provider {
    pub fn new(
        auth: Arc<dyn AuthPort>,
        runtime: Arc<NativeRuntime>,
        model: Model,
        machine: &str,
    ) -> Result<Self, RemoteError> {
        crate::canonical::uuid(machine).map_err(|_| invalid())?;
        let identity = auth.identity();
        if !crate::deploy_run::project_ok(&identity.project) || identity.uid.is_empty() {
            return Err(invalid());
        }
        let root = format!(
            "projects/{}/databases/(default)/documents",
            identity.project
        );
        Ok(Self {
            auth,
            identity,
            runtime,
            model,
            machine: machine.into(),
            root,
            closed: AtomicBool::new(false),
            retry_after_ms: AtomicU64::new(0),
        })
    }
    pub(crate) fn now(&self) -> crate::firebase_auth::Time {
        self.runtime.clock.now()
    }
    pub(crate) fn jitter(&self, max: u64) -> u64 {
        self.runtime.jitter.milliseconds(max)
    }
    pub(crate) fn machine(&self) -> &str {
        &self.machine
    }
    pub(crate) fn name(&self, peer: &str, stream: &str, id: &str) -> Result<String, RemoteError> {
        crate::canonical::uuid(peer).map_err(|_| invalid())?;
        crate::canonical::uuid(id).map_err(|_| invalid())?;
        if !["inbox", "responses"].contains(&stream) {
            return Err(invalid());
        }
        Ok(format!("{}/m/{}/{stream}/{id}", self.root, self.machine))
    }
    async fn call(
        &self,
        operation: Operation,
        suffix: &str,
        body: Value,
        cost: u64,
        before_commit: Option<Box<dyn FnOnce() -> Result<bool, SyncError> + Send + '_>>,
        io: &IoBudget<'_>,
    ) -> Result<Option<crate::firebase_auth::Response>, RemoteError> {
        let bytes = if operation == Operation::Read {
            vec![]
        } else {
            serde_json::to_vec(&body).map_err(|_| invalid())?
        };
        let url = format!(
            "https://firestore.googleapis.com/v1/{}{}",
            self.root, suffix
        );
        let response = self
            .runtime
            .send(
                RestCall {
                    identity: &self.identity,
                    auth: self.auth.as_ref(),
                    model: &self.model,
                    closed: &self.closed,
                    retry_after_ms: &self.retry_after_ms,
                    response_limit: crate::limits::CLOUD_PROVIDER_BYTES,
                    before_commit,
                    operation,
                    url,
                    body: bytes,
                    cost,
                },
                io,
            )
            .await
            .map_err(sync_error)?;
        Ok(response)
    }
    pub(crate) async fn get(
        &self,
        peer: &str,
        stream: &str,
        id: &str,
        io: &IoBudget<'_>,
    ) -> Result<Option<Document>, RemoteError> {
        let name = self.name(peer, stream, id)?;
        let suffix = name.strip_prefix(&self.root).ok_or_else(invalid)?;
        let response = self
            .call(
                Operation::Read,
                suffix,
                Value::Null,
                1 + READ_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(unavailable)?;
        if response.status == 404 {
            return Ok(None);
        }
        if response.status != 200 {
            return Err(status(
                CloudProviderOperation::for_stream(Operation::Read, stream),
                &response,
            ));
        }
        let value = crate::wire::strict_json(&response.body).ok_or_else(invalid)?;
        let document = Document::read(&value, &self.root, &self.machine, stream)?;
        if document.peer() != peer {
            return Err(invalid());
        }
        Ok(Some(document))
    }
    /// Caller durably claims the exact bytes before invoking this once. This method never retries.
    /// Every response, including 2xx, is observed by the same ID before it is called confirmed.
    pub(crate) async fn create(
        &self,
        peer: &str,
        stream: &str,
        id: &str,
        bytes: &[u8],
        claim: Box<dyn FnOnce() -> Result<bool, SyncError> + Send + '_>,
        io: &IoBudget<'_>,
    ) -> Result<Option<Document>, RemoteError> {
        let name = self.name(peer, stream, id)?;
        let write = document::create_carrier(&name, peer, bytes)?;
        let answer = self
            .call(
                Operation::Commit,
                ":commit",
                json!({"writes":[write]}),
                WRITE_LOOKUPS,
                Some(claim),
                io,
            )
            .await?;
        if let Some(answer) = answer
            && (400..500).contains(&answer.status)
            && ![409, 429].contains(&answer.status)
        {
            return Err(status(
                CloudProviderOperation::for_stream(Operation::Commit, stream),
                &answer,
            ));
        }
        // A cancelled/deadline call remains unknown. Never turn it into a retry.
        io.check().map_err(|_| unavailable())?;
        // The create may have committed even when its exact-ID observation is refused,
        // cancelled or unreadable. Do not report a read refusal as a refused mutation.
        self.get(peer, stream, id, io)
            .await
            .map_err(|_| unavailable())
    }
    /// One shallow machine inbox query, independent of peer count.
    pub(crate) async fn inbox(
        &self,
        from_ms: u64,
        limit: usize,
        after: Option<&Value>,
        io: &IoBudget<'_>,
    ) -> Result<Page, RemoteError> {
        if limit == 0 || limit > crate::journal::OPERATIONS as usize {
            return Err(invalid());
        }
        let mut value = json!({"structuredQuery":{"from":[{"collectionId":"inbox","allDescendants":false}],"where":{"fieldFilter":{"field":{"fieldPath":"createdAt"},"op":"GREATER_THAN_OR_EQUAL","value":{"timestampValue":stamp(from_ms)}}},"orderBy":[{"field":{"fieldPath":"createdAt"},"direction":"ASCENDING"},{"field":{"fieldPath":"__name__"},"direction":"ASCENDING"}],"limit":limit}});
        if let Some(after) = after {
            value["structuredQuery"]["startAt"] = after.clone();
        }
        let response = self
            .call(
                Operation::Query,
                &format!("/m/{}:runQuery", self.machine),
                value,
                limit as u64 + READ_LOOKUPS,
                None,
                io,
            )
            .await?
            .ok_or_else(unavailable)?;
        if response.status != 200 {
            return Err(status(CloudProviderOperation::InboxQuery, &response));
        }
        let values = crate::wire::strict_json(&response.body)
            .and_then(|v| v.as_array().cloned())
            .ok_or_else(invalid)?;
        if values.len() > limit + 1 {
            return Err(invalid());
        }
        let mut documents = vec![];
        let mut read_ms = None;
        for value in values {
            let read = document::timestamp(value["readTime"].as_str().ok_or_else(invalid)?)?;
            read_ms = Some(read_ms.map_or(read, |old: u64| old.min(read)));
            if let Some(value) = value.get("document") {
                documents.push(Document::read(value, &self.root, &self.machine, "inbox")?);
            }
        }
        let full = documents.len() == limit;
        Ok(Page {
            documents,
            read_ms: read_ms.ok_or_else(invalid)?,
            full,
        })
    }
    pub(crate) async fn delete(
        &self,
        peer: &str,
        stream: &str,
        id: &str,
        io: &IoBudget<'_>,
    ) -> Result<(), RemoteError> {
        let name = self.name(peer, stream, id)?;
        let response = self
            .call(
                Operation::Commit,
                ":commit",
                json!({"writes":[{"delete":name}]}),
                WRITE_LOOKUPS,
                None,
                io,
            )
            .await?;
        match response {
            Some(r) if (200..300).contains(&r.status) => Ok(()),
            _ => Err(unavailable()),
        }
    }
    pub(crate) fn wait(&self, io: &IoBudget<'_>, ms: u64) -> Result<(), RemoteError> {
        io.check().map_err(|_| unavailable())?;
        let now = self.runtime.clock.now();
        let left = io
            .deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis()
            .min(u64::MAX as u128) as u64;
        if !self
            .runtime
            .clock
            .wait_until(now.monotonic_ms.saturating_add(ms.min(left)), io.cancelled)
        {
            return Err(unavailable());
        }
        io.check().map_err(|_| unavailable())
    }
}
fn stamp(ms: u64) -> String {
    format!(
        "{}T{:02}:{:02}:{:02}.{:03}Z",
        crate::calendar::utc_date(ms),
        ms / 3_600_000 % 24,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}
fn sync_error(error: SyncError) -> RemoteError {
    RemoteError::new(error.code(), "Cloud transport refused the request.")
}
impl CloudProviderOperation {
    fn for_stream(operation: Operation, stream: &str) -> Self {
        match (operation, stream) {
            (Operation::Read, "inbox") => Self::InboxRead,
            (Operation::Read, _) => Self::ResponseRead,
            (Operation::Commit, "inbox") => Self::InboxCreate,
            _ => Self::ResponseCreate,
        }
    }
}
fn status(
    operation: CloudProviderOperation,
    response: &crate::firebase_auth::Response,
) -> RemoteError {
    let error = RemoteError::new(
        if response.status == 403 {
            "REMOTE_CLOUD_ACCESS_DENIED"
        } else {
            "REMOTE_CLOUD_PROVIDER_UNAVAILABLE"
        },
        "Cloud transport could not confirm the request.",
    );
    // The ordinary HTTP owner already bounds the body. Admit just the fixed status enum.
    let error_status = crate::wire::strict_json(&response.body)
        .and_then(|value| serde_json::from_value(value["error"]["status"].clone()).ok());
    error.with_cloud_provider(CloudProviderDiagnostic {
        operation,
        status: response.status,
        error_status,
    })
}

fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_DOCUMENT_INVALID",
        "Invalid cloud carrier document.",
    )
}
fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_CLOUD_UNCONFIRMED",
        "The cloud result could not be confirmed; observe the same operation ID before continuing.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        firebase_auth::{AuthError, Cancellation, Clock, Http, Jitter, Request, Response, Time},
        native_sync::NativeBudgetPort,
        sealed_sync::{AuthFailure, AuthToken},
    };
    use std::{
        future::Future,
        pin::pin,
        sync::{Mutex, atomic::Ordering},
        task::{Context, Poll},
        time::{Duration, Instant},
    };
    const NOW: u64 = 1_800_000_000_000;
    struct Auth;
    impl AuthPort for Auth {
        fn identity(&self) -> AuthIdentity {
            AuthIdentity {
                project: "demo-cloud".into(),
                uid: "peer".into(),
            }
        }
        fn token<'a>(
            &'a self,
            _: &'a IoBudget<'a>,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>>
        {
            Box::pin(async {
                Ok(AuthToken {
                    identity: self.identity(),
                    token: "ordinary-user".into(),
                    expires_at_ms: NOW + 3_600_000,
                })
            })
        }
    }
    struct Ports;
    impl Clock for Ports {
        fn now(&self) -> Time {
            Time {
                wall_ms: NOW,
                monotonic_ms: 0,
            }
        }
        fn wait_until(&self, _: u64, _: &dyn Cancellation) -> bool {
            true
        }
    }
    impl Jitter for Ports {
        fn milliseconds(&self, _: u64) -> u64 {
            0
        }
    }
    impl Http for Ports {
        fn send(&self, _: &Request, _: u64, _: &dyn Cancellation) -> Result<Response, AuthError> {
            panic!("no HTTP before budget/claim admission")
        }
    }
    struct Budget {
        refuse: bool,
        charged: Mutex<Vec<u64>>,
    }
    impl NativeBudgetPort for Budget {
        fn usage(&self, _: &str) -> Result<crate::firestore_budget::NativeUsage, SyncError> {
            Ok(crate::firestore_budget::NativeUsage::new(NOW))
        }
        fn charge(
            &self,
            _: &str,
            _: &Model,
            cost: u64,
            _: Time,
        ) -> Result<crate::firestore_budget::Decision, SyncError> {
            self.charged.lock().unwrap().push(cost);
            Ok(if self.refuse {
                crate::firestore_budget::Decision::Refuse(crate::firestore_budget::Refusal {
                    reset_at_ms: NOW + 1_000,
                    retry_after_ms: 1_000,
                })
            } else {
                crate::firestore_budget::Decision::Allow
            })
        }
        fn flush(&self) -> Result<(), SyncError> {
            Ok(())
        }
    }
    #[test]
    fn preflight_refusal_cannot_claim_a_write_and_a_previous_claim_can_only_observe() {
        for refuse in [true, false] {
            let budget = Arc::new(Budget {
                refuse,
                charged: Mutex::new(vec![]),
            });
            let runtime = NativeRuntime::new(
                Arc::new(Ports),
                Arc::new(Ports),
                Arc::new(Ports),
                budget.clone(),
            );
            let auth = Auth;
            let identity = auth.identity();
            let model = Model::new(4, 4).unwrap();
            let closed = AtomicBool::new(false);
            let retry = AtomicU64::new(0);
            let claimed = Arc::new(AtomicBool::new(false));
            let flag = claimed.clone();
            let io = IoBudget {
                deadline: Instant::now() + Duration::from_secs(1),
                cancelled: &closed,
            };
            let future=runtime.send(RestCall{identity:&identity,auth:&auth,model:&model,closed:&closed,retry_after_ms:&retry,response_limit:crate::limits::CLOUD_PROVIDER_BYTES,before_commit:Some(Box::new(move||{flag.store(true,Ordering::SeqCst);Ok(false)})),operation:Operation::Commit,url:"https://firestore.googleapis.com/v1/projects/demo-cloud/databases/(default)/documents:commit".into(),body:b"{}".to_vec(),cost:WRITE_LOOKUPS},&io);
            let mut future = pin!(future);
            let Poll::Ready(result) = future
                .as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
            else {
                panic!("ready injected Auth")
            };
            if refuse {
                assert!(matches!(result, Err(SyncError::BudgetExhausted(_))));
                assert!(!claimed.load(Ordering::SeqCst));
            } else {
                assert!(
                    result.unwrap().is_none(),
                    "already-claimed writes are ambiguous, never repeated"
                );
                assert!(claimed.load(Ordering::SeqCst));
            }
            assert_eq!(*budget.charged.lock().unwrap(), vec![WRITE_LOOKUPS]);
        }
    }
    struct Rejected(&'static str);
    impl Http for Rejected {
        fn send(
            &self,
            request: &Request,
            _: u64,
            _: &dyn Cancellation,
        ) -> Result<Response, AuthError> {
            assert_eq!(request.operation(), Operation::Query);
            Ok(Response {
                status: 400,
                retry_after_ms: None,
                body: self.0.as_bytes().to_vec(),
            })
        }
    }
    #[test]
    fn provider_refusal_retains_only_fixed_operation_http_status_and_known_status() {
        for (body, known) in [
            (
                r#"{"error":{"status":"FAILED_PRECONDITION","message":"SECRET_MARKER","details":["SECRET_MARKER"]}}"#,
                true,
            ),
            (
                r#"{"error":{"status":"SECRET_MARKER","message":"SECRET_MARKER"}}"#,
                false,
            ),
        ] {
            let budget = Arc::new(Budget {
                refuse: false,
                charged: Mutex::new(vec![]),
            });
            let runtime = Arc::new(NativeRuntime::new(
                Arc::new(Rejected(body)),
                Arc::new(Ports),
                Arc::new(Ports),
                budget,
            ));
            let provider = Provider::new(
                Arc::new(Auth),
                runtime,
                Model::new(4, 4).unwrap(),
                "11111111-1111-4111-8111-111111111111",
            )
            .unwrap();
            let cancelled = AtomicBool::new(false);
            let io = IoBudget {
                deadline: Instant::now() + Duration::from_secs(1),
                cancelled: &cancelled,
            };
            let mut future = pin!(provider.inbox(0, 1, None, &io));
            let Poll::Ready(Err(error)) = future
                .as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
            else {
                panic!("ready refusal");
            };
            let value = error.json_value();
            assert_eq!(error.code, "REMOTE_CLOUD_PROVIDER_UNAVAILABLE");
            assert_eq!(value["error"]["provider"]["operation"], "inbox-query");
            assert_eq!(value["error"]["provider"]["status"], 400);
            if known {
                assert_eq!(
                    value["error"]["provider"]["errorStatus"],
                    "FAILED_PRECONDITION"
                );
            } else {
                assert!(value["error"]["provider"].get("errorStatus").is_none());
            }
            assert!(!value.to_string().contains("SECRET_MARKER"));
        }
    }
    #[test]
    fn exact_id_read_refuses_another_peers_carrier_in_the_shared_machine_collection() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        struct Foreign;
        impl Http for Foreign {
            fn send(
                &self,
                request: &Request,
                _: u64,
                _: &dyn Cancellation,
            ) -> Result<Response, AuthError> {
                assert_eq!(request.operation(), Operation::Read);
                assert!(request.url().ends_with("/m/11111111-1111-4111-8111-111111111111/responses/22222222-2222-4222-8222-222222222222"));
                Ok(Response { status:200, retry_after_ms:None, body:serde_json::to_vec(&json!({
                    "name":"projects/demo-cloud/databases/(default)/documents/m/11111111-1111-4111-8111-111111111111/responses/22222222-2222-4222-8222-222222222222",
                    "createTime":"2026-10-23T04:00:00Z",
                    "fields":{"peer":{"stringValue":"33333333-3333-4333-8333-333333333333"},"carrier":{"bytesValue":STANDARD.encode(b"opaque")},"createdAt":{"timestampValue":"2026-10-23T04:00:00Z"}}
                })).unwrap() })
            }
        }
        let runtime = Arc::new(NativeRuntime::new(
            Arc::new(Foreign),
            Arc::new(Ports),
            Arc::new(Ports),
            Arc::new(Budget {
                refuse: false,
                charged: Mutex::new(vec![]),
            }),
        ));
        let provider = Provider::new(
            Arc::new(Auth),
            runtime,
            Model::new(4, 4).unwrap(),
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(1),
            cancelled: &cancel,
        };
        let mut future = pin!(provider.get(
            "44444444-4444-4444-8444-444444444444",
            "responses",
            "22222222-2222-4222-8222-222222222222",
            &io
        ));
        let Poll::Ready(Err(error)) = future
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop()))
        else {
            panic!("foreign peer must refuse")
        };
        assert_eq!(error.code, "REMOTE_CLOUD_DOCUMENT_INVALID");
    }
}
